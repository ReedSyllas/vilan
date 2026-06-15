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
//! The result is `Program::async_functions`, read by the transformer.

use std::collections::HashSet;

use crate::analyzer::{Expr, Program};
use crate::call_graph::{CallGraph, CallTarget};
use crate::id::Id;

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
    // it, so it is async too. ---
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
