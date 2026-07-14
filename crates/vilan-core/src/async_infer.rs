//! Async inference: decides which functions and closures are async — i.e. which
//! must compile to a JS `async function`/`async () =>`, and which calls must be
//! implicitly `await`ed.
//!
//! Like the `context` pass, this is an effect over the [call graph](crate::call_graph):
//! the leaves are async externs and explicit `await`s, and the effect propagates
//! callee → caller (a function that calls an async function implicitly awaits it,
//! so it becomes async too). An `async { .. }` block is a separate, always-async
//! closure node with no call edge into it, so it is a natural boundary: the
//! awaits inside a spawned promise don't make the enclosing function async.
//!
//! A trait/generic-bounded method call (`(self.inner).fetch()` where `self.inner:
//! T`, `T: Fetcher`) can't be pinned to a single callee at the graph's
//! pre-monomorphization granularity, so the call graph records it as
//! [`CallTarget::Indirect`](crate::call_graph::CallTarget). But Vilan has no `dyn`:
//! every such call resolves to a statically-known impl at each monomorphization.
//! So the effect still propagates — through the *contract*. A dispatched method is
//! treated as async if **any** candidate (an impl's member, or the trait's default)
//! is async, because then some monomorphization awaits it and the caller must be
//! `async`. Over-marking a purely-sync instance is harmless: the transformer awaits
//! a non-promise, which is a JS no-op. Without this the transformer (which resolves
//! the concrete async callee post-monomorphization) would emit an `await` inside a
//! function this pass left non-`async` — invalid JavaScript.
//!
//! The result is `Program::async_functions`, read by the transformer.

use std::collections::HashSet;

use crate::analyzer::{Expr, GenericDispatch, Program};
use crate::call_graph::{CallGraph, CallTarget, IndirectReason};
use crate::id::Id;
use crate::type_::{Type, TypeId};

/// Computes the async set and stores it on the program.
pub fn infer(program: &mut Program) {
    let graph = CallGraph::build(program);
    let mut async_set: HashSet<Id> = HashSet::new();

    // --- Seeds ---
    // Declared-async functions and externs (an extern is async only by
    // declaration, having no body to infer from).
    for (id, function) in &program.functions {
        if function.is_async {
            async_set.insert(*id);
        }
    }
    for (id, external) in &program.external_functions {
        if external.is_async {
            async_set.insert(*id);
        }
    }
    // A node whose own body awaits.
    for node in graph.nodes() {
        if graph.node_awaits(node.id()) {
            async_set.insert(node.id());
        }
    }
    // An `async { .. }` block lowers to an always-async closure.
    for expr in program.entity_map.values() {
        if let Expr::Async(closure_id) = expr {
            async_set.insert(*closure_id);
        }
    }

    // --- Fixpoint: a node that calls an async function/extern implicitly awaits
    // it, so it is async too. A trait/generic-bounded call propagates through its
    // candidate impls (see the module doc). ---
    loop {
        let mut changed = false;
        for node in graph.nodes() {
            let id = node.id();
            if async_set.contains(&id) {
                continue;
            }
            let calls_async = graph.calls_of(id).iter().any(|call| match call.target {
                CallTarget::Function(callee) | CallTarget::External(callee) => {
                    async_set.contains(&callee)
                }
                // A trait/generic-bounded dispatch: async if any candidate impl is.
                CallTarget::Indirect(
                    IndirectReason::GenericMember | IndirectReason::TraitDispatch,
                ) => dispatch_candidates(program, call.call_id)
                    .iter()
                    .any(|member| async_set.contains(member)),
                // A call through an `async || T`-typed value IS an await
                // point — asyncness rides the type (J2). Other higher-order
                // calls stay conservative (the concrete target isn't
                // recoverable), as do variant constructors and
                // immediately-applied closures.
                _ => {
                    let subject_target =
                        program
                            .function_calls
                            .get(&call.call_id)
                            .and_then(|function_call| {
                                match program.entity_map.get(&function_call.subject_id) {
                                    Some(Expr::Local(target)) => Some(*target),
                                    _ => None,
                                }
                            });
                    subject_target.is_some_and(|target| program.async_values.contains(&target))
                }
            });
            if calls_async {
                async_set.insert(id);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // --- The J2 divergence check: an async closure flowing into a PLAIN
    // closure parameter with a non-void return would hand its caller a
    // promise typed as `T`. Void-returning parameters stay legal — that is
    // spawn semantics (fire-and-forget; the turns machinery settles the
    // continuations), and no value is lied about.
    let mut divergences: Vec<(crate::span::Span, String)> = Vec::new();
    for function_call in program.function_calls.values() {
        let Some(Expr::Local(target)) = program.entity_map.get(&function_call.subject_id) else {
            continue;
        };
        let Some(function) = program.functions.get(target) else {
            continue;
        };
        for (argument, parameter) in function_call.argument_ids.iter().zip(&function.parameters) {
            if program.async_values.contains(parameter) {
                continue;
            }
            let Some(parameter_record) = program.parameters.get(parameter) else {
                continue;
            };
            let Some(Type::Closure(_, return_type)) = program
                .type_id_to_type_map
                .get(&parameter_record.type_id)
                .cloned()
            else {
                continue;
            };
            let returns_void = matches!(
                program.type_id_to_type_map.get(&return_type),
                Some(Type::Void) | None
            );
            if returns_void {
                continue;
            }
            // The argument's closure: a literal, or a binding holding one.
            let argument_closure = match program.entity_map.get(argument) {
                Some(Expr::Closure(closure_id)) | Some(Expr::Async(closure_id)) => {
                    Some(*closure_id)
                }
                Some(Expr::Local(source)) => program
                    .variables
                    .get(source)
                    .and_then(|variable| variable.initial)
                    .and_then(|initial| match program.entity_map.get(&initial) {
                        Some(Expr::Closure(closure_id)) | Some(Expr::Async(closure_id)) => {
                            Some(*closure_id)
                        }
                        _ => None,
                    }),
                _ => None,
            };
            let Some(closure_id) = argument_closure else {
                continue;
            };
            if !async_set.contains(&closure_id) {
                continue;
            }
            let span = program
                .span_map
                .get(parameter)
                .map(|span| **span)
                .unwrap_or(crate::span::Span {
                    start: 0,
                    end: 0,
                    context: (),
                });
            divergences.push((
                span,
                format!(
                    "`{}` receives an async closure, but its type awaits nothing — declare it `async || T` (or return void for spawn semantics)",
                    parameter_record.name
                ),
            ));
        }
    }
    for (span, msg) in divergences {
        program.diagnostics.push(crate::error::Error { span, msg });
    }

    // --- Module-level initializers cannot await (backlog §J.3): they run at
    // module load, where there is no enclosing function to become async and
    // no top-level await in the emission model. A call to an
    // (inferred-)async function here would leave a live promise typed as
    // `T` — `state + 1` on it is garbage — so it is refused cleanly.
    // Creating an async closure (or an `async { .. }` block) in an
    // initializer stays legal: nothing awaits at load. `const` initializers
    // never reach here (they are compile-time; the graph skips them).
    // F6 gates the check exactly as it gates emission and coloring: a
    // binding the entry never reaches never runs, so it cannot await —
    // with no user `main` (a library, a fragment) every binding is checked,
    // since each runs in some dependent program.
    let running_bindings = crate::platform_color::entry_function(program)
        .map(|entry| crate::platform_color::reachable_bindings(program, &graph, entry));
    let mut initializer_awaits: Vec<(crate::span::Span, String)> = Vec::new();
    for binding in program.module_level_bindings() {
        if running_bindings
            .as_ref()
            .is_some_and(|running| !running.contains(&binding))
        {
            continue;
        }
        for call in graph.initializer_calls_of(binding) {
            let async_target = match call.target {
                CallTarget::Function(callee) | CallTarget::External(callee) => {
                    async_set.contains(&callee).then_some(callee)
                }
                CallTarget::Indirect(
                    IndirectReason::GenericMember | IndirectReason::TraitDispatch,
                ) => dispatch_candidates(program, call.call_id)
                    .into_iter()
                    .find(|member| async_set.contains(member)),
                // A call through an `async || T`-typed value awaits too.
                _ => program
                    .function_calls
                    .get(&call.call_id)
                    .and_then(|function_call| {
                        match program.entity_map.get(&function_call.subject_id) {
                            Some(Expr::Local(target)) => Some(*target),
                            _ => None,
                        }
                    })
                    .filter(|target| program.async_values.contains(target)),
            };
            let Some(target) = async_target else {
                continue;
            };
            let target_name = program
                .functions
                .get(&target)
                .map(|function| function.name)
                .or_else(|| {
                    program
                        .external_functions
                        .get(&target)
                        .map(|external| external.name)
                })
                .unwrap_or("an async value");
            let binding_name = program
                .variables
                .get(&binding)
                .map(|variable| variable.name)
                .unwrap_or("_");
            let span = program
                .span_map
                .get(&call.call_id)
                .map(|span| **span)
                .unwrap_or(crate::span::Span {
                    start: 0,
                    end: 0,
                    context: (),
                });
            initializer_awaits.push((
                span,
                format!(
                    "the initializer of `{binding_name}` calls `{target_name}`, which is \
                     async — a module-level binding cannot await (module initialization \
                     is synchronous); wrap the work in a function and call it from `main`"
                ),
            ));
        }
    }
    for (span, msg) in initializer_awaits {
        program.diagnostics.push(crate::error::Error { span, msg });
    }

    program.async_functions = async_set;
}

/// The concrete member ids a trait/generic-bounded dispatch at `call_id` could
/// resolve to across monomorphizations: an impl's member for the method, or the
/// trait's own default. The async fixpoint marks the caller async if any is.
pub(crate) fn dispatch_candidates(program: &Program, call_id: Id) -> Vec<Id> {
    let Some(dispatch) = dispatch_at(program, call_id) else {
        return Vec::new();
    };
    match dispatch {
        GenericDispatch::OnConstraint(constraint_id, member) => {
            // Single-bound `T: Trait` resolves precisely to that trait's impls.
            let precise = trait_of(program, constraint_id)
                .map(|trait_id| trait_method_candidates(program, trait_id, member))
                .unwrap_or_default();
            // A multi-bound parameter records only its first bound, so a member
            // from another bound finds nothing precise — fall back to every
            // same-named member (over-approximate but sound).
            if precise.is_empty() {
                members_named(program, member)
            } else {
                precise
            }
        }
        // A trait-default re-dispatch doesn't carry its trait on the record, so
        // consider every same-named member.
        GenericDispatch::OnType(_, member) => members_named(program, member),
    }
}

/// Like [`dispatch_candidates`], but with a resolved concrete RECEIVER type
/// when a per-instantiation walk has one (platform coloring's refinement):
/// only the members that type actually selects — its own impl's declared
/// member, else the trait defaults its impls inherit. Nominal matching
/// (conditional impls over the same head both match) keeps it a sound
/// over-approximation. A `None` receiver is the plain over-approximation.
pub(crate) fn dispatch_candidates_for(
    program: &Program,
    call_id: Id,
    receiver: Option<&Type>,
) -> Vec<Id> {
    let Some(receiver) = receiver else {
        return dispatch_candidates(program, call_id);
    };
    let receiver_head = match receiver {
        Type::Struct(id, _) | Type::Enum(id, _) => *id,
        _ => return dispatch_candidates(program, call_id),
    };
    let Some(dispatch) = dispatch_at(program, call_id) else {
        return Vec::new();
    };
    let member = match dispatch {
        GenericDispatch::OnConstraint(_, member) | GenericDispatch::OnType(_, member) => member,
    };
    let matching: Vec<&crate::analyzer::Implementation> = program
        .implementations
        .iter()
        .filter(|implementation| {
            matches!(
                program.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) | Some(Type::Enum(id, _)) if *id == receiver_head
            )
        })
        .collect();
    // The receiver's own impls: a declared member wins outright.
    let declared: Vec<Id> = matching
        .iter()
        .filter_map(|implementation| implementation.declarations.get(member).copied())
        .collect();
    if !declared.is_empty() {
        return declared;
    }
    // Else the trait defaults those impls inherit.
    let defaults: Vec<Id> = matching
        .iter()
        .flat_map(|implementation| implementation.trait_ids.iter())
        .filter_map(|trait_id| {
            program
                .traits
                .get(trait_id)
                .and_then(|trait_| trait_.declarations.get(member).copied())
        })
        .collect();
    if !defaults.is_empty() {
        return defaults;
    }
    // Nothing nominal matched (an unexpected receiver shape): stay sound.
    dispatch_candidates(program, call_id)
}

/// The `GenericDispatch` recorded for a call site — keyed by the call id (an
/// `OnType` re-dispatch) or by its subject (an `OnConstraint` bounded call),
/// mirroring how the call graph and transformer read it.
pub(crate) fn dispatch_at<'src>(
    program: &Program<'src>,
    call_id: Id,
) -> Option<GenericDispatch<'src>> {
    if let Some(dispatch) = program.generic_dispatch.get(&call_id) {
        return Some(*dispatch);
    }
    let subject_id = program.function_calls.get(&call_id)?.subject_id;
    program.generic_dispatch.get(&subject_id).copied()
}

/// The trait a single bound's constraint id denotes, if it resolves to one.
fn trait_of(program: &Program, constraint_id: TypeId) -> Option<Id> {
    match program.type_id_to_type_map.get(&constraint_id) {
        Some(Type::Trait(trait_id, _)) => Some(*trait_id),
        _ => None,
    }
}

/// Every member named `member` an impl of `trait_id` provides, plus the trait's
/// own default for it — the candidates a dispatch bounded by that trait selects
/// among at monomorphization.
fn trait_method_candidates(program: &Program, trait_id: Id, member: &str) -> Vec<Id> {
    let mut candidates = Vec::new();
    if let Some(trait_) = program.traits.get(&trait_id)
        && let Some(default_id) = trait_.declarations.get(member)
    {
        candidates.push(*default_id);
    }
    for implementation in &program.implementations {
        if implementation.trait_ids.contains(&trait_id)
            && let Some(member_id) = implementation.declarations.get(member)
        {
            candidates.push(*member_id);
        }
    }
    candidates
}

/// Every impl member and trait default with the given name — the fallback when a
/// dispatch's trait can't be pinned (a multi-bound parameter, or an `OnType`
/// re-dispatch). Over-approximate but sound: it can only add async-ness.
fn members_named(program: &Program, member: &str) -> Vec<Id> {
    let mut candidates = Vec::new();
    for implementation in &program.implementations {
        if let Some(member_id) = implementation.declarations.get(member) {
            candidates.push(*member_id);
        }
    }
    for trait_ in program.traits.values() {
        if let Some(member_id) = trait_.declarations.get(member) {
            candidates.push(*member_id);
        }
    }
    candidates
}
