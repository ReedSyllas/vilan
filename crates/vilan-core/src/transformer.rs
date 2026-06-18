use crate::analyzer::{Expr, ExprIfBranch, ExprPattern, Function, Intrinsic, Program};
use crate::error::Error;
use crate::id::Id;
use crate::node::{BinaryOp, ExternBinding};
use crate::type_::{Type, TypeId};
use chumsky::span::Span;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub fn transform<'src>(program: &Program<'src>) -> Result<String, Error> {
    Transformer::new(program, true).transform_entry()
}

/// Interprets a string literal's backslash escapes into the characters they
/// denote (`\n` -> newline, `\t`, `\r`, `\"`, `\\`, `\0`), so the value is the
/// real text — the JS formatter then re-escapes it for output. Borrows the slice
/// unchanged when it has no escapes. An unknown escape keeps both characters.
fn unescape_string(raw: &str) -> Cow<'_, str> {
    if !raw.contains('\\') {
        return Cow::Borrowed(raw);
    }
    let mut result = String::with_capacity(raw.len());
    let mut characters = raw.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('"') => result.push('"'),
            Some('\\') => result.push('\\'),
            Some('0') => result.push('\0'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    Cow::Owned(result)
}

/// The JS source for a runtime helper an intrinsic call needs. `__scan` reads
/// all of stdin once and hands out one line per call; `__parse_i32` returns the
/// `Option<i32>` array form (`[0, n]` = `Some`, `[1]` = `None`).
fn helper_source(name: &str) -> &'static str {
    match name {
        "__scan" => {
            "let __vilan_stdin = null, __vilan_stdin_index = 0;\n\
             function __scan() {\n\
             \tif (__vilan_stdin === null) {\n\
             \t\ttry {\n\
             \t\t\t__vilan_stdin = require(\"fs\").readFileSync(0, \"utf-8\").split(\"\\n\");\n\
             \t\t} catch (error) {\n\
             \t\t\t__vilan_stdin = [];\n\
             \t\t}\n\
             \t}\n\
             \treturn __vilan_stdin_index < __vilan_stdin.length ? __vilan_stdin[__vilan_stdin_index++] : \"\";\n\
             }"
        }
        "__parse_i32" => {
            "function __parse_i32(text) {\n\
             \tconst value = Number.parseInt(text, 10);\n\
             \treturn Number.isNaN(value) ? [ 1 ] : [ 0, value ];\n\
             }"
        }
        "__random_int" => {
            "function __random_int(low, high) {\n\
             \treturn Math.floor(Math.random() * (high - low + 1)) + low;\n\
             }"
        }
        "__random_float" => {
            "function __random_float(low, high) {\n\
             \treturn Math.random() * (high - low) + low;\n\
             }"
        }
        // Value-semantics deep clone. Structs/lists/enums/tuples are arrays, so
        // recurse into them; everything else — primitives and closures — is
        // returned by reference (a closure is immutable, so sharing it is a
        // copy). Unlike `structuredClone`, this doesn't throw on functions.
        "__clone" => {
            "function __clone(value) {\n\
             \treturn Array.isArray(value) ? value.map(__clone) : value;\n\
             }"
        }
        _ => "",
    }
}

/// Whether two types name the same nominal struct/enum, ignoring type
/// arguments — so an `impl List<T>` (subject `List<Generic>`) matches a concrete
/// `List<i32>` value when resolving a member to emit.
fn nominal_matches(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Struct(a_id, _), Type::Struct(b_id, _)) => a_id == b_id,
        (Type::Enum(a_id, _), Type::Enum(b_id, _)) => a_id == b_id,
        _ => a == b,
    }
}

/// Builds a binary expression, gluing adjacent string literals at compile time
/// so concatenations like `"" + "Hello, " + "!"` collapse to a single literal.
/// Because `+` is left-associative, folding here folds whole static runs.
fn binary<'src>(op: BinaryOp, lhs: js::Node<'src>, rhs: js::Node<'src>) -> js::Node<'src> {
    match (op, lhs, rhs) {
        (BinaryOp::Add, js::Node::String(left), js::Node::String(right)) => {
            let mut glued = left.into_owned();
            glued.push_str(&right);
            js::Node::String(Cow::Owned(glued))
        }
        (op, lhs, rhs) => js::Node::Binary(op, Box::new(lhs), Box::new(rhs)),
    }
}

struct Transformer<'src> {
    formatter: Formatter,
    ng: NameGenerator,
    print_fn_id: Id,
    list_new_fn_id: Option<Id>,
    list_push_fn_id: Option<Id>,
    panic_fn_id: Option<Id>,
    program: &'src Program<'src>,
    required_functions: IndexMap<Id, js::Node<'src>>,
    // The active generic-parameter substitution while emitting a monomorphized
    // function body (constraint id -> concrete type id).
    current_substitution: HashMap<TypeId, TypeId>,
    // Monomorphized function variants, keyed by (generic function, concrete
    // type arguments) so each distinct instantiation is emitted exactly once.
    instances: HashMap<(Id, Vec<String>), String>,
    // The concrete type a trait default method is currently being specialized
    // for, so `self.method()` calls in its body re-dispatch to that type's impl.
    current_self_type: Option<TypeId>,
    // Trait default methods specialized per concrete type, keyed by
    // (default function, concrete type) so each is emitted once.
    default_instances: HashMap<(Id, String), String>,
    monomorphized: Vec<js::Node<'src>>,
    // Captures introduced by an `is` test, aliased to the subject's payload
    // slots (e.g. `t[1]`) since they can't be JS bindings in expression position.
    is_bindings: HashMap<Id, js::Node<'src>>,
    // Runtime helper functions (`__scan`, `__parse_i32`, `__random_int`) an
    // intrinsic call needs; emitted as a prelude only when used.
    used_helpers: BTreeSet<&'static str>,
    // Host imports an `@extern` call needs, as module -> imported symbols;
    // emitted as `import { a, b } from "module";` lines at the top.
    used_imports: BTreeMap<String, BTreeSet<String>>,
}

impl<'src> Transformer<'src> {
    fn new(program: &'src Program<'src>, should_pretty_print: bool) -> Self {
        let debug_names = if should_pretty_print {
            program
                .variables
                .iter()
                .map(|(id, variable)| (*id, variable.name.to_string()))
                .chain(
                    program
                        .functions
                        .iter()
                        .map(|(id, function)| (*id, function.name.to_string())),
                )
                .collect::<HashMap<Id, String>>()
        } else {
            HashMap::new()
        };

        let print_fn_id = {
            let std_module_id = *program
                .module_id_by_name
                .get("std")
                .expect("missing std module");
            let std_module = program.modules.get(&std_module_id).unwrap();
            let std_module_scope_id = std_module.body.1;
            let std_module_scope = program.scopes.get(&std_module_scope_id).unwrap();
            let print_fn_id = *std_module_scope
                .name_to_id_map
                .get("print")
                .expect("missing print function in the std module");
            print_fn_id
        };

        Self {
            formatter: if should_pretty_print {
                Formatter::new_pretty()
            } else {
                Formatter::new_compact()
            },
            ng: NameGenerator::new_simple(debug_names),
            print_fn_id,
            list_new_fn_id: program.list_new_fn_id,
            list_push_fn_id: program.list_push_fn_id,
            panic_fn_id: program.panic_fn_id,
            program,
            required_functions: IndexMap::new(),
            current_substitution: HashMap::new(),
            instances: HashMap::new(),
            current_self_type: None,
            default_instances: HashMap::new(),
            monomorphized: Vec::new(),
            is_bindings: HashMap::new(),
            used_helpers: BTreeSet::new(),
            used_imports: BTreeMap::new(),
        }
    }

    fn transform_entry(mut self) -> Result<String, Error> {
        let global_scope = self
            .program
            .scopes
            .get(&self.program.global_scope_id)
            .unwrap();

        let global_variables = self.find_global_variables(
            &global_scope
                .name_to_id_map
                .iter()
                .map(|(_, x)| *x)
                .collect(),
        );

        let main_fn = global_scope
            .name_to_id_map
            .get("main")
            .and_then(|id| self.program.functions.get(id))
            .ok_or_else(|| Error {
                msg: "Cannot execute program without a main function".to_string(),
                span: Span::new((), 0..0),
            })?;
        let main_is_async = self.program.async_functions.contains(&main_fn.id);

        let t_global_variables = self.walk_list(&global_variables);

        let mut t_main_fn_body = self.walk_list(&main_fn.body.0);

        // Emit main's trailing expression (and any statements it expands to).
        // Only a non-void result is forwarded to `process.exit`; a tail that
        // evaluates to void (e.g. a block ending in a loop) exits normally.
        if let Some(value) = self.walk_entity(main_fn.body.1, &mut t_main_fn_body) {
            if !matches!(value, js::Node::Void) {
                let t_exit = js::Node::Call(
                    Box::new(js::Node::Property(
                        Box::new(js::Node::Local("process".to_string())),
                        "exit".to_string(),
                    )),
                    vec![value],
                );
                t_main_fn_body.push(t_exit);
            }
        }

        // An async `main` (it awaits) runs inside an invoked async arrow, since
        // top-level `await` isn't assumed: `(async () => { .. })()`.
        if main_is_async {
            t_main_fn_body = vec![js::Node::Call(
                Box::new(js::Node::Closure(js::Closure {
                    parameters: Vec::new(),
                    body: t_main_fn_body,
                    is_async: true,
                })),
                Vec::new(),
            )];
        }

        let mut t_functions = self.required_functions.into_iter().collect::<Vec<_>>();
        t_functions.sort_by(|a, b| (a.0.0).cmp(&b.0.0));
        let t_functions = t_functions.into_iter().map(|x| x.1);

        // Monomorphized variants are plain function declarations too; ordering
        // among declarations is irrelevant since JS hoists them.
        let t_instances = self.monomorphized.into_iter();

        // Host imports (`import { a, b } from "module";`) from `@extern` calls,
        // then runtime helpers (`__scan`, ...) — both a prelude before the body.
        let imports = self
            .used_imports
            .iter()
            .map(|(module, symbols)| {
                let names = symbols.iter().cloned().collect::<Vec<_>>().join(", ");
                format!("import {{ {} }} from \"{}\";", names, module)
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Value-semantics copies (`own` arguments, aggregate bindings) lower to
        // the `__clone` helper rather than `structuredClone`, which can't copy
        // the closures a struct may hold.
        if !self.program.clone_sites.is_empty() {
            self.used_helpers.insert("__clone");
        }
        let helpers = self
            .used_helpers
            .iter()
            .map(|name| helper_source(name))
            .collect::<Vec<_>>()
            .join("\n");
        let prelude = [imports, helpers]
            .into_iter()
            .filter(|section| !section.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let body = self.formatter.file(
            &t_functions
                .chain(t_instances)
                .chain(t_global_variables.into_iter())
                .chain(t_main_fn_body.into_iter())
                .collect::<Vec<_>>(),
        );
        let output = if prelude.is_empty() {
            body
        } else {
            format!("{}\n{}", prelude, body)
        };
        Ok(format!("{}{}", output, self.formatter.line_break))
    }

    fn find_global_variables(&self, globals: &Vec<Id>) -> Vec<Id> {
        let mut global_variables = Vec::new();

        for id in globals {
            if self.program.variables.contains_key(id) {
                global_variables.push(*id);
            } else if self.program.modules.contains_key(id) {
                let module = self.program.modules.get(id).unwrap();
                let mut children = self.find_global_variables(&module.body.0);
                // println!("x1 {} {:#?} {:#?}", module.name, children, global_variables);
                global_variables.append(&mut children);
                // println!("x2 {:#?}", global_variables);
            }
        }

        global_variables
    }

    fn walk_list(&mut self, list: &Vec<Id>) -> Vec<js::Node<'src>> {
        let mut block = Vec::new();
        self.walk_entities(list, &mut block);
        block
    }

    fn walk_entities(&mut self, ids: &Vec<Id>, mut block: &mut Vec<js::Node<'src>>) {
        for id in ids {
            if let Some(node) = self.walk_entity(*id, &mut block) {
                // A statement whose value is discarded and is `undefined` (e.g.
                // the trailing void of a block used as a statement) is a no-op.
                if matches!(node, js::Node::Void) {
                    continue;
                }
                block.push(node);
            }
        }
    }

    /// Wraps a call in `await` when its target is async (the implicit await), so
    /// the value flows as the resolved `T` rather than a promise.
    fn maybe_await(&self, target_id: Id, node: js::Node<'src>) -> js::Node<'src> {
        if self.program.async_functions.contains(&target_id) {
            js::Node::Await(Box::new(node))
        } else {
            node
        }
    }

    /// Rule 1 (value semantics): wrap a value in `__clone(...)` when the analyzer
    /// marked this binding/assignment as copying an aggregate place that would
    /// otherwise alias its source. `__clone` (not `structuredClone`) so a value
    /// holding closures can be copied.
    fn maybe_clone(&self, value_id: Id, node: js::Node<'src>) -> js::Node<'src> {
        if self.program.clone_sites.contains(&value_id) {
            js::Node::Call(Box::new(js::Node::Local("__clone".to_string())), vec![node])
        } else {
            node
        }
    }

    /// Whether a deref operand views a boxed scalar local — so `*operand` reads
    /// or writes the cell's slot 0. True for a primitive-view binding, or `&` of
    /// a boxed local directly.
    fn derefs_boxed(&self, operand: Id) -> bool {
        match self.program.entity_map.get(&operand) {
            Some(Expr::Local(binding)) => self.program.primitive_views.contains(binding),
            Some(Expr::Reference(inner, _)) => matches!(
                self.program.entity_map.get(inner),
                Some(Expr::Local(root)) if self.program.boxed_locals.contains(root)
            ),
            _ => false,
        }
    }

    fn walk_entity(&mut self, id: Id, block: &mut Vec<js::Node<'src>>) -> Option<js::Node<'src>> {
        let entity = self.program.entity_map.get(&id).unwrap();

        Some(match entity {
            Expr::Error => unreachable!(),
            Expr::Void => js::Node::Void,
            Expr::Null => js::Node::Null,
            Expr::Bool(x) => js::Node::Bool(*x),
            Expr::Number(whole, fraction, suffix) => {
                // `n`-suffixed literals are JS BigInts (`5n`); other suffixes
                // only affect typing and are dropped in the output.
                let whole = if matches!(*suffix, Some("n")) {
                    format!("{whole}n")
                } else {
                    whole.to_string()
                };
                js::Node::Number(whole, fraction.map(|x| x.to_string()))
            }
            Expr::String(x) => js::Node::String(unescape_string(x)),
            Expr::Struct(_) => {
                return None;
            }
            Expr::Enum(_) => {
                return None;
            }
            Expr::Trait(_) => {
                return None;
            }
            Expr::Impl(_) => {
                return None;
            }
            Expr::ExternalFunction(_) => {
                return None;
            }
            Expr::Generic(_) => {
                return None;
            }
            Expr::Function(id) => {
                let function = self.program.functions.get(id).unwrap();
                self.function(function)
            }
            // An enum value is an array whose first element identifies the
            // variant; a bare (data-less) variant is just `[index]`. `bool` is
            // the exception — it lowers to a native boolean.
            Expr::EnumVariant(enum_id, variant_index) => {
                self.variant_value(*enum_id, *variant_index, Vec::new())
            }
            Expr::Local(id) => {
                // A capture from an `is` test aliases the subject's payload slot.
                if let Some(accessor) = self.is_bindings.get(id) {
                    return Some(accessor.clone());
                }
                // A reference to a data-less variant (e.g. `None`) is the
                // variant value itself, not a named binding.
                if let Some(Expr::EnumVariant(enum_id, variant_index)) =
                    self.program.entity_map.get(id)
                {
                    return Some(self.variant_value(*enum_id, *variant_index, Vec::new()));
                }
                // A boxed scalar local reads through its cell's slot 0.
                if self.program.boxed_locals.contains(id) {
                    return Some(js::Node::PropertyIndex(
                        Box::new(js::Node::Local(self.ng.name_for(*id))),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    ));
                }
                js::Node::Local(self.ng.name_for(*id))
            }
            Expr::Field(subject_id, _struct_id, field_index) => {
                let subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                js::Node::PropertyIndex(
                    Box::new(subject),
                    Box::new(js::Node::Number(field_index.to_string(), None)),
                )
            }
            Expr::Call(id) => {
                let function_call = self.program.function_calls.get(id).unwrap().clone();
                let args = function_call
                    .argument_ids
                    .iter()
                    .filter_map(|arg| {
                        // An argument to an `own` parameter is copied (marked in
                        // `clone_sites`), like a binding copy.
                        self.walk_entity(*arg, block)
                            .map(|node| self.maybe_clone(*arg, node))
                    })
                    .collect::<Vec<_>>();

                // `T::member()` inside a monomorphized body: dispatch directly
                // to the concrete type's member that `T` is bound to here.
                if let Some(&(constraint_id, member_name)) = self
                    .program
                    .generic_static_accessors
                    .get(&function_call.subject_id)
                {
                    if let Some(&concrete_type_id) = self.current_substitution.get(&constraint_id) {
                        if let Some(target_id) =
                            self.resolve_member_on_type(concrete_type_id, member_name)
                        {
                            self.ensure_function_emitted(target_id);
                            let name = self.ng.name_for(target_id);
                            let call = js::Node::Call(Box::new(js::Node::Local(name)), args);
                            return Some(self.maybe_await(target_id, call));
                        }
                    }
                }

                // A trait method re-dispatched to the receiver's concrete type: an
                // inherited default called on a concrete value (Gap E, with the
                // type recorded), or a `self`-call inside a default body (no type,
                // dispatched on the type the default is being specialized for).
                if let Some(&(concrete_type, member_name)) =
                    self.program.trait_method_dispatch.get(id)
                {
                    if let Some(type_id) = concrete_type.or(self.current_self_type) {
                        if let Some((name, is_async)) =
                            self.emit_dispatched_method(type_id, member_name)
                        {
                            let call = js::Node::Call(Box::new(js::Node::Local(name)), args);
                            return Some(if is_async {
                                js::Node::Await(Box::new(call))
                            } else {
                                call
                            });
                        }
                    }
                }

                let subject = self
                    .program
                    .entity_map
                    .get(&function_call.subject_id)
                    .unwrap();
                match subject {
                    Expr::Local(target_id) => {
                        let target_id = *target_id;
                        // An external std intrinsic lowers to native JS or a
                        // runtime helper.
                        if let Some(intrinsic) = self.program.intrinsics.get(&target_id).copied() {
                            return Some(self.emit_intrinsic(intrinsic, args));
                        }
                        // An `@extern`-bound external lowers to its host (JS)
                        // import/call, method, or property access.
                        if let Some(binding) = self
                            .program
                            .external_functions
                            .get(&target_id)
                            .and_then(|external| external.extern_binding.clone())
                        {
                            let call = self.emit_extern(target_id, binding, args);
                            return Some(self.maybe_await(target_id, call));
                        }
                        // A variant constructor call builds the enum value
                        // directly: `[variant_index, ...data]` (or a native
                        // boolean for `bool`).
                        if let Some(Expr::EnumVariant(enum_id, variant_index)) =
                            self.program.entity_map.get(&target_id)
                        {
                            return Some(self.variant_value(*enum_id, *variant_index, args));
                        }
                        if target_id == self.print_fn_id {
                            return Some(js::Node::Call(
                                Box::new(js::Node::Property(
                                    Box::new(js::Node::Local("console".to_string())),
                                    "log".to_string(),
                                )),
                                args,
                            ));
                        }
                        // `List::new()` builds an empty JS array.
                        if Some(target_id) == self.list_new_fn_id {
                            return Some(js::Node::Array(Vec::new()));
                        }
                        // `list.push(x)` lowers to the native array method; the
                        // receiver is the method call's first (`self`) argument.
                        if Some(target_id) == self.list_push_fn_id {
                            let mut arguments = args.into_iter();
                            let receiver = arguments.next().unwrap_or(js::Node::Void);
                            return Some(js::Node::Call(
                                Box::new(js::Node::Property(
                                    Box::new(receiver),
                                    "push".to_string(),
                                )),
                                arguments.collect(),
                            ));
                        }
                        // `panic(msg)` lowers to a thrown error. It's wrapped in
                        // an immediately-invoked arrow so it stays valid in
                        // expression position (e.g. a match leg).
                        if Some(target_id) == self.panic_fn_id {
                            let message = args.into_iter().next().unwrap_or(js::Node::Void);
                            return Some(js::Node::Call(
                                Box::new(js::Node::Closure(js::Closure {
                                    parameters: Vec::new(),
                                    body: vec![js::Node::Throw(Box::new(message))],
                                    is_async: false,
                                })),
                                Vec::new(),
                            ));
                        }
                        // A call to a generic function is compiled to a
                        // specialized variant chosen by its concrete type
                        // arguments — no runtime dispatch.
                        let is_generic = self
                            .program
                            .functions
                            .get(&target_id)
                            .map(|f| !f.generic_parameter_constraint_ids.is_empty())
                            .unwrap_or(false);
                        if is_generic && !function_call.generic_argument_ids.is_empty() {
                            let name = self.get_or_create_instance(
                                target_id,
                                &function_call.generic_argument_ids,
                            );
                            let call = js::Node::Call(Box::new(js::Node::Local(name)), args);
                            return Some(self.maybe_await(target_id, call));
                        }
                        // A method on a generic impl whose generics bind to
                        // concrete types from the receiver (`xs.sum()` on
                        // `List<i32>`) is emitted as a monomorphized instance.
                        if let Some(substitution) = self.program.method_call_substitution.get(&id) {
                            let substitution = substitution.clone();
                            let name = self.emit_method_instance(target_id, &substitution);
                            let call = js::Node::Call(Box::new(js::Node::Local(name)), args);
                            return Some(self.maybe_await(target_id, call));
                        }
                        self.ensure_function_emitted(target_id);
                        let name = self.ng.name_for(target_id);
                        let call = js::Node::Call(Box::new(js::Node::Local(name)), args);
                        self.maybe_await(target_id, call)
                    }
                    _ => {
                        let t_subject = self.walk_entity(function_call.subject_id, block).unwrap();
                        js::Node::Call(Box::new(t_subject), args)
                    }
                }
            }
            Expr::Closure(closure_id) => {
                let closure = self.program.closures.get(closure_id).unwrap();
                let parameters = closure
                    .parameters
                    .iter()
                    .map(|parameter_id| js::Parameter {
                        name: self.ng.name_for(*parameter_id),
                    })
                    .collect::<Vec<_>>();
                let mut body = Vec::new();
                let value = self.walk_entity(closure.return_, &mut body);
                if let Some(value) = value {
                    body.push(js::Node::Return(Box::new(value)));
                }
                js::Node::Closure(js::Closure {
                    parameters,
                    body,
                    is_async: self.program.async_functions.contains(closure_id),
                })
            }
            // `async <body>` — the async-block closure invoked with no
            // arguments, yielding a promise: `(async () => { <body> })()`.
            Expr::Async(closure_id) => {
                let closure = self
                    .walk_entity(*closure_id, block)
                    .unwrap_or(js::Node::Void);
                js::Node::Call(Box::new(closure), Vec::new())
            }
            // `await <inner>`.
            Expr::Await(inner) => {
                let inner = self.walk_entity(*inner, block).unwrap_or(js::Node::Void);
                js::Node::Await(Box::new(inner))
            }
            Expr::FunctionReturn(value) => js::Node::Return(Box::new(
                self.walk_entity(*value, block).unwrap_or(js::Node::Void),
            )),
            Expr::Binary(op, lhs, rhs) => {
                let lhs = self.walk_entity(*lhs, block).unwrap_or(js::Node::Void);
                let rhs = self.walk_entity(*rhs, block).unwrap_or(js::Node::Void);
                // An overloaded operator (`a + b` where `a`'s type implements
                // `Add`) compiles to the trait method call `add(a, b)`.
                if let Some(&method_id) = self.program.binary_op_dispatch.get(&id) {
                    self.ensure_function_emitted(method_id);
                    let name = self.ng.name_for(method_id);
                    return Some(js::Node::Call(
                        Box::new(js::Node::Local(name)),
                        vec![lhs, rhs],
                    ));
                }
                binary(*op, lhs, rhs)
            }
            Expr::Unary(operator, operand) => {
                let operand = self.walk_entity(*operand, block).unwrap_or(js::Node::Void);
                js::Node::Unary(*operator, Box::new(operand))
            }
            // A view of an aggregate is the value's own JS reference; a view of a
            // boxed scalar local is its cell. Either way `&x` lowers to the cell
            // / reference, not a `[0]` read.
            Expr::Reference(operand, _) => {
                if let Some(Expr::Local(root)) = self.program.entity_map.get(operand) {
                    if self.program.boxed_locals.contains(root) {
                        return Some(js::Node::Local(self.ng.name_for(*root)));
                    }
                }
                return self.walk_entity(*operand, block);
            }
            // Deref of an aggregate view is the operand itself; deref of a
            // primitive view reads/writes through the cell's slot 0.
            Expr::Dereference(operand) => {
                let value = self.walk_entity(*operand, block);
                if self.derefs_boxed(*operand) {
                    return Some(js::Node::PropertyIndex(
                        Box::new(value.unwrap_or(js::Node::Void)),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    ));
                }
                return value;
            }
            Expr::Variable(id) => {
                if self
                    .program
                    .reference_count
                    .get(id)
                    .map(|x| *x < 1)
                    .unwrap_or(true)
                {
                    return None;
                }
                let name = self.ng.name_for(*id);
                let variable = self.program.variables.get(id).unwrap();
                let value = variable
                    .initial
                    .and_then(|value_id| {
                        self.walk_entity(value_id, block)
                            .map(|node| self.maybe_clone(value_id, node))
                    })
                    .unwrap_or(js::Node::Void);
                // A boxed scalar local is declared as a one-slot cell.
                let value = if self.program.boxed_locals.contains(id) {
                    js::Node::Array(vec![value])
                } else {
                    value
                };
                let js_variable = js::Variable {
                    name,
                    value: Box::new(value),
                };
                if variable.mutable {
                    js::Node::LetVariable(js_variable)
                } else {
                    js::Node::ConstVariable(js_variable)
                }
            }
            Expr::Assignment(target_id, value_id) => {
                let value = self.walk_entity(*value_id, block).unwrap_or(js::Node::Void);
                let value = self.maybe_clone(*value_id, value);
                // `*view = wholeValue` through an aggregate view copies the fields
                // in place, so the view's target (and any aliases) update rather
                // than rebinding the local. A primitive view's `*c` is a `[0]`
                // slot write, handled by the normal path below.
                if let Some(Expr::Dereference(operand)) = self.program.entity_map.get(target_id) {
                    if !self.derefs_boxed(*operand) {
                        let base = self.walk_entity(*operand, block).unwrap_or(js::Node::Void);
                        return Some(js::Node::Call(
                            Box::new(js::Node::Local("Object.assign".to_string())),
                            vec![base, value],
                        ));
                    }
                }
                let target = self
                    .walk_entity(*target_id, block)
                    .unwrap_or(js::Node::Void);
                js::Node::Assignment(Box::new(target), Box::new(value))
            }
            Expr::Parameter(_) => {
                return None;
            }
            Expr::Block(body) => {
                for statement in &body.0 {
                    if let Some(node) = self.walk_entity(*statement, block) {
                        // A statement that lowered to nothing (a void tail, a
                        // self-emitting loop/`if`) leaves no stray `undefined`.
                        if !matches!(node, js::Node::Void) {
                            block.push(node);
                        }
                    }
                }
                return self.walk_entity(body.1, block);
            }
            Expr::For(condition, body) => {
                // Every loop compiles to a `while`; an absent condition is an
                // infinite loop, i.e. `while (true)`.
                let t_condition = condition
                    .and_then(|condition| self.walk_entity(condition, block))
                    .unwrap_or(js::Node::Bool(true));
                let mut t_body = self.walk_list(&body.0);
                match self.program.entity_map.get(&body.1) {
                    Some(Expr::Void) | None => {}
                    Some(_) => {
                        if let Some(node) = self.walk_entity(body.1, &mut t_body) {
                            if !matches!(node, js::Node::Void) {
                                t_body.push(node);
                            }
                        }
                    }
                }
                // A loop is a statement with no value: emit it into the block
                // and yield void, so a loop as a block's tail isn't treated as
                // the block's result.
                block.push(js::Node::While(Box::new(t_condition), t_body));
                js::Node::Void
            }
            Expr::ForEach(iterable_id, item_id, body) => {
                let t_iterable = self
                    .walk_entity(*iterable_id, block)
                    .unwrap_or(js::Node::Void);

                if let Some(&next_id) = self.program.for_each_next.get(&id) {
                    // Iterator protocol: evaluate the iterator once, then loop
                    // calling `next()` until it yields `None` (variant 1); the
                    // `Some` payload (slot 1) is the element.
                    self.ensure_function_emitted(next_id);
                    let next_name = self.ng.name_for(next_id);
                    let iterator_name = self.ng.next_name();
                    let next_value_name = self.ng.next_name();
                    block.push(js::Node::ConstVariable(js::Variable {
                        name: iterator_name.clone(),
                        value: Box::new(t_iterable),
                    }));
                    let mut loop_body = vec![
                        js::Node::ConstVariable(js::Variable {
                            name: next_value_name.clone(),
                            value: Box::new(js::Node::Call(
                                Box::new(js::Node::Local(next_name)),
                                vec![js::Node::Local(iterator_name.clone())],
                            )),
                        }),
                        js::Node::If(js::IfBranch::If(
                            Box::new(js::Node::Binary(
                                BinaryOp::NotEq,
                                Box::new(js::Node::PropertyIndex(
                                    Box::new(js::Node::Local(next_value_name.clone())),
                                    Box::new(js::Node::Number("0".to_string(), None)),
                                )),
                                Box::new(js::Node::Number("0".to_string(), None)),
                            )),
                            vec![js::Node::Break],
                            None,
                        )),
                    ];
                    if let Some(item_id) = item_id {
                        loop_body.push(js::Node::ConstVariable(js::Variable {
                            name: self.ng.name_for(*item_id),
                            value: Box::new(js::Node::PropertyIndex(
                                Box::new(js::Node::Local(next_value_name)),
                                Box::new(js::Node::Number("1".to_string(), None)),
                            )),
                        }));
                    }
                    loop_body.extend(self.walk_list(&body.0));
                    if let Some(Expr::Void) | None = self.program.entity_map.get(&body.1) {
                    } else if let Some(node) = self.walk_entity(body.1, &mut loop_body) {
                        if !matches!(node, js::Node::Void) {
                            loop_body.push(node);
                        }
                    }
                    block.push(js::Node::While(Box::new(js::Node::Bool(true)), loop_body));
                    return Some(js::Node::Void);
                }

                // Otherwise a native `for...of` (a `List` is a JS array).
                let binding = item_id
                    .map(|item_id| self.ng.name_for(item_id))
                    .unwrap_or_else(|| "_".to_string());
                let mut t_body = self.walk_list(&body.0);
                if let Some(Expr::Void) | None = self.program.entity_map.get(&body.1) {
                } else if let Some(node) = self.walk_entity(body.1, &mut t_body) {
                    if !matches!(node, js::Node::Void) {
                        t_body.push(node);
                    }
                }
                block.push(js::Node::ForOf(binding, Box::new(t_iterable), t_body));
                js::Node::Void
            }
            Expr::Jump(target) => match *target {
                "break" => js::Node::Break,
                "continue" => js::Node::Continue,
                _ => js::Node::Void,
            },
            Expr::If(branch) => {
                fn walk_branch<'src>(
                    t: &mut Transformer<'src>,
                    branch: &ExprIfBranch,
                    block: &mut Vec<js::Node<'src>>,
                    expr_variable_name: &mut Option<String>,
                ) -> js::IfBranch<'src> {
                    match branch {
                        ExprIfBranch::If(condition, body, else_) => {
                            let t_condition = t
                                .walk_entity(*condition, block)
                                .unwrap_or(js::Node::Bool(false));
                            let mut t_body = t.walk_list(&body.0);
                            let body_expr = t.program.entity_map.get(&body.1);
                            match body_expr {
                                None => {}
                                Some(Expr::Void) => {}
                                Some(_) => {
                                    let t_block_expr = t.walk_entity(body.1, &mut t_body);
                                    let variable_name =
                                        expr_variable_name.get_or_insert_with(|| t.ng.next_name());
                                    t_body.push(js::Node::Assignment(
                                        Box::new(js::Node::Local(variable_name.clone())),
                                        Box::new(t_block_expr.unwrap_or(js::Node::Null)),
                                    ));
                                }
                            }
                            js::IfBranch::If(
                                Box::new(t_condition),
                                t_body,
                                else_.as_ref().map(|x| {
                                    Box::new(walk_branch(t, x, block, expr_variable_name))
                                }),
                            )
                        }
                        ExprIfBranch::Else(body) => {
                            let mut t_body = t.walk_list(&body.0);
                            let body_expr = t.program.entity_map.get(&body.1);
                            match body_expr {
                                None => {}
                                Some(Expr::Void) => {}
                                Some(_) => {
                                    let t_block_expr = t.walk_entity(body.1, &mut t_body);
                                    let variable_name =
                                        expr_variable_name.get_or_insert_with(|| t.ng.next_name());
                                    t_body.push(js::Node::Assignment(
                                        Box::new(js::Node::Local(variable_name.clone())),
                                        Box::new(t_block_expr.unwrap_or(js::Node::Null)),
                                    ));
                                }
                            }
                            js::IfBranch::Else(t_body)
                        }
                    }
                }
                let mut expr_variable_name = None;
                let branch = walk_branch(self, branch, block, &mut expr_variable_name);
                match expr_variable_name {
                    Some(variable_name) => {
                        let expr_variable = js::Node::LetVariable(js::Variable {
                            name: variable_name.clone(),
                            value: Box::new(js::Node::Null),
                        });
                        block.push(expr_variable);
                        block.push(js::Node::If(branch));
                        js::Node::Local(variable_name)
                    }
                    // A value-less `if` (no branch produces a value) is a
                    // statement: emit it into the block and yield void, so a
                    // trailing `if` isn't mistaken for the block's/function's
                    // result (and wrapped in `return`/`process.exit`).
                    None => {
                        block.push(js::Node::If(branch));
                        js::Node::Void
                    }
                }
            }
            Expr::Is(subject_id, pattern) => {
                // Evaluate the subject once into a temp; the test reads from it,
                // and any captures alias its payload slots.
                let t_subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                let subject_name = self.ng.next_name();
                block.push(js::Node::ConstVariable(js::Variable {
                    name: subject_name.clone(),
                    value: Box::new(t_subject),
                }));
                let mut conditions = Vec::new();
                self.compile_is_pattern(pattern, js::Node::Local(subject_name), &mut conditions);
                // An irrefutable pattern (binding/wildcard/tuple) is always true.
                conditions
                    .into_iter()
                    .reduce(|a, b| js::Node::Binary(BinaryOp::And, Box::new(a), Box::new(b)))
                    .unwrap_or(js::Node::Bool(true))
            }
            Expr::Match(subject_id, legs) => {
                let t_subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                // Evaluate the subject once into a temporary; every variant
                // test and capture reads from it.
                let subject_name = self.ng.next_name();
                block.push(js::Node::ConstVariable(js::Variable {
                    name: subject_name.clone(),
                    value: Box::new(t_subject),
                }));
                let result_name = self.ng.next_name();
                block.push(js::Node::LetVariable(js::Variable {
                    name: result_name.clone(),
                    value: Box::new(js::Node::Null),
                }));
                // Each leg becomes an optional variant test plus a body that
                // declares its captures and assigns the leg's value.
                let mut compiled_legs: Vec<(Option<js::Node<'src>>, Vec<js::Node<'src>>)> =
                    Vec::new();
                for leg in legs {
                    let mut leg_body = Vec::new();
                    let subject = js::Node::Local(subject_name.clone());
                    let condition = if leg.guard.is_none() {
                        // No guard: captures are declared as `const`s in the body.
                        let mut conditions = Vec::new();
                        self.compile_pattern(&leg.pattern, subject, &mut conditions, &mut leg_body);
                        conditions.into_iter().reduce(|a, b| {
                            js::Node::Binary(BinaryOp::And, Box::new(a), Box::new(b))
                        })
                    } else {
                        // Guarded: the guard reads the pattern's captures, so they
                        // can't be `const`s declared inside the matched body — alias
                        // them to the subject's slots (like an `is` test) for the
                        // guard and body, then clear the aliases after this leg.
                        let captures = Self::pattern_capture_ids(&leg.pattern);
                        let mut conditions = Vec::new();
                        self.compile_is_pattern(&leg.pattern, subject, &mut conditions);
                        let mut guard_block = Vec::new();
                        if let Some(guard) = self.walk_entity(leg.guard.unwrap(), &mut guard_block)
                        {
                            conditions.push(guard);
                        }
                        let condition = conditions.into_iter().reduce(|a, b| {
                            js::Node::Binary(BinaryOp::And, Box::new(a), Box::new(b))
                        });
                        let value = self.walk_entity(leg.body, &mut leg_body);
                        leg_body.push(js::Node::Assignment(
                            Box::new(js::Node::Local(result_name.clone())),
                            Box::new(value.unwrap_or(js::Node::Null)),
                        ));
                        for capture in captures {
                            self.is_bindings.remove(&capture);
                        }
                        let is_catch_all = condition.is_none();
                        compiled_legs.push((condition, leg_body));
                        if is_catch_all {
                            break;
                        }
                        continue;
                    };
                    let value = self.walk_entity(leg.body, &mut leg_body);
                    leg_body.push(js::Node::Assignment(
                        Box::new(js::Node::Local(result_name.clone())),
                        Box::new(value.unwrap_or(js::Node::Null)),
                    ));
                    let is_catch_all = condition.is_none();
                    compiled_legs.push((condition, leg_body));
                    if is_catch_all {
                        // Later legs are unreachable.
                        break;
                    }
                }
                // The analyzer verified exhaustiveness, so the final leg can
                // always be the `else` branch.
                if let Some(last_leg) = compiled_legs.last_mut() {
                    last_leg.0 = None;
                }
                let mut chain: Option<js::IfBranch<'src>> = None;
                for (condition, leg_body) in compiled_legs.into_iter().rev() {
                    chain = Some(match condition {
                        None => js::IfBranch::Else(leg_body),
                        Some(condition) => {
                            js::IfBranch::If(Box::new(condition), leg_body, chain.map(Box::new))
                        }
                    });
                }
                match chain {
                    // A lone catch-all needs no branching at all.
                    Some(js::IfBranch::Else(leg_body)) => block.extend(leg_body),
                    Some(chain) => block.push(js::Node::If(chain)),
                    None => {}
                }
                js::Node::Local(result_name)
            }
            Expr::List(ids) => {
                let items = ids
                    .iter()
                    .filter_map(|id| self.walk_entity(*id, block))
                    .collect();
                js::Node::Array(items)
            }
            Expr::Tuple(ids) => {
                let items = ids
                    .iter()
                    .filter_map(|id| self.walk_entity(*id, block))
                    .collect();
                js::Node::Array(items)
            }
            Expr::StructInitializer(_struct_id, assignments) => {
                // let struct_ = self.program.structs.get(struct_id).unwrap();
                // let mut properties_ng = NameGenerator::simple(debug_names);
                let mut properties = assignments
                    .iter()
                    .filter_map(|(i, id)| {
                        // let field = struct_.fields.get(*i).unwrap();
                        let value = self.walk_entity(*id, block);
                        value.map(|x| (i, x))
                    })
                    .collect::<Vec<_>>();
                properties.sort_by(|a, b| a.0.cmp(b.0));
                let items = properties.into_iter().map(|x| x.1).collect::<Vec<_>>();
                js::Node::Array(items)
            }
            Expr::Module(_module_id) => {
                // println!("SEEN MODULE");
                // let module = self.program.modules.get(module_id).expect("failed to find module by id");
                // self.walk_entities(&module.body.0, block);
                return None;
            }
        })
    }

    /// The JS value for an enum variant. `bool` lowers to a native boolean
    /// (`false`/`true`), a numeric (C-like) enum to its integer discriminant, and
    /// every other enum to an array `[index, ...data]`.
    fn variant_value(
        &self,
        enum_id: Id,
        variant_index: usize,
        data: Vec<js::Node<'src>>,
    ) -> js::Node<'src> {
        if Some(enum_id) == self.program.bool_enum_id {
            return js::Node::Bool(variant_index == 1);
        }
        if let Some(discriminant) = self.numeric_enum_discriminant(enum_id, variant_index) {
            return js::Node::Number(discriminant.to_string(), None);
        }
        let mut items = vec![js::Node::Number(variant_index.to_string(), None)];
        items.extend(data);
        js::Node::Array(items)
    }

    /// The integer discriminant of a variant if `enum_id` is a numeric (C-like)
    /// enum, else `None` (it uses the array representation).
    fn numeric_enum_discriminant(&self, enum_id: Id, variant_index: usize) -> Option<i64> {
        let enum_ = self.program.enums.get(&enum_id)?;
        if !enum_.is_numeric {
            return None;
        }
        enum_
            .variants
            .get(variant_index)
            .map(|variant| variant.discriminant)
    }

    /// For a variant of an enum that lowers to a native scalar — `bool`
    /// (`subject === true`) or a numeric enum (`subject === discriminant`) — the
    /// equality test. `None` for array-form enums, which test the `[0]` slot.
    fn scalar_variant_test(
        &self,
        enum_id: Id,
        variant_index: usize,
        subject: &js::Node<'src>,
    ) -> Option<js::Node<'src>> {
        let value = if Some(enum_id) == self.program.bool_enum_id {
            js::Node::Bool(variant_index == 1)
        } else {
            js::Node::Number(
                self.numeric_enum_discriminant(enum_id, variant_index)?
                    .to_string(),
                None,
            )
        };
        Some(js::Node::Binary(
            BinaryOp::Eq,
            Box::new(subject.clone()),
            Box::new(value),
        ))
    }

    // Compiles a match pattern against the JS expression holding the value it
    // matches: variant tests are appended to `conditions` and capture
    // declarations to `bindings`.
    /// Compiles a pattern for an `is` test: collects the boolean test conditions
    /// and records each capture as an alias to the subject's payload slot (so
    /// references compile to `t[i]` rather than a binding statement).
    fn compile_is_pattern(
        &mut self,
        pattern: &ExprPattern,
        subject: js::Node<'src>,
        conditions: &mut Vec<js::Node<'src>>,
    ) {
        match pattern {
            ExprPattern::Wildcard => {}
            ExprPattern::Binding(capture_id) => {
                self.is_bindings.insert(*capture_id, subject);
            }
            ExprPattern::Variant(enum_id, variant_index, payload) => {
                // `bool` and numeric enums lower to native values (see
                // `compile_pattern`), so they test by value, not array slot.
                if let Some(test) = self.scalar_variant_test(*enum_id, *variant_index, &subject) {
                    conditions.push(test);
                    return;
                }
                conditions.push(js::Node::Binary(
                    BinaryOp::Eq,
                    Box::new(js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    )),
                    Box::new(js::Node::Number(variant_index.to_string(), None)),
                ));
                for (data_index, sub_pattern) in payload.iter().enumerate() {
                    let element = js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number((data_index + 1).to_string(), None)),
                    );
                    self.compile_is_pattern(sub_pattern, element, conditions);
                }
            }
            ExprPattern::Tuple(elements) => {
                for (index, sub_pattern) in elements.iter().enumerate() {
                    let element = js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number(index.to_string(), None)),
                    );
                    self.compile_is_pattern(sub_pattern, element, conditions);
                }
            }
            ExprPattern::Literal(literal_id) => {
                conditions.push(self.literal_equality(*literal_id, subject));
            }
        }
    }

    /// Lowers an `@extern`-bound call to its host (JS) form. The first argument
    /// is the receiver for method/property bindings; a `Function` binding with a
    /// module records the import to emit.
    fn emit_extern(
        &mut self,
        target_id: Id,
        binding: ExternBinding<'src>,
        args: Vec<js::Node<'src>>,
    ) -> js::Node<'src> {
        match binding {
            ExternBinding::Function { module, symbol } => {
                if let Some(module) = module {
                    self.used_imports
                        .entry(module.to_string())
                        .or_default()
                        .insert(symbol.to_string());
                }
                js::Node::Call(Box::new(js::Node::Local(symbol.to_string())), args)
            }
            ExternBinding::Method { symbol } => {
                // The JS method name defaults to the external's source name.
                let method = symbol
                    .or_else(|| {
                        self.program
                            .external_functions
                            .get(&target_id)
                            .map(|e| e.name)
                    })
                    .unwrap_or("")
                    .to_string();
                let mut args = args.into_iter();
                let receiver = args.next().unwrap_or(js::Node::Void);
                js::Node::Call(
                    Box::new(js::Node::Property(Box::new(receiver), method)),
                    args.collect(),
                )
            }
            ExternBinding::Get { symbol } => {
                let receiver = args.into_iter().next().unwrap_or(js::Node::Void);
                js::Node::Property(Box::new(receiver), symbol.to_string())
            }
            ExternBinding::Set { symbol } => {
                let mut args = args.into_iter();
                let receiver = args.next().unwrap_or(js::Node::Void);
                let value = args.next().unwrap_or(js::Node::Void);
                js::Node::Assignment(
                    Box::new(js::Node::Property(Box::new(receiver), symbol.to_string())),
                    Box::new(value),
                )
            }
        }
    }

    /// Lowers an `external` std intrinsic call. Method intrinsics take the
    /// receiver as the first argument; helper-backed ones record the helper so
    /// it's emitted in the prelude.
    fn emit_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        args: Vec<js::Node<'src>>,
    ) -> js::Node<'src> {
        // A `str` method that maps directly onto a native JS method: the receiver
        // is `self` (the first argument), the rest pass through as call args.
        fn str_method<'a, I: Iterator<Item = js::Node<'a>>>(
            args: &mut I,
            native: &str,
        ) -> js::Node<'a> {
            let receiver = args.next().unwrap_or(js::Node::Void);
            js::Node::Call(
                Box::new(js::Node::Property(Box::new(receiver), native.to_string())),
                args.collect(),
            )
        }
        let mut args = args.into_iter();
        match intrinsic {
            Intrinsic::Scan => {
                self.used_helpers.insert("__scan");
                js::Node::Call(Box::new(js::Node::Local("__scan".to_string())), Vec::new())
            }
            Intrinsic::StrTrim => str_method(&mut args, "trim"),
            Intrinsic::StrToLowercaseAscii => str_method(&mut args, "toLowerCase"),
            Intrinsic::StrToUppercase => str_method(&mut args, "toUpperCase"),
            Intrinsic::StrContains => str_method(&mut args, "includes"),
            Intrinsic::StrStartsWith => str_method(&mut args, "startsWith"),
            Intrinsic::StrEndsWith => str_method(&mut args, "endsWith"),
            Intrinsic::StrReplace => str_method(&mut args, "replaceAll"),
            Intrinsic::StrRepeat => str_method(&mut args, "repeat"),
            Intrinsic::StrSplit => str_method(&mut args, "split"),
            Intrinsic::StrSubstring => str_method(&mut args, "substring"),
            Intrinsic::StrLen => js::Node::Property(
                Box::new(args.next().unwrap_or(js::Node::Void)),
                "length".to_string(),
            ),
            Intrinsic::ParseI32 => {
                self.used_helpers.insert("__parse_i32");
                js::Node::Call(
                    Box::new(js::Node::Local("__parse_i32".to_string())),
                    vec![args.next().unwrap_or(js::Node::Void)],
                )
            }
            Intrinsic::RandomInt => {
                self.used_helpers.insert("__random_int");
                js::Node::Call(
                    Box::new(js::Node::Local("__random_int".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::RandomFloat => {
                self.used_helpers.insert("__random_float");
                js::Node::Call(
                    Box::new(js::Node::Local("__random_float".to_string())),
                    args.collect(),
                )
            }
        }
    }

    /// `subject === <literal>` — the test a literal pattern compiles to.
    fn literal_equality(&mut self, literal_id: Id, subject: js::Node<'src>) -> js::Node<'src> {
        let mut throwaway = Vec::new();
        let literal = self
            .walk_entity(literal_id, &mut throwaway)
            .unwrap_or(js::Node::Void);
        js::Node::Binary(BinaryOp::Eq, Box::new(subject), Box::new(literal))
    }

    /// The capture variable ids a pattern binds, in order — so a guarded leg can
    /// clear their subject-slot aliases after the leg is compiled.
    fn pattern_capture_ids(pattern: &ExprPattern) -> Vec<Id> {
        let mut ids = Vec::new();
        fn collect(pattern: &ExprPattern, ids: &mut Vec<Id>) {
            match pattern {
                ExprPattern::Binding(capture_id) => ids.push(*capture_id),
                ExprPattern::Variant(_, _, payload) => {
                    for sub_pattern in payload {
                        collect(sub_pattern, ids);
                    }
                }
                ExprPattern::Tuple(elements) => {
                    for sub_pattern in elements {
                        collect(sub_pattern, ids);
                    }
                }
                ExprPattern::Wildcard | ExprPattern::Literal(_) => {}
            }
        }
        collect(pattern, &mut ids);
        ids
    }

    fn compile_pattern(
        &mut self,
        pattern: &ExprPattern,
        subject: js::Node<'src>,
        conditions: &mut Vec<js::Node<'src>>,
        bindings: &mut Vec<js::Node<'src>>,
    ) {
        match pattern {
            ExprPattern::Wildcard => {}
            ExprPattern::Binding(capture_id) => {
                let name = self.ng.name_for(*capture_id);
                let mutable = self
                    .program
                    .variables
                    .get(capture_id)
                    .map(|variable| variable.mutable)
                    .unwrap_or(false);
                let variable = js::Variable {
                    name,
                    value: Box::new(subject),
                };
                bindings.push(if mutable {
                    js::Node::LetVariable(variable)
                } else {
                    js::Node::ConstVariable(variable)
                });
            }
            ExprPattern::Variant(enum_id, variant_index, payload) => {
                // `bool` and numeric (C-like) enums lower to native scalars, so
                // their variants test by value (`subject === true` / `=== -1`)
                // rather than by array discriminant slot.
                if let Some(test) = self.scalar_variant_test(*enum_id, *variant_index, &subject) {
                    conditions.push(test);
                    return;
                }
                conditions.push(js::Node::Binary(
                    BinaryOp::Eq,
                    Box::new(js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    )),
                    Box::new(js::Node::Number(variant_index.to_string(), None)),
                ));
                for (data_index, sub_pattern) in payload.iter().enumerate() {
                    // Variant data sits after the variant index.
                    let element = js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number((data_index + 1).to_string(), None)),
                    );
                    self.compile_pattern(sub_pattern, element, conditions, bindings);
                }
            }
            ExprPattern::Tuple(elements) => {
                // Tuples are plain arrays, so each element is matched
                // positionally with no discriminant.
                for (index, sub_pattern) in elements.iter().enumerate() {
                    let element = js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number(index.to_string(), None)),
                    );
                    self.compile_pattern(sub_pattern, element, conditions, bindings);
                }
            }
            ExprPattern::Literal(literal_id) => {
                conditions.push(self.literal_equality(*literal_id, subject));
            }
        }
    }

    fn function(&mut self, function: &Function<'src>) -> js::Node<'src> {
        let name = self.ng.name_for(function.id);
        self.function_with_name(function, name)
    }

    fn function_with_name(&mut self, function: &Function<'src>, name: String) -> js::Node<'src> {
        let parameters = function
            .parameters
            .iter()
            .map(|parameter_id| js::Parameter {
                name: self.ng.name_for(*parameter_id),
            })
            .collect::<Vec<_>>();
        let mut body = self.walk_list(&function.body.0);
        if let Some(return_expr) = self.walk_entity(function.body.1, &mut body) {
            match return_expr {
                js::Node::Void => {}
                _ => {
                    body.push(js::Node::Return(Box::new(return_expr)));
                }
            }
        }
        js::Node::Function(js::Function {
            name,
            parameters,
            body,
            is_async: self.program.async_functions.contains(&function.id),
        })
    }

    /// Emits a concrete (non-generic) function once, keyed by its id. Any active
    /// substitution and self-type are cleared while walking it, since its body
    /// has no generic parameters of its own and is not a default being
    /// specialized.
    fn ensure_function_emitted(&mut self, function_id: Id) {
        if self.required_functions.contains_key(&function_id) {
            return;
        }
        if let Some(function) = self.program.functions.get(&function_id) {
            let saved = std::mem::take(&mut self.current_substitution);
            let saved_self = self.current_self_type.take();
            let js_function = self.function(function);
            self.current_substitution = saved;
            self.current_self_type = saved_self;
            self.required_functions.insert(function_id, js_function);
        }
    }

    /// Re-dispatches a trait method call to the receiver's concrete `type_id`,
    /// returning the JS name to call: the type's own impl member if it declares
    /// one, otherwise an inherited trait default emitted specialized for the type
    /// (so the default's inner `self.method()` calls dispatch to this type too).
    /// Resolves a trait method on a concrete type to its emitted JS name, paired
    /// with whether that method is async (so the caller can implicitly await it).
    fn emit_dispatched_method(&mut self, type_id: TypeId, member: &str) -> Option<(String, bool)> {
        let type_id = self.resolve_type_id(type_id);
        if let Some(member_id) = self.resolve_member_on_type(type_id, member) {
            self.ensure_function_emitted(member_id);
            let is_async = self.program.async_functions.contains(&member_id);
            return Some((self.ng.name_for(member_id), is_async));
        }
        let default_id = self.resolve_inherited_default(type_id, member)?;
        let is_async = self.program.async_functions.contains(&default_id);
        Some((self.emit_default_instance(default_id, type_id), is_async))
    }

    /// Emits a trait default method specialized for a concrete type, keyed by
    /// (default, type) so each pairing is emitted once. While walking the body,
    /// `current_self_type` is the concrete type so its `self.method()` calls
    /// re-dispatch there.
    fn emit_default_instance(&mut self, default_id: Id, type_id: TypeId) -> String {
        let key = (default_id, self.type_key(type_id));
        if let Some(name) = self.default_instances.get(&key) {
            return name.clone();
        }
        let name = self.ng.next_name();
        self.default_instances.insert(key, name.clone());
        if let Some(function) = self.program.functions.get(&default_id) {
            let saved_self = std::mem::replace(&mut self.current_self_type, Some(type_id));
            let saved_substitution = std::mem::take(&mut self.current_substitution);
            let js_function = self.function_with_name(function, name.clone());
            self.current_self_type = saved_self;
            self.current_substitution = saved_substitution;
            self.monomorphized.push(js_function);
        }
        name
    }

    /// Resolves `member` as an inherited trait *default* on a concrete type — a
    /// member none of the type's impls declare, but a (super)trait it implements
    /// provides with a body. Mirrors the analyzer's Gap E resolution.
    fn resolve_inherited_default(&self, type_id: TypeId, member: &str) -> Option<Id> {
        let type_ = self.program.type_id_to_type_map.get(&type_id)?.clone();
        self.program
            .implementations
            .iter()
            .filter(|implementation| {
                self.program
                    .type_id_to_type_map
                    .get(&implementation.subject)
                    == Some(&type_)
            })
            .flat_map(|implementation| implementation.trait_ids.iter().copied())
            .find_map(|trait_id| self.trait_default_member(trait_id, member))
    }

    /// Searches a trait and its supertraits for a default (bodied) member.
    fn trait_default_member(&self, trait_id: Id, member: &str) -> Option<Id> {
        let mut stack = vec![trait_id];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(trait_) = self.program.traits.get(&id) else {
                continue;
            };
            if let Some(&member_id) = trait_.declarations.get(member) {
                if self.function_has_body(member_id) {
                    return Some(member_id);
                }
            }
            for supertrait_type_id in &trait_.supertraits {
                if let Some(Type::Trait(super_id)) =
                    self.program.type_id_to_type_map.get(supertrait_type_id)
                {
                    stack.push(*super_id);
                }
            }
        }
        None
    }

    /// Whether `member_id` is a function with a source-provided body (a trait
    /// default, as opposed to a signature-only requirement).
    fn function_has_body(&self, member_id: Id) -> bool {
        match self.program.entity_map.get(&member_id) {
            Some(Expr::Function(function_id)) => self
                .program
                .functions
                .get(function_id)
                .map(|function| function.has_body)
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Returns the JS name of the monomorphized variant of `function_id` for
    /// the given concrete type arguments, generating it on first use.
    fn get_or_create_instance(
        &mut self,
        function_id: Id,
        generic_argument_ids: &[TypeId],
    ) -> String {
        let concrete_arguments: Vec<TypeId> = generic_argument_ids
            .iter()
            .map(|type_id| self.resolve_type_id(*type_id))
            .collect();
        let key = (
            function_id,
            concrete_arguments
                .iter()
                .map(|type_id| self.type_key(*type_id))
                .collect::<Vec<_>>(),
        );
        if let Some(name) = self.instances.get(&key) {
            return name.clone();
        }

        let constraint_ids = self
            .program
            .functions
            .get(&function_id)
            .map(|function| function.generic_parameter_constraint_ids.clone())
            .unwrap_or_default();
        let mut substitution = HashMap::new();
        for (constraint_id, concrete_argument) in
            constraint_ids.iter().zip(concrete_arguments.iter())
        {
            substitution.insert(*constraint_id, *concrete_argument);
        }

        let name = self.ng.next_name();
        self.instances.insert(key, name.clone());
        if let Some(function) = self.program.functions.get(&function_id) {
            let saved = std::mem::replace(&mut self.current_substitution, substitution);
            let js_function = self.function_with_name(function, name.clone());
            self.current_substitution = saved;
            self.monomorphized.push(js_function);
        }
        name
    }

    /// Emits a monomorphized instance of a method whose impl generics are bound
    /// to concrete types (`xs.sum()` on `List<i32>` -> `sum` specialized with
    /// `T = i32`), keyed by (method, bound types) so each instantiation is
    /// emitted once. While walking the body, `current_substitution` is the
    /// binding, so `T::default()` and `T`-typed values resolve concretely.
    fn emit_method_instance(
        &mut self,
        method_id: Id,
        substitution: &HashMap<TypeId, TypeId>,
    ) -> String {
        // Resolve each bound type under the active substitution (so a nested
        // instantiation composes) and order by constraint id for a stable key.
        let mut entries: Vec<(TypeId, TypeId)> = substitution
            .iter()
            .map(|(constraint_id, type_id)| (*constraint_id, self.resolve_type_id(*type_id)))
            .collect();
        entries.sort_by_key(|(constraint_id, _)| constraint_id.0);
        let key = (
            method_id,
            entries
                .iter()
                .map(|(_, type_id)| self.type_key(*type_id))
                .collect::<Vec<_>>(),
        );
        if let Some(name) = self.instances.get(&key) {
            return name.clone();
        }
        let substitution: HashMap<TypeId, TypeId> = entries.into_iter().collect();
        let name = self.ng.next_name();
        self.instances.insert(key, name.clone());
        if let Some(function) = self.program.functions.get(&method_id) {
            let saved = std::mem::replace(&mut self.current_substitution, substitution);
            let js_function = self.function_with_name(function, name.clone());
            self.current_substitution = saved;
            self.monomorphized.push(js_function);
        }
        name
    }

    /// Resolves a type id to its concrete form under the active substitution,
    /// following generic parameters to the type they're currently bound to.
    fn resolve_type_id(&self, type_id: TypeId) -> TypeId {
        match self.program.type_id_to_type_map.get(&type_id) {
            Some(Type::Generic(constraint_id)) => self
                .current_substitution
                .get(constraint_id)
                .map(|type_id| self.resolve_type_id(*type_id))
                .unwrap_or(type_id),
            _ => type_id,
        }
    }

    /// A stable key identifying a concrete type, used to deduplicate instances.
    fn type_key(&self, type_id: TypeId) -> String {
        match self.program.type_id_to_type_map.get(&type_id) {
            Some(type_) => format!("{:?}", type_),
            None => format!("?{}", type_id.0),
        }
    }

    /// Finds the function implementing `member` for a concrete type, searching
    /// the implementations whose subject matches that type.
    fn resolve_member_on_type(&self, type_id: TypeId, member: &str) -> Option<Id> {
        let type_ = self.program.type_id_to_type_map.get(&type_id)?;
        match type_ {
            Type::Struct(_, _) | Type::Enum(_, _) => self
                .program
                .implementations
                .iter()
                .filter(|implementation| {
                    self.program
                        .type_id_to_type_map
                        .get(&implementation.subject)
                        .is_some_and(|subject| nominal_matches(subject, type_))
                })
                .find_map(|implementation| implementation.declarations.get(member).copied()),
            _ => None,
        }
    }
}

struct Formatter {
    line_break: &'static str,
    indentation: &'static str,
    space: &'static str,
    array_surround: &'static str,
    // object_surround: &'static str,
}

impl Formatter {
    fn new_pretty() -> Self {
        Self {
            line_break: "\n",
            indentation: "\t",
            space: " ",
            array_surround: " ",
            // object_surround: " ",
        }
    }

    fn new_compact() -> Self {
        Self {
            line_break: "",
            indentation: "",
            space: "",
            array_surround: "",
            // object_surround: "",
        }
    }

    fn file(&self, list: &Vec<js::Node>) -> String {
        self.sequence(list, ";", 0)
    }

    fn sequence(
        &self,
        list: &Vec<js::Node>,
        terminator: &'static str,
        indentation: usize,
    ) -> String {
        list.iter()
            .map(|node| self.node(node, terminator, indentation))
            .collect::<Vec<_>>()
            .join(self.line_break)
    }

    fn node(&self, node: &js::Node, terminator: &'static str, indentation: usize) -> String {
        let text = match node {
            js::Node::Void => format!("undefined{}", terminator),
            js::Node::Null => format!("null{}", terminator),
            js::Node::String(x) => format!("\"{}\"{}", x.escape_default(), terminator),
            js::Node::Number(whole, fraction) => format!(
                "{}{}{}",
                whole,
                fraction
                    .clone()
                    .map(|x| format!(".{x}"))
                    .unwrap_or("".to_string()),
                terminator
            ),
            js::Node::Bool(x) => format!("{}{}", x, terminator),
            js::Node::Array(items) => {
                let s_items = items
                    .iter()
                    .map(|x| self.node(x, "", 0))
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                format!(
                    "[{}{}{}]{}",
                    self.array_surround, s_items, self.array_surround, terminator
                )
            }
            // js::Node::Object(members) => {
            //     let s_members = members
            //         .iter()
            //         .map(|(key, value)| {
            //             format!("{}:{}{}", key, self.space, self.node(value, "", 0))
            //         })
            //         .collect::<Vec<_>>()
            //         .join(format!(",{}", self.space).as_str());
            //     format!(
            //         "{{{}{}{}}}{}",
            //         self.object_surround, s_members, self.object_surround, terminator
            //     )
            // }
            js::Node::Function(function) => {
                let name = function.name.as_str();
                let parameters = function
                    .parameters
                    .iter()
                    .map(|x| x.name.as_str())
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                let body = function
                    .body
                    .iter()
                    .map(|x| self.node(x, ";", indentation + 1))
                    .collect::<Vec<_>>()
                    .join(self.line_break);
                format!(
                    "{}function {}({}){}{{{}{}{}{}}}{}",
                    if function.is_async { "async " } else { "" },
                    name,
                    parameters,
                    self.space,
                    self.line_break,
                    body,
                    self.line_break,
                    self.indentation.repeat(indentation),
                    match terminator {
                        ";" => "",
                        x => x,
                    }
                )
            }
            js::Node::Local(name) => format!("{}{}", name, terminator),
            js::Node::Assignment(subject, value) => format!(
                "{}{}={}{}{}",
                self.node(subject, "", 0),
                self.space,
                self.space,
                self.node(value, "", 0),
                terminator
            ),
            js::Node::Return(value) => match &**value {
                js::Node::Void => format!("return{}", terminator),
                x => format!("return {}{}", self.node(x, "", 0), terminator),
            },
            js::Node::Throw(value) => {
                format!("throw {}{}", self.node(value, "", 0), terminator)
            }
            js::Node::Call(subject, args) => {
                let s_subject = self.node(subject, "", 0);
                // A closure called directly must be parenthesised: `(() => …)()`.
                let s_subject = if matches!(&**subject, js::Node::Closure(_)) {
                    format!("({s_subject})")
                } else {
                    s_subject
                };
                let s_args = args
                    .iter()
                    .map(|x| self.node(x, "", 0))
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                format!("{}({}){}", s_subject, s_args, terminator)
            }
            js::Node::Binary(op, lhs, rhs) => {
                let s_op = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Sub => "-",
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    BinaryOp::Eq => "===",
                    BinaryOp::NotEq => "!==",
                    BinaryOp::Lt => "<",
                    BinaryOp::Gt => ">",
                    BinaryOp::LtEq => "<=",
                    BinaryOp::GtEq => ">=",
                    BinaryOp::And => "&&",
                };
                format!(
                    "{}{}{}{}{}{}",
                    self.node(lhs, "", 0),
                    self.space,
                    s_op,
                    self.space,
                    self.node(rhs, "", 0),
                    terminator
                )
            }
            js::Node::Unary(operator, operand) => {
                // Parenthesise the operand so precedence is preserved — e.g.
                // `!(a < b)` must not render as `!a < b`.
                format!("{}({}){}", operator, self.node(operand, "", 0), terminator)
            }
            js::Node::LetVariable(variable) => {
                let value = self.node(&variable.value, "", 0);
                format!(
                    "let {}{}={}{}{}",
                    variable.name, self.space, self.space, value, terminator
                )
            }
            js::Node::ConstVariable(variable) => {
                let value = self.node(&variable.value, "", 0);
                format!(
                    "const {}{}={}{}{}",
                    variable.name, self.space, self.space, value, terminator
                )
            }
            js::Node::Property(subject, member) => {
                let s_subject = self.node(subject, "", 0);
                format!("{}.{}{}", s_subject, member, terminator)
            }
            js::Node::PropertyIndex(subject, member) => {
                let s_subject = self.node(subject, "", 0);
                let s_member = self.node(member, "", 0);
                format!("{}[{}]{}", s_subject, s_member, terminator)
            }
            js::Node::If(branch) => {
                fn walk_branch(
                    f: &Formatter,
                    branch: &js::IfBranch,
                    indentation: usize,
                    level: u32,
                ) -> String {
                    match branch {
                        js::IfBranch::If(condition, body, else_) => {
                            let s_prefix = if level > 0 { "else " } else { "" };
                            let s_condition = f.node(condition, "", 0);
                            let s_body = body
                                .iter()
                                .map(|x| f.node(x, ";", indentation + 1))
                                .collect::<Vec<_>>()
                                .join(f.line_break);
                            let s_else = else_
                                .as_ref()
                                .map(|x| {
                                    format!(
                                        "{}{}",
                                        f.space,
                                        walk_branch(f, x, indentation, level + 1)
                                    )
                                })
                                .unwrap_or("".to_string());
                            format!(
                                "{}if{}({}){}{{{}{}{}{}}}{}",
                                s_prefix,
                                f.space,
                                s_condition,
                                f.space,
                                f.line_break,
                                s_body,
                                f.line_break,
                                f.indentation.repeat(indentation),
                                s_else
                            )
                        }
                        js::IfBranch::Else(body) => {
                            let s_body = body
                                .iter()
                                .map(|x| f.node(x, ";", indentation + 1))
                                .collect::<Vec<_>>()
                                .join(f.line_break);
                            format!(
                                "else{}{{{}{}{}{}}}",
                                f.space,
                                f.line_break,
                                s_body,
                                f.line_break,
                                f.indentation.repeat(indentation)
                            )
                        }
                    }
                }
                walk_branch(self, branch, indentation, 0)
            }
            js::Node::While(condition, body) => {
                let s_condition = self.node(condition, "", 0);
                let s_body = body
                    .iter()
                    .map(|x| self.node(x, ";", indentation + 1))
                    .collect::<Vec<_>>()
                    .join(self.line_break);
                format!(
                    "while{}({}){}{{{}{}{}{}}}",
                    self.space,
                    s_condition,
                    self.space,
                    self.line_break,
                    s_body,
                    self.line_break,
                    self.indentation.repeat(indentation),
                )
            }
            js::Node::ForOf(binding, iterable, body) => {
                let s_iterable = self.node(iterable, "", 0);
                let s_body = body
                    .iter()
                    .map(|x| self.node(x, ";", indentation + 1))
                    .collect::<Vec<_>>()
                    .join(self.line_break);
                format!(
                    "for{}(const {} of {}){}{{{}{}{}{}}}",
                    self.space,
                    binding,
                    s_iterable,
                    self.space,
                    self.line_break,
                    s_body,
                    self.line_break,
                    self.indentation.repeat(indentation),
                )
            }
            js::Node::Break => format!("break{}", terminator),
            js::Node::Continue => format!("continue{}", terminator),
            js::Node::Closure(closure) => {
                let s_parameters = closure
                    .parameters
                    .iter()
                    .map(|x| x.name.as_str())
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                let s_body = closure
                    .body
                    .iter()
                    .map(|x| self.node(x, ";", indentation + 1))
                    .collect::<Vec<_>>()
                    .join(self.line_break);
                format!(
                    "{}({}){}=>{}{{{}{}{}{}}}{}",
                    if closure.is_async { "async " } else { "" },
                    s_parameters,
                    self.space,
                    self.space,
                    self.line_break,
                    s_body,
                    self.line_break,
                    self.indentation.repeat(indentation),
                    terminator
                )
            }
            js::Node::Await(operand) => {
                // Parenthesise so `await` doesn't bind too loosely (e.g.
                // `await (a + b)`), mirroring the unary `!` rendering.
                format!("await ({}){}", self.node(operand, "", 0), terminator)
            }
        };

        format!("{}{}", self.indentation.repeat(indentation), text)
    }
}

pub mod js {
    use crate::node::BinaryOp;
    use std::borrow::Cow;

    #[derive(Clone, Debug)]
    pub enum Node<'src> {
        Array(Vec<Self>),
        Assignment(Box<Self>, Box<Self>),
        // `await <operand>`.
        Await(Box<Self>),
        Binary(BinaryOp, Box<Self>, Box<Self>),
        Unary(char, Box<Self>),
        Bool(bool),
        Break,
        Call(Box<Self>, Vec<Self>),
        Closure(Closure<'src>),
        ConstVariable(Variable<'src>),
        Continue,
        Function(Function<'src>),
        If(IfBranch<'src>),
        While(Box<Self>, Vec<Self>),
        // `for (const <binding> of <iterable>) { <body> }`. The binding name is
        // `_` for a discarded element.
        ForOf(String, Box<Self>, Vec<Self>),
        LetVariable(Variable<'src>),
        Local(String), // TODO: Consider extracting identifiers into a separate lookup table for late identifier substitution.
        Null,
        Number(String, Option<String>),
        // Object(Vec<(&'src str, Self)>),
        Property(Box<Self>, String),
        PropertyIndex(Box<Self>, Box<Self>),
        Return(Box<Self>),
        String(Cow<'src, str>),
        Throw(Box<Self>),
        Void,
    }

    #[derive(Clone, Debug)]
    pub enum IfBranch<'src> {
        If(Box<Node<'src>>, Vec<Node<'src>>, Option<Box<Self>>),
        Else(Vec<Node<'src>>),
    }

    #[derive(Clone, Debug)]
    pub struct Function<'src> {
        pub name: String,
        pub parameters: Vec<Parameter>,
        pub body: Vec<Node<'src>>,
        pub is_async: bool,
    }

    #[derive(Clone, Debug)]
    pub struct Parameter {
        pub name: String,
    }

    #[derive(Clone, Debug)]
    pub struct Variable<'src> {
        pub name: String,
        pub value: Box<Node<'src>>,
    }

    #[derive(Clone, Debug)]
    pub struct Closure<'src> {
        pub parameters: Vec<Parameter>,
        pub body: Vec<Node<'src>>,
        pub is_async: bool,
    }
}

struct NameGenerator {
    chars: Vec<char>,
    counter: u64,
    names: HashMap<Id, String>,
    debug_names: HashMap<Id, String>,
}

impl NameGenerator {
    fn new(chars: &str, debug_names: HashMap<Id, String>) -> Self {
        Self {
            chars: chars.chars().collect(),
            counter: 0,
            names: HashMap::new(),
            debug_names,
        }
    }

    fn new_simple(debug_names: HashMap<Id, String>) -> Self {
        Self::new(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
            debug_names,
        )
    }

    fn name_for(&mut self, id: Id) -> String {
        self.names.get(&id).map(|x| x.clone()).unwrap_or_else(|| {
            let debug_name = self.debug_names.get(&id).map(|x| x.clone());
            let name = debug_name
                .map(|x| format!("{}/*{}*/", self.next_name(), x))
                .unwrap_or_else(|| self.next_name());
            self.names.insert(id, name.clone());
            name
        })
    }

    fn next_idx(&mut self) -> u64 {
        let c = self.counter;
        self.counter += 1;
        c
    }

    fn next_name(&mut self) -> String {
        let c = self.next_idx();
        self.name_from_idx(c)
    }

    fn name_from_idx(&self, n: u64) -> String {
        let mut s = String::new();
        let mut num = n;
        let base = self.chars.len() as u64;

        loop {
            let remainder = (num % base) as usize;
            s.push(self.chars[remainder]);
            num /= base;
            if num < 1 {
                break;
            }
            num -= 1;
        }

        s.chars().rev().collect()
    }
}
