//! The `const` pass (proposal/const-eval.md): evaluates `const`-marked
//! expressions post-analysis with the macro interpreter, in dependency order,
//! producing plain-data results the transformer serializes in place — plus
//! spanned diagnostics for everything that cannot evaluate. Free variables of
//! a const expression must be compile-time-known: an item (function, struct,
//! enum), or an immutable binding whose initializer is a literal or another
//! `const` expression.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::analyzer::{Expr, Program};
use crate::call_graph::{Call, CallGraph, CallTarget};
use crate::error::Error;
use crate::id::Id;
use crate::interpreter::{self, ConstValue, Limits};
use crate::options::BuildOptions;
use crate::span::Span;
use crate::transformer;

pub fn evaluate(
    program: &Program,
    options: &BuildOptions,
) -> (HashMap<Id, ConstValue>, Vec<(String, String)>, Vec<Error>) {
    let mut state = State {
        program,
        options,
        const_set: program.const_exprs.iter().copied().collect(),
        results: HashMap::new(),
        assets: Vec::new(),
        failed: HashSet::new(),
        in_progress: HashSet::new(),
        errors: Vec::new(),
    };
    state.check_const_only();
    for &expr_id in &program.const_exprs {
        state.evaluate_one(expr_id);
    }
    (state.results, state.assets, state.errors)
}

/// Deduplicates and deterministically orders the collected `(kind, line)`
/// pairs into per-kind file contents (newline-terminated). Lines sort
/// lexically — which is SOUND for the CSS the styling system emits: `.class`
/// rules ('.' = 0x2E) sort before `@media` blocks ('@' = 0x40), so media
/// rules take the later cascade position they need, and pseudo-class rules
/// don't compete with base rules on cascade order at all (their classes are
/// distinct and their specificity is higher).
pub fn assemble_assets(assets: &[(String, String)]) -> BTreeMap<String, String> {
    let mut by_kind: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (kind, line) in assets {
        by_kind.entry(kind).or_default().insert(line);
    }
    by_kind
        .into_iter()
        .map(|(kind, lines)| {
            let mut content = lines.into_iter().collect::<Vec<_>>().join("\n");
            content.push('\n');
            (kind.to_string(), content)
        })
        .collect()
}

struct State<'p, 'src> {
    program: &'p Program<'src>,
    options: &'p BuildOptions,
    const_set: HashSet<Id>,
    results: HashMap<Id, ConstValue>,
    assets: Vec<(String, String)>,
    failed: HashSet<Id>,
    in_progress: HashSet<Id>,
    errors: Vec<Error>,
}

/// How a const expression's free variable is (or isn't) compile-time-known.
enum Known<'src> {
    /// An item or a literal-initialized immutable binding — usable as-is.
    Ok,
    /// An immutable binding whose initializer is a `const` expression:
    /// evaluate that first.
    Const(Id),
    /// A runtime value — an error at the reference.
    Runtime(&'src str),
}

impl<'p, 'src> State<'p, 'src> {
    fn evaluate_one(&mut self, expr_id: Id) -> bool {
        if self.results.contains_key(&expr_id) {
            return true;
        }
        if self.failed.contains(&expr_id) {
            return false;
        }
        if !self.in_progress.insert(expr_id) {
            self.errors.push(Error {
                span: self.span_of(expr_id),
                msg: "`const` expressions form a dependency cycle".to_string(),
            });
            self.failed.insert(expr_id);
            return false;
        }
        let ok = self.evaluate_inner(expr_id);
        self.in_progress.remove(&expr_id);
        if !ok {
            self.failed.insert(expr_id);
        }
        ok
    }

    fn evaluate_inner(&mut self, expr_id: Id) -> bool {
        // The free-variable rule, with precise spans at each reference.
        let mut ok = true;
        let free = self.free_locals(expr_id);
        let external: HashSet<Id> = free.iter().map(|(_, binding)| *binding).collect();
        for (reference_id, binding) in free {
            match self.classify(binding) {
                Known::Ok => {}
                Known::Const(dependency) => {
                    if !self.evaluate_one(dependency) {
                        ok = false;
                    }
                }
                Known::Runtime(name) => {
                    self.errors.push(Error {
                        span: self.span_of(reference_id),
                        msg: format!(
                            "`{name}` is a runtime value; a `const` expression reads only \
                             compile-time-known bindings"
                        ),
                    });
                    ok = false;
                }
            }
        }
        if !ok {
            return false;
        }

        // Assemble the mini-program. Bindings reached through CALLED functions
        // surface as `unresolved` — const-initialized ones get evaluated and
        // the assembly retried; anything else is a diagnostic.
        let mut attempts = 0;
        loop {
            let (mini, unresolved) = transformer::transform_const_program(
                self.program,
                self.options,
                expr_id,
                &external,
                &self.results,
            );
            let mut retry = false;
            for binding in &unresolved {
                match self.classify(*binding) {
                    Known::Ok => {}
                    Known::Const(dependency) => {
                        if self.evaluate_one(dependency) {
                            retry = true;
                        } else {
                            ok = false;
                        }
                    }
                    Known::Runtime(name) => {
                        self.errors.push(Error {
                            span: self.span_of(expr_id),
                            msg: format!(
                                "this `const` expression reaches `{name}`, whose value is not \
                                 compile-time-known"
                            ),
                        });
                        ok = false;
                    }
                }
            }
            if !ok {
                return false;
            }
            if retry && attempts < 4 {
                attempts += 1;
                continue;
            }
            return match interpreter::eval_const(&mini, Limits::default()) {
                Ok((value, assets)) => {
                    self.results.insert(expr_id, value);
                    self.assets.extend(assets);
                    true
                }
                Err(failure) => {
                    self.errors.push(Error {
                        span: self.span_of(expr_id),
                        msg: format!("const evaluation failed: {}", failure.message),
                    });
                    false
                }
            };
        }
    }

    /// The const-only capability check (const-eval.md §2): no RUNTIME call
    /// path may reach `asset::emit`. R = the functions/closures that reach it
    /// through call sites OUTSIDE `const` subtrees; roots (`main`, top-level
    /// initializers) never join R — a root's call into R is the offending
    /// boundary, reported at that call site. Indirect calls (closure values)
    /// are the recorded conservative gap.
    fn check_const_only(&mut self) {
        let Some(emit_id) = self.program.asset_emit_fn_id else {
            return;
        };
        let graph = CallGraph::build(self.program);
        let main_id = self
            .program
            .scopes
            .get(&self.program.global_scope_id)
            .and_then(|scope| scope.name_to_id_map.get("main").copied());

        // Seed: nodes calling `emit` directly through a non-const site.
        let mut in_r: HashSet<Id> = HashSet::new();
        let mut worklist: Vec<Id> = Vec::new();
        let mut boundary_errors: Vec<(Id, Id)> = Vec::new(); // (call site, callee)
        let mut owned_calls: HashSet<Id> = HashSet::new();
        for node in graph.nodes() {
            for call in graph.calls_of(node.id()) {
                owned_calls.insert(call.call_id);
                if !matches!(call.target, CallTarget::External(target) if target == emit_id) {
                    continue;
                }
                if self.in_const_subtree(call.call_id) {
                    continue;
                }
                if Some(node.id()) == main_id {
                    boundary_errors.push((call.call_id, emit_id));
                } else if in_r.insert(node.id()) {
                    worklist.push(node.id());
                }
            }
        }
        // Propagate to callers through non-const sites; roots never join.
        while let Some(member) = worklist.pop() {
            for caller in graph.callers_of(member) {
                let caller_id = caller.id();
                if in_r.contains(&caller_id) {
                    continue;
                }
                let sites: Vec<&Call> = graph
                    .calls_of(caller_id)
                    .iter()
                    .filter(|call| match call.target {
                        CallTarget::Function(target) | CallTarget::Closure(target) => {
                            target == member
                        }
                        _ => false,
                    })
                    .collect();
                for site in sites {
                    if self.in_const_subtree(site.call_id) {
                        continue;
                    }
                    if Some(caller_id) == main_id {
                        boundary_errors.push((site.call_id, member));
                    } else if in_r.insert(caller_id) {
                        worklist.push(caller_id);
                    }
                }
            }
        }
        // Top-level initializers own no graph node: a direct-call site outside
        // every node whose subject resolves to `emit` or an R-function is the
        // same boundary.
        for (call_id, function_call) in &self.program.function_calls {
            if owned_calls.contains(call_id) || self.in_const_subtree(*call_id) {
                continue;
            }
            let Some(Expr::Local(target)) = self.program.entity_map.get(&function_call.subject_id)
            else {
                continue;
            };
            if *target == emit_id || in_r.contains(target) {
                boundary_errors.push((*call_id, *target));
            }
        }
        boundary_errors.sort_by_key(|(site, _)| self.span_of(*site).start);
        boundary_errors.dedup();
        for (site, callee) in boundary_errors {
            let name = if callee == emit_id {
                "asset::emit".to_string()
            } else {
                self.program
                    .functions
                    .get(&callee)
                    .map(|function| format!("`{}` (it reaches `asset::emit`)", function.name))
                    .unwrap_or_else(|| "this call".to_string())
            };
            self.errors.push(Error {
                span: self.span_of(site),
                msg: format!(
                    "{name} is compile-time-only — evaluate this call inside a `const` \
                     expression"
                ),
            });
        }
    }

    /// Whether an entity sits inside any `const` expression's span (same
    /// source file) — the site test the capability check cuts edges by.
    fn in_const_subtree(&self, id: Id) -> bool {
        let Some(source) = self.program.source_of(id) else {
            return false;
        };
        let span = self.span_of(id);
        self.program.const_exprs.iter().any(|&root| {
            self.program.source_of(root) == Some(source) && {
                let root_span = self.span_of(root);
                span.start >= root_span.start && span.end <= root_span.end
            }
        })
    }

    /// The free local references of the const subtree: every `Expr::Local`
    /// whose span lies inside the expression's span (same source file), minus
    /// bindings DECLARED inside it (block `let`s, closure parameters — their
    /// references are internal, not free).
    fn free_locals(&self, root: Id) -> Vec<(Id, Id)> {
        let root_span = self.span_of(root);
        let Some(root_source) = self.program.source_of(root) else {
            return Vec::new();
        };
        let within = |id: Id| -> bool {
            self.program.source_of(id) == Some(root_source)
                && self
                    .program
                    .span_map
                    .get(&id)
                    .map(|span| span.start >= root_span.start && span.end <= root_span.end)
                    .unwrap_or(false)
        };
        let mut references = Vec::new();
        for (id, expr) in &self.program.entity_map {
            if let Expr::Local(binding) = expr
                && within(*id)
                && !within(*binding)
            {
                references.push((*id, *binding));
            }
        }
        // Deterministic diagnostic order.
        references.sort_by_key(|(id, _)| self.span_of(*id).start);
        references
    }

    fn classify(&self, binding: Id) -> Known<'src> {
        if let Some(parameter) = self.program.parameters.get(&binding) {
            return Known::Runtime(parameter.name);
        }
        if let Some(variable) = self.program.variables.get(&binding) {
            if variable.mutable {
                return Known::Runtime(variable.name);
            }
            let Some(initial) = variable.initial else {
                return Known::Runtime(variable.name);
            };
            if self.const_set.contains(&initial) {
                return Known::Const(initial);
            }
            let literal = matches!(
                self.program.entity_map.get(&initial),
                Some(
                    Expr::String(_)
                        | Expr::MultilineString(_)
                        | Expr::Number(..)
                        | Expr::Bool(_)
                        | Expr::Null
                )
            );
            if literal {
                return Known::Ok;
            }
            return Known::Runtime(variable.name);
        }
        // Items — functions, structs, enum constructors — are code, not
        // runtime state; the mini-program emits them.
        Known::Ok
    }

    fn span_of(&self, id: Id) -> Span {
        self.program
            .span_map
            .get(&id)
            .map(|span| **span)
            .unwrap_or((0..0).into())
    }
}
