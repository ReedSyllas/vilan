use crate::analyzer::{Expr, ExprIfBranch, ExprPattern, Function, Program};
use crate::error::Error;
use crate::id::Id;
use crate::node::BinaryOp;
use crate::type_::{Type, TypeId};
use chumsky::span::Span;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::collections::HashMap;

pub fn transform<'src>(program: &Program<'src>) -> Result<String, Error> {
    Transformer::new(program, true).transform_entry()
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
    list_new_fn_id: Id,
    list_push_fn_id: Id,
    panic_fn_id: Option<Id>,
    program: &'src Program<'src>,
    required_functions: IndexMap<Id, js::Node<'src>>,
    // The active generic-parameter substitution while emitting a monomorphized
    // function body (constraint id -> concrete type id).
    current_substitution: HashMap<TypeId, TypeId>,
    // Monomorphized function variants, keyed by (generic function, concrete
    // type arguments) so each distinct instantiation is emitted exactly once.
    instances: HashMap<(Id, Vec<String>), String>,
    monomorphized: Vec<js::Node<'src>>,
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
            monomorphized: Vec::new(),
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

        let mut t_functions = self.required_functions.into_iter().collect::<Vec<_>>();
        t_functions.sort_by(|a, b| (a.0.0).cmp(&b.0.0));
        let t_functions = t_functions.into_iter().map(|x| x.1);

        // Monomorphized variants are plain function declarations too; ordering
        // among declarations is irrelevant since JS hoists them.
        let t_instances = self.monomorphized.into_iter();

        Ok(format!(
            "{}{}",
            self.formatter.file(
                &t_functions
                    .chain(t_instances)
                    .chain(t_global_variables.into_iter())
                    .chain(t_main_fn_body.into_iter())
                    .collect::<Vec<_>>()
            ),
            self.formatter.line_break
        ))
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
            Expr::String(x) => js::Node::String(Cow::Borrowed(x)),
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
            // variant; a bare (data-less) variant is just `[index]`.
            Expr::EnumVariant(_, variant_index) => {
                js::Node::Array(vec![js::Node::Number(variant_index.to_string(), None)])
            }
            Expr::Local(id) => {
                // A reference to a data-less variant (e.g. `None`) is the
                // variant value itself, not a named binding.
                if let Some(Expr::EnumVariant(_, variant_index)) = self.program.entity_map.get(id) {
                    return Some(js::Node::Array(vec![js::Node::Number(
                        variant_index.to_string(),
                        None,
                    )]));
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
                    .filter_map(|arg| self.walk_entity(*arg, block))
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
                            return Some(js::Node::Call(Box::new(js::Node::Local(name)), args));
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
                        // A variant constructor call builds the enum value
                        // directly: `[variant_index, ...data]`.
                        if let Some(Expr::EnumVariant(_, variant_index)) =
                            self.program.entity_map.get(&target_id)
                        {
                            let mut items = vec![js::Node::Number(variant_index.to_string(), None)];
                            items.extend(args);
                            return Some(js::Node::Array(items));
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
                        if target_id == self.list_new_fn_id {
                            return Some(js::Node::Array(Vec::new()));
                        }
                        // `list.push(x)` lowers to the native array method; the
                        // receiver is the method call's first (`self`) argument.
                        if target_id == self.list_push_fn_id {
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
                            return Some(js::Node::Call(Box::new(js::Node::Local(name)), args));
                        }
                        self.ensure_function_emitted(target_id);
                        let name = self.ng.name_for(target_id);
                        js::Node::Call(Box::new(js::Node::Local(name)), args)
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
                js::Node::Closure(js::Closure { parameters, body })
            }
            Expr::FunctionReturn(value) => js::Node::Return(Box::new(
                self.walk_entity(*value, block).unwrap_or(js::Node::Void),
            )),
            Expr::Binary(op, lhs, rhs) => {
                let lhs = self.walk_entity(*lhs, block).unwrap_or(js::Node::Void);
                let rhs = self.walk_entity(*rhs, block).unwrap_or(js::Node::Void);
                binary(*op, lhs, rhs)
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
                    .and_then(|id| self.walk_entity(id, block))
                    .unwrap_or(js::Node::Void);
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
                let target = self
                    .walk_entity(*target_id, block)
                    .unwrap_or(js::Node::Void);
                let value = self.walk_entity(*value_id, block).unwrap_or(js::Node::Void);
                js::Node::Assignment(Box::new(target), Box::new(value))
            }
            Expr::Parameter(_) => {
                return None;
            }
            Expr::Block(body) => {
                for statement in &body.0 {
                    if let Some(node) = self.walk_entity(*statement, block) {
                        block.push(node);
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
                            t_body.push(node);
                        }
                    }
                }
                // A loop is a statement with no value: emit it into the block
                // and yield void, so a loop as a block's tail isn't treated as
                // the block's result.
                block.push(js::Node::While(Box::new(t_condition), t_body));
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
                    None => js::Node::If(branch),
                }
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
                    let mut conditions = Vec::new();
                    self.compile_pattern(
                        &leg.pattern,
                        js::Node::Local(subject_name.clone()),
                        &mut conditions,
                        &mut leg_body,
                    );
                    let condition = conditions
                        .into_iter()
                        .reduce(|a, b| js::Node::Binary(BinaryOp::And, Box::new(a), Box::new(b)));
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

    // Compiles a match pattern against the JS expression holding the value it
    // matches: variant tests are appended to `conditions` and capture
    // declarations to `bindings`.
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
            ExprPattern::Variant(variant_index, payload) => {
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
        })
    }

    /// Emits a concrete (non-generic) function once, keyed by its id. Any
    /// active substitution is cleared while walking it, since its body has no
    /// generic parameters of its own.
    fn ensure_function_emitted(&mut self, function_id: Id) {
        if self.required_functions.contains_key(&function_id) {
            return;
        }
        if let Some(function) = self.program.functions.get(&function_id) {
            let saved = std::mem::take(&mut self.current_substitution);
            let js_function = self.function(function);
            self.current_substitution = saved;
            self.required_functions.insert(function_id, js_function);
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
            Type::Struct(_) => self
                .program
                .implementations
                .iter()
                .filter(|implementation| {
                    self.program
                        .type_id_to_type_map
                        .get(&implementation.subject)
                        == Some(type_)
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
                    "function {}({}){}{{{}{}{}{}}}{}",
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
                    "({}){}=>{}{{{}{}{}{}}}{}",
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
        Binary(BinaryOp, Box<Self>, Box<Self>),
        Bool(bool),
        Break,
        Call(Box<Self>, Vec<Self>),
        Closure(Closure<'src>),
        ConstVariable(Variable<'src>),
        Continue,
        Function(Function<'src>),
        If(IfBranch<'src>),
        While(Box<Self>, Vec<Self>),
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
