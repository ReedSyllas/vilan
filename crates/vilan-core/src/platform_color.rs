//! Platform coloring — function-granular platform admission
//! (proposal/platform-coloring.md, phase 1).
//!
//! Replaces import-site gating for application builds: a build may *load* any
//! module of any layer (they already load for typing), but every function
//! **reachable from the entry** must be runnable on the build platform. A
//! function's requirement is seeded by its definition site — an item defined
//! in a library layer's module requires that layer's platforms; base-layer
//! and user code are unconstrained — and the requirement travels by
//! reachability rather than a fixpoint:
//!
//! - Resolved calls descend into the callee.
//! - Trait/generic-bounded dispatch descends into every **candidate** (the
//!   impls' members and the trait default — `async_infer`'s rule; sound
//!   over-approximation, per-instantiation refinement recorded in the
//!   proposal).
//! - A call through a closure *value* descends nowhere: a closure's body was
//!   already charged to the function that **created** it (the v1 creator
//!   rule), which the walk reaches lexically via the closure-parent links.
//! - Known v1 gap (recorded in the proposal): a *direct* call in a
//!   module-level initializer is not a graph node and is not walked; global
//!   closures are.
//!
//! A violation reports the call chain from the entry (backlog §E.8's
//! standard), anchored at the deepest call site in **user** code.
//!
//! [`requirements`] is the same reachability turned into tooling data: an
//! entry-independent per-function map of rendered requirement lines (what the
//! language server shows on hover), computed caller-ward from the seeds so
//! every function gets a shortest witness chain to the layer it requires.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::analyzer::Program;
use crate::call_graph::{CallGraph, CallTarget, IndirectReason, Node};
use crate::error::Error;
use crate::id::Id;
use crate::span::Span;
use crate::target::Platform;

/// Checks platform admission for everything reachable from the program's
/// entry (`main`), pushing chain-rendered diagnostics for violations. A
/// program with no user `main` (a library module, a fragment) has no entry
/// and nothing to admit — library boundaries are `check_library_contract`'s
/// job.
pub fn check(program: &mut Program, platform: Platform) {
    let Some(entry) = entry_function(program) else {
        return;
    };
    let graph = CallGraph::build(program);
    let children = closure_children(&graph);

    // The DFS stack carries the discovery path: (node, the call span that
    // reached it, whether that call site is in user code).
    let mut visited: HashSet<Id> = HashSet::new();
    let mut trail: Vec<(Id, Option<(Span, bool)>)> = Vec::new();
    let mut diagnostics: Vec<Error> = Vec::new();
    walk(
        program,
        platform,
        &graph,
        &children,
        entry,
        None,
        &mut visited,
        &mut trail,
        &mut diagnostics,
    );
    program.diagnostics.extend(diagnostics);
}

#[allow(clippy::too_many_arguments)]
fn walk(
    program: &Program,
    platform: Platform,
    graph: &CallGraph,
    children: &HashMap<Id, Vec<Id>>,
    node: Id,
    arrived_by: Option<(Span, bool)>,
    visited: &mut HashSet<Id>,
    trail: &mut Vec<(Id, Option<(Span, bool)>)>,
    diagnostics: &mut Vec<Error>,
) {
    if !visited.insert(node) {
        return;
    }
    trail.push((node, arrived_by));

    if let Some(requirement) = requirement_of(program, node) {
        let admitted = requirement
            .patterns
            .iter()
            .any(|pattern| platform.matches(*pattern).is_some());
        if !admitted {
            // Report the BOUNDARY — the first off-platform function reached
            // from admissible code — and do not descend: everything beneath
            // it lives in the same layer, and one chain tells the story.
            diagnostics.push(violation(program, platform, trail, node, requirement));
            trail.pop();
            return;
        }
    }

    for (callee, arrived) in edges(program, graph, children, node) {
        walk(
            program,
            platform,
            graph,
            children,
            callee,
            arrived,
            visited,
            trail,
            diagnostics,
        );
    }

    trail.pop();
}

/// Lexical closure children: creating a closure charges its body to the
/// creator (the v1 creator rule).
fn closure_children(graph: &CallGraph) -> HashMap<Id, Vec<Id>> {
    let mut children: HashMap<Id, Vec<Id>> = HashMap::new();
    for node in graph.nodes() {
        if let Node::Closure(closure_id) = node {
            if let Some(parent) = graph.closure_parent_of(*closure_id) {
                children.entry(parent).or_default().push(*closure_id);
            }
        }
    }
    children
}

/// Everything `node` charges, with the call span that reaches each charge
/// (`None` for a created closure's body): resolved callees and externs,
/// every dispatch candidate of a trait/generic-bounded call, and the
/// closures `node` creates. The single edge vocabulary of this pass — the
/// admission walk and the [`requirements`] fixpoint both consume it, so the
/// two can never drift apart.
fn edges(
    program: &Program,
    graph: &CallGraph,
    children: &HashMap<Id, Vec<Id>>,
    node: Id,
) -> Vec<(Id, Option<(Span, bool)>)> {
    let mut edges = Vec::new();
    for call in graph.calls_of(node) {
        let span = program.span_map.get(&call.call_id).map(|span| **span);
        let in_user = span.is_some() && is_user_code(program, call.call_id);
        let arrived = span.map(|span| (span, in_user));
        match call.target {
            CallTarget::Function(callee)
            | CallTarget::Closure(callee)
            | CallTarget::External(callee) => {
                edges.push((callee, arrived));
            }
            CallTarget::Variant(_) => {}
            CallTarget::Indirect(IndirectReason::Value) => {
                // The creator rule: whoever created the closure was charged
                // for its body; a call through the value adds nothing.
            }
            CallTarget::Indirect(_) => {
                for candidate in crate::async_infer::dispatch_candidates(program, call.call_id) {
                    edges.push((candidate, arrived));
                }
            }
        }
    }
    // Creating a closure charges its body (v1 creator rule).
    if let Some(created) = children.get(&node) {
        for closure in created {
            edges.push((*closure, None));
        }
    }
    edges
}

/// Per-function platform requirements, rendered for tooling: every function,
/// closure, or extern that (transitively) requires a layer maps to a line
/// like
///
/// ```text
/// requires the `process` layer of `std` (via `load (server::store) → exists (std::fs)`)
/// ```
///
/// Unlike [`check`] this is **entry-independent** — a library function nobody
/// calls yet still knows its color, which is exactly what an editor hover
/// wants. Requirements propagate caller-ward from the definition-site seeds
/// (one multi-source BFS per layer label over the same [`edges`] the
/// admission walk uses), and each reached node records the callee it acquired
/// the label through, so following those witnesses callee-ward yields a
/// *shortest* via-chain down to the layer. A seeded node's own line carries
/// no chain. Multiple layers render one line each, in label order.
pub fn requirements(program: &Program) -> HashMap<Id, String> {
    let graph = CallGraph::build(program);
    let children = closure_children(&graph);

    // The node universe: every code-bearing node plus every extern (a leaf
    // that can seed a requirement), in deterministic build order.
    let mut universe: Vec<Id> = graph.nodes().iter().map(|node| node.id()).collect();
    universe.extend(program.external_functions.keys().copied());

    let mut callers: HashMap<Id, Vec<Id>> = HashMap::new();
    for node in graph.nodes() {
        for (callee, _) in edges(program, &graph, &children, node.id()) {
            callers.entry(callee).or_default().push(node.id());
        }
    }

    let mut seeds: BTreeMap<&str, Vec<Id>> = BTreeMap::new();
    for id in &universe {
        if let Some(requirement) = requirement_of(program, *id) {
            seeds.entry(requirement.label).or_default().push(*id);
        }
    }

    let mut lines: HashMap<Id, Vec<String>> = HashMap::new();
    for (label, sources) in &seeds {
        // node → the callee it acquired this label from (`None` = seeded).
        let mut witness: HashMap<Id, Option<Id>> = HashMap::new();
        let mut queue: VecDeque<Id> = VecDeque::new();
        for source in sources {
            witness.insert(*source, None);
            queue.push_back(*source);
        }
        while let Some(node) = queue.pop_front() {
            let Some(callers_of_node) = callers.get(&node) else {
                continue;
            };
            for caller in callers_of_node {
                if !witness.contains_key(caller) {
                    witness.insert(*caller, Some(node));
                    queue.push_back(*caller);
                }
            }
        }
        for id in &universe {
            let Some(acquired_through) = witness.get(id) else {
                continue;
            };
            let mut chain = Vec::new();
            let mut cursor = *acquired_through;
            while let Some(next) = cursor {
                chain.push(frame_label(program, next));
                cursor = witness.get(&next).copied().flatten();
            }
            let line = if chain.is_empty() {
                format!("requires {label}")
            } else {
                format!("requires {label} (via `{}`)", chain.join(" → "))
            };
            lines.entry(*id).or_default().push(line);
        }
    }
    lines
        .into_iter()
        .map(|(id, lines)| (id, lines.join("\n")))
        .collect()
}

struct Requirement<'program> {
    label: &'program str,
    patterns: &'program [crate::target::PlatformPattern],
}

/// The platform requirement seeded by `node`'s definition site: the layer
/// whose root contains its source file, if any. Base-layer and user files
/// (empty-pattern entries or no entry) seed nothing.
fn requirement_of<'program>(program: &'program Program, node: Id) -> Option<Requirement<'program>> {
    let source = program.source_of(node)?;
    let path = program.sources.get(source.0 as usize)?;
    for (root, _library, label, patterns) in &program.layer_platforms {
        if !patterns.is_empty() && path.starts_with(root) {
            return Some(Requirement { label, patterns });
        }
    }
    None
}

/// A frame's display name: bare for user code, `name (lib::module)` for
/// library code — the chain then reads `main → boot (server::store) →
/// exists (std::fs)`.
fn frame_label(program: &Program, id: Id) -> String {
    let name = name_of(program, id);
    if is_user_code(program, id) {
        return name;
    }
    let module = program
        .source_of(id)
        .and_then(|source| program.sources.get(source.0 as usize))
        .and_then(|path| {
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let library = program
                .layer_platforms
                .iter()
                .find(|(root, _, _, _)| path.starts_with(root))
                .map(|(_, library, _, _)| library.clone())?;
            Some(if stem == "lib" {
                library
            } else {
                format!("{library}::{stem}")
            })
        });
    match module {
        Some(module) => format!("{name} ({module})"),
        None => name,
    }
}

/// Whether the entity's file is the user's own code — not under any recorded
/// library root (layers or bases).
fn is_user_code(program: &Program, id: Id) -> bool {
    let Some(source) = program.source_of(id) else {
        return false;
    };
    let Some(path) = program.sources.get(source.0 as usize) else {
        return false;
    };
    !program
        .layer_platforms
        .iter()
        .any(|(root, _, _, _)| path.starts_with(root))
}

fn violation(
    program: &Program,
    platform: Platform,
    trail: &[(Id, Option<(Span, bool)>)],
    node: Id,
    requirement: Requirement,
) -> Error {
    let chain = trail
        .iter()
        .map(|(id, _)| frame_label(program, *id))
        .collect::<Vec<_>>()
        .join(" → ");
    // Anchor at the deepest user-code call site on the path; a violation with
    // no user frame at all (unlikely) falls back to the entry's span.
    let anchor = trail
        .iter()
        .rev()
        .find_map(|(_, arrived)| arrived.and_then(|(span, user)| user.then_some(span)))
        .or_else(|| program.span_map.get(&node).map(|span| **span))
        .unwrap_or(Span {
            start: 0,
            end: 0,
            context: (),
        });
    Error {
        span: anchor,
        msg: format!(
            "`{}` requires {} and cannot run on `{}`\n  reachable from the entry: {}",
            name_of(program, node),
            requirement.label,
            platform.name(),
            chain
        ),
    }
}

fn name_of(program: &Program, id: Id) -> String {
    if let Some(function) = program.functions.get(&id) {
        return function.name.to_string();
    }
    if let Some(external) = program.external_functions.get(&id) {
        return external.name.to_string();
    }
    "closure".to_string()
}

/// The program's entry: a function named `main` defined in user code.
fn entry_function(program: &Program) -> Option<Id> {
    program
        .functions
        .iter()
        .find(|(id, function)| function.name == "main" && is_user_code(program, **id))
        .map(|(id, _)| *id)
}
