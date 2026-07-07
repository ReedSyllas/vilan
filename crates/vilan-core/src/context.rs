//! The `context` threading pass: compiles `std::context::Context` away by
//! threading each context's value as a hidden parameter through every function
//! that transitively reads it, and capturing it into closures that read it.
//!
//! A context is a `Context::new()` value referenced by name (`count_context`).
//! `count_context.run(value, body)` makes `value` the context's value for the
//! dynamic extent of `body`; `count_context.get()` reads it. The pass:
//!
//!   1. Finds every `get`/`run`/`new` site and the context (the receiver
//!      binding) each refers to.
//!   2. Infers, over the [call graph](crate::call_graph), the set of functions
//!      and closures that transitively reach a `get` — these "need" the context
//!      (backward reachability from `get` sites; a closure passed to `run` is a
//!      natural boundary because `run` is an external call, so the need never
//!      propagates past it to the caller).
//!   3. Checks coverage per call: every needs-context node must receive the
//!      value — a function from a needs-context caller, a `run` closure from
//!      `run`, a captured closure from its definition scope. A node that can be
//!      entered without the value (a needs-context `main`, a global initializer,
//!      a needs-context function reachable only indirectly) is a compile error,
//!      not a silent miscompile.
//!   4. Rewrites the IR: appends the hidden parameter to each needs-context
//!      function and `run` closure, threads it at every call, replaces `get()`
//!      with a read of the in-scope parameter, lowers `run(value, body)` to
//!      `body(value)`, and lowers `Context::new()` to an opaque value.
//!
//! The pass is a no-op for programs that never create a `Context`, so it can't
//! change the output of any existing program.

use std::collections::{HashMap, HashSet};

use crate::analyzer::{Expr, Program};
use crate::call_graph::{CallGraph, CallTarget, Node};
use crate::error::Error;
use crate::id::Id;

/// Entry point: thread every context in `program`, or record diagnostics if any
/// context is read where its value can't be supplied.
pub fn thread_contexts(program: &mut Program) {
    let (Some(get_fn), Some(run_fn), Some(new_fn)) = (
        program.context_get_fn_id,
        program.context_run_fn_id,
        program.context_new_fn_id,
    ) else {
        // `context.vl` wasn't loaded — no contexts to thread.
        return;
    };

    let plan = {
        let graph = CallGraph::build(program);
        match analyze(program, &graph, get_fn, run_fn, new_fn) {
            Ok(plan) => plan,
            Err(errors) => {
                program.diagnostics.extend(errors);
                return;
            }
        }
    };

    if !plan.is_empty() {
        apply(program, plan);
    }
}

/// A `get()` call: the call entity, the context it reads, and the function or
/// closure it sits in.
struct GetSite {
    call_id: Id,
    context: Id,
    owner: Node,
}

/// A `run(value, body)` call: the call entity, the context, the value argument,
/// the body-closure argument entity (the call's new subject), and the closure.
struct RunSite {
    call_id: Id,
    value_id: Id,
    closure_entity: Id,
    closure_id: Id,
}

/// The rewrite to apply once analysis succeeds. Node ids are sorted/owned so the
/// plan outlives the borrow of the call graph.
#[derive(Default)]
struct Plan {
    contexts: Vec<Id>,
    /// Nodes (functions and `run` closures) that receive their own hidden
    /// parameter, as `(context, node)`.
    param_nodes: Vec<(Id, Id)>,
    /// Captured closures that read the context from an enclosing node, as
    /// `(context, closure, provider node)` — the closure reuses the provider's
    /// parameter rather than taking its own.
    captures: Vec<(Id, Id, Id)>,
    /// `get()` calls to replace with a read of the in-scope parameter.
    gets: Vec<(Id, Id, Node)>,
    /// Calls to needs-context functions, to thread the value into, as
    /// `(call, context, owner)`.
    thread_calls: Vec<(Id, Id, Node)>,
    /// `run` calls to lower to `body(value)`.
    runs: Vec<RunSite>,
    /// `Context::new()` calls to lower to an opaque value.
    news: Vec<Id>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.gets.is_empty() && self.runs.is_empty() && self.news.is_empty()
    }
}

/// The entity a call's subject resolves to, if it is a direct `Expr::Local`.
fn call_target(program: &Program, call_id: Id) -> Option<Id> {
    let subject_id = program.function_calls.get(&call_id)?.subject_id;
    match program.entity_map.get(&subject_id)? {
        Expr::Local(target) => Some(*target),
        _ => None,
    }
}

/// The binding an entity resolves to, if it is a direct `Expr::Local`.
fn local_target(program: &Program, entity_id: Id) -> Option<Id> {
    match program.entity_map.get(&entity_id)? {
        Expr::Local(target) => Some(*target),
        _ => None,
    }
}

fn span_of(program: &Program, id: Id) -> crate::span::Span {
    program
        .span_map
        .get(&id)
        .map(|span| **span)
        .unwrap_or(crate::span::Span {
            start: 0,
            end: 0,
            context: (),
        })
}

fn context_name<'a>(program: &'a Program, context: Id) -> &'a str {
    program
        .variables
        .get(&context)
        .map(|variable| variable.name)
        .unwrap_or("context")
}

/// Analyzes the program's contexts, producing the rewrite plan or the
/// diagnostics that block it.
fn analyze(
    program: &Program,
    graph: &CallGraph,
    get_fn: Id,
    run_fn: Id,
    new_fn: Id,
) -> Result<Plan, Vec<Error>> {
    let mut errors = Vec::new();

    // call id -> the function/closure it sits in.
    let mut owner_of: HashMap<Id, Node> = HashMap::new();
    for node in graph.nodes() {
        for call in graph.calls_of(node.id()) {
            owner_of.insert(call.call_id, *node);
        }
    }

    // --- Collect get/run/new sites. ---
    let mut gets: Vec<GetSite> = Vec::new();
    let mut runs: Vec<RunSite> = Vec::new();
    let mut news: Vec<Id> = Vec::new();
    let mut contexts: HashSet<Id> = HashSet::new();

    for (&call_id, function_call) in &program.function_calls {
        let Some(target) = call_target(program, call_id) else {
            continue;
        };
        if target == new_fn {
            news.push(call_id);
        } else if target == get_fn {
            // `receiver.get()` — argument 0 is the receiver.
            let receiver = function_call.argument_ids.first().copied();
            let context = receiver.and_then(|receiver| local_target(program, receiver));
            let (Some(context), Some(&owner)) = (context, owner_of.get(&call_id)) else {
                errors.push(Error {
                    span: span_of(program, call_id),
                    msg: "`get` must be called on a context bound to a name".to_string(),
                });
                continue;
            };
            contexts.insert(context);
            gets.push(GetSite {
                call_id,
                context,
                owner,
            });
        } else if target == run_fn {
            // `receiver.run(value, body)` — arguments [receiver, value, body].
            let arguments = &function_call.argument_ids;
            let context = arguments
                .first()
                .copied()
                .and_then(|receiver| local_target(program, receiver));
            let value_id = arguments.get(1).copied();
            let closure_entity = arguments.get(2).copied();
            let closure_id =
                closure_entity.and_then(|entity| match program.entity_map.get(&entity) {
                    Some(Expr::Closure(closure_id)) => Some(*closure_id),
                    _ => None,
                });
            let (Some(context), Some(value_id), Some(closure_entity), Some(closure_id)) =
                (context, value_id, closure_entity, closure_id)
            else {
                errors.push(Error {
                    span: span_of(program, call_id),
                    msg: "`run` must be called on a named context with a closure literal body"
                        .to_string(),
                });
                continue;
            };
            contexts.insert(context);
            runs.push(RunSite {
                call_id,
                value_id,
                closure_entity,
                closure_id,
            });
        }
    }

    if contexts.is_empty() {
        return if errors.is_empty() {
            Ok(Plan::default())
        } else {
            Err(errors)
        };
    }

    // The body closure of every `run`, mapped to the context it binds (the
    // run's receiver). A closure passed to `run` receives the value as a
    // parameter rather than capturing it.
    let mut run_closures: HashMap<Id, Id> = HashMap::new();
    for site in &runs {
        if let Some(context) = program
            .function_calls
            .get(&site.call_id)
            .and_then(|call| call.argument_ids.first().copied())
            .and_then(|receiver| local_target(program, receiver))
        {
            run_closures.insert(site.closure_id, context);
        }
    }

    let mut plan = Plan::default();
    plan.contexts = {
        let mut sorted: Vec<Id> = contexts.iter().copied().collect();
        sorted.sort_by_key(|id| id.0);
        sorted
    };
    plan.news = news;
    plan.runs = runs;

    // --- Entry points the call graph cannot see (for the coverage check's
    // dead-code exemption): a function with NO caller edges is either dead —
    // it cannot run, so it cannot run uncovered — or entered from OUTSIDE the
    // graph, which must stay checked. Outside entries: calls made by
    // top-level statements (the graph has no top-level node), and functions
    // taken as values (called indirectly; the value-use error also fires).
    let owned_call_ids: HashSet<Id> = graph
        .nodes()
        .iter()
        .flat_map(|node| graph.calls_of(node.id()))
        .map(|call| call.call_id)
        .collect();
    let top_level_targets: HashSet<Id> = program
        .function_calls
        .iter()
        .filter(|(call_id, _)| !owned_call_ids.contains(call_id))
        .filter_map(|(_, call)| local_target(program, call.subject_id))
        .collect();
    let call_subject_entities: HashSet<Id> = program
        .function_calls
        .values()
        .map(|call| call.subject_id)
        .collect();
    let value_taken: HashSet<Id> = program
        .entity_map
        .iter()
        .filter_map(|(entity_id, expr)| match expr {
            Expr::Local(target)
                if program.functions.contains_key(target)
                    && !call_subject_entities.contains(entity_id) =>
            {
                Some(*target)
            }
            _ => None,
        })
        .collect();

    // --- Per-context effect inference + coverage. ---
    for &context in &plan.contexts {
        // Seed with the nodes that directly read this context.
        let mut needs: HashSet<Id> = HashSet::new();
        let mut worklist: Vec<Id> = Vec::new();
        for get in gets.iter().filter(|get| get.context == context) {
            if needs.insert(get.owner.id()) {
                worklist.push(get.owner.id());
            }
        }
        // Backward reachability: a caller of a needs-context node needs it too.
        while let Some(id) = worklist.pop() {
            for caller in graph.callers_of(id) {
                if needs.insert(caller.id()) {
                    worklist.push(caller.id());
                }
            }
        }

        let run_closure_ids: HashSet<Id> = run_closures
            .iter()
            .filter(|(_, bound)| **bound == context)
            .map(|(closure, _)| *closure)
            .collect();

        // Classify each needs-context node.
        let is_function = |id: Id| program.functions.contains_key(&id);

        // --- Coverage (greatest fixpoint): assume every node is covered, then
        // remove any that can be entered without the value. A `run` closure
        // always receives the value from `run`, so it is covered even when it
        // doesn't read the context itself (a nested closure may capture it). ---
        let mut bound: HashSet<Id> = needs
            .iter()
            .copied()
            .chain(run_closure_ids.iter().copied())
            .collect();
        loop {
            let mut removed = false;
            for &id in &needs {
                if !bound.contains(&id) || run_closure_ids.contains(&id) {
                    continue;
                }
                let covered = if is_function(id) {
                    let callers = graph.callers_of(id);
                    if callers.is_empty() {
                        // No caller edges: dead code is exempt (it cannot
                        // run); a top-level-called or value-taken function is
                        // entered from outside the graph — uncovered.
                        !top_level_targets.contains(&id) && !value_taken.contains(&id)
                    } else {
                        callers.iter().all(|caller| bound.contains(&caller.id()))
                    }
                } else {
                    // A captured closure is covered iff its defining scope is.
                    graph
                        .closure_parent_of(id)
                        .map(|parent| bound.contains(&parent))
                        .unwrap_or(false)
                };
                if !covered {
                    bound.remove(&id);
                    removed = true;
                }
            }
            if !removed {
                break;
            }
        }

        // Any get whose owner stayed unbound is read outside every `run`.
        for get in gets.iter().filter(|get| get.context == context) {
            if !bound.contains(&get.owner.id()) {
                errors.push(Error {
                    span: span_of(program, get.call_id),
                    msg: format!(
                        "context `{}` is read here, but this code can be reached without an enclosing `run`",
                        context_name(program, context)
                    ),
                });
            }
        }

        // A needs-context function used as a value could be called indirectly,
        // bypassing the threaded parameter — refuse rather than miscompile.
        let needs_functions: HashSet<Id> = needs
            .iter()
            .copied()
            .filter(|&id| is_function(id))
            .collect();
        let call_subjects: HashSet<Id> = program
            .function_calls
            .values()
            .map(|call| call.subject_id)
            .collect();
        for (&entity_id, expr) in &program.entity_map {
            if let Expr::Local(target) = expr {
                if needs_functions.contains(target) && !call_subjects.contains(&entity_id) {
                    errors.push(Error {
                        span: span_of(program, entity_id),
                        msg: format!(
                            "`{}` reads context `{}`, so it can't be used as a value",
                            program
                                .functions
                                .get(target)
                                .map(|function| function.name)
                                .unwrap_or("function"),
                            context_name(program, context)
                        ),
                    });
                }
            }
        }

        if !errors.is_empty() {
            continue;
        }

        // --- Record the rewrite for this context. ---
        // Functions and `run` closures take their own parameter. Every `run`
        // closure does, even one not in `needs`, since `run` always passes it
        // the value (a nested closure may capture it).
        let mut param_nodes: HashSet<Id> = run_closure_ids.clone();
        for &id in &needs {
            if is_function(id) || run_closure_ids.contains(&id) {
                param_nodes.insert(id);
            } else {
                // A captured closure: walk up to the nearest enclosing node that
                // holds the value (a function or `run` closure).
                let mut provider = graph.closure_parent_of(id);
                while let Some(parent) = provider {
                    if is_function(parent) || run_closure_ids.contains(&parent) {
                        plan.captures.push((context, id, parent));
                        break;
                    }
                    provider = graph.closure_parent_of(parent);
                }
            }
        }
        for id in param_nodes {
            plan.param_nodes.push((context, id));
        }

        for get in gets.iter().filter(|get| get.context == context) {
            plan.gets.push((get.call_id, context, get.owner));
        }

        // Thread the value into every call from a needs-context node to a
        // needs-context function.
        for &node_id in &needs {
            let Some(&owner) = graph.nodes().iter().find(|node| node.id() == node_id) else {
                continue;
            };
            for call in graph.calls_of(node_id) {
                if let CallTarget::Function(callee) = call.target {
                    if needs.contains(&callee) {
                        plan.thread_calls.push((call.call_id, context, owner));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(plan)
    } else {
        Err(errors)
    }
}

/// Applies a validated plan, mutating the IR in place.
fn apply(program: &mut Program, plan: Plan) {
    let mut next_id = program.next_entity_id;
    let mut fresh = || {
        let id = Id(next_id);
        next_id += 1;
        id
    };

    // (context, node) -> the parameter id that holds the value inside that node.
    let mut source: HashMap<(Id, Id), Id> = HashMap::new();

    // Give each function and `run` closure its own hidden parameter.
    for &(context, node) in &plan.param_nodes {
        let parameter = fresh();
        program
            .entity_map
            .insert(parameter, Expr::Parameter(parameter));
        if let Some(function) = program.functions.get_mut(&node) {
            function.parameters.push(parameter);
        } else if let Some(closure) = program.closures.get_mut(&node) {
            closure.parameters.push(parameter);
        }
        source.insert((context, node), parameter);
    }

    // A captured closure reuses its provider's parameter.
    for &(context, closure, provider) in &plan.captures {
        if let Some(&parameter) = source.get(&(context, provider)) {
            source.insert((context, closure), parameter);
        }
    }

    // `get()` becomes a read of the in-scope parameter.
    for &(call_id, context, owner) in &plan.gets {
        if let Some(&parameter) = source.get(&(context, owner.id())) {
            program.entity_map.insert(call_id, Expr::Local(parameter));
        }
    }

    // Each call to a needs-context function gets the value appended as an
    // argument, referencing the caller's parameter.
    for &(call_id, context, owner) in &plan.thread_calls {
        if let Some(&parameter) = source.get(&(context, owner.id())) {
            let reference = fresh();
            program.entity_map.insert(reference, Expr::Local(parameter));
            if let Some(call) = program.function_calls.get_mut(&call_id) {
                call.argument_ids.push(reference);
            }
        }
    }

    // `run(value, body)` becomes `body(value)`: the body closure is the new
    // call subject, the value its sole argument (binding the closure's hidden
    // parameter).
    for site in &plan.runs {
        if let Some(call) = program.function_calls.get_mut(&site.call_id) {
            call.subject_id = site.closure_entity;
            call.argument_ids = vec![site.value_id];
        }
    }

    // `Context::new()` lowers to an opaque value; its binding is now unused.
    for &call_id in &plan.news {
        program.entity_map.insert(call_id, Expr::Null);
    }

    program.next_entity_id = next_id;
}
