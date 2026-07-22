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

use std::collections::{HashMap, HashSet};

use crate::analyzer::{Expr, GenericDispatch, Program};
use crate::call_graph::{CallGraph, CallTarget, IndirectReason};
use crate::id::Id;
use crate::type_::{Type, TypeId};

/// Computes the async set and stores it on the program.
pub fn infer(program: &mut Program) {
    let graph = CallGraph::build(program);
    let mut async_set: HashSet<Id> = HashSet::new();

    // Every value each binding ever holds — its initializer plus every
    // reassignment (`mut` rebinds) — for async ADOPTION: a binding holding
    // an async closure through any of them awaits when called.
    let mut held_values: HashMap<Id, Vec<Id>> = HashMap::new();
    for (variable_id, variable) in &program.variables {
        if let Some(initial) = variable.initial {
            held_values.entry(*variable_id).or_default().push(initial);
        }
    }
    for expr in program.entity_map.values() {
        if let Expr::Assignment(target_id, value_id) = expr
            && let Some(Expr::Local(variable_id)) = program.entity_map.get(target_id)
        {
            held_values.entry(*variable_id).or_default().push(*value_id);
        }
    }

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

    // --- Fixpoint: a node that calls an async function/extern implicitly
    // awaits it, so it is async too — interleaved with the ADAPTATION
    // worklist (async-polymorphism.md A.1) until both are stable: adaptation
    // can flip a base function async (it calls an adapted async instance),
    // which the plain fixpoint must then propagate to ITS callers, which can
    // in turn create new adapted instances. Both are monotone over
    // `async_set`, so this terminates.
    let adaptation = loop {
        base_fixpoint(program, &graph, &held_values, &mut async_set);
        let before = async_set.len();
        let adaptation = compute_adaptation(program, &graph, &held_values, &mut async_set);
        if async_set.len() == before {
            break adaptation;
        }
    };
    for (span, msg, note) in adaptation.diagnostics {
        program
            .diagnostics
            .push(crate::error::Error { note, span, msg });
    }
    program.adapted_instances = adaptation.instances;

    // --- Materialize the value-flow channels for emission (J2): adopted
    // bindings — unannotated ones holding an async closure — join
    // `async_values`, so the transformer's Local-subject await covers them;
    // calls through an async field or an async-returning call land in
    // `awaited_calls` for the transformer's non-Local subjects. Done before
    // the divergence and initializer checks so both see the full set.
    let adopted: Vec<Id> = program
        .variables
        .keys()
        .filter(|variable| {
            !program.async_values.contains(variable)
                && binding_holds_async(program, **variable, &held_values, &async_set, 0)
        })
        .copied()
        .collect();
    program.async_values.extend(adopted);
    let awaited: Vec<Id> = program
        .function_calls
        .iter()
        .filter(
            |(_, function_call)| match program.entity_map.get(&function_call.subject_id) {
                Some(Expr::Field(_, struct_id, index)) => {
                    program.async_fields.contains(&(*struct_id, *index))
                }
                Some(Expr::Call(inner)) => call_returns_async_closure(program, *inner),
                // A directly-applied async closure literal (the lowered
                // `run` body): a statically-known await point.
                Some(Expr::Closure(closure_id)) => async_set.contains(closure_id),
                _ => false,
            },
        )
        .map(|(call_id, _)| *call_id)
        .collect();
    program.awaited_calls.extend(awaited);

    // --- The J2 divergence check, post-adaptation: an async closure into a
    // `sync`-contract parameter, or into a host (`external`) callback that
    // cannot await it, is refused. (A PLAIN value-returning parameter now
    // ADAPTS instead of erroring — the worklist above; void-returning
    // parameters stay legal as spawn semantics.)
    let mut divergences: Vec<(crate::span::Span, String)> = Vec::new();
    let no_flags: HashMap<Id, bool> = HashMap::new();
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
            // Void = spawn semantics; an UNRESOLVED return can't be a
            // known lie — only a resolved non-void return diverges.
            let indeterminate = matches!(
                program.type_id_to_type_map.get(&return_type),
                Some(Type::Void) | Some(Type::Unknown) | Some(Type::Unresolved) | None
            );
            if indeterminate {
                continue;
            }
            // The argument VALUE is async through any channel — a literal, a
            // declared/adopted binding or parameter, an `async` field read,
            // an async-returning call (the same shapes that make a call
            // through it await).
            if !value_async_in(program, &held_values, &async_set, &no_flags, &[], *argument) {
                continue;
            }
            let span = program
                .span_map
                .get(parameter)
                .map(|span| **span)
                .unwrap_or(crate::span::Span { start: 0, end: 0 });
            // A `sync`-marked parameter is a deliberate contract
            // (async-polymorphism.md A.2) — refused, with the contract's
            // steer. A PLAIN value-returning parameter ADAPTS (the instance
            // worklist handled it), so it no longer errors here.
            if !program.sync_values.contains(parameter) {
                continue;
            }
            divergences.push((
                span,
                format!(
                    "`{}` requires a synchronous closure (`sync`): its completion is part of the declaring function's synchronous protocol — move the async work outside the callback (e.g. a `turn` with an awaiting body, `Draft`, or a spawned `async` block)",
                    parameter_record.name
                ),
            ));
        }
    }
    // Host boundary: an `external` function cannot await a vilan closure —
    // its value-returning closure parameters are implicitly `sync`
    // (async-polymorphism.md A.4); void ones keep spawn semantics.
    for function_call in program.function_calls.values() {
        let Some(Expr::Local(target)) = program.entity_map.get(&function_call.subject_id) else {
            continue;
        };
        let Some(external) = program.external_functions.get(target) else {
            continue;
        };
        for (argument, parameter) in function_call.argument_ids.iter().zip(&external.parameters) {
            // A DECLARED `async |…| T` parameter is the typed channel: the
            // host explicitly contracts to await the closure (it receives an
            // async function — `__nursery_run` awaits its body this way).
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
            if matches!(
                program.type_id_to_type_map.get(&return_type),
                Some(Type::Void) | Some(Type::Unknown) | Some(Type::Unresolved) | None
            ) {
                continue;
            }
            if !value_async_in(program, &held_values, &async_set, &no_flags, &[], *argument) {
                continue;
            }
            let span = program
                .span_map
                .get(argument)
                .map(|span| **span)
                .unwrap_or(crate::span::Span { start: 0, end: 0 });
            divergences.push((
                span,
                format!(
                    "`{}` is a host (`external`) function — it cannot await a vilan closure, so this parameter only accepts synchronous closures (or a void-returning one for spawn semantics)",
                    external.name
                ),
            ));
        }
    }
    // The same divergence through the FIELD channel: an async closure stored
    // into a plain closure field (struct literal or field assignment) with a
    // non-void return — every later read-and-call would hand back a promise
    // typed `T`. Declared `async || T` fields are exempt (that is the fix).
    let mut field_stores: Vec<(Id, usize, Id)> = Vec::new();
    for expr in program.entity_map.values() {
        match expr {
            // The initializer expr carries its OWN id; the struct def resolves
            // through `struct_initializer_to_def`.
            Expr::StructInitializer(initializer_id, field_values) => {
                let Some(struct_id) = program.struct_initializer_to_def.get(initializer_id) else {
                    continue;
                };
                for (field_index, value_id) in field_values {
                    field_stores.push((*struct_id, *field_index, *value_id));
                }
            }
            Expr::Assignment(target_id, value_id) => {
                if let Some(Expr::Field(_, struct_id, field_index)) =
                    program.entity_map.get(target_id)
                {
                    field_stores.push((*struct_id, *field_index, *value_id));
                }
            }
            _ => {}
        }
    }
    let mut field_divergences: Vec<(crate::span::Span, String)> = Vec::new();
    for (struct_id, field_index, value_id) in field_stores {
        if program.async_fields.contains(&(struct_id, field_index)) {
            continue;
        }
        let Some(struct_) = program.structs.get(&struct_id) else {
            continue;
        };
        let Some(field) = struct_.fields.get(field_index) else {
            continue;
        };
        let Some(Type::Closure(_, return_type)) =
            program.type_id_to_type_map.get(&field.type_id).cloned()
        else {
            continue;
        };
        if matches!(
            program.type_id_to_type_map.get(&return_type),
            Some(Type::Void) | Some(Type::Unknown) | Some(Type::Unresolved) | None
        ) {
            continue;
        }
        if !value_async_in(program, &held_values, &async_set, &no_flags, &[], value_id) {
            continue;
        }
        let span = program
            .span_map
            .get(&value_id)
            .map(|span| **span)
            .unwrap_or(crate::span::Span { start: 0, end: 0 });
        field_divergences.push((
            span,
            format!(
                "field `{}` of `{}` receives an async closure, but its type awaits nothing — declare it `async || T` (or return void for spawn semantics)",
                field.name, struct_.name
            ),
        ));
    }
    divergences.extend(field_divergences);

    // And through the RETURN channel: a function whose declared return type
    // is a plain closure (non-void) returning an async closure. The declared
    // `async || T` return marker is the fix.
    for (function_id, value_id) in &program.return_sites {
        if program.async_returning.contains(function_id) {
            continue;
        }
        let Some(function) = program.functions.get(function_id) else {
            continue;
        };
        let Some(declared) = function.return_type_id else {
            continue;
        };
        let Some(Type::Closure(_, return_type)) =
            program.type_id_to_type_map.get(&declared).cloned()
        else {
            continue;
        };
        if matches!(
            program.type_id_to_type_map.get(&return_type),
            Some(Type::Void) | Some(Type::Unknown) | Some(Type::Unresolved) | None
        ) {
            continue;
        }
        if !value_async_in(program, &held_values, &async_set, &no_flags, &[], *value_id) {
            continue;
        }
        let span = program
            .span_map
            .get(value_id)
            .map(|span| **span)
            .unwrap_or(crate::span::Span { start: 0, end: 0 });
        divergences.push((
            span,
            format!(
                "`{}` returns an async closure, but its declared return type awaits nothing — declare it `async || T` (or return void for spawn semantics)",
                function.name
            ),
        ));
    }

    for (span, msg) in divergences {
        program.diagnostics.push(crate::error::Error {
            note: None,
            span,
            msg,
        });
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
    let initializer_adaptive = adaptive_params_of(program);
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
                // A call whose async closure arguments ADAPT the callee
                // (async-polymorphism.md A.1) awaits like any async call —
                // and a module initializer cannot.
                CallTarget::Function(callee)
                    if {
                        let bits = bits_for_call(
                            program,
                            &held_values,
                            &async_set,
                            &initializer_adaptive,
                            &HashMap::new(),
                            &[],
                            callee,
                            call.call_id,
                        );
                        !bits.is_empty()
                            && program
                                .adapted_instances
                                .get(&(callee, bits))
                                .is_some_and(|instance| instance.is_async)
                    } =>
                {
                    Some(callee)
                }
                CallTarget::Function(callee) | CallTarget::External(callee) => {
                    async_set.contains(&callee).then_some(callee)
                }
                CallTarget::Indirect(
                    IndirectReason::GenericMember | IndirectReason::TraitDispatch,
                ) => dispatch_candidates(program, call.call_id)
                    .into_iter()
                    .find(|member| async_set.contains(member)),
                // A call through an `async || T`-typed value awaits too — as
                // does a directly-applied async closure literal (the lowered
                // `run` body).
                _ => {
                    program.function_calls.get(&call.call_id).and_then(
                        |function_call| match program.entity_map.get(&function_call.subject_id) {
                            Some(Expr::Local(target)) if program.async_values.contains(target) => {
                                Some(*target)
                            }
                            Some(Expr::Closure(closure_id)) if async_set.contains(closure_id) => {
                                Some(*closure_id)
                            }
                            _ => None,
                        },
                    )
                }
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
                });
            let binding_name = program
                .variables
                .get(&binding)
                .map(|variable| variable.name)
                .unwrap_or("_");
            let span = program
                .span_map
                .get(&call.call_id)
                .map(|span| **span)
                .unwrap_or(crate::span::Span { start: 0, end: 0 });
            // A nameless target is an awaiting closure applied directly (an
            // adopted value, or a lowered `run` body) — phrase it as what it
            // is rather than backticking a description.
            let culprit = match target_name {
                Some(name) => format!("calls `{name}`, which is async"),
                None => "runs a closure that awaits".to_string(),
            };
            initializer_awaits.push((
                span,
                format!(
                    "the initializer of `{binding_name}` {culprit} — a module-level \
                     binding cannot await (module initialization is synchronous); wrap \
                     the work in a function and call it from `main`"
                ),
            ));
        }
    }
    for (span, msg) in initializer_awaits {
        program.diagnostics.push(crate::error::Error {
            note: None,
            span,
            msg,
        });
    }

    program.async_functions = async_set;
}

/// Whether a call THROUGH `subject_id` is an await point (J2): the subject is
/// an `async || T`-typed value (declared), a binding HOLDING an async closure
/// (adoption), a read of an `async || T` field, a call to a function whose
/// declared return type carries the marker, or an async closure LITERAL
/// applied directly (the `context` pass lowers `run(value, body)` to
/// `body(value)` — a statically-known callee, awaited like a direct call).
fn subject_awaits(
    program: &Program,
    subject_id: Id,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &HashSet<Id>,
    depth: u32,
) -> bool {
    match program.entity_map.get(&subject_id) {
        Some(Expr::Local(target)) => {
            program.async_values.contains(target)
                || binding_holds_async(program, *target, held_values, async_set, depth)
        }
        Some(Expr::Field(_, struct_id, index)) => {
            program.async_fields.contains(&(*struct_id, *index))
        }
        Some(Expr::Call(inner)) => call_returns_async_closure(program, *inner),
        Some(Expr::Closure(closure_id)) => async_set.contains(closure_id),
        _ => false,
    }
}

/// Adoption (J2): whether the binding holds an async closure through any of
/// its values — the initializer or, for a `mut` binding, any assigned value.
/// A held value counts when it is an async closure literal, a read of an
/// async field, an async-returning call, or another binding that holds one
/// (chains resolve a few hops; the cap keeps a cyclic rebind from looping).
fn binding_holds_async(
    program: &Program,
    variable_id: Id,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &HashSet<Id>,
    depth: u32,
) -> bool {
    if depth > 4 {
        return false;
    }
    let Some(held) = held_values.get(&variable_id) else {
        return false;
    };
    held.iter()
        .any(|value_id| match program.entity_map.get(value_id) {
            Some(Expr::Closure(closure_id)) => async_set.contains(closure_id),
            Some(Expr::Field(_, struct_id, index)) => {
                program.async_fields.contains(&(*struct_id, *index))
            }
            Some(Expr::Call(inner)) => call_returns_async_closure(program, *inner),
            Some(Expr::Local(source)) => {
                binding_holds_async(program, *source, held_values, async_set, depth + 1)
            }
            _ => false,
        })
}

/// Whether the call at `call_id` targets a function whose DECLARED return
/// type is `async || T` — its result is an async closure, so calling that
/// result awaits.
fn call_returns_async_closure(program: &Program, call_id: Id) -> bool {
    program
        .function_calls
        .get(&call_id)
        .is_some_and(|function_call| {
            matches!(
                program.entity_map.get(&function_call.subject_id),
                Some(Expr::Local(target)) if program.async_returning.contains(target)
            )
        })
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

// ---------------------------------------------------------------------------
// Adaptation (async-polymorphism.md A.1): the instance worklist.
// ---------------------------------------------------------------------------

/// The worklist's output: the per-instance emission decisions and the
/// diagnostics it found (transitive `sync` violations, dispatch refusals).
struct Adaptation {
    instances: HashMap<(Id, Vec<Id>), crate::analyzer::AdaptedInstance>,
    diagnostics: Vec<(crate::span::Span, String, Option<crate::error::Note>)>,
}

/// One instance's identity: the function and WHICH of its closure parameters
/// are async, sorted for a stable key.
type InstanceKey = (Id, Vec<Id>);

fn compute_adaptation(
    program: &Program,
    graph: &CallGraph,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &mut HashSet<Id>,
) -> Adaptation {
    let adaptive = adaptive_params_of(program);

    // The component of a root function: itself plus its transitively nested
    // closures — they share the instance's bits context.
    let component = |root: Id| -> Vec<Id> {
        let mut members = vec![root];
        let mut cursor = 0;
        while cursor < members.len() {
            if let Some(children) = graph.closure_children_of(members[cursor]) {
                members.extend(children.iter().copied());
            }
            cursor += 1;
        }
        members
    };

    let mut instance_async: HashMap<InstanceKey, bool> = HashMap::new();
    let mut origins: HashMap<InstanceKey, Id> = HashMap::new();
    let mut dependents: HashMap<InstanceKey, HashSet<InstanceKey>> = HashMap::new();
    let mut pending: Vec<InstanceKey> = program
        .functions
        .keys()
        .map(|function_id| (*function_id, Vec::new()))
        .collect();
    let mut queued: HashSet<InstanceKey> = pending.iter().cloned().collect();

    // Module initializers can also instantiate adapted callees (a top-level
    // `let ids = xs.map(async)`); discover those instances up front so the
    // module-initializer check can refuse the await (an initializer cannot).
    for binding in program.module_level_bindings() {
        for call in graph.initializer_calls_of(binding) {
            if let CallTarget::Function(callee) = call.target {
                let bits = bits_for_call(
                    program,
                    held_values,
                    async_set,
                    &adaptive,
                    &HashMap::new(),
                    &[],
                    callee,
                    call.call_id,
                );
                if !bits.is_empty() {
                    let key = (callee, bits);
                    instance_async.entry(key.clone()).or_insert(false);
                    origins.entry(key.clone()).or_insert(call.call_id);
                    if queued.insert(key.clone()) {
                        pending.push(key);
                    }
                }
            }
        }
    }

    // --- The worklist: process instances until every async flag is stable.
    // Flags only move false -> true, so this terminates. Every call of every
    // member is enumerated on every pass — even members already known async
    // — because enumeration is also DISCOVERY: a call with async closure
    // arguments mints the callee's adapted instance.
    while let Some(key) = pending.pop() {
        queued.remove(&key);
        let (root, ref bits) = key;
        let members = component(root);
        let mut flags: HashMap<Id, bool> = members
            .iter()
            .map(|member| (*member, async_set.contains(member)))
            .collect();
        loop {
            let mut changed = false;
            for member in &members {
                for call in graph.calls_of(*member) {
                    let awaits = match call.target {
                        CallTarget::Function(callee) => {
                            let callee_bits = bits_for_call(
                                program,
                                held_values,
                                async_set,
                                &adaptive,
                                &flags,
                                bits,
                                callee,
                                call.call_id,
                            );
                            if callee_bits.is_empty() {
                                async_set.contains(&callee)
                            } else {
                                let callee_key = (callee, callee_bits);
                                dependents
                                    .entry(callee_key.clone())
                                    .or_default()
                                    .insert(key.clone());
                                match instance_async.get(&callee_key) {
                                    Some(flag) => *flag,
                                    None => {
                                        // Discover the callee instance;
                                        // re-run this one when its flag
                                        // lands.
                                        instance_async.insert(callee_key.clone(), false);
                                        origins.insert(callee_key.clone(), call.call_id);
                                        if queued.insert(callee_key.clone()) {
                                            pending.push(callee_key);
                                        }
                                        false
                                    }
                                }
                            }
                        }
                        CallTarget::External(callee) => async_set.contains(&callee),
                        CallTarget::Indirect(
                            IndirectReason::GenericMember | IndirectReason::TraitDispatch,
                        ) => dispatch_candidates(program, call.call_id)
                            .iter()
                            .any(|candidate| async_set.contains(candidate)),
                        _ => {
                            subject_adapted_here(program, held_values, &flags, bits, call.call_id)
                                || program.function_calls.get(&call.call_id).is_some_and(
                                    |function_call| {
                                        subject_awaits(
                                            program,
                                            function_call.subject_id,
                                            held_values,
                                            async_set,
                                            0,
                                        )
                                    },
                                )
                        }
                    };
                    if awaits && !flags[member] {
                        flags.insert(*member, true);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let root_flag = flags[&root];
        let previous = instance_async.insert(key.clone(), root_flag);
        if root_flag && previous != Some(true) {
            // The flag flipped on: dependents must re-evaluate.
            for dependent in dependents.get(&key).cloned().unwrap_or_default() {
                if queued.insert(dependent.clone()) {
                    pending.push(dependent);
                }
            }
        }
        // A base instance's flips are ordinary coloring: the plain emission
        // of this function/those closures is async.
        if bits.is_empty() {
            for (member, flag) in &flags {
                if *flag {
                    async_set.insert(*member);
                }
            }
        }
    }

    // --- Final pass: with every flag stable, collect each instance's
    // emission decisions and the context-dependent diagnostics.
    let mut instances: HashMap<InstanceKey, crate::analyzer::AdaptedInstance> = HashMap::new();
    let mut diagnostics: Vec<(crate::span::Span, String, Option<crate::error::Note>)> = Vec::new();
    let mut reported: HashSet<(Id, Id)> = HashSet::new();
    let keys: Vec<InstanceKey> = instance_async.keys().cloned().collect();
    for key in keys {
        let (root, ref bits) = key;
        let members = component(root);
        // Recompute the stable flags for this context (cheap; flags are
        // final so one pass over the loop from the worklist would converge
        // identically — reuse the same evaluation by seeding from the
        // global set and iterating).
        let mut flags: HashMap<Id, bool> = members
            .iter()
            .map(|member| (*member, async_set.contains(member)))
            .collect();
        loop {
            let mut changed = false;
            for member in &members {
                if flags[member] {
                    continue;
                }
                let awaits = graph
                    .calls_of(*member)
                    .iter()
                    .any(|call| match call.target {
                        CallTarget::Function(callee) => {
                            async_set.contains(&callee) || {
                                let callee_bits = bits_for_call(
                                    program,
                                    held_values,
                                    async_set,
                                    &adaptive,
                                    &flags,
                                    bits,
                                    callee,
                                    call.call_id,
                                );
                                !callee_bits.is_empty()
                                    && instance_async.get(&(callee, callee_bits)).copied()
                                        == Some(true)
                            }
                        }
                        CallTarget::External(callee) => async_set.contains(&callee),
                        CallTarget::Indirect(
                            IndirectReason::GenericMember | IndirectReason::TraitDispatch,
                        ) => dispatch_candidates(program, call.call_id)
                            .iter()
                            .any(|candidate| async_set.contains(candidate)),
                        _ => {
                            subject_adapted_here(program, held_values, &flags, bits, call.call_id)
                                || program.function_calls.get(&call.call_id).is_some_and(
                                    |function_call| {
                                        subject_awaits(
                                            program,
                                            function_call.subject_id,
                                            held_values,
                                            async_set,
                                            0,
                                        )
                                    },
                                )
                        }
                    });
                if awaits {
                    flags.insert(*member, true);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let mut info = crate::analyzer::AdaptedInstance {
            is_async: flags[&root],
            ..Default::default()
        };
        for member in &members {
            if *flags.get(member).unwrap_or(&false) && !bits.is_empty() && *member != root {
                info.async_closures.insert(*member);
            }
            for call in graph.calls_of(*member) {
                match call.target {
                    CallTarget::Function(callee) => {
                        let callee_bits = bits_for_call(
                            program,
                            held_values,
                            async_set,
                            &adaptive,
                            &flags,
                            bits,
                            callee,
                            call.call_id,
                        );
                        if !callee_bits.is_empty() {
                            let callee_async =
                                instance_async.get(&(callee, callee_bits.clone())).copied()
                                    == Some(true);
                            if callee_async {
                                info.awaited_calls.insert(call.call_id);
                            }
                            info.callee_bits.insert(call.call_id, callee_bits);
                        }
                        // Transitive `sync`/host violations: an argument
                        // that is async ONLY through this instance's bits,
                        // flowing into a refusing position.
                        sync_violations_at(
                            program,
                            held_values,
                            async_set,
                            &flags,
                            bits,
                            callee,
                            call.call_id,
                            origins.get(&key).copied(),
                            &mut reported,
                            &mut diagnostics,
                        );
                    }
                    CallTarget::External(callee) => {
                        extern_violations_at(
                            program,
                            held_values,
                            async_set,
                            &flags,
                            bits,
                            callee,
                            call.call_id,
                            origins.get(&key).copied(),
                            &mut reported,
                            &mut diagnostics,
                        );
                    }
                    CallTarget::Indirect(
                        IndirectReason::GenericMember | IndirectReason::TraitDispatch,
                    ) => {
                        dispatch_refusals_at(
                            program,
                            held_values,
                            async_set,
                            &flags,
                            bits,
                            call.call_id,
                            &mut reported,
                            &mut diagnostics,
                        );
                    }
                    _ => {}
                }
                if subject_adapted_here(program, held_values, &flags, bits, call.call_id) {
                    info.awaited_calls.insert(call.call_id);
                }
            }
        }
        // Base instances only earn an entry when they carry emission data;
        // adapted instances always do (the caller emits them by key).
        if !bits.is_empty()
            || !info.awaited_calls.is_empty()
            || !info.callee_bits.is_empty()
            || !info.async_closures.is_empty()
        {
            instances.insert(key, info);
        }
    }

    Adaptation {
        instances,
        diagnostics,
    }
}

/// Whether the call's subject is async THROUGH this instance's context: a
/// call through an adapted parameter, or through a binding holding a closure
/// that is async under these bits.
fn subject_adapted_here(
    program: &Program,
    held_values: &HashMap<Id, Vec<Id>>,
    flags: &HashMap<Id, bool>,
    bits: &[Id],
    call_id: Id,
) -> bool {
    program
        .function_calls
        .get(&call_id)
        .and_then(|function_call| program.entity_map.get(&function_call.subject_id))
        .is_some_and(|subject| match subject {
            Expr::Local(target) => {
                bits.contains(target)
                    || held_values.get(target).is_some_and(|held| {
                        held.iter().any(|held_id| {
                            matches!(
                                program.entity_map.get(held_id),
                                Some(Expr::Closure(closure_id))
                                    if flags.get(closure_id).copied().unwrap_or(false)
                            )
                        })
                    })
            }
            _ => false,
        })
}

/// Whether the parameter's type is a closure with a RESOLVED, non-void
/// return — the shape that adapts (or, marked, contracts).
fn closure_return_is_value(program: &Program, parameter_id: Id) -> bool {
    let Some(parameter) = program.parameters.get(&parameter_id) else {
        return false;
    };
    let Some(Type::Closure(_, return_type)) =
        program.type_id_to_type_map.get(&parameter.type_id).cloned()
    else {
        return false;
    };
    !matches!(
        program.type_id_to_type_map.get(&return_type),
        Some(Type::Void) | Some(Type::Unknown) | Some(Type::Unresolved) | None
    )
}

/// The span of an entity, or an empty fallback.
fn span_of(program: &Program, id: Id) -> crate::span::Span {
    program
        .span_map
        .get(&id)
        .map(|span| **span)
        .unwrap_or(crate::span::Span { start: 0, end: 0 })
}

/// Transitive `sync` violations at one call: an argument async ONLY through
/// the instance's bits flowing into a `sync`-contract parameter. (The direct
/// case — async at the call site itself — is the global divergence check's.)
#[allow(clippy::too_many_arguments)]
fn sync_violations_at(
    program: &Program,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &HashSet<Id>,
    flags: &HashMap<Id, bool>,
    bits: &[Id],
    callee: Id,
    call_id: Id,
    origin: Option<Id>,
    reported: &mut HashSet<(Id, Id)>,
    diagnostics: &mut Vec<(crate::span::Span, String, Option<crate::error::Note>)>,
) {
    let Some(function) = program.functions.get(&callee) else {
        return;
    };
    let Some(function_call) = program.function_calls.get(&call_id) else {
        return;
    };
    let empty_flags = HashMap::new();
    for (argument, parameter) in function_call.argument_ids.iter().zip(&function.parameters) {
        if !program.sync_values.contains(parameter) || !closure_return_is_value(program, *parameter)
        {
            continue;
        }
        if !value_async_in(program, held_values, async_set, flags, bits, *argument) {
            continue;
        }
        // Direct violations report at the global check with the plain steer.
        if value_async_in(
            program,
            held_values,
            async_set,
            &empty_flags,
            &[],
            *argument,
        ) {
            continue;
        }
        if !reported.insert((call_id, *parameter)) {
            continue;
        }
        let parameter_name = program
            .parameters
            .get(parameter)
            .map(|parameter| parameter.name)
            .unwrap_or("the parameter");
        let primary = origin.unwrap_or(call_id);
        let note = (primary != call_id).then(|| crate::error::Note {
            span: span_of(program, call_id),
            msg: format!("forwarded into the `sync` parameter `{parameter_name}` here"),
            source: program.source_of(call_id),
        });
        diagnostics.push((
            span_of(program, primary),
            format!(
                "this call passes an async closure that reaches `{parameter_name}`, which requires a synchronous closure (`sync`) — move the async work outside the callback (e.g. a `turn` with an awaiting body, `Draft`, or a spawned `async` block)"
            ),
            note,
        ));
    }
}

/// Transitive host-boundary violations: an argument async only through the
/// instance's bits flowing into an `external` function's value-returning
/// closure parameter.
#[allow(clippy::too_many_arguments)]
fn extern_violations_at(
    program: &Program,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &HashSet<Id>,
    flags: &HashMap<Id, bool>,
    bits: &[Id],
    callee: Id,
    call_id: Id,
    origin: Option<Id>,
    reported: &mut HashSet<(Id, Id)>,
    diagnostics: &mut Vec<(crate::span::Span, String, Option<crate::error::Note>)>,
) {
    let Some(external) = program.external_functions.get(&callee) else {
        return;
    };
    let Some(function_call) = program.function_calls.get(&call_id) else {
        return;
    };
    let empty_flags = HashMap::new();
    for (argument, parameter) in function_call.argument_ids.iter().zip(&external.parameters) {
        // The typed channel: a declared `async |…| T` parameter means the
        // host awaits the closure itself (`__nursery_run`'s body parameter).
        if program.async_values.contains(parameter) {
            continue;
        }
        if !closure_return_is_value(program, *parameter) {
            continue;
        }
        if !value_async_in(program, held_values, async_set, flags, bits, *argument) {
            continue;
        }
        if value_async_in(
            program,
            held_values,
            async_set,
            &empty_flags,
            &[],
            *argument,
        ) {
            continue;
        }
        if !reported.insert((call_id, *parameter)) {
            continue;
        }
        let primary = origin.unwrap_or(call_id);
        let note = (primary != call_id).then(|| crate::error::Note {
            span: span_of(program, call_id),
            msg: format!("forwarded to the host function `{}` here", external.name),
            source: program.source_of(call_id),
        });
        diagnostics.push((
            span_of(program, primary),
            format!(
                "this call passes an async closure that reaches the host (`external`) function `{}`, which cannot await a vilan closure — only synchronous closures can cross",
                external.name
            ),
            note,
        ));
    }
}

/// Adaptation cannot ride a trait/generic dispatch (the callee varies per
/// instantiation) — an async closure argument at such a call is refused
/// unless every candidate's parameter at that position is `async`-declared
/// (the typed channel, which works today).
#[allow(clippy::too_many_arguments)]
fn dispatch_refusals_at(
    program: &Program,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &HashSet<Id>,
    flags: &HashMap<Id, bool>,
    bits: &[Id],
    call_id: Id,
    reported: &mut HashSet<(Id, Id)>,
    diagnostics: &mut Vec<(crate::span::Span, String, Option<crate::error::Note>)>,
) {
    let Some(function_call) = program.function_calls.get(&call_id) else {
        return;
    };
    let candidates = dispatch_candidates(program, call_id);
    if candidates.is_empty() {
        return;
    }
    for (position, argument) in function_call.argument_ids.iter().enumerate() {
        if !value_async_in(program, held_values, async_set, flags, bits, *argument) {
            continue;
        }
        // Refuse only where a candidate would need ADAPTATION: a plain,
        // value-returning closure parameter at this position. All-`async`
        // candidates are the typed channel and work as-is.
        let needs_adaptation = candidates.iter().any(|candidate| {
            program
                .functions
                .get(candidate)
                .and_then(|function| function.parameters.get(position))
                .is_some_and(|parameter| {
                    !program.async_values.contains(parameter)
                        && !program.sync_values.contains(parameter)
                        && closure_return_is_value(program, *parameter)
                })
        });
        if !needs_adaptation {
            continue;
        }
        if !reported.insert((call_id, Id(position as u32))) {
            continue;
        }
        diagnostics.push((
            span_of(program, call_id),
            "an async closure cannot adapt a trait/generic-dispatched call (the concrete callee varies per instantiation) — bind the callee concretely, or declare the trait parameter `async || T`".to_string(),
            None,
        ));
    }
}

/// Whether `value_id` is an async closure VALUE in an instance's context:
/// the global channels (typed, adopted holds, fields, returning calls), an
/// adapted parameter of the enclosing instance, or a closure literal whose
/// flag (component-local or global) is set.
fn value_async_in(
    program: &Program,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &HashSet<Id>,
    flags: &HashMap<Id, bool>,
    bits: &[Id],
    value_id: Id,
) -> bool {
    match program.entity_map.get(&value_id) {
        Some(Expr::Closure(closure_id)) | Some(Expr::Async(closure_id)) => flags
            .get(closure_id)
            .copied()
            .unwrap_or_else(|| async_set.contains(closure_id)),
        Some(Expr::Local(target)) => {
            bits.contains(target)
                || program.async_values.contains(target)
                || held_values.get(target).is_some_and(|held| {
                    held.iter()
                        .any(|held_id| match program.entity_map.get(held_id) {
                            Some(Expr::Closure(closure_id)) => flags
                                .get(closure_id)
                                .copied()
                                .unwrap_or_else(|| async_set.contains(closure_id)),
                            Some(Expr::Field(_, struct_id, index)) => {
                                program.async_fields.contains(&(*struct_id, *index))
                            }
                            Some(Expr::Call(inner)) => call_returns_async_closure(program, *inner),
                            _ => false,
                        })
                })
        }
        Some(Expr::Field(_, struct_id, index)) => {
            program.async_fields.contains(&(*struct_id, *index))
        }
        Some(Expr::Call(inner)) => call_returns_async_closure(program, *inner),
        _ => false,
    }
}

/// The callee's adapted-parameter set for one call, in an instance's
/// context — empty when nothing adapts.
#[allow(clippy::too_many_arguments)]
fn bits_for_call(
    program: &Program,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &HashSet<Id>,
    adaptive: &HashMap<Id, HashSet<Id>>,
    flags: &HashMap<Id, bool>,
    bits: &[Id],
    callee: Id,
    call_id: Id,
) -> Vec<Id> {
    let Some(adaptive_params) = adaptive.get(&callee) else {
        return Vec::new();
    };
    let Some(function) = program.functions.get(&callee) else {
        return Vec::new();
    };
    let Some(function_call) = program.function_calls.get(&call_id) else {
        return Vec::new();
    };
    let mut callee_bits: Vec<Id> = function_call
        .argument_ids
        .iter()
        .zip(&function.parameters)
        .filter(|(argument, parameter)| {
            adaptive_params.contains(*parameter)
                && value_async_in(program, held_values, async_set, flags, bits, **argument)
        })
        .map(|(_, parameter)| *parameter)
        .collect();
    callee_bits.sort_by_key(|parameter| parameter.0);
    callee_bits
}

/// The plain (single-bit) async fixpoint: a node that calls an async
/// function/extern/value implicitly awaits it, so it is async too. Run to
/// stability; interleaved with the adaptation worklist by `infer`.
fn base_fixpoint(
    program: &Program,
    graph: &CallGraph,
    held_values: &HashMap<Id, Vec<Id>>,
    async_set: &mut HashSet<Id>,
) {
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
                // point — asyncness rides the type (J2), or the VALUE FLOW:
                // a binding holding an async closure, an async field read, an
                // async-returning call, a directly-applied async closure
                // literal (the lowered `run` body). Other higher-order calls
                // stay conservative (the concrete target isn't recoverable),
                // as do variant constructors.
                _ => program
                    .function_calls
                    .get(&call.call_id)
                    .is_some_and(|function_call| {
                        subject_awaits(program, function_call.subject_id, held_values, async_set, 0)
                    }),
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
}

/// A function's ADAPTIVE parameters: plain (unmarked), closure-typed, with a
/// RESOLVED non-void return. `sync`/`async`-marked and void/unresolved ones
/// never adapt (contract, spawn, or no known lie).
fn adaptive_params_of(program: &Program) -> HashMap<Id, HashSet<Id>> {
    let mut adaptive: HashMap<Id, HashSet<Id>> = HashMap::new();
    for (function_id, function) in &program.functions {
        let params: HashSet<Id> = function
            .parameters
            .iter()
            .filter(|parameter| {
                !program.sync_values.contains(parameter)
                    && !program.async_values.contains(parameter)
                    && closure_return_is_value(program, **parameter)
            })
            .copied()
            .collect();
        if !params.is_empty() {
            adaptive.insert(*function_id, params);
        }
    }
    adaptive
}
