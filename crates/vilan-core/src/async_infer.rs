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
                // A call through a function/closure *value* (higher-order) stays
                // conservative — the concrete target isn't recoverable here — as
                // do variant constructors and immediately-applied closures.
                _ => false,
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

    program.async_functions = async_set;
}

/// The concrete member ids a trait/generic-bounded dispatch at `call_id` could
/// resolve to across monomorphizations: an impl's member for the method, or the
/// trait's own default. The async fixpoint marks the caller async if any is.
fn dispatch_candidates(program: &Program, call_id: Id) -> Vec<Id> {
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

/// The `GenericDispatch` recorded for a call site — keyed by the call id (an
/// `OnType` re-dispatch) or by its subject (an `OnConstraint` bounded call),
/// mirroring how the call graph and transformer read it.
fn dispatch_at<'src>(program: &Program<'src>, call_id: Id) -> Option<GenericDispatch<'src>> {
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
