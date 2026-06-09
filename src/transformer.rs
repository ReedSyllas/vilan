use crate::analyzer::{Expr, ExprIfBranch, Function, Program};
use crate::error::Error;
use crate::id::Id;
use crate::node::BinaryOp;
use crate::type_::{Type, TypeId};
use chumsky::span::Span;
use indexmap::IndexMap;
use std::collections::HashMap;

pub fn transform<'src>(program: &Program<'src>) -> Result<String, Error> {
    Transformer::new(program, true).transform_entry()
}

struct Transformer<'src> {
    formatter: Formatter,
    ng: NameGenerator,
    print_fn_id: Id,
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

        if let Some(x) = self.program.entity_map.get(&main_fn.body.1) {
            match x {
                Expr::Void => {}
                _ => {
                    let t_exit = js::Node::Call(
                        Box::new(js::Node::Property(
                            Box::new(js::Node::Local("process".to_string())),
                            "exit".to_string(),
                        )),
                        self.walk_entity(main_fn.body.1, &mut t_main_fn_body)
                            .map(|x| vec![x])
                            .unwrap_or_else(|| Vec::new()),
                    );
                    t_main_fn_body.push(t_exit)
                }
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
            Expr::Number(whole, fraction) => {
                js::Node::Number(whole.to_string(), fraction.map(|x| x.to_string()))
            }
            Expr::String(x) => js::Node::String(x),
            Expr::Struct(_) => {
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
            Expr::Local(id) => js::Node::Local(self.ng.name_for(*id)),
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
                        if target_id == self.print_fn_id {
                            return Some(js::Node::Call(
                                Box::new(js::Node::Property(
                                    Box::new(js::Node::Local("console".to_string())),
                                    "log".to_string(),
                                )),
                                args,
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
            Expr::Binary(op, lhs, rhs) => js::Node::Binary(
                *op,
                Box::new(self.walk_entity(*lhs, block).unwrap_or(js::Node::Void)),
                Box::new(self.walk_entity(*rhs, block).unwrap_or(js::Node::Void)),
            ),
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
                js::Node::ConstVariable(js::Variable {
                    name,
                    value: Box::new(value),
                })
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
                    "function {}({}){}{{{}{}{}}}{}",
                    name,
                    parameters,
                    self.space,
                    self.line_break,
                    body,
                    self.line_break,
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
            js::Node::Call(subject, args) => {
                let s_subject = self.node(subject, "", 0);
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
                                .join("");
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
                                "{}if{}({}){}{{{}{}{}}}{}",
                                s_prefix,
                                f.space,
                                s_condition,
                                f.space,
                                f.line_break,
                                s_body,
                                f.line_break,
                                s_else
                            )
                        }
                        js::IfBranch::Else(body) => {
                            let s_body = body
                                .iter()
                                .map(|x| f.node(x, ";", indentation + 1))
                                .collect::<Vec<_>>()
                                .join("");
                            format!(
                                "else{}{{{}{}{}}}",
                                f.space, f.line_break, s_body, f.line_break
                            )
                        }
                    }
                }
                walk_branch(self, branch, indentation, 0)
            }
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
                    "({}){}=>{}{{{}{}{}}}{}",
                    s_parameters,
                    self.space,
                    self.space,
                    self.line_break,
                    s_body,
                    self.line_break,
                    terminator
                )
            }
        };

        format!("{}{}", self.indentation.repeat(indentation), text)
    }
}

pub mod js {
    use crate::node::BinaryOp;

    #[derive(Clone, Debug)]
    pub enum Node<'src> {
        Array(Vec<Self>),
        Assignment(Box<Self>, Box<Self>),
        Binary(BinaryOp, Box<Self>, Box<Self>),
        Bool(bool),
        Call(Box<Self>, Vec<Self>),
        Closure(Closure<'src>),
        ConstVariable(Variable<'src>),
        Function(Function<'src>),
        If(IfBranch<'src>),
        LetVariable(Variable<'src>),
        Local(String), // TODO: Consider extracting identifiers into a separate lookup table for late identifier substitution.
        Null,
        Number(String, Option<String>),
        // Object(Vec<(&'src str, Self)>),
        Property(Box<Self>, String),
        PropertyIndex(Box<Self>, Box<Self>),
        Return(Box<Self>),
        String(&'src str),
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
