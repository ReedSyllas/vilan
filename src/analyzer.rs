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
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub fields: Vec<Field<'src>>,
}

#[derive(Debug, Clone)]
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

/// A constraint that a struct initializer's field value must
/// match the corresponding struct field type.
#[derive(Debug, Clone)]
pub struct StructInitializerConstraint<'src> {
    pub struct_id: Id,
    pub struct_name: &'src str,
    pub struct_fields: Vec<Field<'src>>,
    pub generic_argument_ids: Vec<TypeId>,
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub fields: Vec<(&'src str, Id, Span)>,
    pub fields_span: Span,
}

/// A constraint that a variable's type must unify with its
/// initial value's type (possibly multiple times for reassignments).
#[derive(Debug)]
pub struct VariableConstraint {
    pub variable_id: Id,
    pub initial_type_id: TypeId,
    pub value_ids: Vec<Id>,
}

/// A constraint that a call subject expression's type must resolve to
/// a callable type (Function or ExternalFunction) and that each argument
/// must unify with the corresponding parameter.
#[derive(Debug)]
pub struct CallSubjectConstraint {
    pub call_id: Id,
    pub subject_id: Id,
    pub generic_argument_ids: Vec<TypeId>,
    pub argument_ids: Vec<Id>,
    pub arguments_span: Span,
}

/// A constraint that a field accessor's subject type must resolve to
/// a struct and that a field of that struct is accessible by name.
#[derive(Debug)]
pub struct FieldAccessorConstraint<'src> {
    pub id: Id,
    pub subject_id: Id,
    pub member_name: &'src str,
}

impl<'src> StructInitializerConstraint<'src> {
    fn from_walk(
        _id: Id,
        name: &'src str,
        generic_argument_ids: Vec<TypeId>,
        e_fields: Vec<(&'src str, Id, Span)>,
        fields_span: Span,
    ) -> Self {
        Self {
            struct_id: Id(0),
            struct_name: name,
            struct_fields: Vec::new(),
            generic_argument_ids,
            generic_parameter_constraint_ids: Vec::new(),
            fields: e_fields,
            fields_span,
        }
    }
}

impl VariableConstraint {
    fn from_walk(variable_id: Id, initial_type_id: TypeId, value_ids: Vec<Id>) -> Self {
        Self {
            variable_id,
            initial_type_id,
            value_ids,
        }
    }
}

impl CallSubjectConstraint {
    fn from_walk(
        id: Id,
        subject_id: Id,
        generic_argument_ids: Vec<TypeId>,
        argument_ids: Vec<Id>,
        arguments_span: Span,
    ) -> Self {
        Self {
            call_id: id,
            subject_id,
            generic_argument_ids,
            argument_ids,
            arguments_span,
        }
    }
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
    call_subject_constraints: Vec<CallSubjectConstraint>,
    closures: IndexMap<Id, Closure>,
    diagnostics: Vec<Error>,
    entity_id: u32,
    expr_id_to_expr_map: HashMap<Id, Expr<'src>>,
    expr_id_to_scope_id_map: HashMap<Id, Id>,
    expr_id_to_type_id_map: HashMap<Id, TypeId>,
    external_functions: IndexMap<Id, ExternalFunction<'src>>,
    field_accessor_constraints: IndexMap<Id, FieldAccessorConstraint<'src>>,
    function_calls: IndexMap<Id, FunctionCall>,
    functions: IndexMap<Id, Function<'src>>,
    generic_constraint_names: HashMap<TypeId, &'src str>,
    implementations: Vec<Implementation<'src>>,
    module_id_by_name: HashMap<&'src str, Id>,
    modules: IndexMap<Id, Module<'src>>,
    parameters: IndexMap<Id, Parameter<'src>>,
    prepped_field_accessors: Vec<(Id, Id, &'src str)>,
    prepped_imports: Vec<(Vec<&'src str>, &'src str, Id)>,
    prepped_locals: Vec<(Id, &'src str)>,
    prepped_method_calls: Vec<(Id, Id, &'src str, Vec<TypeId>, Vec<Id>, Span)>,
    prepped_static_accessors: Vec<(Id, TypeId, &'src str)>,
    prepped_struct_initializers:
        Vec<(Id, &'src str, Vec<TypeId>, Vec<(&'src str, Id, Span)>, Span)>,
    prepped_type_locals: Vec<(TypeId, &'src str, Id)>,
    prepped_type_static_accessors: Vec<(TypeId, TypeId, &'src str)>,
    reference_count: HashMap<Id, u32>,
    resolved_types: HashMap<Id, TypeId>,
    scope_id: u32,
    scopes: IndexMap<Id, Scope<'src>>,
    span_map: HashMap<Id, &'src Span>,
    struct_initializer_constraints: Vec<StructInitializerConstraint<'src>>,
    structs: IndexMap<Id, Struct<'src>>,
    type_id_to_type_map: HashMap<TypeId, Type>,
    type_id: u32,
    variable_constraints: Vec<VariableConstraint>,
    variables: IndexMap<Id, Variable<'src>>,
}

static EMPTY_SPAN: Span = Span {
    start: 0,
    end: 0,
    context: (),
};

impl<'src> Analyzer<'src> {
    fn new() -> Self {
        Self {
            assignment_values: IndexMap::new(),
            call_subject_constraints: Vec::new(),
            closures: IndexMap::new(),
            diagnostics: Vec::new(),
            entity_id: 0,
            expr_id_to_expr_map: HashMap::new(),
            expr_id_to_scope_id_map: HashMap::new(),
            expr_id_to_type_id_map: HashMap::new(),
            external_functions: IndexMap::new(),
            field_accessor_constraints: IndexMap::new(),
            function_calls: IndexMap::new(),
            functions: IndexMap::new(),
            generic_constraint_names: HashMap::new(),
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
            resolved_types: HashMap::new(),
            scope_id: 0,
            scopes: IndexMap::new(),
            span_map: HashMap::new(),
            struct_initializer_constraints: Vec::new(),
            structs: IndexMap::new(),
            type_id_to_type_map: HashMap::new(),
            type_id: 0,
            variable_constraints: Vec::new(),
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

    fn get_expr_id_by_name(&mut self, name: &'src str, scope_id: Id) -> Id {
        fn resolve<'src>(
            analyzer: &mut Analyzer<'src>,
            name: &'src str,
            scope_id: Id,
        ) -> Option<Id> {
            let scope = analyzer.mut_scope_for_scope_id(scope_id);
            let parent_id = scope.parent_id;
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
            Node::AccessorWithGenerics(name, _generic_arguments) => {
                self.prepped_locals.push((id, name));
                None
            }
            Node::MemberAccessor(subject, member) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                match &member.0 {
                    Node::Accessor(name) => {
                        self.field_accessor_constraints.insert(
                            id,
                            FieldAccessorConstraint {
                                id,
                                subject_id,
                                member_name: name,
                            },
                        );
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
                        self.generic_constraint_names
                            .insert(constraint_type_id, name);
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
                // Defer call type-checking to constraint solving.
                self.call_subject_constraints
                    .push(CallSubjectConstraint::from_walk(
                        id,
                        subject_id,
                        generic_argument_ids,
                        argument_ids,
                        arguments.1,
                    ));
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
                // Collect a variable constraint for type inference.
                // If the variable has an explicit type annotation, the
                // constraint will unify the initial value against that
                // annotation. If not, the value's type becomes the
                // variable's type once resolved.
                let value_ids = self.assignment_values.get(&id).cloned().unwrap_or_default();
                self.variable_constraints
                    .push(VariableConstraint::from_walk(id, type_id, value_ids));
                Some(Expr::Variable(id))
            }
            Node::Struct(name, generic_parameters, body) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let mut generic_parameter_constraint_ids = Vec::new();
                if let Some(generic_parameters) = generic_parameters {
                    for (name, type_) in &generic_parameters.0 {
                        let constraint_type_id = type_
                            .as_ref()
                            .map(|x| self.walk_type_node(x, body_scope_id))
                            .unwrap_or_else(|| Type::Any.get_type_id(self));
                        self.generic_constraint_names
                            .insert(constraint_type_id, name);
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
                self.structs.insert(
                    id,
                    Struct {
                        id,
                        name,
                        generic_parameter_constraint_ids,
                        fields,
                    },
                );
                Some(Expr::Struct(id))
            }
            Node::StructInitializer(name, generic_arguments, fields) => {
                let generic_argument_ids = generic_arguments
                    .as_ref()
                    .map(|x| {
                        x.0.iter()
                            .map(|y| self.walk_type_node(y, scope_id))
                            .collect()
                    })
                    .unwrap_or_else(Vec::new);
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
                            x.0.1.as_ref().map(|y| y.1).unwrap_or(x.1),
                        )
                    })
                    .collect::<Vec<_>>();
                self.struct_initializer_constraints
                    .push(StructInitializerConstraint::from_walk(
                        id,
                        name,
                        generic_argument_ids,
                        e_fields,
                        fields.1,
                    ));
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
                        self.generic_constraint_names
                            .insert(constraint_type_id, name);
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

    fn infer_type(
        &mut self,
        expr_id: Id,
        constraint: &Type,
        substitution_context: &SubstitutionContext,
    ) -> Type {
        self.infer_type_inner(
            expr_id,
            constraint,
            substitution_context,
            &mut HashSet::new(),
        )
    }

    fn infer_type_inner(
        &mut self,
        expr_id: Id,
        constraint: &Type,
        substitution_context: &SubstitutionContext,
        exprs_seen: &mut HashSet<Id>,
    ) -> Type {
        if exprs_seen.contains(&expr_id) {
            return Type::Unresolved;
        }
        exprs_seen.insert(expr_id);

        if let Some(type_id) = self.expr_id_to_type_id_map.get(&expr_id) {
            return type_id.get_type(self);
        }

        let constraint = match constraint {
            Type::Generic(type_id) => substitution_context
                .get(type_id)
                .map(|x| x.get_type(self))
                .unwrap_or_else(|| constraint.clone()),
            x => x.clone(),
        };

        let expr = self.get_entity_by_id(expr_id);

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
                let constraint_items = match constraint {
                    Type::Tuple(items) => items.clone(),
                    _ => Vec::new(),
                };
                let mut items = Vec::with_capacity(item_ids.len());
                for (i, id) in item_ids.clone().iter().enumerate() {
                    let constraint_item = constraint_items
                        .get(i)
                        .map(|x| x.get_type(self))
                        .unwrap_or(Type::Unknown);
                    let inferred = self.infer_type_inner(
                        *id,
                        &constraint_item,
                        substitution_context,
                        exprs_seen,
                    );
                    if matches!(inferred, Type::Unresolved) {
                        return Type::Unresolved;
                    }
                    items.push(inferred.get_type_id(self));
                }
                Type::Tuple(items)
            }
            Expr::Local(subject_id) => {
                let subject = self.infer_type_inner(
                    *subject_id,
                    &constraint,
                    substitution_context,
                    exprs_seen,
                );
                match subject {
                    Type::Unresolved => Type::Unresolved,
                    _ => subject,
                }
            }
            Expr::Function(function_id) => Type::Function(*function_id),
            Expr::Struct(struct_id) => Type::Struct(*struct_id),
            Expr::Module(module_id) => Type::Module(*module_id),
            Expr::Call(id) => {
                let function_call = self.function_calls.get(id).unwrap();
                let subject_type = self.infer_type_inner(
                    function_call.subject_id,
                    &Type::Unknown,
                    substitution_context,
                    exprs_seen,
                );
                match subject_type {
                    Type::Unresolved => Type::Unresolved,
                    Type::Function(_) => Type::Void,
                    x => panic!("type is not callable: {:?}", x),
                }
            }
            Expr::Variable(variable_id) => {
                let variable = self.variables.get(variable_id).unwrap();
                let variable_type = variable.type_id.get_type(self);
                if let Type::Unknown = variable_type {
                    Type::Unresolved
                } else {
                    variable_type
                }
            }
            Expr::Parameter(parameter_id) => {
                let parameter = self.parameters.get(parameter_id).unwrap();
                parameter.type_id.get_type(self)
            }
            Expr::StructInitializer(struct_id, _initializer_fields) => Type::Struct(*struct_id),
            Expr::Generic(type_id) => type_id.get_type(self),
            Expr::Binary(_, lhs_id, _rhs_id) => {
                let lhs =
                    self.infer_type_inner(*lhs_id, &constraint, substitution_context, exprs_seen);
                match lhs {
                    Type::Unresolved => Type::Unresolved,
                    _ => lhs,
                }
            }
            _ => Type::Void,
        };

        inferred_type
    }

    fn reconcile_type(
        &mut self,
        a: &Type,
        b: &Type,
        substitution_context: &SubstitutionContext,
    ) -> Option<(Type, Vec<(TypeId, TypeId)>)> {
        Some(match (a, b) {
            (Type::Any, _) | (_, Type::Unknown) => (a.clone(), Vec::new()),
            (_, Type::Any) | (Type::Unknown, _) => (b.clone(), Vec::new()),
            (Type::Unresolved, _) | (_, Type::Unresolved) => {
                return None;
            }
            (Type::Generic(constraint_id), _) => match substitution_context.get(constraint_id) {
                Some(resolved_id) => {
                    let resolved = resolved_id.get_type(self);
                    let (unified, mut bindings) =
                        self.reconcile_type(&resolved, b, substitution_context)?;
                    bindings.push((*constraint_id, b.clone().get_type_id(self)));
                    (unified, bindings)
                }
                None => {
                    let bindings = vec![(*constraint_id, b.clone().get_type_id(self))];
                    (a.clone(), bindings)
                }
            },
            (_, Type::Generic(constraint_id)) => match substitution_context.get(constraint_id) {
                Some(resolved_id) => {
                    let resolved = resolved_id.get_type(self);
                    let (unified, mut bindings) =
                        self.reconcile_type(a, &resolved, substitution_context)?;
                    bindings.push((*constraint_id, a.clone().get_type_id(self)));
                    (unified, bindings)
                }
                None => {
                    let bindings = vec![(*constraint_id, a.clone().get_type_id(self))];
                    (b.clone(), bindings)
                }
            },
            (Type::Primitive(l), Type::Primitive(r)) => match (l, r) {
                (PrimitiveType::List(l_id), PrimitiveType::List(r_id)) => {
                    let l = l_id.get_type(self);
                    let r = r_id.get_type(self);
                    let (item_type, bindings) =
                        self.reconcile_type(&l, &r, substitution_context)?;
                    let item_type_id = item_type.get_type_id(self);
                    (Type::Primitive(PrimitiveType::List(item_type_id)), bindings)
                }
                (l, r) if l == r => (a.clone(), Vec::new()),
                _ => {
                    return None;
                }
            },
            (Type::Tuple(l_items), Type::Tuple(r_items)) => {
                let mut result_items = Vec::with_capacity(l_items.len());
                let mut all_bindings = Vec::new();
                for (l_item_id, r_item_id) in l_items.iter().zip(r_items.iter()) {
                    let l = l_item_id.get_type(self);
                    let r = r_item_id.get_type(self);
                    let (item, bindings) = self.reconcile_type(&l, &r, substitution_context)?;
                    all_bindings.extend(bindings);
                    result_items.push(item.get_type_id(self));
                }
                (Type::Tuple(result_items), all_bindings)
            }
            (l, r) if l == r => (a.clone(), Vec::new()),
            _ => {
                return None;
            }
        })
    }

    fn compare_type(&self, a: &Type, b: &Type, substitution_context: &SubstitutionContext) -> bool {
        match (a, b) {
            (Type::Unknown, _) | (_, Type::Unknown) | (Type::Any, _) | (_, Type::Any) => true,
            (Type::Unresolved, _) | (_, Type::Unresolved) => false,
            (Type::Generic(constraint_id), _) => {
                let l = substitution_context
                    .get(constraint_id)
                    .map(|x| x.get_type(self))
                    .unwrap_or_else(|| constraint_id.get_type(self));
                return self.compare_type(&l, b, substitution_context);
            }
            (_, Type::Generic(constraint_id)) => {
                let r = substitution_context
                    .get(constraint_id)
                    .map(|x| x.get_type(self))
                    .unwrap_or_else(|| constraint_id.get_type(self));
                return self.compare_type(a, &r, substitution_context);
            }
            (Type::Primitive(l), Type::Primitive(r)) => match (l, r) {
                (PrimitiveType::List(l_id), PrimitiveType::List(r_id)) => {
                    let l = l_id.get_type(self);
                    let r = r_id.get_type(self);
                    self.compare_type(&l, &r, substitution_context)
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
                        self.compare_type(&l, &r, substitution_context)
                    })
            }
            (a, b) if a == b => true,
            _ => false,
        }
    }

    /// Get the resolved type for an expression, checking both maps.
    fn expr_type_for_id(&self, expr_id: Id) -> Type {
        self.expr_id_to_type_id_map
            .get(&expr_id)
            .map(|tid| tid.get_type(self))
            .or_else(|| {
                self.resolved_types
                    .get(&expr_id)
                    .map(|tid| tid.get_type(self))
            })
            .unwrap_or(Type::Unknown)
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
            let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
            self.type_id_to_type_map.insert(type_id, subject_type);
        }

        for (type_id, subject_type_id, member_name) in self.prepped_type_static_accessors.clone() {
            match subject_type_id.get_type(self) {
                Type::Module(module_id) => {
                    let module = self.modules.get(&module_id).unwrap();
                    let member_id = self.get_expr_id_by_name(member_name, module.body.1);
                    let member_type = self.infer_type(member_id, &Type::Unknown, &HashMap::new());
                    self.type_id_to_type_map.insert(type_id, member_type);
                }
                _ => {}
            }
        }

        for (id, subject_id, member_name) in self.prepped_field_accessors.clone() {
            let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
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

        // Method call prepass: resolve method subjects (struct method lookups)
        for (id, subject_id, member_name, generic_argument_ids, mut argument_ids, arguments_span) in
            self.prepped_method_calls.clone()
        {
            let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
            match subject_type {
                Type::Struct(struct_id) => {
                    let struct_name = self.structs.get(&struct_id).unwrap().name;
                    let member_id = self
                        .implementations
                        .iter()
                        .filter(|x| {
                            self.compare_type(
                                &subject_type,
                                &x.subject.get_type(self),
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
            match subject_type {
                Type::Struct(struct_id) => {
                    let struct_name = self.structs.get(&struct_id).unwrap().name;
                    let member_id = *self
                        .implementations
                        .iter()
                        .filter(|x| {
                            self.compare_type(
                                &subject_type,
                                &x.subject.get_type(self),
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

        // --- Constraint solving loop ---
        // Each iteration resolves at least one new type (unknown -> concrete).
        // The maximum number of iterations equals the number of expressions
        // with unknown types, since each resolved type never reverts to unknown.
        let max_iterations = self.struct_initializer_constraints.len()
            + self.variable_constraints.len()
            + self.call_subject_constraints.len()
            + self.field_accessor_constraints.len();

        for _ in 0..max_iterations {
            let mut progress = false;

            // --- Resolve struct initializer constraints ---
            let mut unresolved_constraints = Vec::new();
            for mut constraint in std::mem::take(&mut self.struct_initializer_constraints) {
                let scope_id = self.get_scope_id_for_entity(constraint.struct_id);
                let struct_expr_id = self.get_expr_id_by_name(constraint.struct_name, scope_id);
                constraint.struct_id = struct_expr_id;
                let struct_ = match self.structs.get(&constraint.struct_id) {
                    Some(s) => s,
                    None => {
                        self.diagnostics.push(Error {
                            span: constraint.fields_span.clone(),
                            msg: format!(
                                "cannot initialize a non-struct: {}",
                                constraint.struct_name
                            ),
                        });
                        continue;
                    }
                };
                let generic_param_ids = struct_.generic_parameter_constraint_ids.clone();
                let struct_fields = struct_.fields.clone();
                if constraint.fields.len() != struct_fields.len() {
                    self.diagnostics.push(Error {
                        span: constraint.fields_span.clone(),
                        msg: format!(
                            "Expected {} {}, but got {} instead.",
                            struct_fields.len(),
                            plural(struct_fields.len(), "field", "fields"),
                            constraint.fields.len()
                        ),
                    });
                    continue;
                }
                let mut initializer_fields = IndexMap::new();
                let mut substitution_context = HashMap::new();
                for (i, generic_argument_id) in constraint.generic_argument_ids.iter().enumerate() {
                    let gc = generic_param_ids.get(i);
                    if let Some(gc) = gc {
                        substitution_context.insert(*gc, *generic_argument_id);
                    }
                }
                let mut all_fields_ok = true;
                let mut defer = None;
                let fields = constraint.fields.clone();
                let struct_id = constraint.struct_id;
                for (field_name, field_value, _field_value_span) in &fields {
                    let (struct_field_index, struct_field) = struct_fields
                        .iter()
                        .enumerate()
                        .find(|(_, x)| *x.name == **field_name)
                        .expect(format!("unknown field: {}", field_name).as_str());
                    if !self.expr_id_to_type_id_map.contains_key(field_value)
                        && !self.resolved_types.contains_key(field_value)
                    {
                        defer = Some(constraint);
                        break;
                    }
                    let value_type =
                        self.infer_type(*field_value, &Type::Unknown, &substitution_context);
                    let struct_field_type = struct_field.type_id.get_type(self);
                    if let Type::Unresolved = value_type {
                        defer = Some(constraint);
                        break;
                    }
                    if let Some((_, bindings)) =
                        self.reconcile_type(&value_type, &struct_field_type, &substitution_context)
                    {
                        for (cid, tid) in bindings {
                            substitution_context.insert(cid, tid);
                        }
                    } else {
                        all_fields_ok = false;
                    }
                    initializer_fields.insert(struct_field_index, *field_value);
                }
                if let Some(deferred) = defer {
                    all_fields_ok = false;
                    unresolved_constraints.push(deferred);
                }
                if all_fields_ok {
                    self.expr_id_to_expr_map.insert(
                        struct_id,
                        Expr::StructInitializer(struct_id, initializer_fields),
                    );
                    for (_, v, _) in &fields {
                        if let Some(t) = self.expr_id_to_type_id_map.get(v) {
                            self.resolved_types.insert(struct_id, *t);
                        }
                    }
                    progress = true;
                }
            }
            self.struct_initializer_constraints = unresolved_constraints;
            if !self.struct_initializer_constraints.is_empty() {
                progress = true;
            }

            // --- Resolve field accessor constraints ---
            let mut remaining_accessors = IndexMap::new();
            let accessor_constraints: Vec<_> = std::mem::take(&mut self.field_accessor_constraints)
                .into_iter()
                .collect();
            for (id, constraint) in accessor_constraints {
                let FieldAccessorConstraint {
                    subject_id,
                    member_name,
                    ..
                } = &constraint;

                if !self.expr_id_to_type_id_map.contains_key(subject_id)
                    && !self.resolved_types.contains_key(subject_id)
                {
                    remaining_accessors.insert(id, constraint);
                    continue;
                }

                let subject_type = self.expr_type_for_id(*subject_id);
                match subject_type {
                    Type::Struct(struct_id) => {
                        let struct_ = match self.structs.get(&struct_id) {
                            Some(s) => s,
                            None => {
                                self.diagnostics.push(Error {
                                    span: **self.span_map.get(&id).unwrap(),
                                    msg: format!("subject is not a struct: {}", struct_id.0),
                                });
                                continue;
                            }
                        };
                        let field_index = struct_
                            .fields
                            .iter()
                            .enumerate()
                            .find_map(|(i, x)| (x.name == *member_name).then_some(i))
                            .unwrap();
                        self.expr_id_to_expr_map
                            .insert(id, Expr::Field(*subject_id, struct_id, field_index));
                        let field_type = struct_.fields[field_index].type_id;
                        self.expr_id_to_type_id_map.insert(id, field_type);
                        self.resolved_types.insert(id, field_type);
                    }
                    _ => {
                        self.diagnostics.push(Error {
                            span: **self.span_map.get(&id).unwrap(),
                            msg: format!(
                                "Unhandled subject type for member accessor. The type of `subject` in `subject.{}` was not a struct.",
                                member_name
                            ),
                        });
                    }
                }
                progress = true;
            }
            self.field_accessor_constraints = remaining_accessors;
            if !self.field_accessor_constraints.is_empty() {
                progress = true;
            }

            // --- Resolve variable constraints ---
            let mut remaining_vars = Vec::new();
            let var_constraints: Vec<_> = std::mem::take(&mut self.variable_constraints)
                .into_iter()
                .collect();
            for constraint in var_constraints {
                let VariableConstraint {
                    variable_id,
                    initial_type_id,
                    ref value_ids,
                } = constraint;

                let mut all_resolved = true;
                for &value_id in value_ids.iter() {
                    if !self.expr_id_to_type_id_map.contains_key(&value_id)
                        && !self.resolved_types.contains_key(&value_id)
                    {
                        all_resolved = false;
                        break;
                    }
                }
                if !all_resolved {
                    remaining_vars.push(constraint);
                    continue;
                }

                let mut substitution_context = HashMap::new();
                let mut variable_type = initial_type_id.get_type(self);

                for &value_id in value_ids {
                    let value_type =
                        self.infer_type(value_id, &variable_type, &substitution_context);
                    match self.reconcile_type(&value_type, &variable_type, &substitution_context) {
                        Some((unified, bindings)) => {
                            for (cid, tid) in bindings {
                                substitution_context.insert(cid, tid);
                            }
                            if let Type::Unknown = variable_type {
                                variable_type = unified;
                            }
                        }
                        None => {
                            let expected_str =
                                self.pretty_print_type(&variable_type, &substitution_context);
                            let got_str =
                                self.pretty_print_type(&value_type, &substitution_context);
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&value_id).unwrap(),
                                msg: format!(
                                    "Expected {}, but got {} instead.",
                                    expected_str, got_str
                                ),
                            });
                        }
                    }
                }

                let var_type_id = variable_type.get_type_id(self);
                self.variables.get_mut(&variable_id).unwrap().type_id = var_type_id;
                self.resolved_types.insert(variable_id, var_type_id);
                progress = true;
            }
            self.variable_constraints = remaining_vars;
            if !self.variable_constraints.is_empty() {
                progress = true;
            }

            // --- Resolve call subject constraints ---
            let mut remaining_calls = Vec::new();
            let call_constraints: Vec<_> = std::mem::take(&mut self.call_subject_constraints)
                .into_iter()
                .collect();
            for constraint in call_constraints {
                let CallSubjectConstraint {
                    call_id,
                    subject_id,
                    ref generic_argument_ids,
                    ref argument_ids,
                    arguments_span,
                } = constraint;

                if !self.expr_id_to_type_id_map.contains_key(&subject_id)
                    && !self.resolved_types.contains_key(&subject_id)
                {
                    remaining_calls.push(constraint);
                    continue;
                }

                let subject_expr = self.get_entity_by_id(subject_id);
                match subject_expr {
                    Expr::Local(target_id) => {
                        let target = self.get_entity_by_id(*target_id);
                        let function_data = match target {
                            Expr::Function(function_id) => {
                                let function = self.functions.get(function_id).unwrap();
                                Some((
                                    function.parameters.clone(),
                                    function.generic_parameter_constraint_ids.clone(),
                                ))
                            }
                            Expr::ExternalFunction(external_function_id) => {
                                let function =
                                    self.external_functions.get(external_function_id).unwrap();
                                Some((
                                    function.parameters.clone(),
                                    function.generic_parameter_constraint_ids.clone(),
                                ))
                            }
                            _ => None,
                        };

                        if let Some((parameters, generic_parameter_constraint_ids)) = function_data
                        {
                            if argument_ids.len() != parameters.len() {
                                self.diagnostics.push(Error {
                                    span: arguments_span,
                                    msg: format!(
                                        "Expected {} {}, but got {} instead.",
                                        parameters.len(),
                                        plural(parameters.len(), "argument", "arguments"),
                                        argument_ids.len()
                                    ),
                                });
                            } else {
                                let mut substitution_context = HashMap::new();
                                for (i, generic_argument_id) in
                                    generic_argument_ids.iter().enumerate()
                                {
                                    if let Some(gpc) = generic_parameter_constraint_ids.get(i) {
                                        substitution_context.insert(*gpc, *generic_argument_id);
                                    }
                                }

                                let mut all_args_ok = true;
                                for (i, parameter_id) in parameters.iter().enumerate() {
                                    let parameter = self.parameters.get(parameter_id).unwrap();
                                    let parameter_type = parameter.type_id.get_type(self);
                                    let argument_id = *argument_ids.get(i).unwrap();
                                    let argument_type = self.infer_type(
                                        argument_id,
                                        &parameter_type,
                                        &substitution_context,
                                    );
                                    if matches!(argument_type, Type::Unresolved) {
                                        remaining_calls.push(CallSubjectConstraint {
                                            call_id,
                                            subject_id,
                                            generic_argument_ids: generic_argument_ids.clone(),
                                            argument_ids: argument_ids.clone(),
                                            arguments_span,
                                        });
                                        all_args_ok = false;
                                        break;
                                    }
                                    match self.reconcile_type(
                                        &argument_type,
                                        &parameter_type,
                                        &substitution_context,
                                    ) {
                                        Some((_unified, bindings)) => {
                                            for (cid, tid) in bindings {
                                                substitution_context.insert(cid, tid);
                                            }
                                        }
                                        None => {
                                            let expected = self.pretty_print_type(
                                                &parameter_type,
                                                &substitution_context,
                                            );
                                            let got = self.pretty_print_type(
                                                &argument_type,
                                                &substitution_context,
                                            );
                                            self.diagnostics.push(Error {
                                                span: **self.span_map.get(&argument_id).unwrap(),
                                                msg: format!(
                                                    "Expected {}, but got {} instead.",
                                                    expected, got
                                                ),
                                            });
                                        }
                                    }
                                }

                                if all_args_ok {
                                    self.expr_id_to_expr_map
                                        .insert(call_id, Expr::Call(call_id));
                                }
                            }
                        }
                    }
                    _ => {
                        // Direct function reference
                        match subject_expr {
                            Expr::Function(_) | Expr::ExternalFunction(_) => {
                                self.expr_id_to_expr_map
                                    .insert(call_id, Expr::Call(call_id));
                                progress = true;
                            }
                            _ => {
                                self.diagnostics.push(Error {
                                    span: arguments_span,
                                    msg: "cannot call a non-function value".to_string(),
                                });
                            }
                        }
                    }
                }

                if !remaining_calls.is_empty() {
                    progress = true;
                }
            }
            self.call_subject_constraints = remaining_calls;
            if !self.call_subject_constraints.is_empty() {
                progress = true;
            }

            if !progress {
                break;
            }
        }

        // --- Post-solve diagnostics ---
        for constraint in &self.struct_initializer_constraints {
            self.diagnostics.push(Error {
                span: constraint.fields_span,
                msg: "type of struct initializer could not be resolved".to_string(),
            });
        }
        for constraint in self.field_accessor_constraints.values() {
            self.diagnostics.push(Error {
                span: **(self.span_map.get(&constraint.id).unwrap_or(&&EMPTY_SPAN)),
                msg: "type of accessor subject could not be resolved".to_string(),
            });
        }
        for constraint in &self.variable_constraints {
            self.diagnostics.push(Error {
                span: **(self
                    .span_map
                    .get(&constraint.variable_id)
                    .unwrap_or(&&EMPTY_SPAN)),
                msg: format!(
                    "type of variable '{}' could not be resolved",
                    self.variables
                        .get(&constraint.variable_id)
                        .map(|v| v.name)
                        .unwrap_or("unknown")
                ),
            });
        }
        for constraint in &self.call_subject_constraints {
            self.diagnostics.push(Error {
                span: constraint.arguments_span,
                msg: "type of function call arguments could not be resolved".to_string(),
            });
        }

        // Clear processed constraints
        self.struct_initializer_constraints.clear();
        self.field_accessor_constraints.clear();
        self.variable_constraints.clear();
        self.call_subject_constraints.clear();
    }

    /// Pretty-prints a type for diagnostics, resolving generic names
    /// with their substitution context when available.
    fn pretty_print_type(&self, type_: &Type, substitution: &SubstitutionContext) -> String {
        let mut buf = String::new();
        self.pretty_print_type_inner(type_, substitution, &mut buf, 0);
        buf
    }

    fn pretty_print_type_inner(
        &self,
        type_: &Type,
        substitution: &SubstitutionContext,
        buf: &mut String,
        _depth: usize,
    ) {
        match type_ {
            Type::Any => buf.push_str("type any"),
            Type::Unknown => buf.push_str("type unknown"),
            Type::Unresolved => buf.push_str("type unresolved"),
            Type::Void => buf.push_str("type void"),

            Type::Generic(constraint_id) => {
                let constraint = substitution
                    .get(constraint_id)
                    .map(|x| x.get_type(self))
                    .unwrap_or_else(|| constraint_id.get_type(self));
                let generic_name = self
                    .generic_constraint_names
                    .get(constraint_id)
                    .expect("failed to find generic name");
                let concrete_str = self.pretty_print_type(&constraint, substitution);
                buf.push_str(&format!("generic {} of {}", generic_name, concrete_str));
            }

            Type::Function(id) => {
                let func = self.functions.get(id).unwrap();
                buf.push_str(&format!("fn {}(", func.name));
                let mut first = true;
                for parameter_id in &func.parameters {
                    let parameter = self.parameters.get(parameter_id).unwrap();
                    if !first {
                        buf.push_str(", ");
                    }
                    let parameter_type = parameter.type_id.get_type(self);
                    let parameter_type_str = self.pretty_print_type(&parameter_type, substitution);
                    buf.push_str(&parameter_type_str);
                    first = false;
                }
                buf.push(')');
            }

            Type::Struct(id) => {
                let struct_ = self.structs.get(id).unwrap();
                buf.push_str(&format!("struct {}", struct_.name));
            }

            Type::Module(id) => {
                let module = self.modules.get(id).unwrap();
                buf.push_str(&format!("module {}", module.name));
            }

            Type::Closure(parameters, return_id) => {
                buf.push_str("|");
                for (i, parameter_id) in parameters.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    let parameter_type = parameter_id.get_type(self);
                    buf.push_str(&self.pretty_print_type(&parameter_type, substitution));
                }
                buf.push_str("| ");
                let return_type = return_id.get_type(self);
                buf.push_str(&self.pretty_print_type(&return_type, substitution));
            }

            Type::Primitive(prim) => match prim {
                PrimitiveType::List(item_id) => {
                    buf.push_str("type List<");
                    let item_type = item_id.get_type(self);
                    let item_str = self.pretty_print_type(&item_type, substitution);
                    buf.push_str(&item_str);
                    buf.push('>');
                }
                PrimitiveType::I32 => buf.push_str("type i32"),
                PrimitiveType::U32 => buf.push_str("type u32"),
                PrimitiveType::F64 => buf.push_str("type f64"),
                PrimitiveType::String => buf.push_str("type str"),
                PrimitiveType::Bool => buf.push_str("type bool"),
                PrimitiveType::Null => buf.push_str("type null"),
            },

            Type::Tuple(items) => {
                buf.push_str("type (");
                for (i, item_id) in items.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    let item_type = item_id.get_type(self);
                    let item_str = self.pretty_print_type(&item_type, substitution);
                    buf.push_str(&item_str);
                }
                buf.push(')');
            }
        }
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
    pub type_id_to_type_map: HashMap<TypeId, Type>,
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
        type_id_to_type_map: analyzer.type_id_to_type_map,
        variables: analyzer.variables,
    }
}
