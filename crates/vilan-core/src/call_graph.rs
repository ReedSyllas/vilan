//! A static call graph over the analyzed program: which functions and closures
//! each function/closure calls. Built from the resolved [`Program`] after type
//! checking, it is the foundation for interprocedural effect analyses — the
//! `context` value-threading pass, and later async inference. Both ask the same
//! question, "which functions transitively reach a leaf site?", which is just
//! backward reachability over these edges.
//!
//! Edges are only as precise as call resolution allows. A call through a value
//! (a closure or function held in a variable, parameter, or field), a generic
//! `T::member()` dispatch, or a trait-method re-dispatch can't be pinned to a
//! single callee statically; those are recorded as [`CallTarget::Indirect`] so a
//! consuming pass can refuse to thread through them rather than silently miss
//! them.
//!
//! Granularity is deliberately pre-monomorphization: a generic function is one
//! node, not one per instantiation. An effect is a property of the function, not
//! of a particular type substitution, so every instance inherits whatever the
//! pass decides for the single node.

use std::collections::{HashMap, HashSet};

use crate::analyzer::{Expr, ExprIfBranch, ExprPattern, GenericDispatch, Program};
use crate::id::Id;

/// What a single call site resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallTarget {
    /// A Vilan function with a body — effects propagate through it.
    Function(Id),
    /// An `external`/`[extern]` function: a leaf with no Vilan body. Async
    /// inference treats promise-returning externs as its effect sources.
    External(Id),
    /// An immediately-applied closure literal, `(|| ..)()`.
    Closure(Id),
    /// An enum variant constructor, `Some(x)` — builds a value, calls no body.
    /// Not an effect-bearing edge, but recorded so the graph stays faithful.
    Variant(Id),
    /// A call whose callee isn't statically known.
    Indirect(IndirectReason),
}

/// Why a call site couldn't be resolved to a concrete callee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndirectReason {
    /// Calling a function/closure value held in a variable, parameter, or field.
    Value,
    /// `T::member()` — the concrete member is chosen per monomorphized instance.
    GenericMember,
    /// A trait method re-dispatched to the receiver's concrete type at codegen.
    TraitDispatch,
}

/// A resolved call site within a function or closure body.
#[derive(Clone, Copy, Debug)]
pub struct Call {
    /// The `Expr::Call` entity id (also the key into [`Program::function_calls`]).
    pub call_id: Id,
    pub target: CallTarget,
}

/// A code-bearing graph node: a function or a closure. The distinction is only
/// for reporting — entity ids are globally unique either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Function(Id),
    Closure(Id),
}

impl Node {
    pub fn id(self) -> Id {
        match self {
            Node::Function(id) | Node::Closure(id) => id,
        }
    }
}

#[derive(Debug, Default)]
pub struct CallGraph {
    /// Forward edges: the calls each node makes, in source order.
    calls: HashMap<Id, Vec<Call>>,
    /// Reverse edges: for each function/closure, the nodes that call it. Only
    /// resolved `Function`/`Closure` targets appear; `External`, `Variant`, and
    /// `Indirect` targets have no caller-side entry.
    callers: HashMap<Id, Vec<Node>>,
    /// The lexical parent of each closure (the function/closure it was defined
    /// in), for the capture analysis the `context` pass needs. A closure created
    /// in a global initializer has no entry.
    closure_parent: HashMap<Id, Id>,
    /// The inverse of `closure_parent`, children in build order (deterministic
    /// — successor order feeds shortest-chain witness selection).
    closure_children: HashMap<Id, Vec<Id>>,
    /// Nodes whose own body directly contains an `await` (not inside a nested
    /// closure or `async` block). A seed for async inference.
    awaits: HashSet<Id>,
    /// Every node, in build order (functions, then closures).
    nodes: Vec<Node>,
    /// The module-level bindings each node (function, closure, or another
    /// binding's initializer) REFERENCES, as `(reference expression, binding)`
    /// pairs. A reference is what makes a binding's initializer run (F6: a
    /// dropped binding's initializer does not run), so these are the edges
    /// platform coloring and emission travel to reach initializer code.
    global_references: HashMap<Id, Vec<(Id, Id)>>,
    /// The FUNCTIONS each node references as a value (fn-to-closure
    /// coercion): `(reference expression, function)` pairs. A coerced
    /// function has no creation event for the creator rule, so the
    /// reference site carries the charge.
    function_references: HashMap<Id, Vec<(Id, Id)>>,
    /// The calls inside each module-level binding's initializer. Initializers
    /// are deliberately NOT `nodes()` — async inference doesn't visit them —
    /// so their calls live here instead of `calls`. A `const`-marked
    /// initializer runs in the compile-time interpreter, never on the build
    /// platform, and is skipped entirely.
    initializer_calls: HashMap<Id, Vec<Call>>,
    /// The closures created inside each module-level binding's initializer.
    /// Their `closure_parent_of` stays empty (as it always has); the binding
    /// itself is their creator for coloring purposes.
    initializer_closures: HashMap<Id, Vec<Id>>,
}

impl CallGraph {
    /// Builds the call graph for a fully analyzed program.
    pub fn build(program: &Program) -> CallGraph {
        let mut graph = CallGraph::default();
        let module_bindings: HashSet<Id> = program.module_level_bindings().into_iter().collect();
        let const_exprs: HashSet<Id> = program.const_exprs.iter().copied().collect();

        for (id, function) in &program.functions {
            // A signature-only trait method has no body to walk.
            if !function.has_body {
                continue;
            }
            graph.add_node(
                Node::Function(*id),
                program,
                &module_bindings,
                |collector| {
                    collector.walk_all(&function.body.0);
                    collector.walk(function.body.1);
                },
            );
        }

        // Every closure is collected during analysis, so they can be walked as
        // roots directly; the walk of their defining body only records the
        // lexical parent link (it does not descend into the closure).
        for (id, closure) in &program.closures {
            graph.add_node(Node::Closure(*id), program, &module_bindings, |collector| {
                collector.walk(closure.return_);
            });
        }

        // Module-level bindings: their initializers are code too — they run
        // (in order) when something reachable references the binding. Collect
        // each initializer's calls, created closures, and references to other
        // bindings, WITHOUT making the binding a `nodes()` entry (async
        // inference keeps its exact node set). A `const`-marked initializer is
        // evaluated by the compile-time interpreter and serialized as a value,
        // so at runtime it is data, not code — skipped.
        for binding in program.module_level_bindings() {
            let Some(initial) = program
                .variables
                .get(&binding)
                .and_then(|variable| variable.initial)
            else {
                continue;
            };
            if const_exprs.contains(&initial) {
                continue;
            }
            let mut collector = Collector {
                program,
                globals: &module_bindings,
                calls: Vec::new(),
                nested_closures: Vec::new(),
                global_references: Vec::new(),
                function_references: Vec::new(),
                has_await: false,
                visited: HashSet::new(),
            };
            collector.walk(initial);
            graph.initializer_calls.insert(binding, collector.calls);
            graph
                .initializer_closures
                .insert(binding, collector.nested_closures);
            graph
                .global_references
                .insert(binding, collector.global_references);
            graph
                .function_references
                .insert(binding, collector.function_references);
        }

        graph.build_reverse_edges();
        graph
    }

    /// Walks one node's body with a fresh collector, recording its forward
    /// edges and the parent link of any closure defined directly inside it.
    fn add_node(
        &mut self,
        node: Node,
        program: &Program,
        module_bindings: &HashSet<Id>,
        walk: impl FnOnce(&mut Collector),
    ) {
        self.nodes.push(node);
        let mut collector = Collector {
            program,
            globals: module_bindings,
            calls: Vec::new(),
            nested_closures: Vec::new(),
            global_references: Vec::new(),
            function_references: Vec::new(),
            has_await: false,
            visited: HashSet::new(),
        };
        walk(&mut collector);
        for closure_id in collector.nested_closures {
            self.closure_parent.insert(closure_id, node.id());
            self.closure_children
                .entry(node.id())
                .or_default()
                .push(closure_id);
        }
        if collector.has_await {
            self.awaits.insert(node.id());
        }
        self.calls.insert(node.id(), collector.calls);
        self.global_references
            .insert(node.id(), collector.global_references);
        self.function_references
            .insert(node.id(), collector.function_references);
    }

    fn build_reverse_edges(&mut self) {
        let mut callers: HashMap<Id, Vec<Node>> = HashMap::new();
        for node in &self.nodes {
            for call in &self.calls[&node.id()] {
                let callee = match call.target {
                    CallTarget::Function(id) | CallTarget::Closure(id) => id,
                    _ => continue,
                };
                callers.entry(callee).or_default().push(*node);
            }
        }
        self.callers = callers;
    }

    /// The calls a function or closure makes, in source order.
    pub fn calls_of(&self, id: Id) -> &[Call] {
        self.calls.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The functions and closures that call the given function or closure.
    pub fn callers_of(&self, id: Id) -> &[Node] {
        self.callers.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The function or closure a closure was lexically defined in, if any.
    pub fn closure_parent_of(&self, closure_id: Id) -> Option<Id> {
        self.closure_parent.get(&closure_id).copied()
    }

    /// The module-level bindings the given node references, as `(reference
    /// expression, binding)` pairs. Keyed by functions, closures, and other
    /// bindings' initializers alike.
    pub fn global_references_of(&self, id: Id) -> &[(Id, Id)] {
        self.global_references
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The functions the given node references as values (coercion sites),
    /// as `(reference expression, function)` pairs.
    pub fn function_references_of(&self, id: Id) -> &[(Id, Id)] {
        self.function_references
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The calls inside a module-level binding's (non-`const`) initializer.
    pub fn initializer_calls_of(&self, id: Id) -> &[Call] {
        self.initializer_calls
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The closures a node creates directly in its body, in build order.
    pub fn closure_children_of(&self, id: Id) -> Option<&[Id]> {
        self.closure_children.get(&id).map(Vec::as_slice)
    }

    /// The closures created inside a module-level binding's (non-`const`)
    /// initializer.
    pub fn initializer_closures_of(&self, id: Id) -> &[Id] {
        self.initializer_closures
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Everything `node` charges with its execution, as `(successor, site)`
    /// pairs — `site` is the call or reference expression that reaches the
    /// successor (`None` for a closure the node merely creates). One edge
    /// vocabulary: resolved callees and externs, every dispatch candidate of
    /// a trait/generic-bounded call (`async_infer`'s over-approximation), the
    /// module-level bindings the node references (F6 — referencing is what
    /// makes an initializer run), and the closures it creates. Platform
    /// coloring's admission walk and emission's binding reachability both
    /// consume THIS, so "what runs" has a single definition. A call through a
    /// closure *value* charges nothing here — the closure's body was charged
    /// to its creator.
    pub fn successors(&self, program: &Program, node: Id) -> Vec<(Id, Option<Id>)> {
        let mut successors = Vec::new();
        for call in self
            .calls_of(node)
            .iter()
            .chain(self.initializer_calls_of(node))
        {
            match call.target {
                CallTarget::Function(callee)
                | CallTarget::Closure(callee)
                | CallTarget::External(callee) => {
                    successors.push((callee, Some(call.call_id)));
                }
                CallTarget::Variant(_) => {}
                CallTarget::Indirect(IndirectReason::Value) => {}
                CallTarget::Indirect(_) => {
                    for candidate in crate::async_infer::dispatch_candidates(program, call.call_id)
                    {
                        successors.push((candidate, Some(call.call_id)));
                    }
                }
            }
        }
        for (reference, global) in self.global_references_of(node) {
            successors.push((*global, Some(*reference)));
        }
        for (reference, function) in self.function_references_of(node) {
            successors.push((*function, Some(*reference)));
        }
        if let Some(children) = self.closure_children.get(&node) {
            for closure in children {
                successors.push((*closure, None));
            }
        }
        for closure in self.initializer_closures_of(node) {
            successors.push((*closure, None));
        }
        successors
    }

    // (Binding reachability moved to `platform_color::reachable_bindings`,
    // which threads per-instantiation substitutions — admission, emission,
    // and the async-initializer gate all consume that one definition.)

    /// Every code-bearing node, in build order.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// A human-readable listing of the whole graph, for the `-d` debug dump.
    pub fn debug_dump(&self, program: &Program) -> String {
        let mut output = String::new();
        for node in &self.nodes {
            output.push_str(&self.describe_node(*node, program));
            output.push('\n');
            for call in &self.calls[&node.id()] {
                output.push_str("    -> ");
                output.push_str(&self.describe_target(call, program));
                output.push('\n');
            }
            let callers = self.callers_of(node.id());
            if !callers.is_empty() {
                let names = callers
                    .iter()
                    .map(|caller| self.node_label(*caller, program))
                    .collect::<Vec<_>>()
                    .join(", ");
                output.push_str(&format!("    (called by: {names})\n"));
            }
        }
        output
    }

    fn describe_node(&self, node: Node, program: &Program) -> String {
        match node {
            Node::Function(id) => {
                let name = program.functions.get(&id).map(|f| f.name).unwrap_or("?");
                format!("fun {name} (#{})", id.0)
            }
            Node::Closure(id) => match self.closure_parent.get(&id) {
                Some(parent) => {
                    format!("closure #{} (in {})", id.0, self.id_label(*parent, program))
                }
                None => format!("closure #{} (top-level)", id.0),
            },
        }
    }

    fn describe_target(&self, call: &Call, program: &Program) -> String {
        match call.target {
            CallTarget::Function(id) => {
                let name = program.functions.get(&id).map(|f| f.name).unwrap_or("?");
                format!("{name} (#{}) [function]", id.0)
            }
            CallTarget::External(id) => {
                let name = program
                    .external_functions
                    .get(&id)
                    .map(|f| f.name)
                    .unwrap_or("?");
                format!("{name} (#{}) [external]", id.0)
            }
            CallTarget::Closure(id) => format!("closure #{} [closure]", id.0),
            CallTarget::Variant(id) => format!("#{} [variant constructor]", id.0),
            CallTarget::Indirect(reason) => {
                format!("<indirect: {reason:?}> (call #{})", call.call_id.0)
            }
        }
    }

    /// Whether the given function/closure's own body directly contains an
    /// `await` (not inside a nested closure or `async` block).
    pub fn node_awaits(&self, id: Id) -> bool {
        self.awaits.contains(&id)
    }

    fn node_label(&self, node: Node, program: &Program) -> String {
        self.id_label(node.id(), program)
    }

    fn id_label(&self, id: Id, program: &Program) -> String {
        match program.functions.get(&id) {
            Some(function) => format!("{} #{}", function.name, id.0),
            None => format!("closure #{}", id.0),
        }
    }
}

/// Walks the expression tree of a single node, accumulating its call edges and
/// the closures defined directly inside it. Nested functions and closures are
/// recorded but not descended into — each is a node walked from its own root.
struct Collector<'a, 'src> {
    program: &'a Program<'src>,
    /// The program's module-level bindings — a use of one is recorded as a
    /// reference edge rather than descended into (the binding's initializer
    /// is its own collection unit).
    globals: &'a HashSet<Id>,
    calls: Vec<Call>,
    nested_closures: Vec<Id>,
    global_references: Vec<(Id, Id)>,
    function_references: Vec<(Id, Id)>,
    has_await: bool,
    visited: HashSet<Id>,
}

impl<'a, 'src> Collector<'a, 'src> {
    fn walk_all(&mut self, ids: &[Id]) {
        for id in ids {
            self.walk(*id);
        }
    }

    fn walk(&mut self, id: Id) {
        // Entity ids form a tree, but guard against any shared sub-expression
        // so a single walk can't loop.
        if !self.visited.insert(id) {
            return;
        }
        let Some(expr) = self.program.entity_map.get(&id) else {
            return;
        };
        match expr {
            Expr::Call(call_id) => {
                let target = resolve_target(self.program, *call_id);
                self.calls.push(Call {
                    call_id: *call_id,
                    target,
                });
                // The arguments and (for an indirect call) the subject can hold
                // further calls; walk them, but never the resolved callee body.
                if let Some(function_call) = self.program.function_calls.get(call_id) {
                    // A subject that merely NAMES a function (a direct call, a
                    // wired method subject) is not a function VALUE — recording
                    // it as a coercion reference would add a context-free edge
                    // to the callee, defeating the per-instantiation walk (the
                    // call edge above already carries the charge, WITH its
                    // bindings). Pre-mark it visited so the subject walk skips.
                    let subject_names_a_function =
                        match self.program.entity_map.get(&function_call.subject_id) {
                            Some(Expr::Local(binding)) => {
                                self.program.functions.contains_key(binding)
                            }
                            Some(Expr::Function(_)) => true,
                            _ => false,
                        };
                    if subject_names_a_function {
                        self.visited.insert(function_call.subject_id);
                    }
                    self.walk(function_call.subject_id);
                    self.walk_all(&function_call.argument_ids);
                }
            }
            // A nested closure / function is its own node; record the closure's
            // parent link, but don't fold its calls into this node's. An `async`
            // block lowers to such a closure — it's a separate (always-async)
            // node, so its awaits don't make this node async.
            Expr::Closure(closure_id) | Expr::Async(closure_id) => {
                self.nested_closures.push(*closure_id)
            }
            // An `await` makes this node async; its operand may hold more calls.
            Expr::Await(inner) => {
                self.has_await = true;
                self.walk(*inner);
            }
            // A local binding's calls live in its initializer. A MODULE-LEVEL
            // binding's don't fold in here: using it is recorded as a
            // reference edge, and the initializer is collected once as the
            // binding's own unit (its calls must not inherit this node's
            // async/effect context, nor be re-collected per referencer).
            Expr::Variable(variable_id) => {
                if self.globals.contains(variable_id) {
                    self.global_references.push((id, *variable_id));
                } else if let Some(initial) = self
                    .program
                    .variables
                    .get(variable_id)
                    .and_then(|variable| variable.initial)
                {
                    self.walk(initial);
                }
            }
            // A use of a module-level binding is what makes its initializer
            // run (F6) — an edge, for coloring and emission both. A `Local`
            // can also name a FUNCTION (an import bound to a name, a wired
            // method-call subject): that's a function reference, charged like
            // the `Expr::Function` form below.
            Expr::Local(binding) | Expr::Parameter(binding) => {
                if self.globals.contains(binding) {
                    self.global_references.push((id, *binding));
                } else if self.program.functions.contains_key(binding) {
                    self.function_references.push((id, *binding));
                }
            }
            Expr::Field(subject_id, _, _) | Expr::TupleIndex(subject_id, _, _) => {
                self.walk(*subject_id)
            }
            Expr::Index(subject_id, index_id) => {
                self.walk(*subject_id);
                self.walk(*index_id);
            }
            Expr::TupleComprehension(first, second, third) => {
                self.walk(*first);
                self.walk(*second);
                self.walk(*third);
            }
            Expr::Destructure(subject_id, pattern) => {
                self.walk(*subject_id);
                self.walk_pattern(pattern);
            }
            // A function referenced as a VALUE (fn-to-closure coercion,
            // proposal/fn-coercion.md): unlike a closure, it has no creation
            // event for the creator rule to charge, so the reference site is
            // the charge — analogous to a module-level binding's reference.
            // (Later calls through the value stay `Indirect(Value)`.)
            Expr::Function(function_id) => {
                self.function_references.push((id, *function_id));
            }
            Expr::FunctionReturn(Some(value_id)) => self.walk(*value_id),
            Expr::FunctionReturn(None) => {}
            Expr::TryAssert(receiver_id) => self.walk(*receiver_id),
            Expr::Lift(subject_id, _, continuation_id) => {
                self.walk(*subject_id);
                self.walk(*continuation_id);
            }
            Expr::LiftBinder => {}
            Expr::Binary(_, lhs, rhs) => {
                self.walk(*lhs);
                self.walk(*rhs);
            }
            Expr::Unary(_, operand) | Expr::Reference(operand, _) | Expr::Dereference(operand) => {
                self.walk(*operand)
            }
            Expr::Assignment(target_id, value_id) => {
                self.walk(*target_id);
                self.walk(*value_id);
            }
            Expr::Block((statements, tail)) => {
                self.walk_all(statements);
                self.walk(*tail);
            }
            Expr::For(condition, (statements, tail)) => {
                if let Some(condition) = condition {
                    self.walk(*condition);
                }
                self.walk_all(statements);
                self.walk(*tail);
            }
            Expr::ForEach(iterable, _item, (statements, tail)) => {
                // An iterator-protocol loop calls the resolved `next` on every
                // pass — a real call edge, anchored at the loop itself.
                if let Some(&next_id) = self.program.for_each_next.get(&id) {
                    self.calls.push(Call {
                        call_id: id,
                        target: CallTarget::Function(next_id),
                    });
                }
                self.walk(*iterable);
                self.walk_all(statements);
                self.walk(*tail);
            }
            Expr::If(branch) => self.walk_if(branch),
            Expr::Is(subject_id, pattern) => {
                self.walk(*subject_id);
                self.walk_pattern(pattern);
            }
            Expr::Match(subject_id, legs) => {
                self.walk(*subject_id);
                for leg in legs {
                    self.walk_pattern(&leg.pattern);
                    if let Some(guard) = leg.guard {
                        self.walk(guard);
                    }
                    self.walk(leg.body);
                }
            }
            Expr::List(ids) | Expr::Tuple(ids) => self.walk_all(ids),
            Expr::StructInitializer(_, fields) => {
                for value_id in fields.values() {
                    self.walk(*value_id);
                }
            }
            // Leaves and declarations — nothing nested to charge. Listed
            // explicitly (no catch-all): a NEW `Expr` variant must be
            // classified here, or call/reference collection silently
            // under-sees it — the `Index` blind spot shipped two miscompiles
            // before this list existed.
            Expr::Bool(_)
            | Expr::Enum(_)
            | Expr::EnumVariant(_, _)
            | Expr::Error
            | Expr::ExternalFunction(_)
            | Expr::Generic(_)
            | Expr::Impl(_)
            | Expr::Jump(_)
            | Expr::Module(_)
            | Expr::Null
            | Expr::Number(_, _, _)
            | Expr::String(_)
            | Expr::MultilineString(_)
            | Expr::Struct(_)
            | Expr::Trait(_)
            | Expr::Void
            | Expr::Macro => {}
        }
    }

    fn walk_if(&mut self, branch: &ExprIfBranch) {
        match branch {
            ExprIfBranch::If(condition, (statements, tail), else_) => {
                self.walk(*condition);
                self.walk_all(statements);
                self.walk(*tail);
                if let Some(else_) = else_ {
                    self.walk_if(else_);
                }
            }
            ExprIfBranch::Else((statements, tail)) => {
                self.walk_all(statements);
                self.walk(*tail);
            }
        }
    }

    fn walk_pattern(&mut self, pattern: &ExprPattern) {
        match pattern {
            ExprPattern::Literal(id) => self.walk(*id),
            ExprPattern::Variant(_, _, sub_patterns) => {
                for sub_pattern in sub_patterns {
                    self.walk_pattern(sub_pattern);
                }
            }
            ExprPattern::Tuple(sub_patterns) => {
                for (sub_pattern, _width) in sub_patterns {
                    self.walk_pattern(sub_pattern);
                }
            }
            ExprPattern::Wildcard | ExprPattern::Binding(_) => {}
        }
    }
}

/// Resolves what a call site invokes, mirroring the transformer's dispatch.
fn resolve_target(program: &Program, call_id: Id) -> CallTarget {
    let Some(function_call) = program.function_calls.get(&call_id) else {
        return CallTarget::Indirect(IndirectReason::Value);
    };
    // Codegen re-dispatches these per monomorphized instance, so the concrete
    // callee isn't fixed at this granularity. The dispatch record is keyed by the
    // call's subject (a `T::member` static accessor / field-projected generic) OR
    // by the call id (an instance method call on a generic-bounded receiver, or an
    // `OnType` re-dispatch) — the transformer checks both, so this must too, or an
    // instance dispatch is mistaken for a direct call to the trait's signature.
    for key in [call_id, function_call.subject_id] {
        match program.generic_dispatch.get(&key) {
            Some(GenericDispatch::OnConstraint(..)) => {
                return CallTarget::Indirect(IndirectReason::GenericMember);
            }
            Some(GenericDispatch::OnType(..)) => {
                return CallTarget::Indirect(IndirectReason::TraitDispatch);
            }
            None => {}
        }
    }
    match program.entity_map.get(&function_call.subject_id) {
        Some(Expr::Local(target_id)) => classify_target(program, *target_id),
        // An immediately-applied closure literal, `(|| ..)()`.
        Some(Expr::Closure(closure_id)) => CallTarget::Closure(*closure_id),
        // The subject is some other expression (a returned value, a field, the
        // result of another call): an indirect call through that value.
        _ => CallTarget::Indirect(IndirectReason::Value),
    }
}

/// Classifies the entity a call's `Expr::Local` subject points at.
fn classify_target(program: &Program, target_id: Id) -> CallTarget {
    if program.external_functions.contains_key(&target_id) {
        CallTarget::External(target_id)
    } else if program.functions.contains_key(&target_id) {
        CallTarget::Function(target_id)
    } else if matches!(
        program.entity_map.get(&target_id),
        Some(Expr::EnumVariant(..))
    ) {
        CallTarget::Variant(target_id)
    } else {
        // A variable or parameter holding a function/closure value.
        CallTarget::Indirect(IndirectReason::Value)
    }
}
