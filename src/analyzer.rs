use crate::id::Id;
use crate::node::{BinaryOp, Node, NodeList};
use crate::span::{Span, Spanned};
use crate::type_::{PrimitiveType, Type};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum Entity<'src> {
    Binary(BinaryOp, Id, Id),
    Block((Vec<Id>, Id)),
    Bool(bool),
    Call(Id),
    Error,
    Function(Id),
    FunctionReturn(Id),
    If(EntityIfBranch),
    List(Vec<Id>),
    Local(Id),
    Member(Id, &'src str),
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
pub enum EntityIfBranch {
    If(Id, (Vec<Id>, Id), Option<Box<EntityIfBranch>>),
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
    pub type_: Type,
}

#[derive(Debug)]
pub struct Variable<'src> {
    pub id: Id,
    pub name: &'src str,
    pub initial: Option<Id>,
    pub type_: Type,
    // pub mutable: bool,
}

#[derive(Debug)]
pub struct Struct<'src> {
    pub id: Id,
    pub fields: Vec<Field<'src>>,
}

#[derive(Debug)]
pub struct Field<'src> {
    pub name: &'src str,
    pub type_: Type,
}

#[derive(Debug)]
pub struct Scope<'src> {
    pub id: Id,
    pub parent_id: Option<Id>,
    pub name_id_map: HashMap<&'src str, Id>,
}

#[derive(Debug)]
pub struct Analyzer<'src> {
    assignment_values: HashMap<Id, Vec<Id>>,
    entity_id: u32,
    entity_map: HashMap<Id, Entity<'src>>,
    entity_scope_map: HashMap<Id, Id>,
    external_signatures: HashMap<Id, Type>,
    function_calls: HashMap<Id, FunctionCall>,
    functions: HashMap<Id, Function<'src>>,
    locals: Vec<(Id, &'src str)>,
    struct_initializers: Vec<(Id, &'src str, Vec<(&'src str, Id)>)>,
    parameters: HashMap<Id, Parameter<'src>>,
    reference_count: HashMap<Id, u32>,
    scope_id: u32,
    scopes: HashMap<Id, Scope<'src>>,
    span_map: HashMap<Id, &'src Span>,
    structs: HashMap<Id, Struct<'src>>,
    variables: HashMap<Id, Variable<'src>>,
}

impl<'src> Analyzer<'src> {
    fn new() -> Self {
        Self {
            assignment_values: HashMap::new(),
            entity_id: 0,
            entity_map: HashMap::new(),
            entity_scope_map: HashMap::new(),
            external_signatures: HashMap::new(),
            function_calls: HashMap::new(),
            functions: HashMap::new(),
            locals: Vec::new(),
            parameters: HashMap::new(),
            reference_count: HashMap::new(),
            scope_id: 0,
            scopes: HashMap::new(),
            span_map: HashMap::new(),
            struct_initializers: Vec::new(),
            structs: HashMap::new(),
            variables: HashMap::new(),
        }
    }

    fn new_entity_id(&mut self) -> Id {
        let id = self.entity_id;
        self.entity_id += 1;
        Id(id)
    }

    fn get_entity_by_id(&self, id: Id) -> &Entity<'src> {
        self.entity_map.get(&id).expect(
            format!(
                "failed to get entity for id: {:?} in {:#?}",
                id, self.entity_map
            )
            .as_str(),
        )
    }

    fn new_scope_id(&mut self) -> Id {
        let id = self.scope_id;
        self.scope_id += 1;
        Id(id)
    }

    fn get_scope_by_id(&mut self, scope_id: Id) -> &mut Scope<'src> {
        self.scopes
            .get_mut(&scope_id)
            .expect(format!("failed to get scope for id: {:?}", scope_id.0).as_str())
    }

    fn get_scope_id_for_entity(&mut self, entity_id: Id) -> Id {
        self.entity_scope_map
            .get(&entity_id)
            .map(|scope_id| *scope_id)
            .expect(format!("failed to get scope of entity: {:?}", entity_id.0).as_str())
    }

    fn get_scope_for_entity(&mut self, entity_id: Id) -> &mut Scope<'src> {
        let scope_id = self.get_scope_id_for_entity(entity_id);
        self.get_scope_by_id(scope_id)
    }

    fn create_scope(&mut self, parent_id: Option<Id>) -> Scope<'src> {
        let id = self.new_scope_id();
        Scope {
            id,
            parent_id,
            name_id_map: HashMap::new(),
        }
    }

    fn push_scope(&mut self, scope: Scope<'src>) -> Id {
        let id = scope.id;
        self.scopes.insert(id, scope);
        id
    }

    fn resolve_entity_by_name(&mut self, name: &'src str, scope_id: Id) -> Id {
        fn resolve<'src>(
            analyzer: &mut Analyzer<'src>,
            name: &'src str,
            scope_id: Id,
        ) -> Option<Id> {
            let scope = analyzer.get_scope_by_id(scope_id);
            let parent_id = scope.parent_id;
            // println!("scanning {} in {:?} {:#?}", name, scope_id, scope.name_id_map);
            scope.name_id_map.get(name).map(|x| *x).or_else(|| {
                let subject_id = parent_id
                    .map(|parent_scope_id| resolve(analyzer, name, parent_scope_id))
                    .flatten()?;
                let scope = analyzer.get_scope_by_id(scope_id);
                scope.name_id_map.insert(name, subject_id);
                Some(subject_id)
            })
        }

        resolve(self, name, scope_id).expect(format!("cannot find: {}", name).as_str())
    }

    fn walk_list(&mut self, list: &'src NodeList<'src>, scope_id: Id) -> Vec<Id> {
        list.iter()
            .map(|child| self.walk_node(child, scope_id))
            .collect::<Vec<_>>()
    }

    fn walk_node(
        &mut self,
        node: &'src Spanned<Node<'src>>,
        scope_id: Id, /* , context: enum SemanticContext { Statement, Expression, Type } */
    ) -> Id {
        let id = self.new_entity_id();

        let entity = match &node.0 {
            Node::Error => Some(Entity::Error),
            Node::Void => Some(Entity::Void),
            Node::Null => Some(Entity::Null),
            Node::Bool(x) => Some(Entity::Bool(*x)),
            Node::String(x) => Some(Entity::String(x)),
            Node::Number(whole, fraction) => Some(Entity::Number(whole, *fraction)),
            Node::Local(name) => {
                self.locals.push((id, name));
                None
            }
            Node::Member(subject, name) => {
                let subject_id = self.walk_node(subject, scope_id);
                Some(Entity::Member(subject_id, name))
            }
            Node::Import(_) => None,
            Node::List(items) => {
                let ids = self.walk_list(items, scope_id);
                Some(Entity::List(ids))
            }
            Node::Tuple(items) => {
                let ids = self.walk_list(items, scope_id);
                Some(Entity::Tuple(ids))
            }
            Node::Block(children) => {
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let ids = self.walk_list(&children.0.0, body_scope_id);
                let expr_id = self.walk_node(&children.0.1, body_scope_id);
                Some(Entity::Block((ids, expr_id)))
            }
            Node::If(if_) => {
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let condition_id = self.walk_node(&if_.condition, body_scope_id);
                let then_ids = self.walk_list(&if_.then.0.0, body_scope_id);
                let then_expr_id = self.walk_node(&if_.then.0.1, body_scope_id);
                Some(Entity::If(EntityIfBranch::If(
                    condition_id,
                    (then_ids, then_expr_id),
                    None,
                )))
            }
            Node::Func(function) => {
                let name = function.name.0;
                let scope = self.get_scope_by_id(scope_id);
                scope.name_id_map.insert(name, id);
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
                            type_: x
                                .1
                                .as_ref()
                                .map(|x| self.walk_type(x))
                                .unwrap_or(Type::Unknown),
                        };
                        body_scope.name_id_map.insert(parameter.name, parameter.id);
                        self.parameters.insert(parameter.id, parameter);
                        parameter_id
                    })
                    .collect::<Vec<_>>();
                let body_scope_id = self.push_scope(body_scope);
                let ids = self.walk_list(&function.body.0.0, body_scope_id);
                let expr_id = self.walk_node(&function.body.0.1, body_scope_id);
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
                Some(Entity::Function(id))
            }
            Node::Call(subject, arguments) => {
                let subject_id = self.walk_node(subject, scope_id);
                let argument_ids = self.walk_list(&arguments.0, scope_id);
                self.function_calls.insert(
                    id,
                    FunctionCall {
                        id,
                        subject: subject_id,
                        arguments: argument_ids,
                    },
                );
                Some(Entity::Call(id))
            }
            Node::FuncReturn(value) => {
                let id = self.walk_node(value, scope_id);
                Some(Entity::FunctionReturn(id))
            }
            Node::Binary(op, lhs, rhs) => {
                let lhs_id = self.walk_node(lhs, scope_id);
                let rhs_id = self.walk_node(rhs, scope_id);
                Some(Entity::Binary(*op, lhs_id, rhs_id))
            }
            Node::Let(name, type_, value) => {
                let scope = self.get_scope_by_id(scope_id);
                scope.name_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let initial = value.as_ref().map(|value| {
                    let value_id = self.walk_node(value, scope_id);
                    let assignments = self
                        .assignment_values
                        .entry(id)
                        .or_insert_with(|| Vec::new());
                    assignments.push(value_id);
                    value_id
                });
                let type_ = type_
                    .as_ref()
                    .map(|x| self.walk_type(x))
                    .unwrap_or(Type::Unknown);
                self.variables.insert(
                    id,
                    Variable {
                        id,
                        name,
                        initial,
                        type_,
                    },
                );
                Some(Entity::Variable(id))
            }
            Node::Struct(name, body) => {
                let scope = self.get_scope_by_id(scope_id);
                scope.name_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let mut fields = Vec::new();
                for child in &body.0 {
                    let name = child.0.0;
                    let type_ = child
                        .0
                        .1
                        .as_ref()
                        .map(|x| self.walk_type(x))
                        .unwrap_or(Type::Unknown);
                    fields.push(Field { name, type_ });
                }
                self.structs.insert(id, Struct { id, fields });
                Some(Entity::Struct(id))
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
                                .map(|x| self.walk_node(x, scope_id))
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
        };

        if let Some(entity) = entity {
            self.entity_map.insert(id, entity);
        }

        self.span_map.insert(id, &node.1);
        self.entity_scope_map.insert(id, scope_id);

        id
    }

    fn walk_type(&mut self, node: &Spanned<Node>) -> Type {
        match &node.0 {
            Node::Local(name) => match *name {
                "f64" => Type::Primitive(PrimitiveType::F64),
                "i32" => Type::Primitive(PrimitiveType::I32),
                "u32" => Type::Primitive(PrimitiveType::U32),
                "str" => Type::Primitive(PrimitiveType::String),
                "bool" => Type::Primitive(PrimitiveType::Bool),
                "null" => Type::Primitive(PrimitiveType::Null),
                x => panic!("unknown type: {:?}", x),
            },
            Node::Tuple(types) => {
                Type::Tuple(types.iter().map(|type_| self.walk_type(type_)).collect())
            }
            x => unimplemented!("unhandled type: {:?}", x),
        }
    }

    fn resolve_type_start(&self, id: Id, constraint: Type) -> Type {
        self.resolve_type(id, constraint, &mut HashSet::new())
    }

    fn resolve_type(&self, id: Id, constraint: Type, seen: &mut HashSet<Id>) -> Type {
        if seen.contains(&id) {
            panic!("recursive type found for {:?} in {:#?}", id, seen);
        }

        seen.insert(id);

        if let Some(type_) = self.external_signatures.get(&id) {
            return type_.clone();
        }

        let type_ = match self.get_entity_by_id(id) {
            Entity::Null => Type::Primitive(PrimitiveType::Null),
            Entity::Bool(_) => Type::Primitive(PrimitiveType::Bool),
            Entity::String(_) => Type::Primitive(PrimitiveType::String),
            Entity::Number(_, _) => Type::Primitive(match constraint {
                Type::Primitive(PrimitiveType::F64) => PrimitiveType::F64,
                Type::Primitive(PrimitiveType::U32) => PrimitiveType::U32,
                _ => PrimitiveType::I32,
            }),
            Entity::List(_) => Type::Primitive(PrimitiveType::List(Box::new(Type::Void))),
            Entity::Tuple(item_ids) => {
                let constraint_items = match constraint.clone() {
                    Type::Tuple(items) => items,
                    _ => Vec::new(),
                };
                Type::Tuple(
                    item_ids
                        .iter()
                        .enumerate()
                        .map(|(i, id)| {
                            self.resolve_type(
                                *id,
                                constraint_items
                                    .get(i)
                                    .map(|x| x.clone())
                                    .unwrap_or(Type::Unknown),
                                seen,
                            )
                        })
                        .collect(),
                )
            }
            Entity::Local(subject_id) => self.resolve_type(*subject_id, constraint.clone(), seen),
            Entity::Function(function_id) => {
                let function = self.functions.get(function_id).unwrap();
                // let parameter_types = function.parameters.iter().map(|parameter_id| self.parameters.get(parameter_id).unwrap().type_.clone()).collect::<Vec<_>>();
                // Type::Function(parameter_types, Box::new(Type::Void))
                Type::Function(Vec::new(), Box::new(Type::Void))
            }
            Entity::Call(id) => {
                let function_call = self.function_calls.get(id).unwrap();
                if let Some(struct_) = self.structs.get(&function_call.subject) {
                    Type::Struct(struct_.id)
                } else {
                    let subject_type =
                        self.resolve_type(function_call.subject, Type::Unknown, seen);
                    match subject_type {
                        Type::Function(parameter_types, return_type) => (*return_type).clone(),
                        x => panic!("type is not callable: {:?}", x),
                    }
                }
            }
            _ => Type::Void,
        };

        constraint.reconcile(type_)
    }

    fn build(&mut self) {
        for (id, name) in self.locals.clone() {
            let scope_id = self.get_scope_id_for_entity(id);
            let subject_id = self.resolve_entity_by_name(name, scope_id);
            self.entity_map.insert(id, Entity::Local(subject_id));

            let rc = self.reference_count.entry(subject_id).or_insert(0);
            *rc += 1;
        }

        for (id, name, fields) in self.struct_initializers.clone() {
            let scope_id = self.get_scope_id_for_entity(id);
            let scope = self.get_scope_by_id(scope_id);
            let struct_id = self.resolve_entity_by_name(name, scope_id);
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
            self.entity_map
                .insert(id, Entity::StructInitializer(struct_id, initializer_fields));
        }

        for function_call in self.function_calls.values() {
            match self.get_entity_by_id(function_call.subject) {
                Entity::Local(subject_id) => {
                    if let Some(struct_) = self.structs.get(subject_id) {
                        let mut initializer_fields = HashMap::new();
                        for (id, value) in function_call.arguments.iter().enumerate() {
                            initializer_fields.insert(id, *value);
                        }
                        self.entity_map.insert(
                            function_call.id,
                            Entity::StructInitializer(*subject_id, initializer_fields),
                        );
                    }
                }
                _ => {}
            }
        }

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
    pub entity_map: HashMap<Id, Entity<'src>>,
    pub entity_scope_map: HashMap<Id, Id>,
    pub function_calls: HashMap<Id, FunctionCall>,
    pub functions: HashMap<Id, Function<'src>>,
    pub print_fn_id: Id,
    pub reference_count: HashMap<Id, u32>,
    pub scopes: HashMap<Id, Scope<'src>>,
    pub span_map: HashMap<Id, &'src Span>,
    pub structs: HashMap<Id, Struct<'src>>,
    pub variables: HashMap<Id, Variable<'src>>,
}

pub fn analyze<'src>(nodes: &'src Spanned<NodeList<'src>>) -> Program<'src> {
    let mut analyzer = Analyzer::new();
    let mut root_scope = analyzer.create_scope(None);
    let print_fn_id = analyzer.new_entity_id();
    root_scope.name_id_map.insert("print", print_fn_id);
    analyzer.external_signatures.insert(
        print_fn_id,
        Type::Function(vec![Type::Any], Box::new(Type::Void)),
    );
    let root_scope_id = analyzer.push_scope(root_scope);
    analyzer.walk_list(&nodes.0, root_scope_id);
    analyzer.build();
    Program {
        entity_map: analyzer.entity_map,
        entity_scope_map: analyzer.entity_scope_map,
        function_calls: analyzer.function_calls,
        functions: analyzer.functions,
        print_fn_id,
        reference_count: analyzer.reference_count,
        scopes: analyzer.scopes,
        span_map: analyzer.span_map,
        structs: analyzer.structs,
        variables: analyzer.variables,
    }
}
