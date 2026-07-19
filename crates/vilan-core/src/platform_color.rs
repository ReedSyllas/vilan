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
//! - A **module-level binding** is reached by *reference*: its initializer
//!   runs iff something reachable references it (F6 — the same rule emission
//!   uses), so a reference is an edge, and the initializer's calls, created
//!   closures, and references to other bindings are the binding's out-edges.
//!   A `const`-marked initializer runs in the compile-time interpreter, not
//!   on the build platform — it has no edges and seeds nothing.
//!
//! A violation reports the call chain from the entry (backlog §E.8's
//! standard), anchored at the deepest call site in **user** code.
//!
//! [`requirements`] is the same reachability turned into tooling data: an
//! entry-independent per-function map of rendered requirement lines (what the
//! language server shows on hover), computed caller-ward from the seeds so
//! every function gets a shortest witness chain to the layer it requires.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::analyzer::{GenericDispatch, Program};
use crate::call_graph::{CallGraph, CallTarget, IndirectReason};
use crate::error::Error;
use crate::id::Id;
use crate::span::Span;
use crate::target::Platform;
use crate::type_::{Type, TypeId};

/// Checks platform admission for everything reachable from the program's
/// entry (`main`), pushing chain-rendered diagnostics for violations. A
/// program with no user `main` (a library module, a fragment) has no entry
/// and nothing to admit — library boundaries are `check_library_contract`'s
/// job.
///
/// Reachability is **per instantiation** (§3.2): the walk threads each
/// call's recorded type bindings (`method_call_substitution` — the same
/// single channel monomorphization uses), so a trait/generic-bounded call
/// whose receiver is RESOLVED descends only into the member that
/// instantiation actually selects. `save_it(MemStore { .. })` no longer
/// charges `DiskStore`'s impl just because it exists. An unresolvable
/// binding falls back to every candidate — over-approximate but sound.
pub fn check(program: &mut Program, platform: Platform) {
    let graph = CallGraph::build(program);
    // Declared fences check on EVERY compile, entry or not — fencing library
    // code is their point (platform-coloring.md §3.7).
    let mut diagnostics = check_fences(program, &graph);
    if let Some(entry) = entry_function(program) {
        let mut traversal = Traversal::new(program, &graph, Some(platform));
        traversal.walk(entry, &SubstitutionContext::new(), None);
        diagnostics.extend(traversal.diagnostics);
    }
    program.diagnostics.extend(diagnostics);
}

/// The concrete host platforms the checker enumerates for a fence pattern —
/// the supported hosts (manifest layers use the same vocabulary).
fn known_hosts() -> [Platform; 4] {
    [
        Platform::Node {
            version: crate::target::NODE_LTS,
        },
        Platform::Deno {
            version: crate::target::DENO_CURRENT,
        },
        Platform::Bun {
            version: crate::target::BUN_CURRENT,
        },
        Platform::Browser,
    ]
}

/// Checks every `[platform("…")]` fence: for each concrete host matching a
/// declared pattern, everything reachable from the fenced function must admit
/// that host. Runs regardless of the build target and needs no entry —
/// violations land at the fence with the chain, not at some distant entry in
/// a dependent build. A fence on a generic function walks unbound
/// (dispatches consider every candidate): it promises for every possible
/// instantiation.
fn check_fences(program: &Program, graph: &CallGraph) -> Vec<Error> {
    let mut diagnostics = Vec::new();
    for (id, function) in &program.functions {
        if function.platform_fence.is_empty() {
            continue;
        }
        let fence_label = function
            .platform_fence
            .iter()
            .map(|(pattern, _)| format!("\"{pattern}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let mut checked_platforms: Vec<Platform> = Vec::new();
        for (pattern_text, pattern_span) in &function.platform_fence {
            let Some(patterns) = crate::target::PlatformPattern::parse(pattern_text) else {
                diagnostics.push(Error {
                    note: None,
                    span: *pattern_span,
                    msg: format!(
                        "unknown platform pattern `{pattern_text}` in `[platform(…)]` \
                         (expected `node`/`deno`/`bun`/`browser`, or a family like \
                         `@process`)"
                    ),
                });
                continue;
            };
            for pattern in patterns {
                for host in known_hosts() {
                    if host.matches(pattern).is_some() && !checked_platforms.contains(&host) {
                        checked_platforms.push(host);
                    }
                }
            }
        }
        for host in checked_platforms {
            let mut traversal = Traversal::new(program, graph, Some(host));
            traversal.origin = Origin::Fence {
                function: function.name.to_string(),
                fence: fence_label.clone(),
            };
            traversal.walk(*id, &SubstitutionContext::new(), None);
            diagnostics.extend(traversal.diagnostics);
        }
    }
    diagnostics
}

/// What a violation chain hangs from: the build's entry, or a declared fence.
enum Origin {
    Entry,
    Fence { function: String, fence: String },
}

/// The module-level bindings whose initializers run for a program entered at
/// `entry`, under the SAME per-instantiation reachability the admission walk
/// uses — emission and the async-initializer gate consume this, so
/// emitted ⊆ admitted holds by construction even under the refinement.
pub(crate) fn reachable_bindings(program: &Program, graph: &CallGraph, entry: Id) -> HashSet<Id> {
    let mut traversal = Traversal::new(program, graph, None);
    traversal.walk(entry, &SubstitutionContext::new(), None);
    traversal.reached_bindings
}

/// A per-call type binding: the analyzer's constraint id → bound type id.
type SubstitutionContext = HashMap<TypeId, TypeId>;

/// The contextual DFS shared by admission (`platform` set: check + prune +
/// chain diagnostics) and binding reachability (`platform` empty: collect).
struct Traversal<'a, 'src> {
    program: &'a Program<'src>,
    graph: &'a CallGraph,
    platform: Option<Platform>,
    /// Nodes visited PER instantiation — keyed like `emit_instance`, by the
    /// resolved bindings — so the same generic function re-walks under a
    /// different `T` but recursion still terminates.
    visited: HashSet<(Id, Vec<(u32, u32)>)>,
    trail: Vec<(Id, Option<(Span, bool)>)>,
    diagnostics: Vec<Error>,
    module_bindings: HashSet<Id>,
    reached_bindings: HashSet<Id>,
    origin: Origin,
}

impl<'a, 'src> Traversal<'a, 'src> {
    fn new(program: &'a Program<'src>, graph: &'a CallGraph, platform: Option<Platform>) -> Self {
        Traversal {
            program,
            graph,
            platform,
            visited: HashSet::new(),
            trail: Vec::new(),
            diagnostics: Vec::new(),
            module_bindings: program.module_level_bindings().into_iter().collect(),
            reached_bindings: HashSet::new(),
            origin: Origin::Entry,
        }
    }

    fn walk(
        &mut self,
        node: Id,
        substitution: &SubstitutionContext,
        arrived_by: Option<(Span, bool)>,
    ) {
        let mut key: Vec<(u32, u32)> = substitution
            .iter()
            .map(|(constraint, bound)| (constraint.0, self.resolve_type_id(*bound, substitution).0))
            .collect();
        key.sort_unstable();
        if !self.visited.insert((node, key)) {
            return;
        }
        self.trail.push((node, arrived_by));

        if self.module_bindings.contains(&node) {
            self.reached_bindings.insert(node);
        }

        if let Some(platform) = self.platform {
            if let Some(requirement) = requirement_of(self.program, node) {
                let admitted = requirement
                    .patterns
                    .iter()
                    .any(|pattern| platform.matches(*pattern).is_some());
                if !admitted {
                    // Report the BOUNDARY — the first off-platform function
                    // reached from admissible code — and do not descend:
                    // everything beneath it lives in the same layer, and one
                    // chain tells the story.
                    let error = violation(
                        self.program,
                        platform,
                        &self.trail,
                        node,
                        requirement,
                        &self.origin,
                    );
                    self.diagnostics.push(error);
                    self.trail.pop();
                    return;
                }
            }
        }

        for call in self
            .graph
            .calls_of(node)
            .iter()
            .chain(self.graph.initializer_calls_of(node))
        {
            let arrived = self.arrival(call.call_id);
            match call.target {
                CallTarget::Function(callee)
                | CallTarget::Closure(callee)
                | CallTarget::External(callee) => {
                    let next = self.callee_substitution(call.call_id, callee, substitution);
                    self.walk(callee, &next, arrived);
                }
                CallTarget::Variant(_) => {}
                CallTarget::Indirect(IndirectReason::Value) => {
                    // The creator rule: whoever created the closure was
                    // charged for its body; a call through the value adds
                    // nothing.
                }
                CallTarget::Indirect(_) => {
                    // THE refinement: a resolved receiver selects one impl's
                    // member; an unresolved one keeps every candidate.
                    let receiver = self.dispatch_receiver(call.call_id, substitution);
                    let candidates = crate::async_infer::dispatch_candidates_for(
                        self.program,
                        call.call_id,
                        receiver.as_ref(),
                    );
                    for candidate in candidates {
                        let next = self.callee_substitution(call.call_id, candidate, substitution);
                        self.walk(candidate, &next, arrived);
                    }
                }
            }
        }
        // Referencing a module-level binding runs its initializer (F6);
        // initializers are never generic, so they walk context-free.
        for (reference, global) in self.graph.global_references_of(node) {
            let arrived = self.arrival(*reference);
            self.walk(*global, &SubstitutionContext::new(), arrived);
        }
        // A function passed as a value charges at the reference site; with no
        // call record there is no binding to thread.
        for (reference, function) in self.graph.function_references_of(node) {
            let arrived = self.arrival(*reference);
            self.walk(*function, &SubstitutionContext::new(), arrived);
        }
        // Creating a closure charges its body (v1 creator rule); a closure
        // inherits its creator's bindings — its body uses the enclosing `T`s.
        if let Some(children) = self.graph.closure_children_of(node) {
            for closure in children.to_vec() {
                self.walk(closure, &substitution.clone(), None);
            }
        }
        for closure in self.graph.initializer_closures_of(node).to_vec() {
            self.walk(closure, &SubstitutionContext::new(), None);
        }
        // Synthetic destruction edges (destruction.md §8): the transformer inserts
        // the teardown at each scope exit, so this walk can't see the call
        // otherwise. Walking to the resource's `drop` impl(s) here colors the
        // owning scope by a `@process`-needing drop. Context-free (the drop impl's
        // platform requirement is on its own body, not the owner's `T`), no call
        // site — like a created closure.
        if let Some(drop_methods) = self.program.drop_call_edges.get(&node) {
            for drop_method in drop_methods.clone() {
                self.walk(drop_method, &SubstitutionContext::new(), None);
            }
        }

        self.trail.pop();
    }

    fn arrival(&self, site: Id) -> Option<(Span, bool)> {
        let span = self.program.span_map.get(&site).map(|span| **span)?;
        Some((span, is_user_code(self.program, site)))
    }

    /// The bindings a call hands its callee — the transformer's
    /// `call_substitution` channels, mirrored: the call's generic arguments
    /// zipped with the callee's parameters, else the recorded
    /// `method_call_substitution` entry; either way each bound type resolves
    /// under the CALLER's bindings so nested instantiations compose —
    /// exactly `emit_instance`'s rule. With neither channel, a callee that
    /// shares the caller's constraints inherits them (a nested call inside
    /// a generic body).
    fn callee_substitution(
        &self,
        call_id: Id,
        callee: Id,
        incoming: &SubstitutionContext,
    ) -> SubstitutionContext {
        if let Some(function) = self.program.functions.get(&callee) {
            if !function.generic_parameter_constraint_ids.is_empty() {
                if let Some(function_call) = self.program.function_calls.get(&call_id) {
                    if !function_call.generic_argument_ids.is_empty() {
                        return function
                            .generic_parameter_constraint_ids
                            .iter()
                            .copied()
                            .zip(function_call.generic_argument_ids.iter().copied())
                            .map(|(constraint, bound)| {
                                (constraint, self.resolve_type_id(bound, incoming))
                            })
                            .collect();
                    }
                }
            }
        }
        if let Some(recorded) = self.program.method_call_substitution.get(&call_id) {
            return recorded
                .iter()
                .map(|(constraint, bound)| (*constraint, self.resolve_type_id(*bound, incoming)))
                .collect();
        }
        // No record: pass the caller's bindings through — a call inside a
        // generic body resolves the shared constraints; unrelated keys are
        // inert (nothing looks them up).
        incoming.clone()
    }

    /// Follows `Generic` links through the active bindings (bounded, so a
    /// self-referential binding can't loop).
    fn resolve_type_id(&self, type_id: TypeId, substitution: &SubstitutionContext) -> TypeId {
        let mut current = type_id;
        for _ in 0..16 {
            match self.program.type_id_to_type_map.get(&current) {
                Some(Type::Generic(constraint)) => match substitution.get(constraint) {
                    Some(bound) if *bound != current => current = *bound,
                    _ => break,
                },
                _ => break,
            }
        }
        current
    }

    /// The concrete receiver a dispatch resolves to under the bindings, if
    /// the record + the substitution pin one down.
    fn dispatch_receiver(&self, call_id: Id, substitution: &SubstitutionContext) -> Option<Type> {
        let resolved = match crate::async_infer::dispatch_at(self.program, call_id)? {
            GenericDispatch::OnConstraint(constraint_id, _) => {
                self.resolve_type_id(*substitution.get(&constraint_id)?, substitution)
            }
            GenericDispatch::OnType(receiver, _) => self.resolve_type_id(receiver?, substitution),
        };
        match self.program.type_id_to_type_map.get(&resolved) {
            Some(concrete @ (Type::Struct(_, _) | Type::Enum(_, _))) => Some(concrete.clone()),
            _ => None,
        }
    }
}

/// [`CallGraph::successors`] — the shared edge vocabulary — with each site
/// expression resolved to the diagnostic's raw material: its span and whether
/// it lies in user code (`None` for a created closure's body).
fn edges(program: &Program, graph: &CallGraph, node: Id) -> Vec<(Id, Option<(Span, bool)>)> {
    graph
        .successors(program, node)
        .into_iter()
        .map(|(successor, site)| {
            let arrived = site.and_then(|site| {
                let span = program.span_map.get(&site).map(|span| **span)?;
                Some((span, is_user_code(program, site)))
            });
            (successor, arrived)
        })
        .collect()
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

    // The node universe: every code-bearing node, every extern (a leaf that
    // can seed a requirement), and every module-level binding (whose
    // initializer both seeds and propagates), in deterministic build order.
    let mut universe: Vec<Id> = graph.nodes().iter().map(|node| node.id()).collect();
    universe.extend(program.external_functions.keys().copied());
    universe.extend(program.module_level_bindings());

    let mut callers: HashMap<Id, Vec<Id>> = HashMap::new();
    for id in &universe {
        for (callee, _) in edges(program, &graph, *id) {
            callers.entry(callee).or_default().push(*id);
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

/// Whether `id` is a binding whose initializer is `const`-marked: evaluated
/// by the compile-time interpreter and serialized as a value, so at runtime
/// it is data — it runs nothing and requires nothing of the build platform.
fn is_const_global(program: &Program, id: Id) -> bool {
    program
        .variables
        .get(&id)
        .and_then(|variable| variable.initial)
        .is_some_and(|initial| program.const_exprs.contains(&initial))
}

/// The platform requirement seeded by `node`'s definition site: the layer
/// whose root contains its source file, if any. Base-layer and user files
/// (empty-pattern entries or no entry) seed nothing; a `const` binding is
/// compile-time data and seeds nothing wherever it is defined.
fn requirement_of<'program>(program: &'program Program, node: Id) -> Option<Requirement<'program>> {
    if is_const_global(program, node) {
        return None;
    }
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
    origin: &Origin,
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
    let from = match origin {
        Origin::Entry => "reachable from the entry".to_string(),
        Origin::Fence { function, fence } => {
            format!("reachable from `{function}`, fenced `[platform({fence})]`")
        }
    };
    Error {
        note: None,
        span: anchor,
        msg: format!(
            "`{}` requires {} and cannot run on `{}`\n  {}: {}",
            name_of(program, node),
            requirement.label,
            platform.name(),
            from,
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
    if let Some(variable) = program.variables.get(&id) {
        return variable.name.to_string();
    }
    "closure".to_string()
}

/// The program's entry: a function named `main` defined in user code. Also
/// used by async inference's initializer check — "which initializers run"
/// must mean the same thing to admission, emission, and awaiting.
pub(crate) fn entry_function(program: &Program) -> Option<Id> {
    program
        .functions
        .iter()
        .find(|(id, function)| function.name == "main" && is_user_code(program, **id))
        .map(|(id, _)| *id)
}
