//! The `const` pass (proposal/const-eval.md): evaluates `const`-marked
//! expressions post-analysis with the macro interpreter, in dependency order,
//! producing plain-data results the transformer serializes in place — plus
//! spanned diagnostics for everything that cannot evaluate. Free variables of
//! a const expression must be compile-time-known: an item (function, struct,
//! enum), or an immutable binding whose initializer is a literal or another
//! `const` expression.

use std::collections::{HashMap, HashSet};

use crate::analyzer::{Expr, Program};
use crate::error::Error;
use crate::id::Id;
use crate::interpreter::{self, ConstValue, Limits};
use crate::options::BuildOptions;
use crate::span::Span;
use crate::transformer;

pub fn evaluate(
    program: &Program,
    options: &BuildOptions,
) -> (HashMap<Id, ConstValue>, Vec<Error>) {
    let mut state = State {
        program,
        options,
        const_set: program.const_exprs.iter().copied().collect(),
        results: HashMap::new(),
        failed: HashSet::new(),
        in_progress: HashSet::new(),
        errors: Vec::new(),
    };
    for &expr_id in &program.const_exprs {
        state.evaluate_one(expr_id);
    }
    (state.results, state.errors)
}

struct State<'p, 'src> {
    program: &'p Program<'src>,
    options: &'p BuildOptions,
    const_set: HashSet<Id>,
    results: HashMap<Id, ConstValue>,
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
                Ok(value) => {
                    self.results.insert(expr_id, value);
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
