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
use crate::call_graph::{CallGraph, CallTarget, IndirectReason, Node};
use crate::error::Error;
use crate::id::Id;
use crate::type_::Type;

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
    // Absent only against an older `context.vl` without `get_safe`.
    let get_safe_fn = program.context_get_safe_fn_id;

    let plan = {
        let graph = CallGraph::build(program);
        match analyze(program, &graph, get_fn, get_safe_fn, run_fn, new_fn) {
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

/// A `get()`/`get_safe()` call: the call entity, the context it reads, the
/// function or closure it sits in, and the read's FLAVOR — a strict `get`
/// demands the bare value (and the coverage fence); a `safe` read receives
/// `Option<T>` and never fences (reactive-turns.md §5.1).
struct GetSite {
    call_id: Id,
    context: Id,
    owner: Node,
    safe: bool,
}

/// A `run(value, body)` call: the call entity, the context, the value argument,
/// the body argument entity (the call's new subject), and — for a closure
/// LITERAL body — the closure. `None` when the body is an injected
/// `context`-typed closure VALUE (proposal/ambient-owner.md §5), which
/// carries its own hidden parameter and needs no capture marking.
struct RunSite {
    call_id: Id,
    value_id: Id,
    closure_entity: Id,
    closure_id: Option<Id>,
}

/// How a threaded call site obtains one context's argument: the caller's
/// own parameter (bare or already-`Option`), that parameter `Some`-wrapped
/// (the covered→safe boundary), or a literal `None` (an entry point with no
/// value — a top-level call, or the inlined entry `main`).
enum ThreadForm {
    Param { owner: Node },
    WrapSome { owner: Node },
    NoneLiteral,
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
    /// `get()`/`get_safe()` calls to replace with a read of the in-scope
    /// parameter; `wrap_some` marks a safe read inside a STRICT holder, whose
    /// bare value must be `Some`-wrapped.
    gets: Vec<(Id, Id, Node, bool)>,
    /// Calls to needs-context functions, to thread one context's argument
    /// into, as `(call, context, form)`. ONE channel for every append, built
    /// context-by-context, so a call site's arguments accumulate in
    /// `contexts` order — matching the callee's parameter order — whatever
    /// mix of forms it needs.
    thread_calls: Vec<(Id, Id, ThreadForm)>,
    /// Safe reads INSIDE the entry `main`, which the transformer inlines at
    /// top level (it can carry no hidden parameter): each becomes a literal
    /// `None`.
    none_gets: Vec<Id>,
    /// The `Option::Some` / `Option::None` variant entities, for synthesizing
    /// wraps; resolved once when any safe site exists.
    some_variant: Option<Id>,
    none_variant: Option<Id>,
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
    get_safe_fn: Option<Id>,
    run_fn: Id,
    new_fn: Id,
) -> Result<Plan, Vec<Error>> {
    let mut errors = Vec::new();

    // The entry `main` is special: the transformer inlines its body as the
    // program's top-level statements, so it can never carry a hidden
    // parameter — and semantically it IS the uncovered root. Its safe reads
    // become literal `None`s; a STRICT-needy main fences like any
    // top-level-called function.
    let entry_main: Option<Id> = program
        .scopes
        .get(&program.global_scope_id)
        .and_then(|scope| scope.name_to_id_map.get("main"))
        .copied()
        .filter(|id| program.functions.contains_key(id));

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
        } else if target == get_fn || Some(target) == get_safe_fn {
            let safe = Some(target) == get_safe_fn;
            // `receiver.get()` / `receiver.get_safe()` — argument 0 is the
            // receiver.
            let receiver = function_call.argument_ids.first().copied();
            let context = receiver.and_then(|receiver| local_target(program, receiver));
            let (Some(context), Some(&owner)) = (context, owner_of.get(&call_id)) else {
                let method = if safe { "get_safe" } else { "get" };
                errors.push(Error {
                    note: None,
                    span: span_of(program, call_id),
                    msg: format!("`{method}` must be called on a context bound to a name"),
                });
                continue;
            };
            contexts.insert(context);
            gets.push(GetSite {
                call_id,
                context,
                owner,
                safe,
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
            // An injected `context`-typed closure VALUE is a legal body when
            // its clause is exactly this context (the deferred argument is
            // what `run` supplies) — proposal/ambient-owner.md §5.
            let injected_body = closure_entity
                .and_then(|entity| match program.entity_map.get(&entity) {
                    Some(Expr::Local(target)) => program.parameter_contexts.get(target),
                    _ => None,
                })
                .is_some_and(|clause| context.is_some_and(|context| clause == &vec![context]));
            let (Some(context), Some(value_id), Some(closure_entity)) =
                (context, value_id, closure_entity)
            else {
                errors.push(Error {
                    note: None,
                    span: span_of(program, call_id),
                    msg: "`run` must be called on a named context with a closure literal body"
                        .to_string(),
                });
                continue;
            };
            if closure_id.is_none() && !injected_body {
                errors.push(Error { note: None,
                    span: span_of(program, call_id),
                    msg: "`run` must be called on a named context with a closure literal body, or a closure value whose type is `context`-annotated with exactly this context"
                        .to_string(),
                });
                continue;
            }
            contexts.insert(context);
            runs.push(RunSite {
                call_id,
                value_id,
                closure_entity,
                closure_id,
            });
        }
    }

    if contexts.is_empty() && program.parameter_contexts.is_empty() {
        return if errors.is_empty() {
            // No reads, no runs — but `Context::new()` calls still lower to
            // their opaque value (previously they emitted a dangling call).
            let mut plan = Plan::default();
            plan.news = news;
            Ok(plan)
        } else {
            Err(errors)
        };
    }

    // The body closure of every `run`, mapped to the context it binds (the
    // run's receiver). A closure passed to `run` receives the value as a
    // parameter rather than capturing it.
    let mut run_closures: HashMap<Id, Id> = HashMap::new();
    for site in &runs {
        if let (Some(closure_id), Some(context)) = (
            site.closure_id,
            program
                .function_calls
                .get(&site.call_id)
                .and_then(|call| call.argument_ids.first().copied())
                .and_then(|receiver| local_target(program, receiver)),
        ) {
            run_closures.insert(closure_id, context);
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

    // --- Dispatch edges the shared graph deliberately leaves indirect
    // (backlog B14): a call the analyzer routed through trait dispatch — a
    // trait method on a concrete receiver (`OnType`, which may land on the
    // trait's DEFAULT body) or a generic-bounded member (`OnConstraint`) —
    // has no `CallTarget::Function` edge, so the trait default's gets looked
    // unreachable and its callers uncovered. The graph stays untouched (it is
    // also async inference's graph; conservative edges there would
    // over-propagate async-ness); the context analysis adds the edges
    // LOCALLY: for each dispatch site, every candidate callee — the named
    // member's trait default plus every implementation's override, across the
    // traits declaring that name. Over-approximation is sound here (an extra
    // caller edge only strengthens the coverage demand); the same sites join
    // the threading plan, and a callee that turns out not to need the value
    // simply ignores the extra argument.
    let dispatch_member_name = |call_id: Id| -> Option<&str> {
        let subject_id = program.function_calls.get(&call_id)?.subject_id;
        for key in [call_id, subject_id] {
            match program.generic_dispatch.get(&key) {
                Some(crate::analyzer::GenericDispatch::OnConstraint(_, name))
                | Some(crate::analyzer::GenericDispatch::OnType(_, name)) => return Some(name),
                None => {}
            }
        }
        None
    };
    let dispatch_candidates = |name: &str| -> Vec<Id> {
        let mut candidates = Vec::new();
        for trait_ in program.traits.values() {
            let Some(&declaration_id) = trait_.declarations.get(name) else {
                continue;
            };
            // The trait's own default body, when it has one.
            if program
                .functions
                .get(&declaration_id)
                .is_some_and(|function| function.has_body)
            {
                candidates.push(declaration_id);
            }
            // Every implementation's override of this trait's member.
            for implementation in &program.implementations {
                if implementation.trait_ids.contains(&trait_.id) {
                    if let Some(&member_id) = implementation.declarations.get(name) {
                        candidates.push(member_id);
                    }
                }
            }
        }
        candidates
    };
    // (caller node, call id, candidate callees) per dispatch site.
    let mut dispatch_sites: Vec<(Id, Id, Vec<Id>)> = Vec::new();
    // callee -> the nodes that may reach it through dispatch.
    let mut dispatch_callers: HashMap<Id, Vec<Id>> = HashMap::new();
    for node in graph.nodes() {
        for call in graph.calls_of(node.id()) {
            if !matches!(
                call.target,
                CallTarget::Indirect(IndirectReason::TraitDispatch | IndirectReason::GenericMember)
            ) {
                continue;
            }
            let Some(name) = dispatch_member_name(call.call_id) else {
                continue;
            };
            let candidates = dispatch_candidates(name);
            for &candidate in &candidates {
                dispatch_callers
                    .entry(candidate)
                    .or_default()
                    .push(node.id());
            }
            dispatch_sites.push((node.id(), call.call_id, candidates));
        }
    }

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

    // --- Injected (`context`-typed) closures — proposal/ambient-owner.md §5. ---
    // A clause on a parameter's closure type defers that closure's context
    // binding to its CALL sites: the literal passed in takes its own hidden
    // parameter (no creation capture), each call through the parameter is a
    // read-like demand on the caller (and a threading site), and the value
    // may only flow where the threading can follow it — a call, a forward to
    // a parameter with the SAME clause, or `run`'s body position.
    let mut deferred: HashMap<Id, HashSet<Id>> = HashMap::new(); // ctx -> closures
    let mut injected_calls: HashMap<Id, Vec<(Node, Id)>> = HashMap::new(); // ctx -> (caller, call)
    // The working clause map: declared clauses (parameters AND `let`
    // annotations) plus ADOPTED ones — an unannotated closure-literal binding
    // passed into a clause position adopts that clause (`let add = || ..;`
    // then `.on("click", add)`), exactly as if the literal were written
    // inline: its literal defers, and its direct calls become injected calls.
    let mut value_contexts: HashMap<Id, Vec<Id>> = program.parameter_contexts.clone();
    {
        // Validate each clause names actual contexts, and admit them to the
        // per-context loop (a clause context may have no direct get/run).
        for (&parameter, clause) in &program.parameter_contexts {
            for &context in clause {
                let is_context = program
                    .variables
                    .get(&context)
                    .and_then(|variable| program.type_id_to_type_map.get(&variable.type_id))
                    .is_some_and(|type_| {
                        matches!(
                            type_,
                            Type::Struct(id, _)
                                if program
                                    .structs
                                    .get(id)
                                    .is_some_and(|struct_| struct_.name == "Context")
                        )
                    });
                if is_context {
                    contexts.insert(context);
                } else {
                    errors.push(Error {
                        note: None,
                        span: span_of(program, parameter),
                        msg:
                            "this parameter's `context` clause names a value that is not a context"
                                .to_string(),
                    });
                }
            }
        }

        // Closure literals landing in annotated positions defer; annotated
        // values may forward to a parameter with the SAME clause.
        let mut allowed_forwards: HashSet<Id> = HashSet::new();

        // Clause-typed LET bindings (the ui-boundary follow-up): the
        // binding is a NAMED injected closure. Its initializer literal
        // defers exactly like a literal in a clause parameter position; a
        // same-clause value initializer is a forward; anything else is an
        // escape the threading cannot follow.
        for (&binding_id, clause) in &program.parameter_contexts {
            let Some(variable) = program.variables.get(&binding_id) else {
                // Parameters share the map but have no variable record.
                continue;
            };
            let Some(initial) = variable.initial else {
                continue;
            };
            match program.entity_map.get(&initial) {
                Some(Expr::Closure(closure_id)) => {
                    for &context in clause {
                        deferred.entry(context).or_default().insert(*closure_id);
                    }
                }
                Some(Expr::Local(source))
                    if program.parameter_contexts.get(source) == Some(clause) =>
                {
                    allowed_forwards.insert(initial);
                }
                _ => {
                    errors.push(Error { note: None,
                        span: span_of(program, initial),
                        msg: "a `context`-typed binding takes a closure literal, or a value with the same `context` clause"
                            .to_string(),
                    });
                }
            }
        }

        // Adoption: an argument binding with NO clause of its own, whose
        // initial is a closure literal, adopts the parameter's clause.
        let adoptable = |source: Id| -> Option<Id> {
            let variable = program.variables.get(&source)?;
            let initial = variable.initial?;
            match program.entity_map.get(&initial) {
                Some(Expr::Closure(closure_id)) => Some(*closure_id),
                _ => None,
            }
        };
        for (&call_id, function_call) in &program.function_calls {
            let Some(target) = call_target(program, call_id) else {
                continue;
            };
            let Some(function) = program.functions.get(&target) else {
                continue;
            };
            for (argument, parameter) in function_call.argument_ids.iter().zip(&function.parameters)
            {
                let Some(clause) = value_contexts.get(parameter).cloned() else {
                    continue;
                };
                match program.entity_map.get(argument) {
                    Some(Expr::Closure(closure_id)) => {
                        for &context in &clause {
                            deferred.entry(context).or_default().insert(*closure_id);
                        }
                    }
                    Some(Expr::Local(source)) if value_contexts.get(source) == Some(&clause) => {
                        allowed_forwards.insert(*argument);
                    }
                    Some(Expr::Local(source))
                        if !value_contexts.contains_key(source) && adoptable(*source).is_some() =>
                    {
                        let closure_id = adoptable(*source).expect("just matched");
                        value_contexts.insert(*source, clause.clone());
                        for &context in &clause {
                            deferred.entry(context).or_default().insert(closure_id);
                        }
                        allowed_forwards.insert(*argument);
                    }
                    _ => {
                        errors.push(Error { note: None,
                            span: span_of(program, *argument),
                            msg: "a `context`-typed parameter takes a closure literal, a value with the same `context` clause, or a local closure binding (which adopts the clause)"
                                .to_string(),
                        });
                    }
                }
            }
        }

        // Calls THROUGH an annotated (or adopted) value — after adoption, so
        // a named handler's direct calls demand and thread like any injected
        // call.
        for node in graph.nodes() {
            for call in graph.calls_of(node.id()) {
                let Some(function_call) = program.function_calls.get(&call.call_id) else {
                    continue;
                };
                if let Some(Expr::Local(target)) = program.entity_map.get(&function_call.subject_id)
                {
                    if let Some(clause) = value_contexts.get(target) {
                        for &context in clause {
                            injected_calls
                                .entry(context)
                                .or_default()
                                .push((*node, call.call_id));
                        }
                    }
                }
            }
        }

        // The value-flow restriction: everywhere else an annotated value
        // appears is an escape the threading cannot follow.
        let run_body_entities: HashSet<Id> =
            plan.runs.iter().map(|site| site.closure_entity).collect();
        for (&entity, expr) in &program.entity_map {
            let Expr::Local(target) = expr else {
                continue;
            };
            if !value_contexts.contains_key(target) {
                continue;
            }
            if call_subject_entities.contains(&entity)
                || allowed_forwards.contains(&entity)
                || run_body_entities.contains(&entity)
            {
                continue;
            }
            errors.push(Error { note: None,
                span: span_of(program, entity),
                msg: "an injected (`context`-typed) closure can only be called, forwarded to a parameter with the same `context` clause, or passed to `run`"
                    .to_string(),
            });
        }
    }
    plan.contexts = {
        let mut sorted: Vec<Id> = contexts.iter().copied().collect();
        sorted.sort_by_key(|id| id.0);
        sorted
    };

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
        // A call through an injected closure demands the context on its
        // caller, exactly like a read (proposal/ambient-owner.md §5).
        for (owner, _call) in injected_calls.get(&context).into_iter().flatten() {
            if needs.insert(owner.id()) {
                worklist.push(owner.id());
            }
        }
        // A closure that RECEIVES the value as its own parameter — a `run`
        // body for this context, or a deferred (injected) literal — does not
        // capture from its creator, so needs must not leak to its parent.
        let own_param_closure = |id: Id| -> bool {
            run_closures.get(&id) == Some(&context)
                || deferred
                    .get(&context)
                    .is_some_and(|closures| closures.contains(&id))
        };
        // Backward reachability: a caller of a needs-context node needs it too
        // — through direct edges, through dispatch (B14), and — for CAPTURING
        // closures only — through the enclosing scope (the closure reads its
        // provider's parameter, so the provider must hold one; a stored
        // notify closure created inside `map` makes `map` needy, and `map`
        // created under a turn then hands that turn to the closure).
        while let Some(id) = worklist.pop() {
            for caller in graph.callers_of(id) {
                if needs.insert(caller.id()) {
                    worklist.push(caller.id());
                }
            }
            for caller in dispatch_callers.get(&id).into_iter().flatten() {
                if needs.insert(*caller) {
                    worklist.push(*caller);
                }
            }
            if !own_param_closure(id) {
                if let Some(parent) = graph.closure_parent_of(id) {
                    if needs.insert(parent) {
                        worklist.push(parent);
                    }
                }
            }
        }

        // --- Flavor (reactive-turns.md §5.1): STRICT nodes hold the bare
        // value (a strict `get`, or a call through an injected closure,
        // reaches them) and keep the coverage fence; the rest of `needs` is
        // SAFE — it holds `Option<T>` and never fences. Strictness propagates
        // backward exactly like `needs` (a caller of a strict node must
        // supply the bare value), so strict ⊆ needs.
        let mut strict: HashSet<Id> = HashSet::new();
        let mut strict_worklist: Vec<Id> = Vec::new();
        for get in gets
            .iter()
            .filter(|get| get.context == context && !get.safe)
        {
            if strict.insert(get.owner.id()) {
                strict_worklist.push(get.owner.id());
            }
        }
        for (owner, _call) in injected_calls.get(&context).into_iter().flatten() {
            if strict.insert(owner.id()) {
                strict_worklist.push(owner.id());
            }
        }
        loop {
            while let Some(id) = strict_worklist.pop() {
                for caller in graph.callers_of(id) {
                    if strict.insert(caller.id()) {
                        strict_worklist.push(caller.id());
                    }
                }
                for caller in dispatch_callers.get(&id).into_iter().flatten() {
                    if strict.insert(*caller) {
                        strict_worklist.push(*caller);
                    }
                }
                if !own_param_closure(id) {
                    if let Some(parent) = graph.closure_parent_of(id) {
                        if strict.insert(parent) {
                            strict_worklist.push(parent);
                        }
                    }
                }
            }
            // A dispatch site whose needy candidates MIX flavors would need
            // two argument forms at one call — promote its safe candidates
            // to strict (they gain the fence) and re-propagate.
            let mut promoted = false;
            for (_caller, _call_id, candidates) in &dispatch_sites {
                let needy: Vec<Id> = candidates
                    .iter()
                    .copied()
                    .filter(|candidate| needs.contains(candidate))
                    .collect();
                if needy.is_empty() || needy.iter().all(|id| !strict.contains(id)) {
                    continue;
                }
                for id in needy {
                    if strict.insert(id) {
                        strict_worklist.push(id);
                        promoted = true;
                    }
                }
            }
            if !promoted {
                break;
            }
        }

        let run_closure_ids: HashSet<Id> = run_closures
            .iter()
            .filter(|(_, bound)| **bound == context)
            .map(|(closure, _)| *closure)
            // A deferred (injected) literal behaves like a `run` body here:
            // it always takes its own hidden parameter and is covered by
            // construction — its callers supply the value.
            .chain(deferred.get(&context).into_iter().flatten().copied())
            .collect();

        // Classify each needs-context node.
        let is_function = |id: Id| program.functions.contains_key(&id);

        // --- Coverage (greatest fixpoint): assume every node is covered, then
        // remove any that can be entered without the value. A `run` closure
        // always receives the value from `run`, so it is covered even when it
        // doesn't read the context itself (a nested closure may capture it).
        // Only STRICT nodes are checked — a safe node legitimately runs
        // uncovered (its parameter is then `None`). ---
        let mut bound: HashSet<Id> = needs
            .iter()
            .copied()
            .chain(run_closure_ids.iter().copied())
            .collect();
        // The inlined entry `main` never receives a value.
        if let Some(main) = entry_main {
            bound.remove(&main);
        }
        loop {
            let mut removed = false;
            for &id in &strict {
                if !bound.contains(&id) || run_closure_ids.contains(&id) {
                    continue;
                }
                let covered = if is_function(id) {
                    let callers = graph.callers_of(id);
                    let through_dispatch = dispatch_callers.get(&id);
                    let no_edges =
                        callers.is_empty() && through_dispatch.is_none_or(|list| list.is_empty());
                    if no_edges {
                        // No caller edges: dead code is exempt (it cannot
                        // run); a top-level-called or value-taken function is
                        // entered from outside the graph — uncovered.
                        !top_level_targets.contains(&id) && !value_taken.contains(&id)
                    } else {
                        callers.iter().all(|caller| bound.contains(&caller.id()))
                            && through_dispatch
                                .into_iter()
                                .flatten()
                                .all(|caller| bound.contains(caller))
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

        // Any STRICT get whose owner stayed unbound is read outside every
        // `run`; a safe read never fences.
        for get in gets
            .iter()
            .filter(|get| get.context == context && !get.safe)
        {
            if !bound.contains(&get.owner.id()) {
                errors.push(Error { note: None,
                    span: span_of(program, get.call_id),
                    msg: format!(
                        "context `{}` is read here, but this code can be reached without an enclosing `run`",
                        context_name(program, context)
                    ),
                });
            }
        }
        // Calling an injected closure IS a read: its deferred argument comes
        // from the caller, so an unbound caller has nothing to supply.
        for (owner, call_id) in injected_calls.get(&context).into_iter().flatten() {
            if !bound.contains(&owner.id()) {
                errors.push(Error { note: None,
                    span: span_of(program, *call_id),
                    msg: format!(
                        "an injected closure is called here, but this code can be reached without an enclosing `run` for context `{}`",
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
                        note: None,
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
        // node -> the node whose parameter it reads (itself, or the capture
        // provider) — the parameter's FLAVOR is the provider's.
        let mut provider_of: HashMap<Id, Id> = HashMap::new();
        // Nodes with no value source: the inlined entry `main` (it can carry
        // no hidden parameter), and any closure whose provider chain roots at
        // it — their safe reads and threads become literal `None`s.
        let mut none_rooted: HashSet<Id> = HashSet::new();
        if let Some(main) = entry_main {
            if needs.contains(&main) {
                none_rooted.insert(main);
            }
        }
        for &id in &needs {
            if entry_main == Some(id) {
                continue;
            }
            if is_function(id) || run_closure_ids.contains(&id) {
                param_nodes.insert(id);
                provider_of.insert(id, id);
            } else {
                // A captured closure: walk up to the nearest enclosing node
                // that holds the value (a function or `run` closure). A walk
                // that lands on the entry `main` first has no value to
                // capture — the closure is None-rooted.
                let mut provider = graph.closure_parent_of(id);
                loop {
                    match provider {
                        Some(parent) if entry_main == Some(parent) => {
                            none_rooted.insert(id);
                            break;
                        }
                        Some(parent)
                            if is_function(parent) || run_closure_ids.contains(&parent) =>
                        {
                            plan.captures.push((context, id, parent));
                            provider_of.insert(id, parent);
                            break;
                        }
                        Some(parent) => provider = graph.closure_parent_of(parent),
                        None => {
                            none_rooted.insert(id);
                            break;
                        }
                    }
                }
            }
        }
        for &id in &param_nodes {
            provider_of.entry(id).or_insert(id);
        }
        for id in param_nodes {
            plan.param_nodes.push((context, id));
        }
        // A parameter holds the BARE value when its provider is strict or a
        // `run` closure (which `run` hands the bare value); otherwise it
        // holds `Option<T>`.
        let holds_bare = |node: Id| -> bool {
            provider_of
                .get(&node)
                .map(|provider| strict.contains(provider) || run_closure_ids.contains(provider))
                .unwrap_or(false)
        };

        for get in gets.iter().filter(|get| get.context == context) {
            if none_rooted.contains(&get.owner.id()) {
                // Only reachable for SAFE reads (a strict get here already
                // failed the fence): the value is definitionally absent.
                plan.none_gets.push(get.call_id);
                continue;
            }
            // A safe read of a BARE holder wraps; everything else reads the
            // parameter as-is (bare for strict gets, `Option` for safe reads
            // in safe holders).
            let wrap_some = get.safe && holds_bare(get.owner.id());
            plan.gets.push((get.call_id, context, get.owner, wrap_some));
        }

        // Thread the value into every call from a needs-context node to a
        // needs-context function — direct calls, and dispatch sites whose
        // candidate callees include a needy one (B14; a candidate that does
        // not need the value ignores the extra trailing argument). The
        // argument form follows the flavors: bare→bare and Option→Option
        // pass the parameter through; a BARE holder supplying a SAFE callee
        // `Some`-wraps (the covered→safe boundary). Safe→strict cannot occur
        // (strictness propagated to the caller).
        for &node_id in &needs {
            let Some(&owner) = graph.nodes().iter().find(|node| node.id() == node_id) else {
                continue;
            };
            for call in graph.calls_of(node_id) {
                if let CallTarget::Function(callee) = call.target {
                    if needs.contains(&callee) {
                        if none_rooted.contains(&node_id) {
                            // No value here: safe callees get `None` (a
                            // strict callee under a None root already
                            // fenced).
                            if !strict.contains(&callee) {
                                plan.thread_calls.push((
                                    call.call_id,
                                    context,
                                    ThreadForm::NoneLiteral,
                                ));
                            }
                            continue;
                        }
                        let form = if !strict.contains(&callee) && holds_bare(node_id) {
                            ThreadForm::WrapSome { owner }
                        } else {
                            ThreadForm::Param { owner }
                        };
                        plan.thread_calls.push((call.call_id, context, form));
                    }
                }
            }
        }
        for (caller, call_id, candidates) in &dispatch_sites {
            if !needs.contains(caller) {
                continue;
            }
            let needy: Vec<Id> = candidates
                .iter()
                .copied()
                .filter(|candidate| needs.contains(candidate))
                .collect();
            if needy.is_empty() {
                continue;
            }
            let Some(&owner) = graph.nodes().iter().find(|node| node.id() == *caller) else {
                continue;
            };
            // Mixed flavors were promoted away: needy candidates are now all
            // strict or all safe.
            let callee_safe = needy.iter().all(|id| !strict.contains(id));
            if none_rooted.contains(caller) {
                if callee_safe {
                    plan.thread_calls
                        .push((*call_id, context, ThreadForm::NoneLiteral));
                }
                continue;
            }
            let form = if callee_safe && holds_bare(*caller) {
                ThreadForm::WrapSome { owner }
            } else {
                ThreadForm::Param { owner }
            };
            plan.thread_calls.push((*call_id, context, form));
        }
        // Calls through injected closures: the caller's value rides as the
        // deferred trailing argument (the bare channel).
        for (owner, call_id) in injected_calls.get(&context).into_iter().flatten() {
            plan.thread_calls
                .push((*call_id, context, ThreadForm::Param { owner: *owner }));
        }
        // Top-level calls to safe functions: the entry point with no value —
        // a literal `None` rides along. (Top-level calls to STRICT functions
        // already failed the fence.)
        for (&call_id, function_call) in &program.function_calls {
            if owned_call_ids.contains(&call_id) {
                continue;
            }
            let Some(target) = local_target(program, function_call.subject_id) else {
                continue;
            };
            if needs.contains(&target) && !strict.contains(&target) {
                plan.thread_calls
                    .push((call_id, context, ThreadForm::NoneLiteral));
            }
        }
    }

    // Safe reads synthesize `Some`/`None` — resolve the `Option` variant
    // entities once. Missing `Option` with safe sites in play is a hard
    // error rather than a miscompile.
    let any_safe = gets.iter().any(|get| get.safe);
    let any_none = plan
        .thread_calls
        .iter()
        .any(|(_, _, form)| matches!(form, ThreadForm::NoneLiteral));
    if any_safe || any_none {
        let variants = program
            .enums
            .values()
            .find(|enum_| enum_.name == "Option")
            .and_then(|enum_| program.scopes.get(&enum_.variants_scope_id))
            .map(|scope| {
                (
                    scope.name_to_id_map.get("Some").copied(),
                    scope.name_to_id_map.get("None").copied(),
                )
            });
        match variants {
            Some((Some(some_variant), Some(none_variant))) => {
                plan.some_variant = Some(some_variant);
                plan.none_variant = Some(none_variant);
            }
            _ => errors.push(Error {
                note: None,
                span: crate::span::Span {
                    start: 0,
                    end: 0,
                    context: (),
                },
                msg: "`get_safe` needs `std::option::Option` loaded".to_string(),
            }),
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

    let empty_span = crate::span::Span {
        start: 0,
        end: 0,
        context: (),
    };
    // Synthesizes `Some(parameter)`: a fresh call to the `Option::Some`
    // variant constructor. The transformer lowers a variant-subject call to
    // the variant value directly, so no method records are needed.
    let wrap_in_some = |program: &mut Program, parameter: Id, next: &mut dyn FnMut() -> Id| {
        let some_variant = plan
            .some_variant
            .expect("safe sites resolved the Option variants");
        let subject = next();
        program
            .entity_map
            .insert(subject, Expr::Local(some_variant));
        let value_reference = next();
        program
            .entity_map
            .insert(value_reference, Expr::Local(parameter));
        let call = next();
        program.function_calls.insert(
            call,
            crate::analyzer::FunctionCall {
                id: call,
                subject_id: subject,
                generic_argument_ids: Vec::new(),
                argument_ids: vec![value_reference],
                arguments_span: empty_span,
            },
        );
        program.entity_map.insert(call, Expr::Call(call));
        call
    };

    // `get()` becomes a read of the in-scope parameter; a safe read of a
    // BARE holder becomes `Some(parameter)` (the get's own call entity is
    // rewritten into the wrap, its method records purged like `run`'s).
    for &(call_id, context, owner, wrap_some) in &plan.gets {
        if let Some(&parameter) = source.get(&(context, owner.id())) {
            if wrap_some {
                let some_variant = plan
                    .some_variant
                    .expect("safe sites resolved the Option variants");
                let subject = fresh();
                program
                    .entity_map
                    .insert(subject, Expr::Local(some_variant));
                let value_reference = fresh();
                program
                    .entity_map
                    .insert(value_reference, Expr::Local(parameter));
                if let Some(call) = program.function_calls.get_mut(&call_id) {
                    call.subject_id = subject;
                    call.generic_argument_ids = Vec::new();
                    call.argument_ids = vec![value_reference];
                }
                program.method_call_substitution.remove(&call_id);
                program.generic_dispatch.remove(&call_id);
            } else {
                program.entity_map.insert(call_id, Expr::Local(parameter));
            }
        }
    }

    // Each call to a needs-context function gets the value appended as an
    // argument — the caller's parameter, `Some`-wrapped at a covered→safe
    // boundary.
    for &(call_id, context, ref form) in &plan.thread_calls {
        let argument = match *form {
            ThreadForm::Param { owner } => {
                let Some(&parameter) = source.get(&(context, owner.id())) else {
                    continue;
                };
                let reference = fresh();
                program.entity_map.insert(reference, Expr::Local(parameter));
                reference
            }
            ThreadForm::WrapSome { owner } => {
                let Some(&parameter) = source.get(&(context, owner.id())) else {
                    continue;
                };
                wrap_in_some(program, parameter, &mut fresh)
            }
            ThreadForm::NoneLiteral => {
                let none_variant = plan
                    .none_variant
                    .expect("safe sites resolved the Option variants");
                let reference = fresh();
                program
                    .entity_map
                    .insert(reference, Expr::Local(none_variant));
                reference
            }
        };
        if let Some(call) = program.function_calls.get_mut(&call_id) {
            call.argument_ids.push(argument);
        }
    }

    // Safe reads inside the inlined entry `main` are literal `None`s.
    for &call_id in &plan.none_gets {
        let none_variant = plan
            .none_variant
            .expect("safe sites resolved the Option variants");
        program
            .entity_map
            .insert(call_id, Expr::Local(none_variant));
    }

    // `run(value, body)` becomes `body(value)`: the body closure is the new
    // call subject, the value its sole argument (binding the closure's hidden
    // parameter).
    for site in &plan.runs {
        if let Some(call) = program.function_calls.get_mut(&site.call_id) {
            call.subject_id = site.closure_entity;
            call.argument_ids = vec![site.value_id];
        }
        // The call entity keeps its id, so purge the METHOD-call records the
        // analyzer attached to `Context::run` — a stale substitution would
        // make the emitter monomorphize the new subject (for a value body, a
        // plain parameter) as if it were a generic function.
        program.method_call_substitution.remove(&site.call_id);
        program.generic_dispatch.remove(&site.call_id);
    }

    // `Context::new()` lowers to an opaque value; its binding is now unused.
    for &call_id in &plan.news {
        program.entity_map.insert(call_id, Expr::Null);
    }

    program.next_entity_id = next_id;
}
