//! The macro engine's execution core (proposal/macro-engine.md §5): a fueled
//! evaluator over `js::Node` — the transformer's output AST — run inside the
//! compiler. Executing the *emitted* tree rather than the analyzed vilan IR
//! keeps every lowering decision (generic dispatch, monomorphization,
//! value-semantics copies, match compilation) in the transformer, so the
//! interpreter cannot disagree with codegen about what a program means; the
//! residual claim — "this evaluator matches a JS engine on the emitted
//! subset" — is tested directly by the compiled-vs-interpreted equivalence
//! suite (`tests/interpreter.rs`).
//!
//! The emitted subset is tiny and closed: values are undefined / null / bool /
//! number / BigInt / string / array / `Set` / `Map` / closure, plus the one
//! `{ v }` cell `Shared` uses — no general objects (structs are positional
//! arrays), no classes, no prototypes, no `this`. Sandboxing is a *missing
//! capability*, not a check: the impure runtime helpers and `[extern]` host
//! imports simply have no implementation here, and fail as clean
//! "not available at expansion time" errors.
//!
//! Deliberate v1 bounds (each fails loudly, never silently):
//! - `BigInt` is backed by `i128`; overflow is an error (JS BigInt is
//!   arbitrary-precision).
//! - No async: `async`/`await` are rejected — macro bodies are synchronous.
//! - String ops use UTF-16 code-unit semantics like JS; lone surrogates from
//!   slicing are replaced (`from_utf16_lossy`) rather than preserved.

use crate::node::BinaryOp;
use crate::transformer::{JsProgram, js};
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

/// Execution budgets. Fuel is decremented once per node evaluated; call depth
/// bounds closure/function recursion. Both exhaust into clean errors.
#[derive(Clone, Copy)]
pub struct Limits {
    pub fuel: u64,
    pub call_depth: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000,
            call_depth: 512,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The fuel budget ran out — the macro-loop backstop.
    Fuel,
    /// Call recursion exceeded the depth cap.
    Depth,
    /// The program needs a capability the expansion environment doesn't have
    /// (async, host imports, an impure helper, an unimplemented host method).
    Unsupported,
    /// The program threw (a vilan `panic`).
    Thrown,
    /// A semantic impossibility for emitted code — indicates a compiler or
    /// interpreter bug, not a user error.
    Internal,
}

#[derive(Debug)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

impl Failure {
    fn unsupported(what: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Unsupported,
            message: format!("{} is not available at expansion time", what.into()),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Internal,
            message: message.into(),
        }
    }
}

/// A compile-time evaluation result (proposal/const-eval.md): the plain-data
/// subset of [`Value`], owned so it can live in `Program` across phases. The
/// transformer serializes it in place of the `const` expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    BigInt(i128),
    Str(String),
    Array(Vec<ConstValue>),
    Set(Vec<ConstValue>),
    Map(Vec<(ConstValue, ConstValue)>),
}

/// Converts an interpreter value to plain data, or names what blocks it.
fn value_to_const(value: &Value) -> Result<ConstValue, &'static str> {
    Ok(match value {
        Value::Undefined => ConstValue::Undefined,
        Value::Null => ConstValue::Null,
        Value::Bool(b) => ConstValue::Bool(*b),
        Value::Number(n) => ConstValue::Number(*n),
        Value::BigInt(n) => ConstValue::BigInt(*n),
        Value::Str(s) => ConstValue::Str(s.to_string()),
        Value::Array(items) => ConstValue::Array(
            items
                .borrow()
                .iter()
                .map(value_to_const)
                .collect::<Result<_, _>>()?,
        ),
        Value::Set(items) => ConstValue::Set(
            items
                .borrow()
                .values()
                .map(value_to_const)
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(entries) => ConstValue::Map(
            entries
                .borrow()
                .values()
                .map(|(key, value)| {
                    Ok::<_, &'static str>((value_to_const(key)?, value_to_const(value)?))
                })
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(_) => return Err("a `Shared` cell"),
        Value::Closure(_) => return Err("a closure"),
    })
}

/// Evaluates a const mini-program — the functions and bindings one `const`
/// expression needs, ending in `const __const_result = <expr>;` — and returns
/// the result as plain data (const-eval.md §1).
pub fn eval_const<'a>(
    program: &'a JsProgram<'a>,
    limits: Limits,
) -> Result<(ConstValue, Vec<(String, String)>), Failure> {
    check_capabilities(program)?;
    let mut interpreter = Interpreter {
        fuel: limits.fuel,
        depth_left: limits.call_depth,
        stdout: String::new(),
        exited: None,
        assets: Vec::new(),
        // The one context where `asset::emit` is live (const-eval.md §3).
        allow_assets: true,
    };
    let globals = Scope::root();
    match interpreter.exec_body(&program.nodes, &globals)? {
        Flow::Normal => {}
        _ => {
            return Err(Failure::internal(
                "control flow escaped the const expression",
            ));
        }
    }
    let Some(result) = lookup(&globals, "__const_result") else {
        return Err(Failure::internal(
            "the const result binding was not emitted",
        ));
    };
    let value = value_to_const(&result).map_err(|what| Failure {
        kind: FailureKind::Unsupported,
        message: format!("a `const` result must be plain data; this evaluates to {what}"),
    })?;
    Ok((value, interpreter.assets))
}

pub struct RunOutput {
    /// Everything `console.log` printed, newline-terminated per call — the
    /// interpreter's analog of the compiled program's stdout.
    pub stdout: String,
    /// `process.exit(code)`'s argument, or 0 for a normal finish.
    pub exit_code: i32,
}

/// Runs a whole transformed program: hoists its function declarations, then
/// executes its statements in order. This is the equivalence suite's entry;
/// macro expansion (Phase 1) drives the same evaluator per `macro fun` call.
pub fn run_program<'a>(program: &'a JsProgram<'a>, limits: Limits) -> Result<RunOutput, Failure> {
    check_capabilities(program)?;
    let mut interpreter = Interpreter {
        fuel: limits.fuel,
        depth_left: limits.call_depth,
        stdout: String::new(),
        exited: None,
        assets: Vec::new(),
        allow_assets: false,
    };
    let globals = Scope::root();
    let result = interpreter.exec_body(&program.nodes, &globals);
    if let Some(exit_code) = interpreter.exited {
        return Ok(RunOutput {
            stdout: interpreter.stdout,
            exit_code,
        });
    }
    match result {
        Ok(Flow::Normal) => Ok(RunOutput {
            stdout: interpreter.stdout,
            exit_code: 0,
        }),
        Ok(Flow::Return(_)) => Err(Failure::internal("`return` outside a function")),
        Ok(Flow::Break | Flow::Continue) => {
            Err(Failure::internal("`break`/`continue` outside a loop"))
        }
        Err(failure) => Err(failure),
    }
}

/// A program needing host capabilities cannot run at expansion time.
fn check_capabilities(program: &JsProgram) -> Result<(), Failure> {
    if !program.imports.is_empty() {
        return Err(Failure::unsupported("host bindings ([extern])"));
    }
    for helper in &program.helpers {
        // The impure helpers are absent by design; everything else is native.
        if matches!(
            *helper,
            "__scan" | "__env" | "__args" | "__random_int" | "__random_float"
        ) {
            return Err(Failure::unsupported(format!("`{helper}`")));
        }
    }
    Ok(())
}

/// Runs one function of a transformed program — the macro-expansion entry
/// (macro-engine.md §3). Executes the program's top level first (hoisting its
/// functions, initializing module-level globals), then calls `entry` with the
/// given argument expressions, expecting a `macro_std` `Source` back (a
/// one-field struct, compiled to `[text]`); returns its text.
pub fn run_entry<'a>(
    program: &'a JsProgram<'a>,
    entry: &str,
    arguments: &'a [js::Node<'a>],
    limits: Limits,
) -> Result<String, Failure> {
    check_capabilities(program)?;
    let mut interpreter = Interpreter {
        fuel: limits.fuel,
        depth_left: limits.call_depth,
        stdout: String::new(),
        exited: None,
        assets: Vec::new(),
        allow_assets: false,
    };
    let globals = Scope::root();
    match interpreter.exec_body(&program.nodes, &globals)? {
        Flow::Normal => {}
        _ => return Err(Failure::internal("control flow escaped the top level")),
    }
    let Some(callee) = lookup(&globals, entry) else {
        return Err(Failure::internal(format!(
            "the macro entry `{entry}` was not emitted"
        )));
    };
    let mut values = Vec::with_capacity(arguments.len());
    for argument in arguments {
        values.push(interpreter.eval(argument, &globals)?);
    }
    let result = interpreter.call_value(&callee, values)?;
    // A `Source` is `struct Source { text: str }` — the one-field positional
    // array `[text]`.
    if let Value::Array(slots) = &result {
        let slots = slots.borrow();
        if let Some(Value::Str(text)) = slots.first() {
            return Ok(text.to_string());
        }
    }
    Err(Failure {
        kind: FailureKind::Thrown,
        message: "the macro did not return a `Source` (build one with `macro_std::source(..)`)"
            .to_string(),
    })
}

// --- Values ---

/// A `Set`/`Map` key: JS SameValueZero over the primitives vilan admits as
/// keys (`NaN` equals itself, `-0` equals `0`). Emitted code never keys a
/// container by a reference value; hitting one is an internal error.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Undefined,
    Null,
    Bool(bool),
    /// The f64's bits with `NaN` canonicalized and `-0` folded to `0`.
    Number(u64),
    BigInt(i128),
    Str(Rc<str>),
}

impl Key {
    fn of(value: &Value) -> Result<Self, Failure> {
        Ok(match value {
            Value::Undefined => Key::Undefined,
            Value::Null => Key::Null,
            Value::Bool(x) => Key::Bool(*x),
            Value::Number(n) => {
                let canonical = if n.is_nan() {
                    f64::NAN
                } else if *n == 0.0 {
                    0.0
                } else {
                    *n
                };
                Key::Number(canonical.to_bits())
            }
            Value::BigInt(n) => Key::BigInt(*n),
            Value::Str(s) => Key::Str(s.clone()),
            _ => {
                return Err(Failure::internal(
                    "a non-primitive Set/Map key reached the interpreter",
                ));
            }
        })
    }
}

#[derive(Clone)]
enum Value<'a> {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    BigInt(i128),
    Str(Rc<str>),
    Array(Rc<RefCell<Vec<Value<'a>>>>),
    /// JS `Set`: insertion-ordered, SameValueZero identity. Stores the first
    /// inserted value per key so iteration yields the original values.
    Set(Rc<RefCell<IndexMap<Key, Value<'a>>>>),
    /// JS `Map`: insertion-ordered; stores `(original key value, value)`.
    Map(Rc<RefCell<IndexMap<Key, (Value<'a>, Value<'a>)>>>),
    /// The one object shape emitted code uses: `Shared`'s `{ v }` cell, plus
    /// whatever `JSON.parse` produces. Cloned by reference (like `__clone`).
    Object(Rc<RefCell<IndexMap<Rc<str>, Value<'a>>>>),
    Closure(Rc<ClosureData<'a>>),
}

struct ClosureData<'a> {
    parameters: &'a [js::Parameter],
    body: &'a [js::Node<'a>],
    env: Env<'a>,
    /// The declaration name for named functions (inspect prints it).
    name: Option<&'a str>,
}

// --- Environment ---

type Env<'a> = Rc<RefCell<Scope<'a>>>;

struct Scope<'a> {
    vars: HashMap<&'a str, Value<'a>>,
    parent: Option<Env<'a>>,
}

impl<'a> Scope<'a> {
    fn root() -> Env<'a> {
        Rc::new(RefCell::new(Scope {
            vars: HashMap::new(),
            parent: None,
        }))
    }

    fn child(parent: &Env<'a>) -> Env<'a> {
        Rc::new(RefCell::new(Scope {
            vars: HashMap::new(),
            parent: Some(parent.clone()),
        }))
    }
}

fn lookup<'a>(env: &Env<'a>, name: &str) -> Option<Value<'a>> {
    let scope = env.borrow();
    if let Some(value) = scope.vars.get(name) {
        return Some(value.clone());
    }
    scope
        .parent
        .as_ref()
        .and_then(|parent| lookup(parent, name))
}

/// Writes through the scope chain to wherever `name` is bound. Emitted code
/// always declares before assigning, so a miss is an internal error.
fn assign<'a>(env: &Env<'a>, name: &'a str, value: Value<'a>) -> Result<(), Failure> {
    let mut scope = env.borrow_mut();
    if let Some(slot) = scope.vars.get_mut(name) {
        *slot = value;
        return Ok(());
    }
    match &scope.parent {
        Some(parent) => {
            let parent = parent.clone();
            drop(scope);
            assign(&parent, name, value)
        }
        None => Err(Failure::internal(format!(
            "assignment to undeclared `{name}`"
        ))),
    }
}

// --- Control flow ---

enum Flow<'a> {
    Normal,
    Return(Value<'a>),
    Break,
    Continue,
}

struct Interpreter {
    fuel: u64,
    depth_left: u32,
    stdout: String,
    /// Set by `process.exit(code)`. Exit unwinds as an error through every
    /// frame (node exits the whole process from anywhere); `run_program`
    /// checks this first and converts the unwind back into success.
    exited: Option<i32>,
    /// `(kind, line)` pairs `asset::emit` accumulated (const-eval.md §3).
    assets: Vec<(String, String)>,
    /// `asset::emit` is live only under `eval_const`; anywhere else it is a
    /// capability miss.
    allow_assets: bool,
}

impl Interpreter {
    fn charge(&mut self) -> Result<(), Failure> {
        if self.fuel == 0 {
            return Err(Failure {
                kind: FailureKind::Fuel,
                message: "the fuel budget was exhausted".to_string(),
            });
        }
        self.fuel -= 1;
        Ok(())
    }

    // --- Statements ---

    /// Executes a statement list: function declarations hoist (bound over the
    /// current scope before anything runs, as JS hoists them), then statements
    /// run in order.
    fn exec_body<'a>(
        &mut self,
        body: &'a [js::Node<'a>],
        env: &Env<'a>,
    ) -> Result<Flow<'a>, Failure> {
        for node in body {
            if let js::Node::Function(function) = node {
                if function.is_async {
                    return Err(Failure::unsupported("async (macro bodies are synchronous)"));
                }
                let closure = Value::Closure(Rc::new(ClosureData {
                    parameters: &function.parameters,
                    body: &function.body,
                    env: env.clone(),
                    name: Some(function.name.as_str()),
                }));
                env.borrow_mut()
                    .vars
                    .insert(function.name.as_str(), closure);
            }
        }
        for node in body {
            match self.exec_statement(node, env)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_statement<'a>(
        &mut self,
        node: &'a js::Node<'a>,
        env: &Env<'a>,
    ) -> Result<Flow<'a>, Failure> {
        self.charge()?;
        match node {
            js::Node::Function(_) => Ok(Flow::Normal), // hoisted
            js::Node::LetVariable(variable) | js::Node::ConstVariable(variable) => {
                let value = self.eval(&variable.value, env)?;
                env.borrow_mut().vars.insert(variable.name.as_str(), value);
                Ok(Flow::Normal)
            }
            js::Node::Return(value) => {
                let value = self.eval(value, env)?;
                Ok(Flow::Return(value))
            }
            js::Node::Break => Ok(Flow::Break),
            js::Node::Continue => Ok(Flow::Continue),
            js::Node::Throw(value) => {
                let value = self.eval(value, env)?;
                Err(Failure {
                    kind: FailureKind::Thrown,
                    message: self.to_js_string(&value)?,
                })
            }
            js::Node::If(branch) => self.exec_if(branch, env),
            // `try { <body> } finally { <finally> }` — scope-end destruction
            // (destruction.md §7). The body's completion (a value, a `Flow`, or a
            // thrown `Failure`) is held while the `finally` runs; the `finally`'s
            // own abnormal completion REPLACES the in-flight one, matching JS —
            // and §5's "a drop that panics during unwind replaces the in-flight
            // error". `ret` / `break` / `continue` / a throw in the body all run
            // the `finally` on the way out.
            js::Node::Try(body, finally) => {
                let body_result = self.exec_body(body, env);
                match self.exec_body(finally, env)? {
                    Flow::Normal => body_result,
                    other => Ok(other),
                }
            }
            js::Node::While(condition, body) => {
                loop {
                    self.charge()?;
                    let condition = self.eval(condition, env)?;
                    if !truthy(&condition) {
                        break;
                    }
                    // A fresh scope per iteration: `let`s inside the body are
                    // per-iteration bindings (closures capture each turn's).
                    let iteration = Scope::child(env);
                    match self.exec_body(body, &iteration)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            js::Node::ForOf(binding, iterable, body) => {
                let iterable = self.eval(iterable, env)?;
                match iterable {
                    // Index-based over the live array, like the JS array
                    // iterator: element writes during iteration are visible.
                    Value::Array(items) => {
                        let mut index = 0;
                        loop {
                            let element = {
                                let items = items.borrow();
                                if index >= items.len() {
                                    break;
                                }
                                items[index].clone()
                            };
                            index += 1;
                            match self.run_for_iteration(binding, element, body, env)? {
                                Flow::Normal | Flow::Continue => {}
                                Flow::Break => break,
                                other => return Ok(other),
                            }
                        }
                    }
                    Value::Set(entries) => {
                        let snapshot: Vec<Value> = entries.borrow().values().cloned().collect();
                        for element in snapshot {
                            match self.run_for_iteration(binding, element, body, env)? {
                                Flow::Normal | Flow::Continue => {}
                                Flow::Break => break,
                                other => return Ok(other),
                            }
                        }
                    }
                    Value::Map(entries) => {
                        let snapshot: Vec<Value> = entries
                            .borrow()
                            .values()
                            .map(|(key, value)| {
                                Value::Array(Rc::new(RefCell::new(vec![
                                    key.clone(),
                                    value.clone(),
                                ])))
                            })
                            .collect();
                        for element in snapshot {
                            match self.run_for_iteration(binding, element, body, env)? {
                                Flow::Normal | Flow::Continue => {}
                                Flow::Break => break,
                                other => return Ok(other),
                            }
                        }
                    }
                    other => {
                        return Err(Failure::internal(format!(
                            "for..of over a non-iterable ({})",
                            type_name(&other)
                        )));
                    }
                }
                Ok(Flow::Normal)
            }
            // An expression in statement position (calls, assignments).
            other => {
                self.eval(other, env)?;
                Ok(Flow::Normal)
            }
        }
    }

    fn run_for_iteration<'a>(
        &mut self,
        binding: &'a str,
        element: Value<'a>,
        body: &'a [js::Node<'a>],
        env: &Env<'a>,
    ) -> Result<Flow<'a>, Failure> {
        let iteration = Scope::child(env);
        iteration.borrow_mut().vars.insert(binding, element);
        self.exec_body(body, &iteration)
    }

    fn exec_if<'a>(
        &mut self,
        branch: &'a js::IfBranch<'a>,
        env: &Env<'a>,
    ) -> Result<Flow<'a>, Failure> {
        match branch {
            js::IfBranch::If(condition, body, else_) => {
                let condition = self.eval(condition, env)?;
                if truthy(&condition) {
                    let scope = Scope::child(env);
                    self.exec_body(body, &scope)
                } else if let Some(else_) = else_ {
                    self.exec_if(else_, env)
                } else {
                    Ok(Flow::Normal)
                }
            }
            js::IfBranch::Else(body) => {
                let scope = Scope::child(env);
                self.exec_body(body, &scope)
            }
        }
    }

    // --- Expressions ---

    fn eval<'a>(&mut self, node: &'a js::Node<'a>, env: &Env<'a>) -> Result<Value<'a>, Failure> {
        self.charge()?;
        match node {
            js::Node::Void => Ok(Value::Undefined),
            js::Node::Null => Ok(Value::Null),
            js::Node::Bool(x) => Ok(Value::Bool(*x)),
            js::Node::String(x) => Ok(Value::Str(Rc::from(x.as_ref()))),
            js::Node::Number(whole, fraction) => parse_number_literal(whole, fraction.as_deref()),
            js::Node::Array(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    if let js::Node::Spread(operand) = item {
                        let operand = self.eval(operand, env)?;
                        spread_into(&operand, &mut values)?;
                    } else {
                        values.push(self.eval(item, env)?);
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            js::Node::Spread(_) => Err(Failure::internal("spread outside an array literal")),
            js::Node::Local(name) => self.eval_local(name, env),
            js::Node::Closure(closure) => {
                if closure.is_async {
                    return Err(Failure::unsupported("async (macro bodies are synchronous)"));
                }
                Ok(Value::Closure(Rc::new(ClosureData {
                    parameters: &closure.parameters,
                    body: &closure.body,
                    env: env.clone(),
                    name: None,
                })))
            }
            js::Node::Function(function) => {
                // A function in expression position (never emitted today, but
                // exec_body's hoisting handles the statement form; keep the
                // expression form coherent).
                Ok(Value::Closure(Rc::new(ClosureData {
                    parameters: &function.parameters,
                    body: &function.body,
                    env: env.clone(),
                    name: Some(function.name.as_str()),
                })))
            }
            js::Node::Await(_) => Err(Failure::unsupported("await (macro bodies are synchronous)")),
            js::Node::Unary(operator, operand) => {
                let operand = self.eval(operand, env)?;
                match operator {
                    '!' => Ok(Value::Bool(!truthy(&operand))),
                    '-' => match operand {
                        Value::Number(n) => Ok(Value::Number(-n)),
                        Value::BigInt(n) => n
                            .checked_neg()
                            .map(Value::BigInt)
                            .ok_or_else(|| bigint_overflow()),
                        other => Err(Failure::internal(format!(
                            "unary `-` on {}",
                            type_name(&other)
                        ))),
                    },
                    other => Err(Failure::internal(format!("unary operator `{other}`"))),
                }
            }
            js::Node::Binary(op, lhs, rhs) => self.eval_binary(*op, lhs, rhs, env),
            js::Node::Assignment(target, value) => {
                let value = self.eval(value, env)?;
                self.write_target(target, value.clone(), env)?;
                Ok(value)
            }
            js::Node::Property(subject, member) => {
                let subject = self.eval(subject, env)?;
                self.read_property(&subject, member)
            }
            js::Node::PropertyIndex(subject, index) => {
                let subject = self.eval(subject, env)?;
                let index = self.eval(index, env)?;
                self.read_index(&subject, &index)
            }
            js::Node::Call(subject, arguments) => self.eval_call(subject, arguments, env),
            other => Err(Failure::internal(format!(
                "statement node in expression position: {other:?}"
            ))),
        }
    }

    fn eval_local<'a>(&mut self, name: &'a str, env: &Env<'a>) -> Result<Value<'a>, Failure> {
        if let Some(value) = lookup(env, name) {
            return Ok(value);
        }
        // Free names are host references; the renamer guarantees user bindings
        // never collide with these (they're reserved).
        Err(match name {
            "__scan" | "__env" | "__args" | "__random_int" | "__random_float" => {
                Failure::unsupported(format!("`{name}`"))
            }
            _ => Failure::internal(format!("`{name}` is not defined")),
        })
    }

    // --- Calls ---

    fn eval_call<'a>(
        &mut self,
        subject: &'a js::Node<'a>,
        arguments: &'a [js::Node<'a>],
        env: &Env<'a>,
    ) -> Result<Value<'a>, Failure> {
        // Host forms are dispatched structurally, before generic evaluation:
        // `console.log(..)` / `process.exit(..)` are Property calls on free
        // names; everything else host-side is a dotted `Local` name.
        if let js::Node::Property(receiver, method) = subject {
            if let js::Node::Local(base) = &**receiver {
                if lookup(env, base).is_none() {
                    return self.call_host_property(base, method, arguments, env);
                }
            }
            let receiver = self.eval(receiver, env)?;
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(self.eval(argument, env)?);
            }
            return self.call_method(&receiver, method, values);
        }
        if let js::Node::Local(name) = subject {
            if lookup(env, name).is_none() {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(self.eval(argument, env)?);
                }
                return self.call_host(name, values);
            }
        }
        let callee = self.eval(subject, env)?;
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            values.push(self.eval(argument, env)?);
        }
        self.call_value(&callee, values)
    }

    fn call_value<'a>(
        &mut self,
        callee: &Value<'a>,
        arguments: Vec<Value<'a>>,
    ) -> Result<Value<'a>, Failure> {
        let Value::Closure(closure) = callee else {
            return Err(Failure::internal(format!(
                "call of a non-function ({})",
                type_name(callee)
            )));
        };
        if self.depth_left == 0 {
            return Err(Failure {
                kind: FailureKind::Depth,
                message: "the call-depth cap was exceeded".to_string(),
            });
        }
        self.depth_left -= 1;
        let scope = Scope::child(&closure.env);
        {
            let mut scope = scope.borrow_mut();
            for (index, parameter) in closure.parameters.iter().enumerate() {
                let value = arguments.get(index).cloned().unwrap_or(Value::Undefined);
                scope.vars.insert(parameter.name.as_str(), value);
            }
        }
        let result = self.exec_body(closure.body, &scope);
        self.depth_left += 1;
        match result? {
            Flow::Return(value) => Ok(value),
            Flow::Normal => Ok(Value::Undefined),
            Flow::Break | Flow::Continue => {
                Err(Failure::internal("`break`/`continue` escaped a function"))
            }
        }
    }

    /// `console.log(..)` and `process.exit(..)` — the only Property-shaped
    /// host calls the backend emits.
    fn call_host_property<'a>(
        &mut self,
        base: &str,
        method: &str,
        arguments: &'a [js::Node<'a>],
        env: &Env<'a>,
    ) -> Result<Value<'a>, Failure> {
        match (base, method) {
            ("console", "log") => {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(self.eval(argument, env)?);
                }
                let mut line = String::new();
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        line.push(' ');
                    }
                    line.push_str(&self.inspect(value, true)?);
                }
                self.stdout.push_str(&line);
                self.stdout.push('\n');
                Ok(Value::Undefined)
            }
            ("process", "exit") => {
                let code = match arguments.first() {
                    Some(node) => match self.eval(node, env)? {
                        Value::Number(n) => n as i32,
                        Value::Undefined => 0,
                        other => {
                            return Err(Failure::internal(format!(
                                "process.exit with {}",
                                type_name(&other)
                            )));
                        }
                    },
                    None => 0,
                };
                self.exited = Some(code);
                Err(Failure::internal("process.exit unwind"))
            }
            ("process", _) | ("document", _) | ("window", _) => {
                Err(Failure::unsupported(format!("`{base}.{method}`")))
            }
            _ => Err(Failure::internal(format!(
                "unknown host call `{base}.{method}`"
            ))),
        }
    }

    /// Free-name host calls: the `__` runtime helpers (implemented natively,
    /// mirroring their JS sources in `helper_source`) and the dotted host
    /// globals the backend emits.
    fn call_host<'a>(
        &mut self,
        name: &str,
        arguments: Vec<Value<'a>>,
    ) -> Result<Value<'a>, Failure> {
        let take = |index: usize| -> Value<'a> {
            if index < arguments.len() {
                arguments[index].clone()
            } else {
                Value::Undefined
            }
        };
        match name {
            // `print` reaches here as the extern binding `[extern("console.log")]`
            // — a dotted free name, unlike the Property-shaped emission.
            "console.log" => {
                let mut line = String::new();
                for (index, value) in arguments.iter().enumerate() {
                    if index > 0 {
                        line.push(' ');
                    }
                    line.push_str(&self.inspect(value, true)?);
                }
                self.stdout.push_str(&line);
                self.stdout.push('\n');
                Ok(Value::Undefined)
            }
            // Both constructors accept an optional entries/values array — the
            // shape a serialized const result uses (`new Map([[k, v], ..])`).
            "new Set" => {
                let mut set = IndexMap::new();
                if let Value::Array(items) = take(0) {
                    for item in items.borrow().iter() {
                        set.insert(Key::of(item)?, item.clone());
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(set))))
            }
            "new Map" => {
                let mut map = IndexMap::new();
                if let Value::Array(entries) = take(0) {
                    for entry in entries.borrow().iter() {
                        let Value::Array(pair) = entry else {
                            return Err(Failure::internal(
                                "a Map entry must be a [key, value] pair",
                            ));
                        };
                        let pair = pair.borrow();
                        let key = pair.first().cloned().unwrap_or(Value::Undefined);
                        let value = pair.get(1).cloned().unwrap_or(Value::Undefined);
                        map.insert(Key::of(&key)?, (key, value));
                    }
                }
                Ok(Value::Map(Rc::new(RefCell::new(map))))
            }
            "__clone" => Ok(deep_clone(&take(0))),
            // `[value; n]` — n independent slots. `deep_clone` per slot matches the
            // emitted helper (a primitive copies, an aggregate is cloned) for both
            // kinds, so node and the interpreter agree.
            "__repeat" => {
                let value = take(0);
                let count = expect_number(&take(1))?;
                let count = if count >= 0.0 && count.fract() == 0.0 {
                    count as usize
                } else {
                    0
                };
                let items = (0..count).map(|_| deep_clone(&value)).collect();
                Ok(Value::Array(Rc::new(RefCell::new(items))))
            }
            "__shared_new" => {
                let mut cell = IndexMap::new();
                cell.insert(Rc::from("v"), take(0));
                Ok(Value::Object(Rc::new(RefCell::new(cell))))
            }
            "__list_get" => {
                let list = expect_array(&take(0))?;
                let index = expect_number(&take(1))?;
                let list = list.borrow();
                if index >= 0.0 && (index as usize) < list.len() && index.fract() == 0.0 {
                    Ok(option_some(deep_clone(&list[index as usize])))
                } else {
                    Ok(option_none())
                }
            }
            "__list_pop" => {
                let list = expect_array(&take(0))?;
                let mut list = list.borrow_mut();
                match list.pop() {
                    Some(value) => Ok(option_some(value)),
                    None => Ok(option_none()),
                }
            }
            // `asset::emit` — the const-only compile-time effect: live only
            // under `eval_const` (const-eval.md §3); anywhere else (macro
            // expansion, the equivalence runner) it is a capability miss.
            "__emit_asset" => {
                if !self.allow_assets {
                    return Err(Failure::unsupported(
                        "`asset::emit` outside a `const` expression",
                    ));
                }
                let kind = expect_str(&take(0))?;
                let line = expect_str(&take(1))?;
                self.assets.push((kind.to_string(), line.to_string()));
                Ok(Value::Undefined)
            }
            // The checked subscripts, matching the emitted `__at*` helpers: an
            // out-of-bounds index is a panic (`Thrown`), so a macro-time
            // violation fails the expansion with the same message a runtime
            // one prints.
            "__at" => {
                let list = expect_array(&take(0))?;
                let index = expect_number(&take(1))?;
                let list = list.borrow();
                if index >= 0.0 && (index as usize) < list.len() && index.fract() == 0.0 {
                    Ok(list[index as usize].clone())
                } else {
                    Err(index_out_of_bounds(list.len(), index))
                }
            }
            "__at_put" => {
                let list = expect_array(&take(0))?;
                let index = expect_number(&take(1))?;
                let value = take(2);
                let mut list = list.borrow_mut();
                if index >= 0.0 && (index as usize) < list.len() && index.fract() == 0.0 {
                    list[index as usize] = value.clone();
                    Ok(value)
                } else {
                    Err(index_out_of_bounds(list.len(), index))
                }
            }
            "__at_view" => {
                let list = expect_array(&take(0))?;
                let index = expect_number(&take(1))?;
                let length = list.borrow().len();
                if index >= 0.0 && (index as usize) < length && index.fract() == 0.0 {
                    Ok(Value::Array(Rc::new(RefCell::new(vec![
                        Value::Array(list),
                        Value::Number(index),
                    ]))))
                } else {
                    Err(index_out_of_bounds(length, index))
                }
            }
            "__map_get" => {
                let map = expect_map(&take(0))?;
                let key = Key::of(&take(1))?;
                let map = map.borrow();
                match map.get(&key) {
                    Some((_, value)) => Ok(option_some(deep_clone(value))),
                    None => Ok(option_none()),
                }
            }
            "__map_keys" => {
                let map = expect_map(&take(0))?;
                let keys = map
                    .borrow()
                    .values()
                    .map(|(key, _)| deep_clone(key))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(keys))))
            }
            "__map_values" => {
                let map = expect_map(&take(0))?;
                let values = map
                    .borrow()
                    .values()
                    .map(|(_, value)| deep_clone(value))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            // `[...set[0].values()]` — the `Set` struct is `[table]`; iterate the
            // backing map's stored originals by reference (matching the spread).
            "__set_iter" => {
                let set = take(0);
                let Value::Array(elements) = &set else {
                    return Err(Failure::internal("__set_iter on a non-Set".to_string()));
                };
                let table = elements
                    .borrow()
                    .first()
                    .cloned()
                    .unwrap_or(Value::Undefined);
                let map = expect_map(&table)?;
                let values = map
                    .borrow()
                    .values()
                    .map(|(_, value)| value.clone())
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            "__json_tag" => match take(0) {
                Value::Str(s) => Ok(Value::Str(s)),
                Value::Object(object) => match object.borrow().keys().next() {
                    Some(key) => Ok(Value::Str(key.clone())),
                    None => Ok(Value::Undefined),
                },
                other => Err(Failure::internal(format!(
                    "__json_tag on {}",
                    type_name(&other)
                ))),
            },
            // The normalized JSON kind: `typeof`, with arrays and null named
            // (mirrors the `__json_kind` codegen helper). Basis for the decode
            // type checks (`JsonValue.kind()`).
            "__json_kind" => {
                let kind = match take(0) {
                    Value::Null => "null",
                    Value::Array(_) => "array",
                    Value::Number(_) | Value::BigInt(_) => "number",
                    Value::Str(_) => "string",
                    Value::Bool(_) => "boolean",
                    Value::Object(_) => "object",
                    Value::Undefined => "undefined",
                    _ => "object",
                };
                Ok(Value::Str(Rc::from(kind)))
            }
            // The canonical key: a primitive keys as itself; an aggregate (array
            // or object) canonicalizes to its JSON string (mirrors `__hash`).
            "__hash" => {
                let value = take(0);
                match value {
                    Value::Array(_) | Value::Object(_) => {
                        let mut out = String::new();
                        json_stringify(&value, &mut out)?;
                        Ok(Value::Str(Rc::from(out.as_str())))
                    }
                    other => Ok(other),
                }
            }
            "__parse_i32" => {
                let text = expect_str(&take(0))?;
                let trimmed = text.trim();
                let integer_shaped = {
                    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
                    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                };
                let value = js_string_to_number(trimmed);
                if integer_shaped && (-2147483648.0..=2147483647.0).contains(&value) {
                    Ok(option_some(Value::Number(value)))
                } else {
                    Ok(option_none())
                }
            }
            "__parse_f64" => {
                let text = expect_str(&take(0))?;
                let trimmed = text.trim();
                let value = js_string_to_number(trimmed);
                if trimmed.is_empty() || value.is_nan() {
                    Ok(option_none())
                } else {
                    Ok(option_some(Value::Number(value)))
                }
            }
            "__try_parse_json" => {
                let text = expect_str(&take(0))?;
                match json_parse(&text) {
                    Ok(value) => Ok(option_some(value)),
                    Err(_) => Ok(option_none()),
                }
            }
            "JSON.stringify" => {
                let mut out = String::new();
                match json_stringify(&take(0), &mut out)? {
                    true => Ok(Value::Str(Rc::from(out.as_str()))),
                    false => Ok(Value::Undefined),
                }
            }
            "JSON.parse" => {
                let text = expect_str(&take(0))?;
                json_parse(&text).map_err(|message| Failure {
                    kind: FailureKind::Thrown,
                    message,
                })
            }
            "String" => Ok(Value::Str(Rc::from(self.to_js_string(&take(0))?.as_str()))),
            "Boolean" => Ok(Value::Bool(truthy(&take(0)))),
            "Number" => to_number(&take(0)).map(Value::Number),
            "Number.isNaN" => Ok(Value::Bool(
                matches!(take(0), Value::Number(n) if n.is_nan()),
            )),
            "Number.isFinite" => Ok(Value::Bool(
                matches!(take(0), Value::Number(n) if n.is_finite()),
            )),
            "Math.abs" => Ok(Value::Number(expect_number(&take(0))?.abs())),
            "Math.sin" => Ok(Value::Number(expect_number(&take(0))?.sin())),
            "Math.cos" => Ok(Value::Number(expect_number(&take(0))?.cos())),
            "Math.tan" => Ok(Value::Number(expect_number(&take(0))?.tan())),
            "Math.asin" => Ok(Value::Number(expect_number(&take(0))?.asin())),
            "Math.acos" => Ok(Value::Number(expect_number(&take(0))?.acos())),
            "Math.atan" => Ok(Value::Number(expect_number(&take(0))?.atan())),
            "Math.atan2" => Ok(Value::Number(
                expect_number(&take(0))?.atan2(expect_number(&take(1))?),
            )),
            "Math.exp" => Ok(Value::Number(expect_number(&take(0))?.exp())),
            "Math.log" => Ok(Value::Number(expect_number(&take(0))?.ln())),
            "Math.log2" => Ok(Value::Number(expect_number(&take(0))?.log2())),
            "Math.log10" => Ok(Value::Number(expect_number(&take(0))?.log10())),
            "Math.cbrt" => Ok(Value::Number(expect_number(&take(0))?.cbrt())),
            "Math.hypot" => Ok(Value::Number(
                expect_number(&take(0))?.hypot(expect_number(&take(1))?),
            )),
            "Math.sign" => {
                // JS semantics: NaN passes through, ±0 keep their sign.
                let n = expect_number(&take(0))?;
                let sign = if n.is_nan() || n == 0.0 {
                    n
                } else if n > 0.0 {
                    1.0
                } else {
                    -1.0
                };
                Ok(Value::Number(sign))
            }
            "Math.floor" => Ok(Value::Number(expect_number(&take(0))?.floor())),
            "Math.trunc" => Ok(Value::Number(expect_number(&take(0))?.trunc())),
            "Math.ceil" => Ok(Value::Number(expect_number(&take(0))?.ceil())),
            "Math.sqrt" => Ok(Value::Number(expect_number(&take(0))?.sqrt())),
            "Math.round" => {
                // JS rounds half UP (toward +∞): Math.round(-0.5) is -0.
                let n = expect_number(&take(0))?;
                Ok(Value::Number((n + 0.5).floor()))
            }
            "Math.pow" => Ok(Value::Number(
                expect_number(&take(0))?.powf(expect_number(&take(1))?),
            )),
            "Math.max" | "Math.min" => {
                let mut result = if name == "Math.max" {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                };
                for argument in &arguments {
                    let n = expect_number(argument)?;
                    if n.is_nan() {
                        result = f64::NAN;
                        break;
                    }
                    result = if name == "Math.max" {
                        result.max(n)
                    } else {
                        result.min(n)
                    };
                }
                Ok(Value::Number(result))
            }
            "Object.keys" => {
                let Value::Object(object) = take(0) else {
                    return Err(Failure::internal("Object.keys on a non-object"));
                };
                let keys = object
                    .borrow()
                    .keys()
                    .map(|key| Value::Str(key.clone()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(keys))))
            }
            // Structs are arrays, so the view write-through merge assigns
            // array onto array: index properties copy, a longer target keeps
            // its tail (JS index-property semantics).
            "Object.assign" => match take(0) {
                Value::Object(target) => {
                    for source in arguments.iter().skip(1) {
                        let Value::Object(source) = source else {
                            return Err(Failure::internal("Object.assign from a non-object"));
                        };
                        let entries: Vec<_> = source
                            .borrow()
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect();
                        for (key, value) in entries {
                            target.borrow_mut().insert(key, value);
                        }
                    }
                    Ok(Value::Object(target))
                }
                Value::Array(target) => {
                    for source in arguments.iter().skip(1) {
                        let Value::Array(source) = source else {
                            return Err(Failure::internal(
                                "Object.assign onto an array from a non-array",
                            ));
                        };
                        let elements: Vec<Value> = source.borrow().clone();
                        let mut target = target.borrow_mut();
                        for (index, element) in elements.into_iter().enumerate() {
                            if index >= target.len() {
                                target.resize(index + 1, Value::Undefined);
                            }
                            target[index] = element;
                        }
                    }
                    Ok(Value::Array(target))
                }
                other => Err(Failure::internal(format!(
                    "Object.assign on {}",
                    type_name(&other)
                ))),
            },
            "Object.hasOwn" => {
                let Value::Object(object) = take(0) else {
                    return Ok(Value::Bool(false));
                };
                let key = self.to_js_string(&take(1))?;
                Ok(Value::Bool(object.borrow().contains_key(key.as_str())))
            }
            "Array.from" => {
                let mut values = Vec::new();
                spread_into(&take(0), &mut values)?;
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            "Array.isArray" => Ok(Value::Bool(matches!(take(0), Value::Array(_)))),
            "__scan" | "__env" | "__args" | "__random_int" | "__random_float" => {
                Err(Failure::unsupported(format!("`{name}`")))
            }
            "fetch" | "setTimeout" | "setInterval" | "structuredClone" => {
                Err(Failure::unsupported(format!("`{name}`")))
            }
            other => Err(Failure::internal(format!("unknown host call `{other}`"))),
        }
    }

    /// Method calls on values — the JS prototype methods the backend emits for
    /// intrinsics (`str.trim()`, `set.add(..)`, tuple-comprehension `.map`, …).
    fn call_method<'a>(
        &mut self,
        receiver: &Value<'a>,
        method: &str,
        arguments: Vec<Value<'a>>,
    ) -> Result<Value<'a>, Failure> {
        let argument = |index: usize| -> Value<'a> {
            arguments.get(index).cloned().unwrap_or(Value::Undefined)
        };
        match receiver {
            Value::Array(items) => match method {
                "push" => {
                    let mut items = items.borrow_mut();
                    for value in arguments {
                        items.push(value);
                    }
                    Ok(Value::Number(items.len() as f64))
                }
                "pop" => Ok(items.borrow_mut().pop().unwrap_or(Value::Undefined)),
                "map" => {
                    let callback = argument(0);
                    let snapshot: Vec<Value> = items.borrow().clone();
                    let mut mapped = Vec::with_capacity(snapshot.len());
                    for (index, element) in snapshot.into_iter().enumerate() {
                        let result = self.call_value(
                            &callback,
                            vec![
                                element,
                                Value::Number(index as f64),
                                Value::Array(items.clone()),
                            ],
                        )?;
                        mapped.push(result);
                    }
                    Ok(Value::Array(Rc::new(RefCell::new(mapped))))
                }
                "slice" => {
                    let items = items.borrow();
                    let len = items.len() as f64;
                    let start = relative_index(argument(0), 0.0, len)?;
                    let end = relative_index(argument(1), len, len)?;
                    let slice: Vec<Value> = if start < end {
                        items[start as usize..end as usize].to_vec()
                    } else {
                        Vec::new()
                    };
                    Ok(Value::Array(Rc::new(RefCell::new(slice))))
                }
                "join" => {
                    let separator = match argument(0) {
                        Value::Undefined => ",".to_string(),
                        other => self.to_js_string(&other)?,
                    };
                    let items = items.borrow().clone();
                    let mut out = String::new();
                    for (index, item) in items.iter().enumerate() {
                        if index > 0 {
                            out.push_str(&separator);
                        }
                        if !matches!(item, Value::Undefined | Value::Null) {
                            out.push_str(&self.to_js_string(item)?);
                        }
                    }
                    Ok(Value::Str(Rc::from(out.as_str())))
                }
                "includes" => {
                    let needle = argument(0);
                    let found = items
                        .borrow()
                        .iter()
                        .any(|item| strict_equals(item, &needle));
                    Ok(Value::Bool(found))
                }
                // `arr.keys()` — the index iterator `for e in &mut container`
                // lowers through. Iteration is index-based over a fixed length,
                // so a snapshot of the indices is equivalent.
                "keys" => {
                    let indices = (0..items.borrow().len())
                        .map(|index| Value::Number(index as f64))
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(indices))))
                }
                other => Err(Failure::unsupported(format!("the array method `{other}`"))),
            },
            Value::Str(s) => self.call_string_method(s, method, arguments),
            Value::Set(entries) => match method {
                "add" => {
                    let value = argument(0);
                    let key = Key::of(&value)?;
                    entries.borrow_mut().entry(key).or_insert(value);
                    Ok(receiver.clone())
                }
                "has" => Ok(Value::Bool(
                    entries.borrow().contains_key(&Key::of(&argument(0))?),
                )),
                "delete" => Ok(Value::Bool(
                    entries
                        .borrow_mut()
                        .shift_remove(&Key::of(&argument(0))?)
                        .is_some(),
                )),
                other => Err(Failure::unsupported(format!("the Set method `{other}`"))),
            },
            Value::Map(entries) => match method {
                "set" => {
                    let key_value = argument(0);
                    let key = Key::of(&key_value)?;
                    entries.borrow_mut().insert(key, (key_value, argument(1)));
                    Ok(receiver.clone())
                }
                "get" => {
                    let key = Key::of(&argument(0))?;
                    Ok(entries
                        .borrow()
                        .get(&key)
                        .map(|(_, value)| value.clone())
                        .unwrap_or(Value::Undefined))
                }
                "has" => Ok(Value::Bool(
                    entries.borrow().contains_key(&Key::of(&argument(0))?),
                )),
                "delete" => Ok(Value::Bool(
                    entries
                        .borrow_mut()
                        .shift_remove(&Key::of(&argument(0))?)
                        .is_some(),
                )),
                other => Err(Failure::unsupported(format!("the Map method `{other}`"))),
            },
            other => Err(Failure::unsupported(format!(
                "the method `{method}` on {}",
                type_name(other)
            ))),
        }
    }

    fn call_string_method<'a>(
        &mut self,
        s: &Rc<str>,
        method: &str,
        arguments: Vec<Value<'a>>,
    ) -> Result<Value<'a>, Failure> {
        let argument = |index: usize| -> Value<'a> {
            arguments.get(index).cloned().unwrap_or(Value::Undefined)
        };
        let str_result = |out: String| Ok(Value::Str(Rc::from(out.as_str())));
        match method {
            "trim" => str_result(s.trim().to_string()),
            // `str.code_at` — the UTF-16 code unit as a number (NaN out of
            // range, like the host).
            "charCodeAt" => {
                let index = expect_number(&argument(0))?;
                let unit = (index >= 0.0 && index.fract() == 0.0)
                    .then(|| s.encode_utf16().nth(index as usize))
                    .flatten();
                Ok(Value::Number(
                    unit.map(|unit| unit as f64).unwrap_or(f64::NAN),
                ))
            }
            "toLowerCase" => str_result(s.to_lowercase()),
            "toUpperCase" => str_result(s.to_uppercase()),
            "includes" => Ok(Value::Bool(s.contains(&*expect_str(&argument(0))?))),
            "startsWith" => Ok(Value::Bool(s.starts_with(&*expect_str(&argument(0))?))),
            "endsWith" => Ok(Value::Bool(s.ends_with(&*expect_str(&argument(0))?))),
            "repeat" => {
                let count = expect_number(&argument(0))?;
                if count < 0.0 || !count.is_finite() {
                    return Err(Failure {
                        kind: FailureKind::Thrown,
                        message: "Invalid count value".to_string(),
                    });
                }
                str_result(s.repeat(count as usize))
            }
            "replaceAll" => {
                let from = expect_str(&argument(0))?;
                let to = expect_str(&argument(1))?;
                if from.is_empty() {
                    return Err(Failure::unsupported("replaceAll with an empty pattern"));
                }
                str_result(s.replace(&*from, &to))
            }
            "split" => {
                let separator = expect_str(&argument(0))?;
                let parts: Vec<Value> = if separator.is_empty() {
                    // JS splits into UTF-16 code units.
                    s.encode_utf16()
                        .map(|unit| {
                            Value::Str(Rc::from(String::from_utf16_lossy(&[unit]).as_str()))
                        })
                        .collect()
                } else {
                    s.split(&*separator)
                        .map(|part| Value::Str(Rc::from(part)))
                        .collect()
                };
                Ok(Value::Array(Rc::new(RefCell::new(parts))))
            }
            "substring" => {
                // JS substring: clamp to [0, len] in UTF-16 units, swap if
                // start > end.
                let units: Vec<u16> = s.encode_utf16().collect();
                let len = units.len() as f64;
                let index_of = |value: Value<'a>| -> Result<usize, Failure> {
                    let n = match value {
                        Value::Undefined => len,
                        other => to_number(&other)?,
                    };
                    let n = if n.is_nan() { 0.0 } else { n };
                    Ok(n.clamp(0.0, len) as usize)
                };
                let mut start = index_of(argument(0))?;
                let mut end = if arguments.len() > 1 {
                    index_of(argument(1))?
                } else {
                    units.len()
                };
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                }
                str_result(String::from_utf16_lossy(&units[start..end]))
            }
            other => Err(Failure::unsupported(format!("the string method `{other}`"))),
        }
    }

    // --- Property access ---

    fn read_property<'a>(
        &mut self,
        subject: &Value<'a>,
        member: &str,
    ) -> Result<Value<'a>, Failure> {
        match (subject, member) {
            (Value::Str(s), "length") => Ok(Value::Number(s.encode_utf16().count() as f64)),
            (Value::Array(items), "length") => Ok(Value::Number(items.borrow().len() as f64)),
            (Value::Set(entries), "size") => Ok(Value::Number(entries.borrow().len() as f64)),
            (Value::Map(entries), "size") => Ok(Value::Number(entries.borrow().len() as f64)),
            (Value::Object(object), key) => Ok(object
                .borrow()
                .get(key)
                .cloned()
                .unwrap_or(Value::Undefined)),
            (other, member) => Err(Failure::internal(format!(
                "property `{member}` on {}",
                type_name(other)
            ))),
        }
    }

    fn read_index<'a>(
        &mut self,
        subject: &Value<'a>,
        index: &Value<'a>,
    ) -> Result<Value<'a>, Failure> {
        match subject {
            Value::Array(items) => {
                // JS canonicalizes string keys: `arr["0"]` is `arr[0]` (the
                // from_json path indexes tuple payloads with string keys).
                let n = match index {
                    Value::Str(key) => match key.parse::<usize>() {
                        Ok(n) => n as f64,
                        Err(_) => return Ok(Value::Undefined),
                    },
                    other => expect_number(other)?,
                };
                let items = items.borrow();
                if n >= 0.0 && n.fract() == 0.0 && (n as usize) < items.len() {
                    Ok(items[n as usize].clone())
                } else {
                    Ok(Value::Undefined)
                }
            }
            Value::Object(object) => {
                let key = self.to_js_string(index)?;
                Ok(object
                    .borrow()
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or(Value::Undefined))
            }
            other => Err(Failure::internal(format!(
                "indexing into {}",
                type_name(other)
            ))),
        }
    }

    fn write_target<'a>(
        &mut self,
        target: &'a js::Node<'a>,
        value: Value<'a>,
        env: &Env<'a>,
    ) -> Result<(), Failure> {
        match target {
            js::Node::Local(name) => assign(env, name.as_str(), value),
            js::Node::PropertyIndex(subject, index) => {
                let subject = self.eval(subject, env)?;
                let index = self.eval(index, env)?;
                match subject {
                    Value::Array(items) => {
                        let n = expect_number(&index)?;
                        if n < 0.0 || n.fract() != 0.0 {
                            return Err(Failure::internal("non-integer array index write"));
                        }
                        let n = n as usize;
                        let mut items = items.borrow_mut();
                        if n >= items.len() {
                            items.resize(n + 1, Value::Undefined);
                        }
                        items[n] = value;
                        Ok(())
                    }
                    Value::Object(object) => {
                        let key = self.to_js_string(&index)?;
                        object.borrow_mut().insert(Rc::from(key.as_str()), value);
                        Ok(())
                    }
                    other => Err(Failure::internal(format!(
                        "index write into {}",
                        type_name(&other)
                    ))),
                }
            }
            js::Node::Property(subject, member) => {
                let subject = self.eval(subject, env)?;
                match subject {
                    Value::Object(object) => {
                        object.borrow_mut().insert(Rc::from(member.as_str()), value);
                        Ok(())
                    }
                    other => Err(Failure::internal(format!(
                        "property write on {}",
                        type_name(&other)
                    ))),
                }
            }
            other => Err(Failure::internal(format!(
                "unsupported assignment target: {other:?}"
            ))),
        }
    }

    // --- Operators ---

    fn eval_binary<'a>(
        &mut self,
        op: BinaryOp,
        lhs: &'a js::Node<'a>,
        rhs: &'a js::Node<'a>,
        env: &Env<'a>,
    ) -> Result<Value<'a>, Failure> {
        // && and || short-circuit and yield an *operand*, not a bool.
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let left = self.eval(lhs, env)?;
            let take_right = match op {
                BinaryOp::And => truthy(&left),
                _ => !truthy(&left),
            };
            return if take_right {
                self.eval(rhs, env)
            } else {
                Ok(left)
            };
        }
        let left = self.eval(lhs, env)?;
        let right = self.eval(rhs, env)?;
        match op {
            BinaryOp::Add => match (&left, &right) {
                (Value::Str(_), _) | (_, Value::Str(_)) => {
                    let mut out = self.to_js_string(&left)?;
                    out.push_str(&self.to_js_string(&right)?);
                    Ok(Value::Str(Rc::from(out.as_str())))
                }
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
                (Value::BigInt(a), Value::BigInt(b)) => a
                    .checked_add(*b)
                    .map(Value::BigInt)
                    .ok_or_else(bigint_overflow),
                _ => Err(mixed_types_error("+", &left, &right)),
            },
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                match (&left, &right) {
                    (Value::Number(a), Value::Number(b)) => Ok(Value::Number(match op {
                        BinaryOp::Sub => a - b,
                        BinaryOp::Mul => a * b,
                        BinaryOp::Rem => a % b,
                        _ => a / b,
                    })),
                    (Value::BigInt(a), Value::BigInt(b)) => {
                        let result = match op {
                            BinaryOp::Sub => a.checked_sub(*b),
                            BinaryOp::Mul => a.checked_mul(*b),
                            _ => {
                                if *b == 0 {
                                    return Err(Failure {
                                        kind: FailureKind::Thrown,
                                        message: "Division by zero".to_string(),
                                    });
                                }
                                if matches!(op, BinaryOp::Rem) {
                                    a.checked_rem(*b)
                                } else {
                                    a.checked_div(*b)
                                }
                            }
                        };
                        result.map(Value::BigInt).ok_or_else(bigint_overflow)
                    }
                    _ => Err(mixed_types_error(op_symbol(op), &left, &right)),
                }
            }
            BinaryOp::Shl
            | BinaryOp::Shr
            | BinaryOp::BitAnd
            | BinaryOp::BitXor
            | BinaryOp::BitOr => match (&left, &right) {
                (Value::Number(a), Value::Number(b)) => {
                    let a = to_int32(*a);
                    let result = match op {
                        BinaryOp::Shl => a.wrapping_shl(to_uint32(*b) & 31),
                        BinaryOp::Shr => a.wrapping_shr(to_uint32(*b) & 31),
                        BinaryOp::BitAnd => a & to_int32(*b),
                        BinaryOp::BitXor => a ^ to_int32(*b),
                        _ => a | to_int32(*b),
                    };
                    Ok(Value::Number(result as f64))
                }
                (Value::BigInt(a), Value::BigInt(b)) => {
                    let result = match op {
                        BinaryOp::Shl => {
                            let shift = u32::try_from(*b).map_err(|_| bigint_overflow())?;
                            a.checked_shl(shift).ok_or_else(bigint_overflow)?
                        }
                        BinaryOp::Shr => {
                            let shift = u32::try_from(*b).map_err(|_| bigint_overflow())?;
                            a.checked_shr(shift).ok_or_else(bigint_overflow)?
                        }
                        BinaryOp::BitAnd => a & b,
                        BinaryOp::BitXor => a ^ b,
                        _ => a | b,
                    };
                    Ok(Value::BigInt(result))
                }
                _ => Err(mixed_types_error(op_symbol(op), &left, &right)),
            },
            BinaryOp::UShr => match (&left, &right) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(
                    (to_uint32(*a) >> (to_uint32(*b) & 31)) as f64,
                )),
                _ => Err(mixed_types_error(">>>", &left, &right)),
            },
            BinaryOp::Eq => Ok(Value::Bool(strict_equals(&left, &right))),
            BinaryOp::NotEq => Ok(Value::Bool(!strict_equals(&left, &right))),
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq => {
                let ordering = match (&left, &right) {
                    (Value::Str(a), Value::Str(b)) => a.encode_utf16().cmp(b.encode_utf16()),
                    (Value::Number(a), Value::Number(b)) => match a.partial_cmp(b) {
                        Some(ordering) => ordering,
                        None => return Ok(Value::Bool(false)), // NaN
                    },
                    (Value::BigInt(a), Value::BigInt(b)) => a.cmp(b),
                    _ => return Err(mixed_types_error(op_symbol(op), &left, &right)),
                };
                Ok(Value::Bool(match op {
                    BinaryOp::Lt => ordering.is_lt(),
                    BinaryOp::Gt => ordering.is_gt(),
                    BinaryOp::LtEq => ordering.is_le(),
                    _ => ordering.is_ge(),
                }))
            }
            BinaryOp::And | BinaryOp::Or => unreachable!("short-circuited above"),
        }
    }

    // --- Conversions & printing ---

    /// JS `ToString` — string concatenation and `String(x)`.
    fn to_js_string(&mut self, value: &Value) -> Result<String, Failure> {
        Ok(match value {
            Value::Undefined => "undefined".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(x) => x.to_string(),
            Value::Number(n) => js_number_to_string(*n),
            Value::BigInt(n) => n.to_string(),
            Value::Str(s) => s.to_string(),
            Value::Array(items) => {
                let items = items.borrow().clone();
                let mut out = String::new();
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    if !matches!(item, Value::Undefined | Value::Null) {
                        out.push_str(&self.to_js_string(item)?);
                    }
                }
                out
            }
            Value::Object(_) => "[object Object]".to_string(),
            Value::Set(_) => "[object Set]".to_string(),
            Value::Map(_) => "[object Map]".to_string(),
            Value::Closure(_) => {
                return Err(Failure::unsupported("stringifying a function"));
            }
        })
    }

    /// Node's `console.log` rendering — what the equivalence suite compares
    /// against real stdout. Top-level strings print raw; nested ones quote.
    fn inspect(&mut self, value: &Value, top_level: bool) -> Result<String, Failure> {
        Ok(match value {
            Value::Str(s) if top_level => s.to_string(),
            Value::Str(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => "-0".to_string(),
            Value::Number(n) => js_number_to_string(*n),
            Value::BigInt(n) => format!("{n}n"),
            Value::Undefined => "undefined".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(x) => x.to_string(),
            Value::Array(items) => {
                let items = items.borrow().clone();
                if items.is_empty() {
                    "[]".to_string()
                } else {
                    let mut out = String::from("[ ");
                    for (index, item) in items.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(&self.inspect(item, false)?);
                    }
                    out.push_str(" ]");
                    out
                }
            }
            Value::Set(entries) => {
                let values: Vec<Value> = entries.borrow().values().cloned().collect();
                let mut out = format!("Set({})", values.len());
                if values.is_empty() {
                    out.push_str(" {}");
                } else {
                    out.push_str(" { ");
                    for (index, item) in values.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(&self.inspect(item, false)?);
                    }
                    out.push_str(" }");
                }
                out
            }
            Value::Map(entries) => {
                let pairs: Vec<(Value, Value)> = entries.borrow().values().cloned().collect();
                let mut out = format!("Map({})", pairs.len());
                if pairs.is_empty() {
                    out.push_str(" {}");
                } else {
                    out.push_str(" { ");
                    for (index, (key, value)) in pairs.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        let _ = write!(
                            out,
                            "{} => {}",
                            self.inspect(key, false)?,
                            self.inspect(value, false)?
                        );
                    }
                    out.push_str(" }");
                }
                out
            }
            Value::Object(object) => {
                let entries: Vec<(Rc<str>, Value)> = object
                    .borrow()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                if entries.is_empty() {
                    "{}".to_string()
                } else {
                    let mut out = String::from("{ ");
                    for (index, (key, value)) in entries.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        let _ = write!(out, "{}: {}", key, self.inspect(value, false)?);
                    }
                    out.push_str(" }");
                    out
                }
            }
            Value::Closure(closure) => match closure.name {
                Some(name) => format!("[Function: {name}]"),
                None => "[Function (anonymous)]".to_string(),
            },
        })
    }
}

// --- Value helpers ---

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::BigInt(_) => "a BigInt",
        Value::Str(_) => "a string",
        Value::Array(_) => "an array",
        Value::Set(_) => "a Set",
        Value::Map(_) => "a Map",
        Value::Object(_) => "an object",
        Value::Closure(_) => "a function",
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Bool(x) => *x,
        Value::Number(n) => !(*n == 0.0 || n.is_nan()),
        Value::BigInt(n) => *n != 0,
        Value::Str(s) => !s.is_empty(),
        _ => true,
    }
}

/// JS `===`: same-type value equality; reference identity for containers.
fn strict_equals<'a>(a: &Value<'a>, b: &Value<'a>) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x == y, // NaN !== NaN, -0 === 0
        (Value::BigInt(x), Value::BigInt(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Set(x), Value::Set(y)) => Rc::ptr_eq(x, y),
        (Value::Map(x), Value::Map(y)) => Rc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        (Value::Closure(x), Value::Closure(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// `__clone`'s semantics exactly: arrays/Sets/Maps copy deeply, everything
/// else — primitives, closures, and `{ v }` cells — shares by reference.
fn deep_clone<'a>(value: &Value<'a>) -> Value<'a> {
    match value {
        Value::Array(items) => {
            let cloned = items.borrow().iter().map(deep_clone).collect();
            Value::Array(Rc::new(RefCell::new(cloned)))
        }
        Value::Set(entries) => {
            let cloned = entries
                .borrow()
                .iter()
                .map(|(key, value)| (key.clone(), deep_clone(value)))
                .collect();
            Value::Set(Rc::new(RefCell::new(cloned)))
        }
        Value::Map(entries) => {
            let cloned = entries
                .borrow()
                .iter()
                .map(|(key, (key_value, value))| {
                    (key.clone(), (deep_clone(key_value), deep_clone(value)))
                })
                .collect();
            Value::Map(Rc::new(RefCell::new(cloned)))
        }
        other => other.clone(),
    }
}

fn spread_into<'a>(value: &Value<'a>, out: &mut Vec<Value<'a>>) -> Result<(), Failure> {
    match value {
        Value::Array(items) => {
            out.extend(items.borrow().iter().cloned());
            Ok(())
        }
        Value::Set(entries) => {
            out.extend(entries.borrow().values().cloned());
            Ok(())
        }
        Value::Map(entries) => {
            out.extend(entries.borrow().values().map(|(key, value)| {
                Value::Array(Rc::new(RefCell::new(vec![key.clone(), value.clone()])))
            }));
            Ok(())
        }
        other => Err(Failure::internal(format!("spread of {}", type_name(other)))),
    }
}

/// The `Option` array forms the backend uses: `Some(v)` = `[0, v]`,
/// `None` = `[1]`.
fn option_some(value: Value) -> Value {
    Value::Array(Rc::new(RefCell::new(vec![Value::Number(0.0), value])))
}

fn option_none<'a>() -> Value<'a> {
    Value::Array(Rc::new(RefCell::new(vec![Value::Number(1.0)])))
}

fn expect_number(value: &Value) -> Result<f64, Failure> {
    match value {
        Value::Number(n) => Ok(*n),
        other => Err(Failure::internal(format!(
            "expected a number, got {}",
            type_name(other)
        ))),
    }
}

/// The checked-subscript panic (`__at`/`__at_put`/`__at_view`), worded exactly
/// as the emitted helpers throw it. `Thrown`, not `Internal`: an out-of-bounds
/// subscript is the macro's own bug, like a `panic` in its body.
fn index_out_of_bounds(length: usize, index: f64) -> Failure {
    Failure {
        kind: FailureKind::Thrown,
        message: format!("index out of bounds: the length is {length} but the index is {index}"),
    }
}

fn expect_str(value: &Value) -> Result<Rc<str>, Failure> {
    match value {
        Value::Str(s) => Ok(s.clone()),
        other => Err(Failure::internal(format!(
            "expected a string, got {}",
            type_name(other)
        ))),
    }
}

fn expect_array<'a>(value: &Value<'a>) -> Result<Rc<RefCell<Vec<Value<'a>>>>, Failure> {
    match value {
        Value::Array(items) => Ok(items.clone()),
        other => Err(Failure::internal(format!(
            "expected an array, got {}",
            type_name(other)
        ))),
    }
}

#[allow(clippy::type_complexity)]
fn expect_map<'a>(
    value: &Value<'a>,
) -> Result<Rc<RefCell<IndexMap<Key, (Value<'a>, Value<'a>)>>>, Failure> {
    match value {
        Value::Map(entries) => Ok(entries.clone()),
        other => Err(Failure::internal(format!(
            "expected a Map, got {}",
            type_name(other)
        ))),
    }
}

fn bigint_overflow() -> Failure {
    Failure::unsupported("a BigInt beyond 128 bits (the expansion engine's v1 bound)")
}

fn mixed_types_error(op: &str, left: &Value<'_>, right: &Value<'_>) -> Failure {
    Failure::internal(format!(
        "`{op}` on {} and {}",
        type_name(left),
        type_name(right)
    ))
}

fn op_symbol(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Rem => "%",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        BinaryOp::UShr => ">>>",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitXor => "^",
        BinaryOp::BitOr => "|",
        BinaryOp::Eq => "===",
        BinaryOp::NotEq => "!==",
        BinaryOp::Lt => "<",
        BinaryOp::Gt => ">",
        BinaryOp::LtEq => "<=",
        BinaryOp::GtEq => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
    }
}

// --- Numeric semantics ---

/// ECMA-262 `ToInt32`.
fn to_int32(n: f64) -> i32 {
    to_uint32(n) as i32
}

/// ECMA-262 `ToUint32`.
fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let n = n.trunc();
    let modulo = n.rem_euclid(4294967296.0);
    modulo as u32
}

/// A `js::Node::Number` literal: plain decimals, hex (`0x..`), and BigInt
/// (`..n`) — the forms the backend writes through from vilan literals.
fn parse_number_literal<'a>(whole: &str, fraction: Option<&str>) -> Result<Value<'a>, Failure> {
    let whole = whole.replace('_', "");
    if let Some(digits) = whole.strip_suffix('n') {
        let n = digits.parse::<i128>().map_err(|_| bigint_overflow())?;
        return Ok(Value::BigInt(n));
    }
    if let Some(hex) = whole
        .strip_prefix("0x")
        .or_else(|| whole.strip_prefix("0X"))
    {
        let n = u64::from_str_radix(hex, 16)
            .map_err(|_| Failure::internal(format!("bad hex literal `{whole}`")))?;
        return Ok(Value::Number(n as f64));
    }
    let text = match fraction {
        Some(fraction) => format!("{whole}.{fraction}"),
        None => whole.clone(),
    };
    text.parse::<f64>()
        .map(Value::Number)
        .map_err(|_| Failure::internal(format!("bad number literal `{text}`")))
}

/// JS `Number(string)` (ToNumber on a trimmed string).
fn js_string_to_number(trimmed: &str) -> f64 {
    if trimmed.is_empty() {
        return 0.0;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16)
            .map(|n| n as f64)
            .unwrap_or(f64::NAN);
    }
    if let Some(oct) = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
    {
        return u64::from_str_radix(oct, 8)
            .map(|n| n as f64)
            .unwrap_or(f64::NAN);
    }
    if let Some(bin) = trimmed
        .strip_prefix("0b")
        .or_else(|| trimmed.strip_prefix("0B"))
    {
        return u64::from_str_radix(bin, 2)
            .map(|n| n as f64)
            .unwrap_or(f64::NAN);
    }
    match trimmed {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust's f64 parser accepts the JS decimal grammar plus "nan"/"inf"
    // spellings JS rejects — filter those.
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.contains("nan") || lowered.contains("inf") {
        return f64::NAN;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// JS `ToNumber`.
fn to_number(value: &Value) -> Result<f64, Failure> {
    Ok(match value {
        Value::Number(n) => *n,
        Value::Bool(true) => 1.0,
        Value::Bool(false) | Value::Null => 0.0,
        Value::Undefined => f64::NAN,
        Value::Str(s) => js_string_to_number(s.trim()),
        other => {
            return Err(Failure::internal(format!(
                "ToNumber on {}",
                type_name(other)
            )));
        }
    })
}

/// ECMA-262 `Number::toString(10)`: shortest round-trip digits, integer forms
/// without a decimal point, exponential notation past the |n| ≥ 1e21 and
/// |n| < 1e-6 thresholds.
pub(crate) fn js_number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let magnitude = n.abs();
    if magnitude >= 1e21 {
        // "1e+21" — Rust writes "1e21"; JS puts a '+' on positive exponents.
        let s = format!("{n:e}");
        return match s.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with('-') => {
                format!("{mantissa}e+{exponent}")
            }
            _ => s,
        };
    }
    if magnitude < 1e-6 {
        return format!("{n:e}");
    }
    // Rust's `{}` is shortest-round-trip decimal without exponent — exactly
    // JS's rendering inside the thresholds.
    format!("{n}")
}

/// The number a slice method's relative index resolves to (JS `slice`).
fn relative_index(value: Value, default: f64, len: f64) -> Result<f64, Failure> {
    let n = match value {
        Value::Undefined => default,
        other => to_number(&other)?,
    };
    let n = if n.is_nan() { 0.0 } else { n.trunc() };
    Ok(if n < 0.0 {
        (len + n).max(0.0)
    } else {
        n.min(len)
    })
}

// --- JSON ---

/// `JSON.stringify`, over the values emitted code can hold. Returns whether
/// anything was written (`undefined`/functions stringify to nothing).
fn json_stringify(value: &Value, out: &mut String) -> Result<bool, Failure> {
    match value {
        Value::Undefined | Value::Closure(_) => Ok(false),
        Value::Null => {
            out.push_str("null");
            Ok(true)
        }
        Value::Bool(x) => {
            out.push_str(if *x { "true" } else { "false" });
            Ok(true)
        }
        Value::Number(n) => {
            if n.is_finite() {
                out.push_str(&js_number_to_string(*n));
            } else {
                out.push_str("null");
            }
            Ok(true)
        }
        Value::BigInt(_) => Err(Failure {
            kind: FailureKind::Thrown,
            message: "Do not know how to serialize a BigInt".to_string(),
        }),
        Value::Str(s) => {
            json_escape(s, out);
            Ok(true)
        }
        Value::Array(items) => {
            out.push('[');
            let items = items.borrow();
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                if !json_stringify(item, out)? {
                    out.push_str("null");
                }
            }
            out.push(']');
            Ok(true)
        }
        Value::Object(object) => {
            out.push('{');
            let mut first = true;
            for (key, value) in object.borrow().iter() {
                let mut piece = String::new();
                if json_stringify(value, &mut piece)? {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    json_escape(key, out);
                    out.push(':');
                    out.push_str(&piece);
                }
            }
            out.push('}');
            Ok(true)
        }
        // JS serializes Set/Map as plain objects with no enumerable keys.
        Value::Set(_) | Value::Map(_) => {
            out.push_str("{}");
            Ok(true)
        }
    }
}

fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.parse` — a small strict parser producing interpreter values
/// (objects parse to `Value::Object`, preserving key order like JS).
fn json_parse<'a>(text: &str) -> Result<Value<'a>, String> {
    let bytes = text.as_bytes();
    let mut position = 0;
    let value = json_parse_value(text, bytes, &mut position)?;
    json_skip_whitespace(bytes, &mut position);
    if position != bytes.len() {
        return Err("Unexpected non-whitespace character after JSON data".to_string());
    }
    Ok(value)
}

fn json_skip_whitespace(bytes: &[u8], position: &mut usize) {
    while *position < bytes.len() && matches!(bytes[*position], b' ' | b'\t' | b'\n' | b'\r') {
        *position += 1;
    }
}

fn json_parse_value<'a>(
    text: &str,
    bytes: &[u8],
    position: &mut usize,
) -> Result<Value<'a>, String> {
    json_skip_whitespace(bytes, position);
    let Some(&byte) = bytes.get(*position) else {
        return Err("Unexpected end of JSON input".to_string());
    };
    match byte {
        b'n' => json_expect(text, position, "null").map(|_| Value::Null),
        b't' => json_expect(text, position, "true").map(|_| Value::Bool(true)),
        b'f' => json_expect(text, position, "false").map(|_| Value::Bool(false)),
        b'"' => json_parse_string(text, bytes, position).map(|s| Value::Str(Rc::from(s.as_str()))),
        b'[' => {
            *position += 1;
            let mut items = Vec::new();
            json_skip_whitespace(bytes, position);
            if bytes.get(*position) == Some(&b']') {
                *position += 1;
                return Ok(Value::Array(Rc::new(RefCell::new(items))));
            }
            loop {
                items.push(json_parse_value(text, bytes, position)?);
                json_skip_whitespace(bytes, position);
                match bytes.get(*position) {
                    Some(b',') => *position += 1,
                    Some(b']') => {
                        *position += 1;
                        return Ok(Value::Array(Rc::new(RefCell::new(items))));
                    }
                    _ => return Err("Expected ',' or ']' in JSON array".to_string()),
                }
            }
        }
        b'{' => {
            *position += 1;
            let mut object = IndexMap::new();
            json_skip_whitespace(bytes, position);
            if bytes.get(*position) == Some(&b'}') {
                *position += 1;
                return Ok(Value::Object(Rc::new(RefCell::new(object))));
            }
            loop {
                json_skip_whitespace(bytes, position);
                if bytes.get(*position) != Some(&b'"') {
                    return Err("Expected a string key in JSON object".to_string());
                }
                let key = json_parse_string(text, bytes, position)?;
                json_skip_whitespace(bytes, position);
                if bytes.get(*position) != Some(&b':') {
                    return Err("Expected ':' in JSON object".to_string());
                }
                *position += 1;
                let value = json_parse_value(text, bytes, position)?;
                object.insert(Rc::from(key.as_str()), value);
                json_skip_whitespace(bytes, position);
                match bytes.get(*position) {
                    Some(b',') => *position += 1,
                    Some(b'}') => {
                        *position += 1;
                        return Ok(Value::Object(Rc::new(RefCell::new(object))));
                    }
                    _ => return Err("Expected ',' or '}' in JSON object".to_string()),
                }
            }
        }
        b'-' | b'0'..=b'9' => {
            let start = *position;
            if bytes.get(*position) == Some(&b'-') {
                *position += 1;
            }
            while *position < bytes.len()
                && matches!(
                    bytes[*position],
                    b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
                )
            {
                *position += 1;
            }
            text[start..*position]
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|_| "Invalid number in JSON".to_string())
        }
        other => Err(format!("Unexpected token '{}' in JSON", char::from(other))),
    }
}

fn json_expect(text: &str, position: &mut usize, literal: &str) -> Result<(), String> {
    if text[*position..].starts_with(literal) {
        *position += literal.len();
        Ok(())
    } else {
        Err(format!("Expected `{literal}` in JSON"))
    }
}

fn json_parse_string(text: &str, bytes: &[u8], position: &mut usize) -> Result<String, String> {
    debug_assert_eq!(bytes[*position], b'"');
    *position += 1;
    let mut out = String::new();
    let mut chars = text[*position..].char_indices();
    while let Some((offset, c)) = chars.next() {
        match c {
            '"' => {
                *position += offset + 1;
                return Ok(out);
            }
            '\\' => {
                let Some((_, escaped)) = chars.next() else {
                    return Err("Unterminated escape in JSON string".to_string());
                };
                match escaped {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let Some((_, digit)) = chars.next() else {
                                return Err("Bad \\u escape in JSON string".to_string());
                            };
                            code = code * 16
                                + digit.to_digit(16).ok_or("Bad \\u escape in JSON string")?;
                        }
                        // Surrogate pairs: not reconstructed in v1 (macro
                        // inputs are source text, not binary); a lone
                        // surrogate becomes the replacement character.
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    other => return Err(format!("Bad escape `\\{other}` in JSON string")),
                }
            }
            c if (c as u32) < 0x20 => {
                return Err("Unescaped control character in JSON string".to_string());
            }
            c => out.push(c),
        }
    }
    Err("Unterminated JSON string".to_string())
}
