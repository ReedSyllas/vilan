use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::error::Error;
use crate::id::Id;
use crate::node::{BinaryOp, ImportBranch, Node, NodeIfBranch, NodeList};
use crate::span::{Span, Spanned};
use crate::type_::{PrimitiveType, SubstitutionContext, Type, TypeId};
use crate::util::plural;

#[derive(Clone, Debug)]
pub enum Expr<'src> {
    Binary(BinaryOp, Id, Id),
    Block((Vec<Id>, Id)),
    Bool(bool),
    Call(Id),
    Closure(Id),
    Error,
    ExternalFunction(Id),
    Field(Id, Id, usize),
    Function(Id),
    FunctionReturn(Id),
    Generic(TypeId),
    If(ExprIfBranch),
    Impl(Id),
    List(Vec<Id>),
    Local(Id),
    Module(Id),
    Null,
    Number(&'src str, Option<&'src str>),
    Parameter(Id),
    String(&'src str),
    Struct(Id),
    StructInitializer(Id, IndexMap<usize, Id>),
    Tuple(Vec<Id>),
    Variable(Id),
    Void,
}

#[derive(Clone, Debug)]
pub enum ExprIfBranch {
    If(Id, (Vec<Id>, Id), Option<Box<ExprIfBranch>>),
    Else((Vec<Id>, Id)),
}

#[derive(Debug)]
pub struct Function<'src> {
    pub id: Id,
    pub name: &'src str,
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub parameters: Vec<Id>,
    pub body: (Vec<Id>, Id, Id),
    pub call_count: u32,
}

#[derive(Debug)]
pub struct ExternalFunction<'src> {
    pub id: Id,
    pub name: &'src str,
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub parameters: Vec<Id>,
    pub return_type_id: TypeId,
    pub call_count: u32,
}

#[derive(Debug, Clone)]
pub struct FunctionCall {
    pub id: Id,
    pub subject_id: Id,
    pub generic_argument_ids: Vec<TypeId>,
    pub argument_ids: Vec<Id>,
    pub arguments_span: Span,
}

#[derive(Debug)]
pub struct Parameter<'src> {
    pub id: Id,
    pub function_id: Id,
    pub name: &'src str,
    pub type_id: TypeId,
}

#[derive(Debug)]
pub struct Variable<'src> {
    pub id: Id,
    pub name: &'src str,
    pub initial: Option<Id>,
    pub type_id: TypeId,
    // pub mutable: bool,
}

#[derive(Debug)]
pub struct Struct<'src> {
    pub id: Id,
    pub name: &'src str,
    pub fields: Vec<Field<'src>>,
}

#[derive(Debug)]
pub struct Field<'src> {
    pub name: &'src str,
    pub type_id: TypeId,
}

#[derive(Debug)]
pub struct Implementation<'src> {
    pub subject: TypeId,
    pub declarations: IndexMap<&'src str, Id>,
}

#[derive(Debug)]
pub struct Module<'src> {
    pub id: Id,
    pub name: &'src str,
    pub body: (Vec<Id>, Id),
}

#[derive(Debug)]
pub struct Closure {
    pub id: Id,
    pub parameters: Vec<Id>,
    pub return_: Id,
}

#[derive(Debug)]
pub struct Scope<'src> {
    pub id: Id,
    pub parent_id: Option<Id>,
    pub name_to_id_map: IndexMap<&'src str, Id>,
}

#[derive(Debug)]
pub struct Analyzer<'src> {
    assignment_values: IndexMap<Id, Vec<Id>>,
    closures: IndexMap<Id, Closure>,
    diagnostics: Vec<Error>,
    entity_id: u32,
    expr_id_to_expr_map: HashMap<Id, Expr<'src>>,
    expr_id_to_scope_id_map: HashMap<Id, Id>,
    expr_id_to_type_id_map: HashMap<Id, TypeId>,
    external_functions: IndexMap<Id, ExternalFunction<'src>>,
    function_calls: IndexMap<Id, FunctionCall>,
    functions: IndexMap<Id, Function<'src>>,
    implementations: Vec<Implementation<'src>>,
    module_id_by_name: HashMap<&'src str, Id>,
    modules: IndexMap<Id, Module<'src>>,
    parameters: IndexMap<Id, Parameter<'src>>,
    prepped_field_accessors: Vec<(Id, Id, &'src str)>,
    prepped_imports: Vec<(Vec<&'src str>, &'src str, Id)>,
    prepped_locals: Vec<(Id, &'src str)>,
    prepped_method_calls: Vec<(Id, Id, &'src str, Vec<TypeId>, Vec<Id>, Span)>,
    prepped_static_accessors: Vec<(Id, TypeId, &'src str)>,
    prepped_struct_initializers: Vec<(Id, &'src str, Vec<(&'src str, Id)>)>,
    prepped_type_locals: Vec<(TypeId, &'src str, Id)>,
    prepped_type_static_accessors: Vec<(TypeId, TypeId, &'src str)>,
    reference_count: HashMap<Id, u32>,
    scope_id: u32,
    scopes: IndexMap<Id, Scope<'src>>,
    span_map: HashMap<Id, &'src Span>,
    structs: IndexMap<Id, Struct<'src>>,
    type_id_to_type_map: HashMap<TypeId, Type>,
    type_id: u32,
    variables: IndexMap<Id, Variable<'src>>,
}

impl<'src> Analyzer<'src> {
    fn new() -> Self {
        Self {
            assignment_values: IndexMap::new(),
            closures: IndexMap::new(),
            diagnostics: Vec::new(),
            entity_id: 0,
            expr_id_to_expr_map: HashMap::new(),
            expr_id_to_scope_id_map: HashMap::new(),
            expr_id_to_type_id_map: HashMap::new(),
            external_functions: IndexMap::new(),
            function_calls: IndexMap::new(),
            functions: IndexMap::new(),
            implementations: Vec::new(),
            module_id_by_name: HashMap::new(),
            modules: IndexMap::new(),
            parameters: IndexMap::new(),
            prepped_field_accessors: Vec::new(),
            prepped_imports: Vec::new(),
            prepped_locals: Vec::new(),
            prepped_method_calls: Vec::new(),
            prepped_static_accessors: Vec::new(),
            prepped_struct_initializers: Vec::new(),
            prepped_type_locals: Vec::new(),
            prepped_type_static_accessors: Vec::new(),
            reference_count: HashMap::new(),
            scope_id: 0,
            scopes: IndexMap::new(),
            span_map: HashMap::new(),
            structs: IndexMap::new(),
            type_id_to_type_map: HashMap::new(),
            type_id: 0,
            variables: IndexMap::new(),
        }
    }

    fn new_entity_id(&mut self) -> Id {
        let id = self.entity_id;
        self.entity_id += 1;
        Id(id)
    }

    fn get_entity_by_id(&self, id: Id) -> &Expr<'src> {
        self.expr_id_to_expr_map.get(&id).expect(
            format!(
                "failed to get entity for id: {:?} in {:#?}",
                id, self.expr_id_to_expr_map
            )
            .as_str(),
        )
    }

    fn new_scope_id(&mut self) -> Id {
        let id = self.scope_id;
        self.scope_id += 1;
        Id(id)
    }

    fn mut_scope_for_scope_id(&mut self, scope_id: Id) -> &mut Scope<'src> {
        self.scopes
            .get_mut(&scope_id)
            .expect(format!("failed to get scope for id: {:?}", scope_id.0).as_str())
    }

    fn get_scope_id_for_entity(&mut self, entity_id: Id) -> Id {
        self.expr_id_to_scope_id_map
            .get(&entity_id)
            .map(|scope_id| *scope_id)
            .expect(format!("failed to get scope of entity: {:?}", entity_id.0).as_str())
    }

    fn get_scope_for_entity(&mut self, entity_id: Id) -> &mut Scope<'src> {
        let scope_id = self.get_scope_id_for_entity(entity_id);
        self.mut_scope_for_scope_id(scope_id)
    }

    fn create_scope(&mut self, parent_id: Option<Id>) -> Scope<'src> {
        let id = self.new_scope_id();
        Scope {
            id,
            parent_id,
            name_to_id_map: IndexMap::new(),
        }
    }

    fn push_scope(&mut self, scope: Scope<'src>) -> Id {
        let id = scope.id;
        self.scopes.insert(id, scope);
        id
    }

    fn create_owned_scope(&mut self, parent_id: Option<Id>) -> &mut Scope<'src> {
        let id = self.new_scope_id();
        let scope = Scope {
            id,
            parent_id,
            name_to_id_map: IndexMap::new(),
        };
        self.scopes.insert(id, scope);
        self.scopes.get_mut(&id).unwrap()
    }

    fn new_type_id(&mut self) -> TypeId {
        let id = self.type_id;
        self.type_id += 1;
        TypeId(id)
    }

    fn type_id_for_type(&mut self, type_: Type) -> TypeId {
        // TODO: Implement interning.
        let type_id = self.new_type_id();
        self.type_id_to_type_map.insert(type_id, type_);
        type_id
    }

    fn get_type_by_type_id(&self, type_id: TypeId) -> Type {
        self.type_id_to_type_map.get(&type_id).unwrap().clone()
    }

    // fn resolve_type_by_name(&mut self, name: &'src str, scope_id: Id) -> TypeId {
    //     fn resolve<'src>(
    //         analyzer: &mut Analyzer<'src>,
    //         name: &'src str,
    //         scope_id: Id,
    //     ) -> Option<TypeId> {
    //         let scope = analyzer.mut_scope_for_scope_id(scope_id);
    //         let parent_id = scope.parent_id;
    //         // println!("scanning {} in {:?} {:#?}", name, scope_id, scope.name_id_map);
    //         scope.name_to_type_id_map.get(name).map(|x| *x).or_else(|| {
    //             let subject_id = parent_id
    //                 .map(|parent_scope_id| resolve(analyzer, name, parent_scope_id))
    //                 .flatten()?;
    //             let scope = analyzer.mut_scope_for_scope_id(scope_id);
    //             scope.name_to_type_id_map.insert(name, subject_id);
    //             Some(subject_id)
    //         })
    //     }

    //     resolve(self, name, scope_id).expect(format!("cannot find: {}", name).as_str())
    // }

    fn get_expr_id_by_name(&mut self, name: &'src str, scope_id: Id) -> Id {
        fn resolve<'src>(
            analyzer: &mut Analyzer<'src>,
            name: &'src str,
            scope_id: Id,
        ) -> Option<Id> {
            let scope = analyzer.mut_scope_for_scope_id(scope_id);
            let parent_id = scope.parent_id;
            // println!("scanning {} in {:?} {:#?}", name, scope_id, scope.name_id_map);
            scope.name_to_id_map.get(name).map(|x| *x).or_else(|| {
                let subject_id = parent_id
                    .map(|parent_scope_id| resolve(analyzer, name, parent_scope_id))
                    .flatten()?;
                let scope = analyzer.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, subject_id);
                Some(subject_id)
            })
        }

        resolve(self, name, scope_id).expect(format!("cannot find: {}", name).as_str())
    }

    fn walk_expr_nodes(&mut self, list: &'src NodeList<'src>, scope_id: Id) -> Vec<Id> {
        list.iter()
            .map(|child| self.walk_expr_node(child, scope_id))
            .collect::<Vec<_>>()
    }

    fn walk_expr_node(&mut self, node: &'src Spanned<Node<'src>>, scope_id: Id) -> Id {
        let id = self.new_entity_id();

        let entity = match &node.0 {
            Node::Error => Some(Expr::Error),
            Node::Void => Some(Expr::Void),
            Node::Null => Some(Expr::Null),
            Node::Bool(x) => Some(Expr::Bool(*x)),
            Node::String(x) => Some(Expr::String(x)),
            Node::Number(whole, fraction) => Some(Expr::Number(whole, *fraction)),
            Node::Accessor(name) => {
                self.prepped_locals.push((id, name));
                None
            }
            Node::MemberAccessor(subject, member) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                match &member.0 {
                    Node::Accessor(name) => {
                        self.prepped_field_accessors.push((id, subject_id, *name));
                    }
                    Node::Number(name, _) => {
                        self.prepped_field_accessors.push((id, subject_id, *name));
                    }
                    Node::Call(call_subject, call_generic_arguments, call_arguments) => {
                        match &call_subject.0 {
                            Node::Accessor(name) => {
                                let argument_ids =
                                    self.walk_expr_nodes(&call_arguments.0, scope_id);
                                let generic_argument_ids = call_generic_arguments
                                    .as_ref()
                                    .map(|x| {
                                        x.0.iter()
                                            .map(|y| self.walk_type_node(y, scope_id))
                                            .collect()
                                    })
                                    .unwrap_or_else(Vec::new);
                                self.prepped_method_calls.push((
                                    id,
                                    subject_id,
                                    *name,
                                    generic_argument_ids,
                                    argument_ids,
                                    call_arguments.1,
                                ));
                            }
                            _ => panic!("expected identifier"),
                        }
                    }
                    _ => panic!("expected identifier"),
                }
                None
            }
            Node::StaticAccessor(subject, member_name) => {
                let subject_type_id = self.walk_type_node(subject, scope_id);
                self.prepped_static_accessors
                    .push((id, subject_type_id, member_name));
                None
            }
            Node::Import(root_branch) => {
                fn walk_branch<'src>(
                    s: &mut Analyzer<'src>,
                    branch: &ImportBranch<'src>,
                    mut path: Vec<&'src str>,
                    scope_id: Id,
                ) {
                    match branch {
                        ImportBranch::Path(name, child_branch) => match child_branch {
                            None => {
                                s.prepped_imports.push((path, name, scope_id));
                            }
                            Some(branch) => {
                                path.push(name);
                                walk_branch(s, branch, path, scope_id);
                            }
                        },
                        ImportBranch::Set(branches) => {
                            for branch in branches {
                                walk_branch(s, branch, path.clone(), scope_id);
                            }
                        }
                    }
                }
                walk_branch(self, root_branch, Vec::new(), scope_id);
                None
            }
            Node::List(items) => {
                let ids = self.walk_expr_nodes(items, scope_id);
                Some(Expr::List(ids))
            }
            Node::Tuple(items) => {
                let ids = self.walk_expr_nodes(items, scope_id);
                Some(Expr::Tuple(ids))
            }
            Node::Block(children) => {
                let body_scope_id = self.create_owned_scope(Some(scope_id)).id;
                let ids = self.walk_expr_nodes(&children.0.0, body_scope_id);
                let expr_id = self.walk_expr_node(&children.0.1, body_scope_id);
                Some(Expr::Block((ids, expr_id)))
            }
            Node::If(if_) => {
                fn walk_branch<'src>(
                    s: &mut Analyzer<'src>,
                    branch: &'src NodeIfBranch,
                    scope_id: Id,
                ) -> ExprIfBranch {
                    match branch {
                        NodeIfBranch::If(if_) => {
                            let body_scope_id = s.create_owned_scope(Some(scope_id)).id;
                            let condition_id = s.walk_expr_node(&if_.condition, body_scope_id);
                            let then_ids = s.walk_expr_nodes(&if_.then.0.0, body_scope_id);
                            let then_expr_id = s.walk_expr_node(&if_.then.0.1, body_scope_id);
                            ExprIfBranch::If(
                                condition_id,
                                (then_ids, then_expr_id),
                                if_.else_
                                    .as_ref()
                                    .map(|x| Box::new(walk_branch(s, &x.0, scope_id))),
                            )
                        }
                        NodeIfBranch::Else(body) => {
                            let body_scope_id = s.create_owned_scope(Some(scope_id)).id;
                            let else_ids = s.walk_expr_nodes(&body.0.0, body_scope_id);
                            let else_expr_id = s.walk_expr_node(&body.0.1, body_scope_id);
                            ExprIfBranch::Else((else_ids, else_expr_id))
                        }
                    }
                }
                Some(Expr::If(walk_branch(self, if_, scope_id)))
            }
            Node::Func(function) => {
                let name = function.name.0;
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let mut body_scope = self.create_scope(Some(scope_id));
                let parameters = function
                    .parameters
                    .0
                    .iter()
                    .map(|x| {
                        let parameter_id = self.new_entity_id();
                        let parameter = Parameter {
                            id: parameter_id,
                            function_id: id,
                            name: x.0,
                            type_id: x
                                .1
                                .as_ref()
                                .map(|x| self.walk_type_node(x, body_scope.id))
                                .unwrap_or(Type::Unknown.get_type_id(self)),
                        };
                        body_scope
                            .name_to_id_map
                            .insert(parameter.name, parameter_id);
                        self.parameters.insert(parameter_id, parameter);
                        self.expr_id_to_expr_map
                            .insert(parameter_id, Expr::Parameter(parameter_id));
                        parameter_id
                    })
                    .collect::<Vec<_>>();
                let body_scope_id = self.push_scope(body_scope);
                let mut generic_parameter_constraint_ids = Vec::new();
                if let Some(generic_parameters) = &function.generic_parameters {
                    for (name, type_) in &generic_parameters.0 {
                        let constraint_type_id = type_
                            .as_ref()
                            .map(|x| self.walk_type_node(x, body_scope_id))
                            .unwrap_or_else(|| Type::Any.get_type_id(self));
                        generic_parameter_constraint_ids.push(constraint_type_id);
                        let type_id = Type::Generic(constraint_type_id).get_type_id(self);
                        let expr_id = self.new_entity_id();
                        self.expr_id_to_expr_map
                            .insert(expr_id, Expr::Generic(constraint_type_id));
                        self.expr_id_to_scope_id_map.insert(expr_id, body_scope_id);
                        self.expr_id_to_type_id_map.insert(expr_id, type_id);
                        let body_scope = self.mut_scope_for_scope_id(body_scope_id);
                        body_scope.name_to_id_map.insert(name, expr_id);
                    }
                }
                let ids = self.walk_expr_nodes(&function.body.0.0, body_scope_id);
                let expr_id = self.walk_expr_node(&function.body.0.1, body_scope_id);
                self.functions.insert(
                    id,
                    Function {
                        id,
                        name,
                        generic_parameter_constraint_ids,
                        parameters,
                        body: (ids, expr_id, body_scope_id),
                        call_count: 0,
                    },
                );
                Some(Expr::Function(id))
            }
            Node::Call(subject, generic_arguments, arguments) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                let argument_ids = self.walk_expr_nodes(&arguments.0, scope_id);
                let generic_argument_ids = generic_arguments
                    .as_ref()
                    .map(|x| {
                        x.0.iter()
                            .map(|y| self.walk_type_node(y, scope_id))
                            .collect()
                    })
                    .unwrap_or_else(Vec::new);
                self.function_calls.insert(
                    id,
                    FunctionCall {
                        id,
                        subject_id,
                        generic_argument_ids,
                        argument_ids,
                        arguments_span: arguments.1,
                    },
                );
                Some(Expr::Call(id))
            }
            Node::FuncReturn(value) => {
                let id = self.walk_expr_node(value, scope_id);
                Some(Expr::FunctionReturn(id))
            }
            Node::Binary(op, lhs, rhs) => {
                let lhs_id = self.walk_expr_node(lhs, scope_id);
                let rhs_id = self.walk_expr_node(rhs, scope_id);
                Some(Expr::Binary(*op, lhs_id, rhs_id))
            }
            Node::Let(name, type_, value) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let initial = value.as_ref().map(|value| {
                    let value_id = self.walk_expr_node(value, scope_id);
                    let assignments = self
                        .assignment_values
                        .entry(id)
                        .or_insert_with(|| Vec::new());
                    assignments.push(value_id);
                    value_id
                });
                let type_id = type_
                    .as_ref()
                    .map(|x| self.walk_type_node(x, scope_id))
                    .unwrap_or(Type::Unknown.get_type_id(self));
                self.variables.insert(
                    id,
                    Variable {
                        id,
                        name,
                        initial,
                        type_id,
                    },
                );
                Some(Expr::Variable(id))
            }
            Node::Struct(name, generic_parameters, body) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                if let Some(generic_parameters) = generic_parameters {
                    for (name, type_) in &generic_parameters.0 {
                        let constraint_type_id = type_
                            .as_ref()
                            .map(|x| self.walk_type_node(x, body_scope_id))
                            .unwrap_or_else(|| Type::Any.get_type_id(self));
                        let type_id = Type::Generic(constraint_type_id).get_type_id(self);
                        let expr_id = self.new_entity_id();
                        self.expr_id_to_expr_map
                            .insert(expr_id, Expr::Generic(constraint_type_id));
                        self.expr_id_to_scope_id_map.insert(expr_id, body_scope_id);
                        self.expr_id_to_type_id_map.insert(expr_id, type_id);
                        let body_scope = self.mut_scope_for_scope_id(body_scope_id);
                        body_scope.name_to_id_map.insert(name, expr_id);
                    }
                }
                let mut fields = Vec::new();
                for child in &body.0 {
                    let name = child.0.0;
                    let type_id = child
                        .0
                        .1
                        .as_ref()
                        .map(|x| self.walk_type_node(x, body_scope_id))
                        .unwrap_or(Type::Unknown.get_type_id(self));
                    fields.push(Field { name, type_id });
                }
                self.structs.insert(id, Struct { id, name, fields });
                Some(Expr::Struct(id))
            }
            Node::StructInitializer(name, generic_arguments, fields) => {
                let e_fields = fields
                    .0
                    .iter()
                    .map(|x| {
                        (
                            x.0.0,
                            x.0.1
                                .as_ref()
                                .map(|x| self.walk_expr_node(x, scope_id))
                                .unwrap_or_else(|| {
                                    let local_id = self.new_entity_id();
                                    self.prepped_locals.push((local_id, name));
                                    local_id
                                }),
                        )
                    })
                    .collect::<Vec<_>>();
                self.prepped_struct_initializers.push((id, name, e_fields));
                None
            }
            Node::Impl(subject, generic_parameters, body) => {
                let subject = self.walk_type_node(subject, scope_id);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                if let Some(generic_parameters) = generic_parameters {
                    for (name, type_) in &generic_parameters.0 {
                        let constraint_type_id = type_
                            .as_ref()
                            .map(|x| self.walk_type_node(x, body_scope_id))
                            .unwrap_or_else(|| Type::Any.get_type_id(self));
                        let type_id = Type::Generic(constraint_type_id).get_type_id(self);
                        let expr_id = self.new_entity_id();
                        self.expr_id_to_expr_map
                            .insert(expr_id, Expr::Generic(constraint_type_id));
                        self.expr_id_to_scope_id_map.insert(expr_id, body_scope_id);
                        self.expr_id_to_type_id_map.insert(expr_id, type_id);
                        let body_scope = self.mut_scope_for_scope_id(body_scope_id);
                        body_scope.name_to_id_map.insert(name, expr_id);
                    }
                }
                self.walk_expr_nodes(&body.0, body_scope_id);
                let body_scope = self.scopes.get(&body_scope_id).unwrap();
                self.implementations.push(Implementation {
                    subject,
                    declarations: body_scope.name_to_id_map.clone(),
                });

                Some(Expr::Impl(id))
            }
            Node::Closure(closure) => {
                let mut body_scope = self.create_scope(Some(scope_id));
                let parameters = closure
                    .parameters
                    .0
                    .iter()
                    .map(|x| {
                        let parameter_id = self.new_entity_id();
                        let parameter = Parameter {
                            id: parameter_id,
                            function_id: id,
                            name: x.0,
                            type_id: x
                                .1
                                .as_ref()
                                .map(|x| self.walk_type_node(x, scope_id))
                                .unwrap_or(Type::Unknown.get_type_id(self)),
                        };
                        body_scope
                            .name_to_id_map
                            .insert(parameter.name, parameter_id);
                        self.parameters.insert(parameter_id, parameter);
                        self.expr_id_to_expr_map
                            .insert(parameter_id, Expr::Parameter(parameter_id));
                        parameter_id
                    })
                    .collect::<Vec<_>>();
                let body_scope_id = self.push_scope(body_scope);
                let expr_id = self.walk_expr_node(&closure.return_value, body_scope_id);
                self.closures.insert(
                    id,
                    Closure {
                        id,
                        parameters,
                        return_: expr_id,
                    },
                );
                Some(Expr::Closure(id))
            }
            Node::ClosureType(_, _) => panic!("found a type in the expression context"),
            Node::Module(name, body) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let body = self.walk_expr_nodes(&body.0, body_scope_id);
                self.modules.insert(
                    id,
                    Module {
                        id,
                        name,
                        body: (body, body_scope_id),
                    },
                );
                Some(Expr::Module(id))
            }
        };

        if let Some(entity) = entity {
            self.expr_id_to_expr_map.insert(id, entity);
        }

        self.span_map.insert(id, &node.1);
        self.expr_id_to_scope_id_map.insert(id, scope_id);

        id
    }

    fn walk_type_node(&mut self, node: &Spanned<Node<'src>>, scope_id: Id) -> TypeId {
        let type_id = self.new_type_id();

        let type_: Option<Type> = match &node.0 {
            Node::Accessor(name) => match *name {
                "any" => Some(Type::Any),
                "f64" => Some(Type::Primitive(PrimitiveType::F64)),
                "i32" => Some(Type::Primitive(PrimitiveType::I32)),
                "u32" => Some(Type::Primitive(PrimitiveType::U32)),
                "str" => Some(Type::Primitive(PrimitiveType::String)),
                "bool" => Some(Type::Primitive(PrimitiveType::Bool)),
                "null" => Some(Type::Primitive(PrimitiveType::Null)),
                _ => {
                    self.prepped_type_locals.push((type_id, name, scope_id));
                    None
                }
            },
            Node::StaticAccessor(subject, member_name) => {
                let subject_type_id = self.walk_type_node(subject, scope_id);
                self.prepped_type_static_accessors
                    .push((type_id, subject_type_id, member_name));
                None
            }
            Node::Tuple(types) => Some(Type::Tuple(
                types
                    .iter()
                    .map(|type_| self.walk_type_node(type_, scope_id))
                    .collect(),
            )),
            Node::ClosureType(parameters, return_type) => {
                let t_parameter_type_ids = parameters
                    .0
                    .iter()
                    .map(|x| self.walk_type_node(&*x.1, scope_id))
                    .collect::<Vec<_>>();
                let t_return_type_id = return_type
                    .as_ref()
                    .map(|return_type| self.walk_type_node(&*return_type, scope_id))
                    .unwrap_or_else(|| Type::Unknown.get_type_id(self));
                Some(Type::Closure(t_parameter_type_ids, t_return_type_id))
            }
            x => unimplemented!("unhandled type node: {:?}", x),
        };

        if let Some(type_) = type_ {
            self.type_id_to_type_map.insert(type_id, type_);
        }

        type_id
    }

    fn infer_type_start(
        &mut self,
        expr_id: Id,
        constraint: Type,
        substitution_context: &SubstitutionContext,
    ) -> Type {
        self.infer_type(
            expr_id,
            constraint,
            substitution_context,
            &mut HashSet::new(),
        )
    }

    fn infer_type(
        &mut self,
        expr_id: Id,
        constraint: Type,
        substitution_context: &SubstitutionContext,
        exprs_seen: &mut HashSet<Id>,
    ) -> Type {
        if exprs_seen.contains(&expr_id) {
            panic!(
                "recursive type found for {:?} in {:#?}",
                expr_id, exprs_seen
            );
        }
        exprs_seen.insert(expr_id);

        if let Some(type_id) = self.expr_id_to_type_id_map.get(&expr_id) {
            return type_id.get_type(self);
        }

        let expr = self.get_entity_by_id(expr_id);

        println!("resolve_type/input {expr:?} {constraint:?}");

        let inferred_type: Type = match expr {
            Expr::Null => Type::Primitive(PrimitiveType::Null),
            Expr::Bool(_) => Type::Primitive(PrimitiveType::Bool),
            Expr::String(_) => Type::Primitive(PrimitiveType::String),
            Expr::Number(_, _) => Type::Primitive(match constraint {
                Type::Primitive(PrimitiveType::F64) => PrimitiveType::F64,
                Type::Primitive(PrimitiveType::U32) => PrimitiveType::U32,
                _ => PrimitiveType::I32,
            }),
            Expr::List(_) => Type::Primitive(PrimitiveType::List(Type::Void.get_type_id(self))),
            Expr::Tuple(item_ids) => {
                let constraint_items = match constraint.clone() {
                    Type::Tuple(items) => items,
                    _ => Vec::new(),
                };
                Type::Tuple(
                    item_ids
                        .clone()
                        .iter()
                        .enumerate()
                        .map(|(i, id)| {
                            let constraint_item = constraint_items
                                .get(i)
                                .map(|x| x.get_type(self))
                                .unwrap_or(Type::Unknown);
                            self.infer_type(*id, constraint_item, substitution_context, exprs_seen)
                                .get_type_id(self)
                        })
                        .collect(),
                )
            }
            Expr::Local(subject_id) => self.infer_type(
                *subject_id,
                constraint.clone(),
                substitution_context,
                exprs_seen,
            ),
            Expr::Function(function_id) => Type::Function(*function_id),
            Expr::Struct(struct_id) => Type::Struct(*struct_id),
            Expr::Module(module_id) => Type::Module(*module_id),
            Expr::Call(id) => {
                let id = *id;
                let function_call = self.function_calls.get(&id).unwrap();
                let subject_type = self.infer_type(
                    function_call.subject_id,
                    Type::Unknown,
                    substitution_context,
                    exprs_seen,
                );
                match subject_type {
                    Type::Function(_) => Type::Void,
                    x => panic!("type is not callable: {:?}", x),
                }
            }
            Expr::Variable(variable_id) => {
                let variable = self.variables.get(variable_id).unwrap();
                variable.type_id.get_type(self)
            }
            Expr::Parameter(parameter_id) => {
                let parameter = self.parameters.get(parameter_id).unwrap();
                parameter.type_id.get_type(self)
            }
            Expr::Generic(type_id) => type_id.get_type(self),
            _ => Type::Void,
        };

        println!("resolve_type/inference {:?}", inferred_type);

        self.reconcile_type(constraint.clone(), inferred_type, substitution_context)
            .unwrap_or(constraint)
    }

    fn reconcile_type(
        &mut self,
        a: Type,
        b: Type,
        substitution_context: &SubstitutionContext,
    ) -> Option<Type> {
        println!(
            "reconcile_type a={:?} b={:?} substitution_context={:?}",
            a, b, substitution_context
        );

        Some(match (&a, &b) {
            (Type::Any, _) | (_, Type::Unknown) => a,
            (_, Type::Any) | (Type::Unknown, _) => b,
            (Type::Generic(constraint_id), _) => {
                let l = substitution_context
                    .get(&constraint_id)
                    .map(|x| x.get_type(self))
                    .unwrap_or_else(|| constraint_id.get_type(self));
                return self.reconcile_type(l, b, substitution_context);
            }
            (_, Type::Generic(constraint_id)) => {
                let r = substitution_context
                    .get(&constraint_id)
                    .map(|x| x.get_type(self))
                    .unwrap_or_else(|| constraint_id.get_type(self));
                return self.reconcile_type(a, r, substitution_context);
            }
            (Type::Primitive(l), Type::Primitive(r)) => match (l, r) {
                (PrimitiveType::List(l_id), PrimitiveType::List(r_id)) => {
                    let l = l_id.get_type(self);
                    let r = r_id.get_type(self);
                    let item_type_id = self
                        .reconcile_type(l.clone(), r, substitution_context)
                        .unwrap_or(l)
                        .get_type_id(self);
                    Type::Primitive(PrimitiveType::List(item_type_id))
                }
                (l, r) if l == r => a,
                _ => {
                    return None;
                }
            },
            (Type::Tuple(l_items), Type::Tuple(r_items)) => Type::Tuple(
                l_items
                    .iter()
                    .zip(r_items.iter())
                    .map(|(l_item_id, r_item_id)| {
                        let l = l_item_id.get_type(self);
                        let r = r_item_id.get_type(self);
                        self.reconcile_type(l.clone(), r, substitution_context)
                            .unwrap_or(l)
                            .get_type_id(self)
                    })
                    .collect(),
            ),
            (l, r) if l == r => a,
            _ => {
                return None;
            }
        })
    }

    fn compare_type(&self, a: Type, b: Type, substitution_context: &SubstitutionContext) -> bool {
        println!(
            "compare_type a={:?} b={:?} substitution_context={:?}",
            a, b, substitution_context
        );

        match (&a, &b) {
            (Type::Unknown, _) | (_, Type::Unknown) | (Type::Any, _) | (_, Type::Any) => true,
            (Type::Generic(constraint_id), _) => {
                let l = substitution_context
                    .get(&constraint_id)
                    .map(|x| x.get_type(self))
                    .unwrap_or_else(|| constraint_id.get_type(self));
                return self.compare_type(l, b, substitution_context);
            }
            (_, Type::Generic(constraint_id)) => {
                let r = substitution_context
                    .get(&constraint_id)
                    .map(|x| x.get_type(self))
                    .unwrap_or_else(|| constraint_id.get_type(self));
                return self.compare_type(a, r, substitution_context);
            }
            (Type::Primitive(l), Type::Primitive(r)) => match (l, r) {
                (PrimitiveType::List(l_id), PrimitiveType::List(r_id)) => {
                    let l = l_id.get_type(self);
                    let r = r_id.get_type(self);
                    self.compare_type(l, r, substitution_context)
                }
                (a, b) if a == b => true,
                _ => false,
            },
            (Type::Tuple(l_items), Type::Tuple(r_items)) => {
                l_items
                    .iter()
                    .zip(r_items.iter())
                    .all(|(l_item_id, r_item_id)| {
                        let l = l_item_id.get_type(self);
                        let r = r_item_id.get_type(self);
                        self.compare_type(l, r, substitution_context)
                    })
            }
            (a, b) if a == b => true,
            _ => false,
        }
    }

    /// Walk both type trees in parallel, collecting (constraint_id, concrete_type_id) pairs
    /// for any unbound generics found at corresponding positions.
    fn collect_type_bindings(
        &mut self,
        param_type_id: TypeId,
        argument_type: &Type,
        substitution: &SubstitutionContext,
    ) -> Vec<(TypeId, TypeId)> {
        let mut bindings = Vec::new();
        self.bind_collect(param_type_id, argument_type, substitution, &mut bindings);
        bindings
    }

    fn bind_collect(
        &mut self,
        param_type_id: TypeId,
        argument_type: &Type,
        substitution: &SubstitutionContext,
        bindings: &mut Vec<(TypeId, TypeId)>,
    ) {
        let param_type = param_type_id.get_type(self);
        match (&param_type, argument_type) {
            (Type::Generic(constraint_id), _) => {
                if !substitution.contains_key(constraint_id) {
                    bindings.push((*constraint_id, argument_type.clone().get_type_id(self)));
                }
            }
            (Type::Closure(param_params, param_ret), Type::Closure(arg_params, arg_ret)) => {
                for (pp_id, ap_id) in param_params.iter().zip(arg_params.iter()) {
                    let ap_type = ap_id.get_type(self);
                    self.bind_collect(*pp_id, &ap_type, substitution, bindings);
                }
                let arg_ret_type = arg_ret.get_type(self);
                self.bind_collect(*param_ret, &arg_ret_type, substitution, bindings);
            }
            (Type::Primitive(PrimitiveType::List(item_id)), Type::Primitive(PrimitiveType::List(arg_item_id))) => {
                let arg_item_type = arg_item_id.get_type(self);
                self.bind_collect(*item_id, &arg_item_type, substitution, bindings);
            }
            (Type::Tuple(param_items), Type::Tuple(arg_items)) => {
                for (pp_id, ap) in param_items.iter().zip(arg_items.iter()) {
                    let ap_type = ap.get_type(self);
                    self.bind_collect(*pp_id, &ap_type, substitution, bindings);
                }
            }
            _ => {}
        }
    }

    fn build(&mut self) {
        for (path, name, scope_id) in self.prepped_imports.clone() {
            let mut path = path.iter().map(|x| *x).chain(std::iter::once(name));
            let root = path.next().unwrap();
            let module_id = self
                .module_id_by_name
                .get(root)
                .expect(format!("failed to import module {root}").as_str());
            let mut target_id = *module_id;
            let module_scope_id = self.modules.get(module_id).unwrap().body.1;
            for part in path {
                target_id = self.get_expr_id_by_name(part, module_scope_id);
            }
            let scope = self.mut_scope_for_scope_id(scope_id);
            scope.name_to_id_map.insert(name, target_id);
        }

        for (id, name) in self.prepped_locals.clone() {
            let scope_id = self.get_scope_id_for_entity(id);
            let subject_id = self.get_expr_id_by_name(name, scope_id);
            let rc = self.reference_count.entry(subject_id).or_insert(0);
            *rc += 1;
            self.expr_id_to_expr_map.insert(id, Expr::Local(subject_id));
        }

        for (type_id, name, scope_id) in self.prepped_type_locals.clone() {
            let subject_id = self.get_expr_id_by_name(name, scope_id);
            let subject_type = self.infer_type_start(subject_id, Type::Unknown, &HashMap::new());
            self.type_id_to_type_map.insert(type_id, subject_type);
        }

        for (type_id, subject_type_id, member_name) in self.prepped_type_static_accessors.clone() {
            match subject_type_id.get_type(self) {
                Type::Module(module_id) => {
                    let module = self.modules.get(&module_id).unwrap();
                    let member_id = self.get_expr_id_by_name(member_name, module.body.1);
                    let member_type =
                        self.infer_type_start(member_id, Type::Unknown, &HashMap::new());
                    self.type_id_to_type_map.insert(type_id, member_type);
                }
                _ => {}
            }
        }

        for (id, subject_id, member_name) in self.prepped_field_accessors.clone() {
            let subject_type = self.infer_type_start(subject_id, Type::Unknown, &HashMap::new());
            match subject_type {
                Type::Struct(struct_id) => {
                    let struct_ = self.structs.get(&struct_id).unwrap();
                    let field_index = struct_
                        .fields
                        .iter()
                        .enumerate()
                        .find_map(|(i, x)| (x.name == member_name).then_some(i))
                        .unwrap();
                    self.expr_id_to_expr_map
                        .insert(id, Expr::Field(subject_id, struct_id, field_index));
                }
                _ => unimplemented!(
                    "Unhandled subject type for member accessor. In other words, the type of `subject` in `subject.member` was not handled. This is a compiler bug, not one with your code."
                ),
            }
        }

        for (id, subject_id, member_name, generic_argument_ids, mut argument_ids, arguments_span) in
            self.prepped_method_calls.clone()
        {
            let subject_type = self.infer_type_start(subject_id, Type::Unknown, &HashMap::new());
            match subject_type {
                Type::Struct(struct_id) => {
                    let struct_name = self.structs.get(&struct_id).unwrap().name;
                    let member_id = self
                        .implementations
                        .iter()
                        .filter(|x| {
                            self.compare_type(
                                subject_type.clone(),
                                x.subject.get_type(self),
                                &HashMap::new(),
                            )
                        })
                        .find_map(|x| {
                            x.declarations.get(member_name).and_then(|member_id| {
                                match self.get_entity_by_id(*member_id) {
                                    Expr::Function(function_id) => {
                                        let function = self.functions.get(function_id).unwrap();
                                        function.parameters.get(0).and_then(|parameter_id| {
                                            let parameter =
                                                self.parameters.get(parameter_id).unwrap();
                                            (parameter.name == "self").then_some(*member_id)
                                        })
                                    }
                                    _ => panic!("method subject is not a function"),
                                }
                            })
                        })
                        .expect(format!("cannot find {} in {}", member_name, struct_name).as_str());
                    let member_local_id = self.new_entity_id();
                    self.expr_id_to_expr_map
                        .insert(member_local_id, Expr::Local(member_id));
                    argument_ids.insert(0, subject_id);
                    self.function_calls.insert(
                        id,
                        FunctionCall {
                            id,
                            subject_id: member_local_id,
                            generic_argument_ids,
                            argument_ids,
                            arguments_span,
                        },
                    );
                    self.expr_id_to_expr_map.insert(id, Expr::Call(id));
                }
                _ => unimplemented!(
                    "Unhandled subject type for method call. In other words, the type of `subject` in `subject.method()` was not handled. This is a compiler bug, not one with your code."
                ),
            }
        }

        for (id, subject_type_id, member_name) in self.prepped_static_accessors.clone() {
            let subject_type = subject_type_id.get_type(self);
            println!("prepped_static_accessors {member_name} {subject_type:#?}");
            match subject_type {
                Type::Struct(struct_id) => {
                    let struct_name = self.structs.get(&struct_id).unwrap().name;
                    let member_id = *self
                        .implementations
                        .iter()
                        .filter(|x| {
                            self.compare_type(
                                subject_type.clone(),
                                x.subject.get_type(self),
                                &HashMap::new(),
                            )
                        })
                        .find_map(|x| x.declarations.get(member_name))
                        .expect(format!("cannot find {} in {}", member_name, struct_name).as_str());
                    self.expr_id_to_expr_map.insert(id, Expr::Local(member_id));
                }
                Type::Module(module_id) => {
                    let module = self.modules.get(&module_id).unwrap();

                    let member_id = self.get_expr_id_by_name(member_name, module.body.1);
                    let rc = self.reference_count.entry(member_id).or_insert(0);
                    *rc += 1;

                    self.expr_id_to_expr_map.insert(id, Expr::Local(member_id));
                }
                _ => {}
            }
        }

        for (id, name, fields) in self.prepped_struct_initializers.clone() {
            let scope_id = self.get_scope_id_for_entity(id);
            let struct_id = self.get_expr_id_by_name(name, scope_id);
            let struct_ = self
                .structs
                .get(&struct_id)
                .expect("cannot initialize a non-struct");
            let mut initializer_fields = IndexMap::new();
            for field in fields {
                let index = struct_
                    .fields
                    .iter()
                    .position(|x| x.name == field.0)
                    .expect(format!("unknown field: {}", field.0).as_str());
                initializer_fields.insert(index, field.1);
            }
            self.expr_id_to_expr_map
                .insert(id, Expr::StructInitializer(struct_id, initializer_fields));
        }

        let function_call_ids = self.function_calls.keys().copied().collect::<Vec<_>>();
        for function_call_id in function_call_ids {
            let function_call = self.function_calls.get(&function_call_id).unwrap().clone();
            let subject = self.get_entity_by_id(function_call.subject_id);
            match subject {
                Expr::Local(target_id) => {
                    let target = self.get_entity_by_id(*target_id);
                    let function_data = match target {
                        Expr::Function(function_id) => {
                            let function = self.functions.get(function_id).unwrap();
                            println!("NORMAL FUNCTION CALLED {}", function.name);
                            Some((
                                function.parameters.clone(),
                                function.generic_parameter_constraint_ids.clone(),
                            ))
                        }
                        Expr::ExternalFunction(external_function_id) => {
                            let function =
                                self.external_functions.get(external_function_id).unwrap();
                            println!("EXTERNAL FUNCTION CALLED {}", function.name);
                            Some((
                                function.parameters.clone(),
                                function.generic_parameter_constraint_ids.clone(),
                            ))
                        }
                        _ => None,
                    };
                    if let Some((parameters, generic_parameter_constraint_ids)) = function_data {
                        if function_call.argument_ids.len() != parameters.len() {
                            self.diagnostics.push(Error {
                                span: function_call.arguments_span,
                                msg: format!(
                                    "Expected {} {}, but got {} instead.",
                                    parameters.len(),
                                    plural(parameters.len(), "argument", "arguments"),
                                    function_call.argument_ids.len()
                                ),
                            });
                        } else {
                            let mut substitution_context = HashMap::new();
                            for (i, parameter_id) in parameters.iter().enumerate() {
                                let parameter = self.parameters.get(parameter_id).unwrap();
                                let param_type_id = parameter.type_id;
                                let parameter_type = parameter.type_id.get_type(self);
                                for (i, generic_argument_id) in
                                    function_call.generic_argument_ids.iter().enumerate()
                                {
                                    let generic_parameter_constraint_id =
                                        generic_parameter_constraint_ids.get(i);
                                    if let Some(generic_parameter_constraint_id) =
                                        generic_parameter_constraint_id
                                    {
                                        substitution_context.insert(
                                            *generic_parameter_constraint_id,
                                            *generic_argument_id,
                                        );
                                    }
                                }
                                println!(
                                    "FUNCTION CALL (1) {:?} (2) {:?} (3) {:?}",
                                    function_call.generic_argument_ids,
                                    generic_parameter_constraint_ids,
                                    substitution_context
                                );
                                let argument_id = *function_call.argument_ids.get(i).unwrap();
                                let argument_type = self.infer_type_start(
                                    argument_id,
                                    Type::Unknown,
                                    &substitution_context,
                                );
                                if self.compare_type(
                                    parameter_type.clone(),
                                    argument_type.clone(),
                                    &substitution_context,
                                ) {
                                    let bindings = self.collect_type_bindings(
                                        param_type_id,
                                        &argument_type,
                                        &substitution_context,
                                    );
                                    for (constraint_id, type_id) in bindings {
                                        substitution_context.insert(constraint_id, type_id);
                                    }
                                } else {
                                    self.diagnostics.push(Error {
                                        span: **self.span_map.get(&argument_id).unwrap(),
                                        msg: format!(
                                            "Expected type {:?}, but got type {:?} instead.",
                                            parameter_type, argument_type,
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {
                    println!("cannot type check call to value with indirect function call yet")
                }
            }
        }

        // for (subject_id, value_ids) in self.assignment_values.clone() {
        // 	if let Some(variable) = self.variables.get(&subject_id) {
        // 		let mut type_ = variable.type_.clone();

        // 		for value_id in value_ids {
        // 			type_ = self.resolve_type_start(value_id, type_);
        // 		}

        // 		self.variables.get_mut(&subject_id).unwrap().type_ = type_;
        // 	}
        // }
    }
}

impl TypeId {
    fn get_type(self, analyzer: &Analyzer) -> Type {
        analyzer.get_type_by_type_id(self)
    }
}

impl Type {
    fn get_type_id(self, analyzer: &mut Analyzer) -> TypeId {
        analyzer.type_id_for_type(self)
    }
}

#[derive(Debug)]
pub struct Program<'src> {
    pub closures: IndexMap<Id, Closure>,
    pub diagnostics: Vec<Error>,
    pub entity_map: HashMap<Id, Expr<'src>>,
    pub entity_scope_map: HashMap<Id, Id>,
    pub function_calls: IndexMap<Id, FunctionCall>,
    pub functions: IndexMap<Id, Function<'src>>,
    pub global_scope_id: Id,
    pub module_id_by_name: HashMap<&'src str, Id>,
    pub modules: IndexMap<Id, Module<'src>>,
    pub reference_count: HashMap<Id, u32>,
    pub scopes: IndexMap<Id, Scope<'src>>,
    pub span_map: HashMap<Id, &'src Span>,
    pub structs: IndexMap<Id, Struct<'src>>,
    pub variables: IndexMap<Id, Variable<'src>>,
}

pub fn analyze<'src>(nodes: &'src Spanned<NodeList<'src>>) -> Program<'src> {
    let mut analyzer = Analyzer::new();
    let mut std_module_scope = analyzer.create_scope(None);
    let print_fn_id = analyzer.new_entity_id();
    let print_fn_message_parameter_id = {
        let parameter_id = analyzer.new_entity_id();
        let parameter = Parameter {
            id: parameter_id,
            function_id: print_fn_id,
            name: "message",
            type_id: Type::Any.get_type_id(&mut analyzer),
        };
        analyzer.parameters.insert(parameter_id, parameter);
        parameter_id
    };
    let print_fn_name = "print";
    let print_fn = ExternalFunction {
        id: print_fn_id,
        name: print_fn_name,
        generic_parameter_constraint_ids: Vec::new(),
        parameters: vec![print_fn_message_parameter_id],
        return_type_id: Type::Void.get_type_id(&mut analyzer),
        call_count: 0,
    };
    analyzer.external_functions.insert(print_fn_id, print_fn);
    analyzer
        .expr_id_to_expr_map
        .insert(print_fn_id, Expr::ExternalFunction(print_fn_id));
    std_module_scope
        .name_to_id_map
        .insert(print_fn_name, print_fn_id);
    {
        let print_fn_type_id = analyzer.new_type_id();
        analyzer
            .type_id_to_type_map
            .insert(print_fn_type_id, Type::Function(print_fn_id));
        analyzer
            .expr_id_to_type_id_map
            .insert(print_fn_id, print_fn_type_id);
    }
    let std_module_scope_id = analyzer.push_scope(std_module_scope);
    let std_module_id = analyzer.new_entity_id();
    let std_module = Module {
        id: std_module_id,
        name: "std",
        body: (Vec::new(), std_module_scope_id),
    };
    analyzer
        .module_id_by_name
        .insert(std_module.name, std_module_id);
    analyzer.modules.insert(std_module_id, std_module);
    analyzer
        .expr_id_to_expr_map
        .insert(std_module_id, Expr::Module(std_module_id));
    let global_scope = analyzer.create_scope(None);
    let global_scope_id = analyzer.push_scope(global_scope);
    analyzer.walk_expr_nodes(&nodes.0, global_scope_id);
    analyzer.build();
    Program {
        closures: analyzer.closures,
        diagnostics: analyzer.diagnostics,
        entity_map: analyzer.expr_id_to_expr_map,
        entity_scope_map: analyzer.expr_id_to_scope_id_map,
        function_calls: analyzer.function_calls,
        functions: analyzer.functions,
        global_scope_id,
        module_id_by_name: analyzer.module_id_by_name,
        modules: analyzer.modules,
        reference_count: analyzer.reference_count,
        scopes: analyzer.scopes,
        span_map: analyzer.span_map,
        structs: analyzer.structs,
        variables: analyzer.variables,
    }
}
