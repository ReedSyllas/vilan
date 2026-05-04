use crate::id::Id;
use crate::node::{BinaryOp, Node, NodeIfBranch, NodeList};
use crate::span::{Span, Spanned};
use crate::type_::{PrimitiveType, Type, TypeId};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum Expr<'src> {
    Binary(BinaryOp, Id, Id),
    Block((Vec<Id>, Id)),
    Bool(bool),
    Call(Id),
    Error,
    Field(Id, &'src str),
    Function(Id),
    FunctionReturn(Id),
    If(ExprIfBranch),
    Impl(Id),
    List(Vec<Id>),
    Local(Id),
    Null,
    Number(&'src str, Option<&'src str>),
    String(&'src str),
    Struct(Id),
    StructInitializer(Id, HashMap<usize, Id>),
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
    pub parameters: Vec<Id>,
    pub body: (Vec<Id>, Id, Id),
    pub call_count: u32,
}

#[derive(Debug)]
pub struct FunctionCall {
    pub id: Id,
    pub subject: Id,
    pub arguments: Vec<Id>,
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
    pub declarations: HashMap<&'src str, Id>,
}

#[derive(Debug)]
pub struct Scope<'src> {
    pub id: Id,
    pub parent_id: Option<Id>,
    pub name_to_id_map: HashMap<&'src str, Id>,
    // pub name_to_type_id_map: HashMap<&'src str, TypeId>,
}

#[derive(Debug)]
pub struct Analyzer<'src> {
    assignment_values: HashMap<Id, Vec<Id>>,
    entity_id: u32,
    expr_id_to_type_id_map: HashMap<Id, TypeId>,
    expr_map: HashMap<Id, Expr<'src>>,
    expr_scope_map: HashMap<Id, Id>,
    function_calls: HashMap<Id, FunctionCall>,
    functions: HashMap<Id, Function<'src>>,
    implementations: Vec<Implementation<'src>>,
    locals: Vec<(Id, &'src str)>,
    parameters: HashMap<Id, Parameter<'src>>,
    reference_count: HashMap<Id, u32>,
    scope_id: u32,
    scopes: HashMap<Id, Scope<'src>>,
    span_map: HashMap<Id, &'src Span>,
    static_accessors: Vec<(Id, TypeId, &'src str)>,
    struct_initializers: Vec<(Id, &'src str, Vec<(&'src str, Id)>)>,
    structs: HashMap<Id, Struct<'src>>,
    type_id_to_type_map: HashMap<TypeId, Type>,
    type_id: u32,
    type_locals: Vec<(TypeId, &'src str, Id)>,
    variables: HashMap<Id, Variable<'src>>,
}

impl<'src> Analyzer<'src> {
    fn new() -> Self {
        Self {
            assignment_values: HashMap::new(),
            entity_id: 0,
            expr_id_to_type_id_map: HashMap::new(),
            expr_map: HashMap::new(),
            expr_scope_map: HashMap::new(),
            function_calls: HashMap::new(),
            functions: HashMap::new(),
            implementations: Vec::new(),
            locals: Vec::new(),
            parameters: HashMap::new(),
            reference_count: HashMap::new(),
            scope_id: 0,
            scopes: HashMap::new(),
            span_map: HashMap::new(),
            static_accessors: Vec::new(),
            struct_initializers: Vec::new(),
            structs: HashMap::new(),
            type_id_to_type_map: HashMap::new(),
            type_id: 0,
            type_locals: Vec::new(),
            variables: HashMap::new(),
        }
    }

    fn new_entity_id(&mut self) -> Id {
        let id = self.entity_id;
        self.entity_id += 1;
        Id(id)
    }

    fn get_entity_by_id(&self, id: Id) -> &Expr<'src> {
        self.expr_map.get(&id).expect(
            format!(
                "failed to get entity for id: {:?} in {:#?}",
                id, self.expr_map
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
        self.expr_scope_map
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
            name_to_id_map: HashMap::new(),
            // name_to_type_id_map: HashMap::new(),
        }
    }

    fn push_scope(&mut self, scope: Scope<'src>) -> Id {
        let id = scope.id;
        self.scopes.insert(id, scope);
        id
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
                self.locals.push((id, name));
                None
            }
            Node::MemberAccessor(subject, name) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                Some(Expr::Field(subject_id, name))
            }
            Node::StaticAccessor(subject, name) => {
                let subject_type_id = self.walk_type_node(subject, scope_id);
                self.static_accessors.push((id, subject_type_id, name));
                None
            }
            Node::Import(_) => None,
            Node::List(items) => {
                let ids = self.walk_expr_nodes(items, scope_id);
                Some(Expr::List(ids))
            }
            Node::Tuple(items) => {
                let ids = self.walk_expr_nodes(items, scope_id);
                Some(Expr::Tuple(ids))
            }
            Node::Block(children) => {
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
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
                            let body_scope = s.create_scope(Some(scope_id));
                            let body_scope_id = s.push_scope(body_scope);
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
                            let body_scope = s.create_scope(Some(scope_id));
                            let body_scope_id = s.push_scope(body_scope);
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
                                .map(|x| self.walk_type_node(x, scope_id))
                                .unwrap_or(self.type_id_for_type(Type::Unknown)),
                        };
                        body_scope
                            .name_to_id_map
                            .insert(parameter.name, parameter.id);
                        self.parameters.insert(parameter.id, parameter);
                        parameter_id
                    })
                    .collect::<Vec<_>>();
                let body_scope_id = self.push_scope(body_scope);
                let ids = self.walk_expr_nodes(&function.body.0.0, body_scope_id);
                let expr_id = self.walk_expr_node(&function.body.0.1, body_scope_id);
                self.functions.insert(
                    id,
                    Function {
                        id,
                        name,
                        parameters,
                        body: (ids, expr_id, body_scope_id),
                        call_count: 0,
                    },
                );
                Some(Expr::Function(id))
            }
            Node::Call(subject, arguments) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                let argument_ids = self.walk_expr_nodes(&arguments.0, scope_id);
                self.function_calls.insert(
                    id,
                    FunctionCall {
                        id,
                        subject: subject_id,
                        arguments: argument_ids,
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
                    .unwrap_or(self.type_id_for_type(Type::Unknown));
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
            Node::Struct(name, body) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let mut fields = Vec::new();
                for child in &body.0 {
                    let name = child.0.0;
                    let type_id = child
                        .0
                        .1
                        .as_ref()
                        .map(|x| self.walk_type_node(x, scope_id))
                        .unwrap_or(self.type_id_for_type(Type::Unknown));
                    fields.push(Field { name, type_id });
                }
                self.structs.insert(id, Struct { id, name, fields });
                Some(Expr::Struct(id))
            }
            Node::StructInitializer(name, fields) => {
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
                                    self.locals.push((local_id, name));
                                    local_id
                                }),
                        )
                    })
                    .collect::<Vec<_>>();
                self.struct_initializers.push((id, name, e_fields));
                None
            }
            Node::Impl(subject, body) => {
                let subject = self.walk_type_node(subject, scope_id);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                self.walk_expr_nodes(&body.0, body_scope_id);
                let body_scope = self.scopes.get(&body_scope_id).unwrap();
                self.implementations.push(Implementation {
                    subject,
                    declarations: body_scope.name_to_id_map.clone(),
                });

                Some(Expr::Impl(id))
            }
        };

        if let Some(entity) = entity {
            self.expr_map.insert(id, entity);
        }

        self.span_map.insert(id, &node.1);
        self.expr_scope_map.insert(id, scope_id);

        id
    }

    fn walk_type_node(&mut self, node: &Spanned<Node<'src>>, scope_id: Id) -> TypeId {
        let type_id = self.new_type_id();

        let type_: Option<Type> = match &node.0 {
            Node::Accessor(name) => match *name {
                "f64" => Some(Type::Primitive(PrimitiveType::F64)),
                "i32" => Some(Type::Primitive(PrimitiveType::I32)),
                "u32" => Some(Type::Primitive(PrimitiveType::U32)),
                "str" => Some(Type::Primitive(PrimitiveType::String)),
                "bool" => Some(Type::Primitive(PrimitiveType::Bool)),
                "null" => Some(Type::Primitive(PrimitiveType::Null)),
                x => {
                    self.type_locals.push((type_id, name, scope_id));
                    None
                }
            },
            Node::Tuple(types) => Some(Type::Tuple(
                types
                    .iter()
                    .map(|type_| self.walk_type_node(type_, scope_id))
                    .collect(),
            )),
            x => unimplemented!("unhandled type node: {:?}", x),
        };

        if let Some(type_) = type_ {
            self.type_id_to_type_map.insert(type_id, type_);
        }

        type_id
    }

    fn resolve_type_start(&mut self, expr_id: Id, constraint_type_id: TypeId) -> TypeId {
        self.resolve_type(expr_id, constraint_type_id, &mut HashSet::new())
    }

    fn resolve_type(
        &mut self,
        expr_id: Id,
        constraint_type_id: TypeId,
        exprs_seen: &mut HashSet<Id>,
    ) -> TypeId {
        if exprs_seen.contains(&expr_id) {
            panic!(
                "recursive type found for {:?} in {:#?}",
                expr_id, exprs_seen
            );
        }
        exprs_seen.insert(expr_id);

        if let Some(type_id) = self.expr_id_to_type_id_map.get(&expr_id) {
            return *type_id;
        }

        let constraint = self.get_type_by_type_id(constraint_type_id);

        let expr = self.get_entity_by_id(expr_id);

        let inferred_type_id: TypeId = match expr {
            Expr::Null => self.type_id_for_type(Type::Primitive(PrimitiveType::Null)),
            Expr::Bool(_) => self.type_id_for_type(Type::Primitive(PrimitiveType::Bool)),
            Expr::String(_) => self.type_id_for_type(Type::Primitive(PrimitiveType::String)),
            Expr::Number(_, _) => self.type_id_for_type(Type::Primitive(match constraint {
                Type::Primitive(PrimitiveType::F64) => PrimitiveType::F64,
                Type::Primitive(PrimitiveType::U32) => PrimitiveType::U32,
                _ => PrimitiveType::I32,
            })),
            Expr::List(_) => {
                let list_type_id = self.type_id_for_type(Type::Void);
                self.type_id_for_type(Type::Primitive(PrimitiveType::List(list_type_id)))
            }
            Expr::Tuple(item_ids) => {
                let constraint_items = match constraint.clone() {
                    Type::Tuple(items) => items,
                    _ => Vec::new(),
                };
                let type_ = Type::Tuple(
                    item_ids
                        .clone()
                        .iter()
                        .enumerate()
                        .map(|(i, id)| {
                            let fallback_type_id = self.type_id_for_type(Type::Unknown);
                            self.resolve_type(
                                *id,
                                constraint_items
                                    .get(i)
                                    .map(|x| x.clone())
                                    .unwrap_or(fallback_type_id),
                                exprs_seen,
                            )
                        })
                        .collect(),
                );
                self.type_id_for_type(type_)
            }
            Expr::Local(subject_id) => {
                self.resolve_type(*subject_id, constraint_type_id, exprs_seen)
            }
            Expr::Function(function_id) => self.type_id_for_type(Type::Function(*function_id)),
            Expr::Struct(struct_id) => self.type_id_for_type(Type::Struct(*struct_id)),
            Expr::Call(id) => {
                let id = *id;
                let against_type = self.type_id_for_type(Type::Unknown);
                let function_call = self.function_calls.get(&id).unwrap();
                let subject_type_id =
                    self.resolve_type(function_call.subject, against_type, exprs_seen);
                let subject_type = self.get_type_by_type_id(subject_type_id);
                match subject_type {
                    Type::Function(function_id) => self.type_id_for_type(Type::Void),
                    x => panic!("type is not callable: {:?}", x),
                }
            }
            _ => self.type_id_for_type(Type::Void),
        };

        self.reconcile_type(constraint_type_id, inferred_type_id)
    }

    fn reconcile_type(&mut self, a_id: TypeId, b_id: TypeId) -> TypeId {
        let a = self.get_type_by_type_id(a_id);
        let b = self.get_type_by_type_id(b_id);
        match (a, b) {
            (a, Type::Unknown) => a_id,
            (Type::Unknown, b) => b_id,
            (Type::Primitive(a), Type::Primitive(b)) => match (a, b) {
                (PrimitiveType::List(a), PrimitiveType::List(b)) => {
                    let type_ = Type::Primitive(PrimitiveType::List(self.reconcile_type(a, b)));
                    self.type_id_for_type(type_)
                }
                (a, b) if a == b => a_id,
                (a, b) => panic!("types {:#?} and {:#?} are mismatched", a, b),
            },
            (Type::Tuple(aa), Type::Tuple(bb)) => {
                let type_ = Type::Tuple(
                    aa.iter()
                        .zip(bb.iter())
                        .map(|(a, b)| self.reconcile_type(*a, *b))
                        .collect(),
                );
                self.type_id_for_type(type_)
            }
            (a, b) if a == b => a_id,
            (a, b) => panic!("types {:#?} and {:#?} are mismatched", a, b),
        }
    }

    fn compare_type(&self, a_id: TypeId, b_id: TypeId) -> bool {
        let a = self.get_type_by_type_id(a_id);
        let b = self.get_type_by_type_id(b_id);
        match (a, b) {
            (a, Type::Unknown) => true,
            (Type::Unknown, b) => true,
            (Type::Primitive(a), Type::Primitive(b)) => match (a, b) {
                (PrimitiveType::List(a_id), PrimitiveType::List(b_id)) => {
                    self.compare_type(a_id, b_id)
                }
                (a, b) if a == b => true,
                _ => false,
            },
            (Type::Tuple(aa), Type::Tuple(bb)) => aa
                .iter()
                .zip(bb.iter())
                .all(|(a_id, b_id)| self.compare_type(*a_id, *b_id)),
            (a, b) if a == b => true,
            _ => false,
        }
    }

    fn build(&mut self) {
        for (id, name) in self.locals.clone() {
            let scope_id = self.get_scope_id_for_entity(id);
            let subject_id = self.get_expr_id_by_name(name, scope_id);
            self.expr_map.insert(id, Expr::Local(subject_id));

            let rc = self.reference_count.entry(subject_id).or_insert(0);
            *rc += 1;
        }

        for (type_id, name, scope_id) in self.type_locals.clone() {
            let subject_id = self.get_expr_id_by_name(name, scope_id);
            let constraint_type_id = self.type_id_for_type(Type::Unknown);
            let subject_type_id = self.resolve_type_start(subject_id, constraint_type_id);
            let subject_type = self.get_type_by_type_id(subject_type_id);
            self.type_id_to_type_map.insert(type_id, subject_type);
        }

        for (id, subject_type_id, name) in self.static_accessors.clone() {
            let subject_type = self.get_type_by_type_id(subject_type_id);
            match subject_type {
                Type::Struct(struct_id) => {
                    let struct_name = self.structs.get(&struct_id).unwrap().name;
                    let member_id = *self
                        .implementations
                        .iter()
                        .filter(|x| self.compare_type(subject_type_id, x.subject))
                        .find_map(|x| x.declarations.get(name))
                        .expect(format!("cannot find {} in {}", name, struct_name).as_str());
                    self.expr_map.insert(id, Expr::Local(member_id));
                }
                _ => {}
            }
        }

        for (id, name, fields) in self.struct_initializers.clone() {
            let scope_id = self.get_scope_id_for_entity(id);
            let scope = self.mut_scope_for_scope_id(scope_id);
            let struct_id = self.get_expr_id_by_name(name, scope_id);
            let struct_ = self
                .structs
                .get(&struct_id)
                .expect("cannot initialize a non-struct");
            let mut initializer_fields = HashMap::new();
            for field in fields {
                let index = struct_
                    .fields
                    .iter()
                    .position(|x| x.name == field.0)
                    .expect(format!("unknown field: {}", field.0).as_str());
                initializer_fields.insert(index, field.1);
            }
            self.expr_map
                .insert(id, Expr::StructInitializer(struct_id, initializer_fields));
        }

        // for function_call in self.function_calls.values() {
        //     match self.get_entity_by_id(function_call.subject) {
        //         Entity::Local(subject_id) => {
        //             if let Some(struct_) = self.structs.get(subject_id) {
        //                 let mut initializer_fields = HashMap::new();
        //                 for (id, value) in function_call.arguments.iter().enumerate() {
        //                     initializer_fields.insert(id, *value);
        //                 }
        //                 self.entity_map.insert(
        //                     function_call.id,
        //                     Entity::StructInitializer(*subject_id, initializer_fields),
        //                 );
        //             }
        //         }
        //         _ => {}
        //     }
        // }

        // TODO: Type check all entities.
        //       Requires caching to prevent extra work.
        // for entity_id in self.entity_map.keys() {
        // 	self.resolve_type_start(entity_id, Type::Unknown);
        // }

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

#[derive(Debug)]
pub struct Program<'src> {
    pub entity_map: HashMap<Id, Expr<'src>>,
    pub entity_scope_map: HashMap<Id, Id>,
    pub function_calls: HashMap<Id, FunctionCall>,
    pub functions: HashMap<Id, Function<'src>>,
    pub global_scope_id: Id,
    pub print_fn_id: Id,
    pub reference_count: HashMap<Id, u32>,
    pub scopes: HashMap<Id, Scope<'src>>,
    pub span_map: HashMap<Id, &'src Span>,
    pub structs: HashMap<Id, Struct<'src>>,
    pub variables: HashMap<Id, Variable<'src>>,
}

pub fn analyze<'src>(nodes: &'src Spanned<NodeList<'src>>) -> Program<'src> {
    let mut analyzer = Analyzer::new();
    let mut global_scope = analyzer.create_scope(None);
    let print_fn_id = analyzer.new_entity_id();
    let print_fn_type_id = analyzer.new_type_id();
    global_scope.name_to_id_map.insert("print", print_fn_id);
    analyzer
        .type_id_to_type_map
        .insert(print_fn_type_id, Type::Function(print_fn_id));
    analyzer
        .expr_id_to_type_id_map
        .insert(print_fn_id, print_fn_type_id);
    let global_scope_id = analyzer.push_scope(global_scope);
    analyzer.walk_expr_nodes(&nodes.0, global_scope_id);
    analyzer.build();
    Program {
        entity_map: analyzer.expr_map,
        entity_scope_map: analyzer.expr_scope_map,
        function_calls: analyzer.function_calls,
        functions: analyzer.functions,
        global_scope_id,
        print_fn_id,
        reference_count: analyzer.reference_count,
        scopes: analyzer.scopes,
        span_map: analyzer.span_map,
        structs: analyzer.structs,
        variables: analyzer.variables,
    }
}
