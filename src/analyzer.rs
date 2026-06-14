use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::error::Error;
use crate::id::Id;
use crate::node::{
    BinaryOp, GenericParameters, ImportBranch, Node, NodeIfBranch, NodeList, Pattern,
};
use crate::span::{Span, Spanned};
use crate::type_::{PrimitiveType, SubstitutionContext, Type, TypeId};
use crate::util::plural;

#[derive(Clone, Debug)]
pub enum Expr<'src> {
    // An assignment to a local: target accessor and the (possibly desugared,
    // e.g. `x + v` for `x += v`) value expression.
    Assignment(Id, Id),
    Binary(BinaryOp, Id, Id),
    Block((Vec<Id>, Id)),
    Bool(bool),
    Call(Id),
    Closure(Id),
    // An enum declaration.
    Enum(Id),
    // A reference to one variant of an enum: the enum and the variant index.
    EnumVariant(Id, usize),
    Error,
    ExternalFunction(Id),
    Field(Id, Id, usize),
    // A loop: the optional condition and the body (statements, trailing expr).
    For(Option<Id>, (Vec<Id>, Id)),
    Function(Id),
    FunctionReturn(Id),
    Generic(TypeId),
    If(ExprIfBranch),
    Impl(Id),
    // A `jump break` / `jump continue` — the target keyword.
    Jump(&'src str),
    // `subject is pattern` — a boolean pattern test. Captures bind into the
    // surrounding scope.
    Is(Id, ExprPattern),
    List(Vec<Id>),
    Local(Id),
    // A match expression: the subject and the resolved legs.
    Match(Id, Vec<ExprMatchLeg>),
    Module(Id),
    Null,
    Number(&'src str, Option<&'src str>, Option<&'src str>),
    Parameter(Id),
    String(&'src str),
    Struct(Id),
    StructInitializer(Id, IndexMap<usize, Id>),
    Trait(Id),
    Tuple(Vec<Id>),
    Variable(Id),
    Void,
}

#[derive(Clone, Debug)]
pub enum ExprIfBranch {
    If(Id, (Vec<Id>, Id), Option<Box<ExprIfBranch>>),
    Else((Vec<Id>, Id)),
}

#[derive(Clone, Debug)]
pub struct ExprMatchLeg {
    pub pattern: ExprPattern,
    pub body: Id,
}

// A fully resolved match pattern, ready for code generation.
#[derive(Clone, Debug)]
pub enum ExprPattern {
    // `_` — matches anything without binding it.
    Wildcard,
    // A capture binding the matched value to a variable entity.
    Binding(Id),
    // A variant test: the owning enum's id, the variant index, and payload
    // sub-patterns. The enum id lets the transformer lower `bool` patterns to a
    // native boolean comparison rather than the array discriminant form.
    Variant(Id, usize, Vec<ExprPattern>),
    // A tuple destructure, matching each element positionally.
    Tuple(Vec<ExprPattern>),
}

#[derive(Debug)]
pub struct Function<'src> {
    pub id: Id,
    pub name: &'src str,
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub parameters: Vec<Id>,
    /// The declared return type, if annotated. Used in preference to inferring
    /// the body's type, which matters for generic returns like `(): T`.
    pub return_type_id: Option<TypeId>,
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
    pub mutable: bool,
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
pub struct Enum<'src> {
    pub id: Id,
    pub name: &'src str,
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub variants: Vec<EnumVariantDeclaration<'src>>,
    // The namespace scope holding the variant entities by name, reachable
    // through `use Enum::{ ... }` or `Enum::Variant`.
    pub variants_scope_id: Id,
}

#[derive(Debug)]
pub struct EnumVariantDeclaration<'src> {
    pub name: &'src str,
    pub data_type_ids: Vec<TypeId>,
}

// A match pattern as walked, with variant names not yet resolved.
#[derive(Debug)]
enum WalkPattern<'src> {
    Wildcard,
    Binding(Id),
    Variant(&'src str, Span, Option<Vec<WalkPattern<'src>>>),
    Tuple(Span, Vec<WalkPattern<'src>>),
}

// A match expression awaiting subject and pattern resolution.
#[derive(Debug)]
struct PreppedMatch<'src> {
    id: Id,
    subject_id: Id,
    scope_id: Id,
    legs: Vec<(WalkPattern<'src>, Id)>,
    span: Span,
}

// An `is` pattern test awaiting subject and pattern resolution. Its captures are
// already walked into `scope_id`.
#[derive(Debug)]
struct PreppedIs<'src> {
    id: Id,
    subject_id: Id,
    scope_id: Id,
    pattern: WalkPattern<'src>,
}

#[derive(Debug)]
pub struct Implementation<'src> {
    pub subject: TypeId,
    pub declarations: IndexMap<&'src str, Id>,
}

#[derive(Debug)]
pub struct Trait<'src> {
    pub id: Id,
    pub name: &'src str,
    /// The members the trait declares, keyed by name. For a required method
    /// without a default body these point at signature-only functions.
    pub declarations: IndexMap<&'src str, Id>,
}

/// A pending `impl Subject with Trait` conformance check: the subject type
/// must provide every member the named trait requires.
#[derive(Debug)]
pub struct TraitImplCheck<'src> {
    pub subject_type_id: TypeId,
    pub trait_name: &'src str,
    pub scope_id: Id,
    pub declarations: IndexMap<&'src str, Id>,
    pub span: Span,
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
    pub initializer_id: Id,
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
        initializer_id: Id,
        name: &'src str,
        generic_argument_ids: Vec<TypeId>,
        e_fields: Vec<(&'src str, Id, Span)>,
        fields_span: Span,
    ) -> Self {
        Self {
            initializer_id,
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
    enums: IndexMap<Id, Enum<'src>>,
    expr_id_to_expr_map: HashMap<Id, Expr<'src>>,
    expr_id_to_scope_id_map: HashMap<Id, Id>,
    expr_id_to_type_id_map: HashMap<Id, TypeId>,
    external_functions: IndexMap<Id, ExternalFunction<'src>>,
    field_accessor_constraints: IndexMap<Id, FieldAccessorConstraint<'src>>,
    function_calls: IndexMap<Id, FunctionCall>,
    functions: IndexMap<Id, Function<'src>>,
    generic_constraint_names: HashMap<TypeId, &'src str>,
    // Static accessors whose subject is a generic parameter (e.g. `T::default`),
    // recorded as `accessor_id -> (constraint_id, member_name)` so the
    // transformer can re-resolve them per monomorphized instantiation.
    generic_static_accessors: HashMap<Id, (TypeId, &'src str)>,
    implementations: Vec<Implementation<'src>>,
    module_id_by_name: HashMap<&'src str, Id>,
    modules: IndexMap<Id, Module<'src>>,
    parameters: IndexMap<Id, Parameter<'src>>,
    // The built-in `std` structs that back scalar primitives, keyed by name
    // (`i32`, `str`, ...). Used to type literals and resolve primitive names.
    primitive_struct_ids: HashMap<&'static str, Id>,
    // The source-defined `enum bool` (from `std/boolean.vl`), captured after the
    // module loads. `bool` literals, comparisons, and `is` tests all type as
    // this enum; the transformer lowers it to a native JS boolean.
    bool_enum_id: Option<Id>,
    // Assignments awaiting local resolution: (target accessor id, value id).
    prepped_assignments: Vec<(Id, Id)>,
    prepped_field_accessors: Vec<(Id, Id, &'src str)>,
    prepped_imports: Vec<(Vec<&'src str>, &'src str, Id, Span)>,
    prepped_locals: Vec<(Id, &'src str)>,
    prepped_is: Vec<PreppedIs<'src>>,
    prepped_matches: Vec<PreppedMatch<'src>>,
    prepped_method_calls: Vec<(Id, Id, &'src str, Vec<TypeId>, Vec<Id>, Span)>,
    prepped_static_accessors: Vec<(Id, TypeId, &'src str)>,
    prepped_struct_initializers:
        Vec<(Id, &'src str, Vec<TypeId>, Vec<(&'src str, Id, Span)>, Span)>,
    prepped_trait_impls: Vec<TraitImplCheck<'src>>,
    prepped_type_locals: Vec<(TypeId, &'src str, Id, Span)>,
    prepped_uses: Vec<(Vec<&'src str>, &'src str, Id, Span)>,
    prepped_type_static_accessors: Vec<(TypeId, TypeId, &'src str, Span)>,
    reference_count: HashMap<Id, u32>,
    resolved_types: HashMap<Id, TypeId>,
    scope_id: u32,
    scopes: IndexMap<Id, Scope<'src>>,
    span_map: HashMap<Id, &'src Span>,
    struct_initializer_constraints: Vec<StructInitializerConstraint<'src>>,
    struct_initializer_to_def: HashMap<Id, Id>, // initializer_id -> struct definition id
    structs: IndexMap<Id, Struct<'src>>,
    traits: IndexMap<Id, Trait<'src>>,
    type_id_to_type_map: HashMap<TypeId, Type>,
    type_id: u32,
    variable_constraints: Vec<VariableConstraint>,
    variables: IndexMap<Id, Variable<'src>>,
    // True while walking a trait body, where a bodyless method is a legitimate
    // requirement; elsewhere a bodyless function must be declared `external`.
    walking_trait_body: bool,
    // The `std` `panic` intrinsic, if loaded. A call to it never returns, so it
    // types as `any` (unifying with any expected type) and lowers to a `throw`.
    panic_fn_id: Option<Id>,
}

static EMPTY_SPAN: Span = Span {
    start: 0,
    end: 0,
    context: (),
};

// Flattens an `import`/`use` tree into (path, leaf-name) pairs, e.g.
// `a::{ b, c::d }` becomes `([a], b)` and `([a, c], d)`.
fn flatten_namespace_branch<'src>(
    branch: &ImportBranch<'src>,
    path: Vec<&'src str>,
    entries: &mut Vec<(Vec<&'src str>, &'src str)>,
) {
    match branch {
        ImportBranch::Path(name, child_branch) => match child_branch {
            None => entries.push((path, name)),
            Some(child) => {
                let mut path = path;
                path.push(name);
                flatten_namespace_branch(child, path, entries);
            }
        },
        ImportBranch::Set(branches) => {
            for child in branches {
                flatten_namespace_branch(child, path.clone(), entries);
            }
        }
    }
}

impl<'src> Analyzer<'src> {
    fn new() -> Self {
        Self {
            assignment_values: IndexMap::new(),
            call_subject_constraints: Vec::new(),
            closures: IndexMap::new(),
            diagnostics: Vec::new(),
            entity_id: 0,
            enums: IndexMap::new(),
            expr_id_to_expr_map: HashMap::new(),
            expr_id_to_scope_id_map: HashMap::new(),
            expr_id_to_type_id_map: HashMap::new(),
            external_functions: IndexMap::new(),
            field_accessor_constraints: IndexMap::new(),
            function_calls: IndexMap::new(),
            functions: IndexMap::new(),
            generic_constraint_names: HashMap::new(),
            generic_static_accessors: HashMap::new(),
            implementations: Vec::new(),
            module_id_by_name: HashMap::new(),
            modules: IndexMap::new(),
            parameters: IndexMap::new(),
            primitive_struct_ids: HashMap::new(),
            bool_enum_id: None,
            prepped_assignments: Vec::new(),
            prepped_field_accessors: Vec::new(),
            prepped_imports: Vec::new(),
            prepped_locals: Vec::new(),
            prepped_is: Vec::new(),
            prepped_matches: Vec::new(),
            prepped_method_calls: Vec::new(),
            prepped_static_accessors: Vec::new(),
            prepped_struct_initializers: Vec::new(),
            prepped_trait_impls: Vec::new(),
            prepped_type_locals: Vec::new(),
            prepped_type_static_accessors: Vec::new(),
            prepped_uses: Vec::new(),
            reference_count: HashMap::new(),
            resolved_types: HashMap::new(),
            scope_id: 0,
            scopes: IndexMap::new(),
            span_map: HashMap::new(),
            struct_initializer_constraints: Vec::new(),
            struct_initializer_to_def: HashMap::new(),
            structs: IndexMap::new(),
            traits: IndexMap::new(),
            type_id_to_type_map: HashMap::new(),
            type_id: 0,
            variable_constraints: Vec::new(),
            variables: IndexMap::new(),
            walking_trait_body: false,
            panic_fn_id: None,
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

    /// The type of a built-in scalar primitive (`i32`, `str`, ...), which is
    /// the `std` struct that now backs it.
    fn primitive_struct_type(&self, name: &str) -> Type {
        Type::Struct(self.primitive_struct_ids[name])
    }

    /// The boolean type — the source-defined `enum bool`. Falls back to
    /// `Unresolved` if `boolean.vl` has not been captured yet, so the constraint
    /// solver defers rather than panicking.
    fn bool_type(&self) -> Type {
        match self.bool_enum_id {
            Some(id) => Type::Enum(id),
            None => Type::Unresolved,
        }
    }

    /// Registers a built-in scalar primitive as an empty `std` struct, recording
    /// its id so literals and primitive names can resolve to it. Like `print`,
    /// it is an externally-provided declaration with no user-written source.
    fn register_primitive_struct(&mut self, name: &'static str) -> Id {
        let id = self.new_entity_id();
        self.structs.insert(
            id,
            Struct {
                id,
                name,
                generic_parameter_constraint_ids: Vec::new(),
                fields: Vec::new(),
            },
        );
        self.expr_id_to_expr_map.insert(id, Expr::Struct(id));
        self.reference_count.entry(id).or_insert(0);
        self.primitive_struct_ids.insert(name, id);
        id
    }

    /// Resolves a name to its declaring entity by walking the scope chain,
    /// returning `None` if it is not in scope (callers turn that into a
    /// user-facing diagnostic). Resolved names are cached into the originating
    /// scope so repeated lookups stay cheap.
    fn try_get_expr_id_by_name(&mut self, name: &'src str, scope_id: Id) -> Option<Id> {
        let scope = self.mut_scope_for_scope_id(scope_id);
        let parent_id = scope.parent_id;
        scope.name_to_id_map.get(name).map(|x| *x).or_else(|| {
            let subject_id = parent_id
                .map(|parent_scope_id| self.try_get_expr_id_by_name(name, parent_scope_id))
                .flatten()?;
            let scope = self.mut_scope_for_scope_id(scope_id);
            scope.name_to_id_map.insert(name, subject_id);
            Some(subject_id)
        })
    }

    /// Walks the optional generic parameters of a declaration into `scope_id`,
    /// registering each as a `Generic` type bound by its constraint, and
    /// returns the constraint type ids in declaration order.
    fn register_generic_parameters(
        &mut self,
        generic_parameters: &Option<GenericParameters<'src>>,
        scope_id: Id,
    ) -> Vec<TypeId> {
        let mut generic_parameter_constraint_ids = Vec::new();
        if let Some(generic_parameters) = generic_parameters {
            for parameter in &generic_parameters.0 {
                // A parameter may carry several bounds (`T: A + B`); the first
                // is used as its constraint for now. Defaults (`B = Self`) are
                // accepted by the grammar but not yet applied.
                let constraint_type_id = parameter
                    .bounds
                    .first()
                    .map(|bound| self.walk_type_node(bound, scope_id))
                    .unwrap_or_else(|| Type::Any.get_type_id(self));
                self.register_generic_parameter(parameter.name, constraint_type_id, scope_id);
                generic_parameter_constraint_ids.push(constraint_type_id);
            }
        }
        generic_parameter_constraint_ids
    }

    /// Registers a single generic parameter named `name` (bound by the
    /// constraint type) into `scope_id`.
    fn register_generic_parameter(
        &mut self,
        name: &'src str,
        constraint_type_id: TypeId,
        scope_id: Id,
    ) {
        self.generic_constraint_names
            .insert(constraint_type_id, name);
        let type_id = Type::Generic(constraint_type_id).get_type_id(self);
        let expr_id = self.new_entity_id();
        self.expr_id_to_expr_map
            .insert(expr_id, Expr::Generic(constraint_type_id));
        self.expr_id_to_scope_id_map.insert(expr_id, scope_id);
        self.expr_id_to_type_id_map.insert(expr_id, type_id);
        let scope = self.mut_scope_for_scope_id(scope_id);
        scope.name_to_id_map.insert(name, expr_id);
    }

    /// Collects the named members declared in a trait/impl body scope,
    /// excluding the implicit `Self` binding and generic parameters.
    fn collect_declarations(&self, scope_id: Id) -> IndexMap<&'src str, Id> {
        self.scopes
            .get(&scope_id)
            .unwrap()
            .name_to_id_map
            .iter()
            .filter(|(name, expr_id)| {
                **name != "Self"
                    && !matches!(
                        self.expr_id_to_expr_map.get(*expr_id),
                        Some(Expr::Generic(_))
                    )
            })
            .map(|(name, expr_id)| (*name, *expr_id))
            .collect()
    }

    /// Registers the `Self` type within a trait/impl body scope. `self_type_id`
    /// is the concrete subject type for an `impl`, or an abstract placeholder
    /// (e.g. `any`) for a `trait`.
    fn register_self_type(&mut self, scope_id: Id, self_type_id: TypeId) {
        let self_id = self.new_entity_id();
        self.expr_id_to_type_id_map.insert(self_id, self_type_id);
        self.expr_id_to_scope_id_map.insert(self_id, scope_id);
        self.span_map.insert(self_id, &EMPTY_SPAN);
        let scope = self.mut_scope_for_scope_id(scope_id);
        scope.name_to_id_map.insert("Self", self_id);
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
            Node::Number(whole, fraction, suffix) => Some(Expr::Number(whole, *fraction, *suffix)),
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
                    Node::Number(name, _, _) => {
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
                let mut entries = Vec::new();
                flatten_namespace_branch(root_branch, Vec::new(), &mut entries);
                for (path, name) in entries {
                    self.prepped_imports.push((path, name, scope_id, node.1));
                }
                None
            }
            Node::Use(root_branch) => {
                let mut entries = Vec::new();
                flatten_namespace_branch(root_branch, Vec::new(), &mut entries);
                for (path, name) in entries {
                    self.prepped_uses.push((path, name, scope_id, node.1));
                }
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
            Node::For(condition, body) => {
                let body_scope_id = self.create_owned_scope(Some(scope_id)).id;
                let condition_id = condition
                    .as_ref()
                    .map(|condition| self.walk_expr_node(condition, body_scope_id));
                let ids = self.walk_expr_nodes(&body.0.0, body_scope_id);
                let expr_id = self.walk_expr_node(&body.0.1, body_scope_id);
                Some(Expr::For(condition_id, (ids, expr_id)))
            }
            // NOTE: `for x in iterable` parses and walks (binding `x`, walking
            // the iterable and body), but full `Iterable`-driven typing and
            // lowering is not implemented yet — it currently lowers like an
            // unconditioned loop. Not reached by the current reachable set.
            Node::ForIn(variable, iterable, body) => {
                let _iterable_id = self.walk_expr_node(iterable, scope_id);
                let body_scope_id = self.create_owned_scope(Some(scope_id)).id;
                if *variable != "_" {
                    let variable_id = self.new_entity_id();
                    let unknown_type_id = Type::Unknown.get_type_id(self);
                    self.variables.insert(
                        variable_id,
                        Variable {
                            id: variable_id,
                            name: variable,
                            initial: None,
                            type_id: unknown_type_id,
                            mutable: false,
                        },
                    );
                    self.expr_id_to_expr_map
                        .insert(variable_id, Expr::Variable(variable_id));
                    self.expr_id_to_scope_id_map
                        .insert(variable_id, body_scope_id);
                    self.span_map.insert(variable_id, &node.1);
                    self.reference_count.entry(variable_id).or_insert(0);
                    self.mut_scope_for_scope_id(body_scope_id)
                        .name_to_id_map
                        .insert(variable, variable_id);
                }
                let ids = self.walk_expr_nodes(&body.0.0, body_scope_id);
                let expr_id = self.walk_expr_node(&body.0.1, body_scope_id);
                Some(Expr::For(None, (ids, expr_id)))
            }
            // `subject is pattern` — walk the subject and the pattern (binding
            // captures into the current scope so a guarded `if`/`&&` can use
            // them), then resolve the pattern against the subject type in
            // `build`. The expression itself types as `bool`.
            Node::Is(subject, pattern) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                let walk_pattern = self.walk_pattern(pattern, scope_id);
                self.prepped_is.push(PreppedIs {
                    id,
                    subject_id,
                    scope_id,
                    pattern: walk_pattern,
                });
                None
            }
            // Re-export visibility is not tracked yet; walking the inner
            // statement is enough to bind it into the current scope.
            Node::Export(inner) => {
                self.walk_expr_node(inner, scope_id);
                None
            }
            // Only `!` exists today and yields `bool`; full lowering deferred.
            Node::Unary(_operator, operand) => {
                let _operand_id = self.walk_expr_node(operand, scope_id);
                Some(Expr::Bool(true))
            }
            Node::Jump(target) => Some(Expr::Jump(target)),
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
                            type_id: match &x.1 {
                                Some(type_node) => self.walk_type_node(type_node, body_scope.id),
                                // A bare `self` parameter takes the enclosing
                                // `Self` type (the impl/trait subject).
                                None if x.0 == "self" => self
                                    .try_get_expr_id_by_name("Self", scope_id)
                                    .and_then(|self_id| {
                                        self.expr_id_to_type_id_map.get(&self_id).copied()
                                    })
                                    .unwrap_or_else(|| Type::Unknown.get_type_id(self)),
                                None => Type::Unknown.get_type_id(self),
                            },
                        };
                        // `_` eats the argument: it stays positional but is
                        // never referenceable.
                        if parameter.name != "_" {
                            body_scope
                                .name_to_id_map
                                .insert(parameter.name, parameter_id);
                        }
                        self.parameters.insert(parameter_id, parameter);
                        self.expr_id_to_expr_map
                            .insert(parameter_id, Expr::Parameter(parameter_id));
                        parameter_id
                    })
                    .collect::<Vec<_>>();
                let body_scope_id = self.push_scope(body_scope);
                let generic_parameter_constraint_ids =
                    self.register_generic_parameters(&function.generic_parameters, body_scope_id);
                // The return type is resolved in the body scope so it can refer
                // to the function's own generic parameters (e.g. `(): T`).
                let return_type_id = function
                    .return_type
                    .as_ref()
                    .map(|return_type| self.walk_type_node(return_type, body_scope_id));
                if function.external {
                    // An `external` function is an intrinsic: no Vilan body, a
                    // declared (or void) return type, registered as an external
                    // function with a callable type so calls infer their return.
                    if function.body.is_some() {
                        self.diagnostics.push(Error {
                            span: function.name.1,
                            msg: "an `external` function cannot have a body".to_string(),
                        });
                    }
                    let return_type_id =
                        return_type_id.unwrap_or_else(|| Type::Void.get_type_id(self));
                    self.external_functions.insert(
                        id,
                        ExternalFunction {
                            id,
                            name,
                            generic_parameter_constraint_ids,
                            parameters,
                            return_type_id,
                            call_count: 0,
                        },
                    );
                    let function_type_id = self.new_type_id();
                    self.type_id_to_type_map
                        .insert(function_type_id, Type::Function(id));
                    self.expr_id_to_type_id_map.insert(id, function_type_id);
                    Some(Expr::ExternalFunction(id))
                } else {
                    let (ids, expr_id) = match &function.body {
                        Some(body) => {
                            let ids = self.walk_expr_nodes(&body.0.0, body_scope_id);
                            let expr_id = self.walk_expr_node(&body.0.1, body_scope_id);
                            (ids, expr_id)
                        }
                        None => {
                            // A signature without a body is only legitimate as a
                            // trait method requirement; anywhere else it must be
                            // declared `external`.
                            if !self.walking_trait_body {
                                self.diagnostics.push(Error {
                                    span: function.name.1,
                                    msg: format!(
                                        "function '{}' must have a body or be declared `external`",
                                        name
                                    ),
                                });
                            }
                            let void_id = self.new_entity_id();
                            self.expr_id_to_expr_map.insert(void_id, Expr::Void);
                            self.expr_id_to_scope_id_map.insert(void_id, body_scope_id);
                            self.span_map.insert(void_id, &EMPTY_SPAN);
                            (Vec::new(), void_id)
                        }
                    };
                    self.functions.insert(
                        id,
                        Function {
                            id,
                            name,
                            generic_parameter_constraint_ids,
                            parameters,
                            return_type_id,
                            body: (ids, expr_id, body_scope_id),
                            call_count: 0,
                        },
                    );
                    Some(Expr::Function(id))
                }
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
            Node::Let(name, type_, value, mutable) => {
                // `_` eats the value: the binding is never referenceable.
                if *name != "_" {
                    let scope = self.mut_scope_for_scope_id(scope_id);
                    scope.name_to_id_map.insert(name, id);
                }
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
                        mutable: *mutable,
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
            Node::Assign(name, op, value) => {
                let value_id = self.walk_expr_node(value, scope_id);
                let target_id = self.new_entity_id();
                self.prepped_locals.push((target_id, name));
                self.expr_id_to_scope_id_map.insert(target_id, scope_id);
                self.span_map.insert(target_id, &node.1);
                // A compound assignment like `x += v` desugars to `x = x + v`.
                let stored_value_id = match op {
                    Some(op) => {
                        let lhs_id = self.new_entity_id();
                        self.prepped_locals.push((lhs_id, name));
                        self.expr_id_to_scope_id_map.insert(lhs_id, scope_id);
                        self.span_map.insert(lhs_id, &node.1);
                        let binary_id = self.new_entity_id();
                        self.expr_id_to_expr_map
                            .insert(binary_id, Expr::Binary(*op, lhs_id, value_id));
                        self.expr_id_to_scope_id_map.insert(binary_id, scope_id);
                        self.span_map.insert(binary_id, &node.1);
                        binary_id
                    }
                    None => value_id,
                };
                self.prepped_assignments.push((target_id, stored_value_id));
                Some(Expr::Assignment(target_id, stored_value_id))
            }
            Node::Struct(name, generic_parameters, external, body) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let generic_parameter_constraint_ids =
                    self.register_generic_parameters(generic_parameters, body_scope_id);
                // A bodyless `struct Name;` is only valid when `external`; an
                // ordinary struct must list its fields in `{ .. }` (possibly
                // empty).
                if !external && body.is_none() {
                    self.diagnostics.push(Error {
                        span: node.1,
                        msg: format!(
                            "struct '{}' must declare a body or be declared `external`",
                            name
                        ),
                    });
                }
                let mut fields = Vec::new();
                for child in body.iter().flat_map(|body| &body.0) {
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
            Node::Enum(name, generic_parameters, variants) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let generic_parameter_constraint_ids =
                    self.register_generic_parameters(generic_parameters, body_scope_id);
                // Variants live in the enum's own namespace, reachable through
                // `use Enum::{ ... }` or `Enum::Variant` — not the outer scope.
                let variants_scope = self.create_scope(None);
                let variants_scope_id = self.push_scope(variants_scope);
                let mut variant_declarations = Vec::new();
                for (variant_index, variant) in variants.0.iter().enumerate() {
                    let variant_name = variant.0.0;
                    let data_type_ids = variant
                        .0
                        .1
                        .iter()
                        .map(|data_type| self.walk_type_node(data_type, body_scope_id))
                        .collect();
                    let variant_id = self.new_entity_id();
                    self.expr_id_to_expr_map
                        .insert(variant_id, Expr::EnumVariant(id, variant_index));
                    self.expr_id_to_scope_id_map.insert(variant_id, scope_id);
                    self.span_map.insert(variant_id, &variant.1);
                    self.reference_count.entry(variant_id).or_insert(0);
                    let variants_scope = self.mut_scope_for_scope_id(variants_scope_id);
                    variants_scope
                        .name_to_id_map
                        .insert(variant_name, variant_id);
                    variant_declarations.push(EnumVariantDeclaration {
                        name: variant_name,
                        data_type_ids,
                    });
                }
                self.enums.insert(
                    id,
                    Enum {
                        id,
                        name,
                        generic_parameter_constraint_ids,
                        variants: variant_declarations,
                        variants_scope_id,
                    },
                );
                Some(Expr::Enum(id))
            }
            Node::Match(subject, legs) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                let mut walked_legs = Vec::new();
                for (pattern, body) in &legs.0 {
                    // Each leg scopes its captures to its own body.
                    let leg_scope_id = self.create_owned_scope(Some(scope_id)).id;
                    let walked_pattern = self.walk_pattern(pattern, leg_scope_id);
                    let body_id = self.walk_expr_node(body, leg_scope_id);
                    walked_legs.push((walked_pattern, body_id));
                }
                self.prepped_matches.push(PreppedMatch {
                    id,
                    subject_id,
                    scope_id,
                    legs: walked_legs,
                    span: node.1,
                });
                // The entity is inserted once the subject type and the leg
                // patterns have been resolved.
                None
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
                                .map(|value| self.walk_expr_node(value, scope_id))
                                .unwrap_or_else(|| {
                                    // Field shorthand `S { field }` means
                                    // `S { field: field }`: the value is the
                                    // local named after the field itself.
                                    let local_id = self.new_entity_id();
                                    self.prepped_locals.push((local_id, x.0.0));
                                    self.expr_id_to_scope_id_map.insert(local_id, scope_id);
                                    self.span_map.insert(local_id, &x.1);
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
            Node::Impl(subject, generic_parameters, traits, body) => {
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                // The impl's generic parameters are declared by the `<...>` on
                // the subject (`impl List<type T: str>`), each optionally
                // bounded by a constraint. The trait clause (`with Iterator<T>`)
                // only uses these parameters, so it does not declare any.
                self.register_generic_parameters(generic_parameters, body_scope_id);
                let subject = self.walk_type_node(subject, body_scope_id);
                // Within an `impl`, `Self` refers to the subject type.
                self.register_self_type(body_scope_id, subject);
                self.walk_expr_nodes(&body.0, body_scope_id);
                let declarations = self.collect_declarations(body_scope_id);
                // `impl Subject with A + B` must satisfy each trait; record a
                // conformance check per trait to run once declarations are known.
                for trait_ in traits {
                    let trait_name = match &trait_.0 {
                        Node::Accessor(name) | Node::AccessorWithGenerics(name, _) => Some(*name),
                        _ => None,
                    };
                    if let Some(trait_name) = trait_name {
                        self.prepped_trait_impls.push(TraitImplCheck {
                            subject_type_id: subject,
                            trait_name,
                            scope_id,
                            declarations: declarations.clone(),
                            span: trait_.1,
                        });
                    }
                }
                self.implementations.push(Implementation {
                    subject,
                    declarations,
                });

                Some(Expr::Impl(id))
            }
            Node::Trait(name, generic_parameters, _supertraits, body) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                self.register_generic_parameters(generic_parameters, body_scope_id);
                // Inside a trait, `Self` is the (abstract) implementing type.
                let any_type_id = Type::Any.get_type_id(self);
                self.register_self_type(body_scope_id, any_type_id);
                // Bodyless methods are legitimate requirements inside a trait.
                let was_walking_trait_body = self.walking_trait_body;
                self.walking_trait_body = true;
                self.walk_expr_nodes(&body.0, body_scope_id);
                self.walking_trait_body = was_walking_trait_body;
                let declarations = self.collect_declarations(body_scope_id);
                self.traits.insert(
                    id,
                    Trait {
                        id,
                        name,
                        declarations,
                    },
                );
                Some(Expr::Trait(id))
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
                        // `_` eats the argument: it stays positional but is
                        // never referenceable.
                        if parameter.name != "_" {
                            body_scope
                                .name_to_id_map
                                .insert(parameter.name, parameter_id);
                        }
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

    // Walks a match-leg pattern, creating capture variables in the leg's
    // scope. Variant names stay unresolved until the subject type is known.
    fn walk_pattern(
        &mut self,
        pattern: &'src Spanned<Pattern<'src>>,
        scope_id: Id,
    ) -> WalkPattern<'src> {
        match &pattern.0 {
            Pattern::Wildcard => WalkPattern::Wildcard,
            Pattern::Binding(name, mutable) => {
                let name = *name;
                let capture_id = self.new_entity_id();
                let unknown_type_id = Type::Unknown.get_type_id(self);
                self.variables.insert(
                    capture_id,
                    Variable {
                        id: capture_id,
                        name,
                        initial: None,
                        type_id: unknown_type_id,
                        mutable: *mutable,
                    },
                );
                self.expr_id_to_expr_map
                    .insert(capture_id, Expr::Variable(capture_id));
                self.expr_id_to_scope_id_map.insert(capture_id, scope_id);
                self.span_map.insert(capture_id, &pattern.1);
                self.reference_count.entry(capture_id).or_insert(0);
                // `_` eats the value: it matches but is never referenceable.
                if name != "_" {
                    let scope = self.mut_scope_for_scope_id(scope_id);
                    scope.name_to_id_map.insert(name, capture_id);
                }
                WalkPattern::Binding(capture_id)
            }
            Pattern::Variant(name, payload) => WalkPattern::Variant(
                name,
                pattern.1,
                payload.as_ref().map(|patterns| {
                    patterns
                        .iter()
                        .map(|sub_pattern| self.walk_pattern(sub_pattern, scope_id))
                        .collect()
                }),
            ),
            Pattern::Tuple(patterns) => WalkPattern::Tuple(
                pattern.1,
                patterns
                    .iter()
                    .map(|sub_pattern| self.walk_pattern(sub_pattern, scope_id))
                    .collect(),
            ),
        }
    }

    // Resolves a walked pattern against the type it matches: variant names are
    // looked up and verified to belong to the matched enum, and captures take
    // the type of the value they bind. Returns `None` after emitting a
    // diagnostic when the pattern is invalid.
    fn resolve_pattern(
        &mut self,
        pattern: &WalkPattern<'src>,
        expected_type_id: TypeId,
        lookup_scope_id: Id,
    ) -> Option<ExprPattern> {
        match pattern {
            WalkPattern::Wildcard => Some(ExprPattern::Wildcard),
            WalkPattern::Binding(capture_id) => {
                let capture_id = *capture_id;
                self.variables.get_mut(&capture_id).unwrap().type_id = expected_type_id;
                self.resolved_types.insert(capture_id, expected_type_id);
                Some(ExprPattern::Binding(capture_id))
            }
            WalkPattern::Variant(name, span, payload) => {
                let span = *span;
                let entity = match self.try_get_expr_id_by_name(name, lookup_scope_id) {
                    Some(entity) => entity,
                    // `true`/`false` are keywords, not names in scope, so they
                    // never resolve by lookup. When the subject is a `bool`,
                    // resolve them against the `bool` enum's variants directly so
                    // `match flag { true => .., false => .. }` needs no imports.
                    None if matches!(
                        expected_type_id.get_type(self),
                        Type::Enum(id) if Some(id) == self.bool_enum_id
                    ) =>
                    {
                        match self.bool_enum_id.and_then(|bool_id| {
                            let variants_scope_id = self.enums.get(&bool_id)?.variants_scope_id;
                            self.scopes
                                .get(&variants_scope_id)?
                                .name_to_id_map
                                .get(name)
                                .copied()
                        }) {
                            Some(entity) => entity,
                            None => {
                                self.diagnostics.push(Error {
                                    span,
                                    msg: format!("cannot find '{}' in this scope", name),
                                });
                                return None;
                            }
                        }
                    }
                    None => {
                        self.diagnostics.push(Error {
                            span,
                            msg: format!("cannot find '{}' in this scope", name),
                        });
                        return None;
                    }
                };
                let (enum_id, variant_index) = match self.expr_id_to_expr_map.get(&entity) {
                    Some(Expr::EnumVariant(enum_id, variant_index)) => (*enum_id, *variant_index),
                    _ => {
                        self.diagnostics.push(Error {
                            span,
                            msg: format!("'{}' is not an enum variant", name),
                        });
                        return None;
                    }
                };
                match expected_type_id.get_type(self) {
                    Type::Enum(expected_enum_id) if expected_enum_id == enum_id => {}
                    Type::Unknown | Type::Any | Type::Generic(_) => {}
                    Type::Enum(_) => {
                        self.diagnostics.push(Error {
                            span,
                            msg: format!("variant '{}' does not belong to the matched enum", name),
                        });
                        return None;
                    }
                    other => {
                        let subject_str = self.pretty_print_type(&other, &HashMap::new());
                        self.diagnostics.push(Error {
                            span,
                            msg: format!("cannot match an enum variant against {}", subject_str),
                        });
                        return None;
                    }
                }
                let data_type_ids = self.enums.get(&enum_id).unwrap().variants[variant_index]
                    .data_type_ids
                    .clone();
                let payload_patterns: &[WalkPattern] = payload.as_deref().unwrap_or(&[]);
                if payload_patterns.len() != data_type_ids.len() {
                    self.diagnostics.push(Error {
                        span,
                        msg: format!(
                            "variant '{}' carries {} {}, but the pattern has {}",
                            name,
                            data_type_ids.len(),
                            plural(data_type_ids.len(), "value", "values"),
                            payload_patterns.len()
                        ),
                    });
                    return None;
                }
                let mut resolved_payload = Vec::new();
                for (sub_pattern, data_type_id) in payload_patterns.iter().zip(data_type_ids) {
                    resolved_payload.push(self.resolve_pattern(
                        sub_pattern,
                        data_type_id,
                        lookup_scope_id,
                    )?);
                }
                Some(ExprPattern::Variant(
                    enum_id,
                    variant_index,
                    resolved_payload,
                ))
            }
            WalkPattern::Tuple(span, patterns) => {
                // Element types come from the matched tuple type when known;
                // otherwise each element resolves against `Unknown`.
                let element_type_ids = match expected_type_id.get_type(self) {
                    Type::Tuple(ids) if ids.len() == patterns.len() => ids,
                    _ => {
                        let unknown = Type::Unknown.get_type_id(self);
                        vec![unknown; patterns.len()]
                    }
                };
                let _ = span;
                let mut resolved = Vec::new();
                for (sub_pattern, element_type_id) in patterns.iter().zip(element_type_ids) {
                    resolved.push(self.resolve_pattern(
                        sub_pattern,
                        element_type_id,
                        lookup_scope_id,
                    )?);
                }
                Some(ExprPattern::Tuple(resolved))
            }
        }
    }

    fn walk_type_node(&mut self, node: &Spanned<Node<'src>>, scope_id: Id) -> TypeId {
        let type_id = self.new_type_id();

        let type_: Option<Type> = match &node.0 {
            Node::Accessor(name) => match *name {
                // `any` is the top type, not a struct, so it stays built in.
                // Scalar primitives are `std` structs resolved by name (they're
                // registered in the prelude/global scope), like any other type.
                "any" => Some(Type::Any),
                "void" => Some(Type::Void),
                _ => {
                    self.prepped_type_locals
                        .push((type_id, name, scope_id, node.1));
                    None
                }
            },
            // Generic arguments are currently erased from the nominal type:
            // `FromFn<T>` resolves to the same type as `FromFn`. The
            // arguments still matter where they declare implicit impl
            // generics or drive monomorphization at call sites.
            Node::AccessorWithGenerics(name, _generic_arguments) => {
                self.prepped_type_locals
                    .push((type_id, name, scope_id, node.1));
                None
            }
            Node::StaticAccessor(subject, member_name) => {
                let subject_type_id = self.walk_type_node(subject, scope_id);
                self.prepped_type_static_accessors.push((
                    type_id,
                    subject_type_id,
                    member_name,
                    node.1,
                ));
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
        // Also check resolved_types for types resolved during constraint solving.
        if let Some(&type_id) = self.resolved_types.get(&expr_id) {
            return type_id.get_type(self);
        }

        let constraint = match constraint {
            Type::Generic(type_id) => substitution_context
                .get(type_id)
                .map(|x| x.get_type(self))
                .unwrap_or_else(|| constraint.clone()),
            x => x.clone(),
        };

        // The entity may not exist yet (e.g. a deferred field accessor whose
        // subject type is still unknown, like `my_id.n` before `my_id` is
        // inferred). Treat it as unresolved so the constraint solver defers and
        // retries, rather than panicking.
        let expr = match self.expr_id_to_expr_map.get(&expr_id) {
            Some(expr) => expr,
            None => return Type::Unresolved,
        };

        let inferred_type: Type = match expr {
            Expr::Null => self.primitive_struct_type("null"),
            Expr::Bool(_) => self.bool_type(),
            // `subject is pattern` is a boolean test.
            Expr::Is(_, _) => self.bool_type(),
            Expr::String(_) => self.primitive_struct_type("str"),
            // A numeric literal's type comes from its suffix (`5u32`), then a
            // fractional part (`0.0` is a float), then the expected type
            // (`0` against an `f64` field), defaulting to `i32`.
            Expr::Number(_, fraction, suffix) => {
                let name = match *suffix {
                    Some("u32") => "u32",
                    Some("i32") => "i32",
                    Some("f64") | Some("f") => "f64",
                    Some("n") => "BigInt",
                    _ => {
                        if fraction.is_some() {
                            "f64"
                        } else {
                            match &constraint {
                                Type::Struct(id) if *id == self.primitive_struct_ids["f64"] => {
                                    "f64"
                                }
                                Type::Struct(id) if *id == self.primitive_struct_ids["u32"] => {
                                    "u32"
                                }
                                Type::Struct(id) if *id == self.primitive_struct_ids["i32"] => {
                                    "i32"
                                }
                                Type::Struct(id) if *id == self.primitive_struct_ids["BigInt"] => {
                                    "BigInt"
                                }
                                _ => "i32",
                            }
                        }
                    }
                };
                self.primitive_struct_type(name)
            }
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
            Expr::Enum(enum_id) => Type::Enum(*enum_id),
            // A bare variant reference is a value of the enum (e.g. `None`); a
            // variant with data acts as a constructor whose call also yields
            // the enum.
            Expr::EnumVariant(enum_id, _) => Type::Enum(*enum_id),
            Expr::Trait(trait_id) => Type::Trait(*trait_id),
            Expr::Module(module_id) => Type::Module(*module_id),
            Expr::Call(id) => {
                // The call may not have been wired up yet (its `FunctionCall`
                // is recorded once the subject resolves). Defer until it is.
                let (subject_id, generic_argument_ids) = match self.function_calls.get(id) {
                    Some(function_call) => (
                        function_call.subject_id,
                        function_call.generic_argument_ids.clone(),
                    ),
                    None => return Type::Unresolved,
                };
                let subject_type = self.infer_type_inner(
                    subject_id,
                    &Type::Unknown,
                    substitution_context,
                    exprs_seen,
                );
                match subject_type {
                    Type::Unresolved => Type::Unresolved,
                    // Calling a closure-typed value (e.g. `(self.fn)()`)
                    // yields the closure's return type.
                    Type::Closure(_, return_type_id) => {
                        let return_type = return_type_id.get_type(self);
                        self.substitute_type(&return_type, substitution_context)
                    }
                    // Calling a variant constructor (e.g. `Some(1)`) yields a
                    // value of the enum.
                    Type::Enum(enum_id) => Type::Enum(enum_id),
                    // A call's type is the callee's return type: its declared
                    // return type if annotated, otherwise the inferred type of
                    // its body — with the call's generic arguments substituted
                    // for the function's generic parameters.
                    Type::Function(function_id) => {
                        // `panic(..)` never returns, so its call types as `any`,
                        // which unifies with any expected type (e.g. it can be
                        // the sole body of a function with any return type).
                        if Some(function_id) == self.panic_fn_id {
                            return Type::Any;
                        }
                        let function = self.functions.get(&function_id).map(|f| {
                            (
                                f.generic_parameter_constraint_ids.clone(),
                                f.return_type_id,
                                f.body.1,
                            )
                        });
                        let Some((generic_constraint_ids, return_type_id, body_return_id)) =
                            function
                        else {
                            // An external function: use its declared return type.
                            return self
                                .external_functions
                                .get(&function_id)
                                .map(|f| f.return_type_id.get_type(self))
                                .unwrap_or(Type::Void);
                        };
                        let mut substitution_context = substitution_context.clone();
                        for (i, constraint_id) in generic_constraint_ids.iter().enumerate() {
                            if let Some(argument_id) = generic_argument_ids.get(i) {
                                substitution_context.insert(*constraint_id, *argument_id);
                            }
                        }
                        match return_type_id {
                            Some(return_type_id) => {
                                let return_type = return_type_id.get_type(self);
                                self.substitute_type(&return_type, &substitution_context)
                            }
                            None => self.infer_type_inner(
                                body_return_id,
                                &Type::Unknown,
                                &substitution_context,
                                exprs_seen,
                            ),
                        }
                    }
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
            Expr::StructInitializer(initializer_id, _initializer_fields) => {
                // Look up the actual struct definition ID from the mapping.
                // If not found, defer and let downstream constraints handle it.
                if let Some(&struct_def_id) = self.struct_initializer_to_def.get(initializer_id) {
                    Type::Struct(struct_def_id)
                } else {
                    Type::Struct(*initializer_id)
                }
            }
            Expr::Generic(type_id) => type_id.get_type(self),
            // Comparisons produce a `bool`; arithmetic produces the operand
            // type (taken from the left-hand side).
            Expr::Binary(
                BinaryOp::Eq
                | BinaryOp::NotEq
                | BinaryOp::Lt
                | BinaryOp::Gt
                | BinaryOp::LtEq
                | BinaryOp::GtEq,
                _,
                _,
            ) => self.bool_type(),
            Expr::Binary(_, lhs_id, _rhs_id) => {
                let lhs =
                    self.infer_type_inner(*lhs_id, &constraint, substitution_context, exprs_seen);
                match lhs {
                    Type::Unresolved => Type::Unresolved,
                    _ => lhs,
                }
            }
            // A block's type is the type of its trailing expression.
            Expr::Block((_, trailing_expr_id)) => {
                let trailing_expr_id = *trailing_expr_id;
                self.infer_type_inner(
                    trailing_expr_id,
                    &constraint,
                    substitution_context,
                    exprs_seen,
                )
            }
            Expr::Closure(closure_id) => {
                let closure = self.closures.get(closure_id).unwrap();
                let parameter_ids = closure.parameters.clone();
                let return_expr_id = closure.return_;
                let parameter_type_ids = parameter_ids
                    .iter()
                    .map(|parameter_id| self.parameters.get(parameter_id).unwrap().type_id)
                    .collect::<Vec<_>>();
                let return_type = self.infer_type_inner(
                    return_expr_id,
                    &Type::Unknown,
                    substitution_context,
                    exprs_seen,
                );
                match return_type {
                    Type::Unresolved => Type::Unresolved,
                    _ => Type::Closure(parameter_type_ids, return_type.get_type_id(self)),
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
            (
                Type::Primitive(PrimitiveType::List(l_id)),
                Type::Primitive(PrimitiveType::List(r_id)),
            ) => {
                let l = l_id.get_type(self);
                let r = r_id.get_type(self);
                let (item_type, bindings) = self.reconcile_type(&l, &r, substitution_context)?;
                let item_type_id = item_type.get_type_id(self);
                (Type::Primitive(PrimitiveType::List(item_type_id)), bindings)
            }
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
            (
                Type::Closure(l_parameter_ids, l_return_id),
                Type::Closure(r_parameter_ids, r_return_id),
            ) => {
                if l_parameter_ids.len() != r_parameter_ids.len() {
                    return None;
                }
                let mut result_parameter_ids = Vec::with_capacity(l_parameter_ids.len());
                let mut all_bindings = Vec::new();
                for (l_parameter_id, r_parameter_id) in
                    l_parameter_ids.iter().zip(r_parameter_ids.iter())
                {
                    let l = l_parameter_id.get_type(self);
                    let r = r_parameter_id.get_type(self);
                    let (parameter, bindings) =
                        self.reconcile_type(&l, &r, substitution_context)?;
                    all_bindings.extend(bindings);
                    result_parameter_ids.push(parameter.get_type_id(self));
                }
                let l_return = l_return_id.get_type(self);
                let r_return = r_return_id.get_type(self);
                let (return_type, bindings) =
                    self.reconcile_type(&l_return, &r_return, substitution_context)?;
                all_bindings.extend(bindings);
                let return_type_id = return_type.get_type_id(self);
                (
                    Type::Closure(result_parameter_ids, return_type_id),
                    all_bindings,
                )
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
            (
                Type::Primitive(PrimitiveType::List(l_id)),
                Type::Primitive(PrimitiveType::List(r_id)),
            ) => {
                let l = l_id.get_type(self);
                let r = r_id.get_type(self);
                self.compare_type(&l, &r, substitution_context)
            }
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
            (
                Type::Closure(l_parameter_ids, l_return_id),
                Type::Closure(r_parameter_ids, r_return_id),
            ) => {
                l_parameter_ids.len() == r_parameter_ids.len()
                    && l_parameter_ids.iter().zip(r_parameter_ids.iter()).all(
                        |(l_parameter_id, r_parameter_id)| {
                            let l = l_parameter_id.get_type(self);
                            let r = r_parameter_id.get_type(self);
                            self.compare_type(&l, &r, substitution_context)
                        },
                    )
                    && {
                        let l = l_return_id.get_type(self);
                        let r = r_return_id.get_type(self);
                        self.compare_type(&l, &r, substitution_context)
                    }
            }
            (a, b) if a == b => true,
            _ => false,
        }
    }

    /// Resolves any generic type parameters in `type_` using the substitution
    /// context, e.g. turning the return type `T` of `default<T>` into `Id` for
    /// a call `default<Id>()`.
    fn substitute_type(&self, type_: &Type, substitution_context: &SubstitutionContext) -> Type {
        match type_ {
            Type::Generic(constraint_id) => substitution_context
                .get(constraint_id)
                .map(|type_id| {
                    let resolved = type_id.get_type(self);
                    self.substitute_type(&resolved, substitution_context)
                })
                .unwrap_or_else(|| type_.clone()),
            _ => type_.clone(),
        }
    }

    fn build(&mut self) {
        for (path, name, scope_id, span) in self.prepped_imports.clone() {
            // A `self` leaf re-binds the namespace it sits in under its own name
            // (e.g. `Option::{ self }` binds `Option`); otherwise the leaf is the
            // final path segment and binds under its own name.
            let (segments, bind_name): (Vec<&str>, &str) = if name == "self" {
                match path.last() {
                    Some(last) => (path.clone(), last),
                    None => {
                        self.diagnostics.push(Error {
                            span,
                            msg: "`self` import has no enclosing namespace".to_string(),
                        });
                        continue;
                    }
                }
            } else {
                let mut segments = path.clone();
                segments.push(name);
                (segments, name)
            };
            let mut segments = segments.into_iter();
            let root = segments.next().unwrap();
            let module_id = match self.module_id_by_name.get(root) {
                Some(module_id) => *module_id,
                None => {
                    self.diagnostics.push(Error {
                        span,
                        msg: format!("cannot find module '{}' to import", root),
                    });
                    continue;
                }
            };
            let mut target_id = module_id;
            let mut namespace_scope_id = self.modules.get(&module_id).unwrap().body.1;
            let mut unresolved = false;
            for part in segments {
                match self.try_get_expr_id_by_name(part, namespace_scope_id) {
                    Some(id) => {
                        target_id = id;
                        // If this segment is itself a namespace — a module or an
                        // enum (whose namespace holds its variants) — descend into
                        // it so the next segment resolves there, e.g. `pkg::io::print`
                        // or `std::option::Option::Some`.
                        let sub_scope_id = match self.expr_id_to_expr_map.get(&id) {
                            Some(Expr::Module(sub_module_id)) => {
                                self.modules.get(sub_module_id).map(|module| module.body.1)
                            }
                            Some(Expr::Enum(enum_id)) => {
                                self.enums.get(enum_id).map(|enum_| enum_.variants_scope_id)
                            }
                            _ => None,
                        };
                        if let Some(sub_scope_id) = sub_scope_id {
                            namespace_scope_id = sub_scope_id;
                        }
                    }
                    None => {
                        self.diagnostics.push(Error {
                            span,
                            msg: format!("cannot find '{}' in the imported path", part),
                        });
                        unresolved = true;
                        break;
                    }
                }
            }
            if unresolved {
                continue;
            }
            let scope = self.mut_scope_for_scope_id(scope_id);
            scope.name_to_id_map.insert(bind_name, target_id);
        }

        // --- Resolve `use` statements ---
        // `use Namespace::{ a, b }` binds items out of a namespace — a module
        // or an enum (whose namespace holds its variants) — into the scope the
        // statement appears in.
        for (path, name, scope_id, span) in std::mem::take(&mut self.prepped_uses) {
            let mut segments = path.iter().copied().chain(std::iter::once(name));
            let root = segments.next().unwrap();
            let mut current = match self.try_get_expr_id_by_name(root, scope_id) {
                Some(entity) => entity,
                None => {
                    self.diagnostics.push(Error {
                        span,
                        msg: format!("cannot find '{}' in this scope", root),
                    });
                    continue;
                }
            };
            let mut resolved = true;
            for segment in segments {
                let namespace_scope_id = match self.expr_id_to_expr_map.get(&current) {
                    Some(Expr::Module(module_id)) => {
                        self.modules.get(module_id).map(|module| module.body.1)
                    }
                    Some(Expr::Enum(enum_id)) => {
                        self.enums.get(enum_id).map(|enum_| enum_.variants_scope_id)
                    }
                    _ => None,
                };
                let Some(namespace_scope_id) = namespace_scope_id else {
                    self.diagnostics.push(Error {
                        span,
                        msg: "`use` requires a namespace (a module or an enum)".to_string(),
                    });
                    resolved = false;
                    break;
                };
                current = match self
                    .scopes
                    .get(&namespace_scope_id)
                    .and_then(|scope| scope.name_to_id_map.get(segment))
                    .copied()
                {
                    Some(entity) => entity,
                    None => {
                        self.diagnostics.push(Error {
                            span,
                            msg: format!("cannot find '{}' in the `use` path", segment),
                        });
                        resolved = false;
                        break;
                    }
                };
            }
            if resolved {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, current);
            }
        }

        for (id, name) in self.prepped_locals.clone() {
            let scope_id = self.get_scope_id_for_entity(id);
            match self.try_get_expr_id_by_name(name, scope_id) {
                Some(subject_id) => {
                    let rc = self.reference_count.entry(subject_id).or_insert(0);
                    *rc += 1;
                    self.expr_id_to_expr_map.insert(id, Expr::Local(subject_id));
                }
                None => {
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                        msg: format!("cannot find '{}' in this scope", name),
                    });
                    self.expr_id_to_expr_map.insert(id, Expr::Error);
                }
            }
        }

        // --- Wire assignments to their variables ---
        // Each assignment targets a (now resolved) local. The assigned value
        // joins the variable's constraint so reassignments are type checked
        // against the variable's type, and assigning to an immutable binding
        // is rejected.
        for (target_id, value_id) in std::mem::take(&mut self.prepped_assignments) {
            let variable_id = match self.expr_id_to_expr_map.get(&target_id) {
                Some(Expr::Local(variable_id)) => *variable_id,
                _ => continue,
            };
            match self.variables.get(&variable_id) {
                Some(variable) if !variable.mutable => {
                    let name = variable.name;
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&target_id).unwrap_or(&&EMPTY_SPAN),
                        msg: format!(
                            "cannot assign to immutable variable '{}'. Declare it with `mut` instead.",
                            name
                        ),
                    });
                }
                Some(_) => {}
                None => {
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&target_id).unwrap_or(&&EMPTY_SPAN),
                        msg: "cannot assign to this expression".to_string(),
                    });
                    continue;
                }
            }
            if let Some(constraint) = self
                .variable_constraints
                .iter_mut()
                .find(|constraint| constraint.variable_id == variable_id)
            {
                constraint.value_ids.push(value_id);
            }
        }

        for (type_id, name, scope_id, span) in self.prepped_type_locals.clone() {
            match self.try_get_expr_id_by_name(name, scope_id) {
                Some(subject_id) => {
                    let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
                    self.type_id_to_type_map.insert(type_id, subject_type);
                }
                None => {
                    self.diagnostics.push(Error {
                        span,
                        msg: format!("cannot find type '{}'", name),
                    });
                    self.type_id_to_type_map.insert(type_id, Type::Unknown);
                }
            }
        }

        for (type_id, subject_type_id, member_name, span) in
            self.prepped_type_static_accessors.clone()
        {
            match subject_type_id.get_type(self) {
                Type::Module(module_id) => {
                    let module = self.modules.get(&module_id).unwrap();
                    let module_scope_id = module.body.1;
                    let module_name = module.name;
                    match self.try_get_expr_id_by_name(member_name, module_scope_id) {
                        Some(member_id) => {
                            let member_type =
                                self.infer_type(member_id, &Type::Unknown, &HashMap::new());
                            self.type_id_to_type_map.insert(type_id, member_type);
                        }
                        None => {
                            self.diagnostics.push(Error {
                                span,
                                msg: format!(
                                    "cannot find '{}' in module '{}'",
                                    member_name, module_name
                                ),
                            });
                            self.type_id_to_type_map.insert(type_id, Type::Unknown);
                        }
                    }
                }
                _ => {}
            }
        }

        for (id, subject_id, member_name) in self.prepped_field_accessors.clone() {
            let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
            match subject_type {
                Type::Struct(struct_id) => {
                    let struct_ = self.structs.get(&struct_id).unwrap();
                    let struct_name = struct_.name;
                    let field_index = struct_
                        .fields
                        .iter()
                        .enumerate()
                        .find_map(|(i, x)| (x.name == member_name).then_some(i));
                    match field_index {
                        Some(field_index) => {
                            self.expr_id_to_expr_map
                                .insert(id, Expr::Field(subject_id, struct_id, field_index));
                        }
                        None => {
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                                msg: format!(
                                    "struct '{}' has no field '{}'",
                                    struct_name, member_name
                                ),
                            });
                            self.expr_id_to_expr_map.insert(id, Expr::Error);
                        }
                    }
                }
                subject_type => {
                    let subject_str = self.pretty_print_type(&subject_type, &HashMap::new());
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                        msg: format!("cannot access field '{}' on {}", member_name, subject_str),
                    });
                    self.expr_id_to_expr_map.insert(id, Expr::Error);
                }
            }
        }

        for (id, subject_type_id, member_name) in self.prepped_static_accessors.clone() {
            let subject_type = subject_type_id.get_type(self);
            match subject_type {
                // Static access on a nominal type (`Id::new`), a trait
                // (`Iterator::from_fn`), or an enum (`Option::Some`): look the
                // member up among the variants (for enums) and the matching
                // implementations.
                Type::Struct(_) | Type::Trait(_) | Type::Enum(_) => {
                    let variant_id = match &subject_type {
                        Type::Enum(enum_id) => {
                            let variants_scope_id =
                                self.enums.get(enum_id).unwrap().variants_scope_id;
                            self.scopes
                                .get(&variants_scope_id)
                                .and_then(|scope| scope.name_to_id_map.get(member_name))
                                .copied()
                        }
                        _ => None,
                    };
                    let member_id = variant_id.or_else(|| {
                        self.implementations
                            .iter()
                            .filter(|x| {
                                self.compare_type(
                                    &subject_type,
                                    &x.subject.get_type(self),
                                    &HashMap::new(),
                                )
                            })
                            .find_map(|x| x.declarations.get(member_name))
                            .copied()
                    });
                    match member_id {
                        Some(member_id) => {
                            let rc = self.reference_count.entry(member_id).or_insert(0);
                            *rc += 1;
                            self.expr_id_to_expr_map.insert(id, Expr::Local(member_id));
                        }
                        None => {
                            let subject_str =
                                self.pretty_print_type(&subject_type, &HashMap::new());
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                                msg: format!("cannot find '{}' in {}", member_name, subject_str),
                            });
                        }
                    }
                }
                Type::Module(module_id) => {
                    let module = self.modules.get(&module_id).unwrap();
                    let module_scope_id = module.body.1;
                    let module_name = module.name;
                    match self.try_get_expr_id_by_name(member_name, module_scope_id) {
                        Some(member_id) => {
                            let rc = self.reference_count.entry(member_id).or_insert(0);
                            *rc += 1;
                            self.expr_id_to_expr_map.insert(id, Expr::Local(member_id));
                        }
                        None => {
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                                msg: format!(
                                    "cannot find '{}' in module '{}'",
                                    member_name, module_name
                                ),
                            });
                            self.expr_id_to_expr_map.insert(id, Expr::Error);
                        }
                    }
                }
                // `T::member` where `T` is a generic parameter: resolve through
                // the trait the parameter is bound by (e.g. `T: Default`).
                Type::Generic(constraint_id) => match constraint_id.get_type(self) {
                    Type::Trait(trait_id) => {
                        // Record the accessor so codegen can monomorphize it to
                        // the concrete type's member at each call site.
                        self.generic_static_accessors
                            .insert(id, (constraint_id, member_name));
                        let member_id = self
                            .traits
                            .get(&trait_id)
                            .and_then(|trait_| trait_.declarations.get(member_name).copied());
                        match member_id {
                            Some(member_id) => {
                                let rc = self.reference_count.entry(member_id).or_insert(0);
                                *rc += 1;
                                self.expr_id_to_expr_map.insert(id, Expr::Local(member_id));
                            }
                            None => {
                                let trait_name = self.traits.get(&trait_id).map(|t| t.name);
                                self.diagnostics.push(Error {
                                    span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                                    msg: format!(
                                        "trait '{}' has no member '{}'",
                                        trait_name.unwrap_or("?"),
                                        member_name
                                    ),
                                });
                            }
                        }
                    }
                    _ => {
                        self.diagnostics.push(Error {
                            span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                            msg: format!(
                                "cannot access '{}' on an unconstrained type parameter",
                                member_name
                            ),
                        });
                    }
                },
                _ => {}
            }
        }

        // --- Check trait conformance for `impl Subject with Trait` ---
        for check in std::mem::take(&mut self.prepped_trait_impls) {
            let trait_id = match self.try_get_expr_id_by_name(check.trait_name, check.scope_id) {
                Some(trait_id) => trait_id,
                None => {
                    self.diagnostics.push(Error {
                        span: check.span,
                        msg: format!("cannot find trait '{}'", check.trait_name),
                    });
                    continue;
                }
            };
            let required: Vec<&'src str> = match self.traits.get(&trait_id) {
                Some(trait_) => trait_.declarations.keys().copied().collect(),
                None => {
                    self.diagnostics.push(Error {
                        span: check.span,
                        msg: format!("'{}' is not a trait", check.trait_name),
                    });
                    continue;
                }
            };
            let subject_name = match check.subject_type_id.get_type(self) {
                Type::Struct(struct_id) => self
                    .structs
                    .get(&struct_id)
                    .map(|s| s.name)
                    .unwrap_or("type"),
                _ => "type",
            };
            for member_name in required {
                if !check.declarations.contains_key(member_name) {
                    self.diagnostics.push(Error {
                        span: check.span,
                        msg: format!(
                            "'{}' does not implement trait '{}': missing '{}'",
                            subject_name, check.trait_name, member_name
                        ),
                    });
                }
            }
        }

        // --- Constraint solving loop ---
        // Each iteration resolves at least one new type (unknown -> concrete).
        // The maximum number of iterations equals the number of expressions
        // with unknown types, since each resolved type never reverts to unknown.
        let max_iterations = self.struct_initializer_constraints.len()
            + self.variable_constraints.len()
            + self.call_subject_constraints.len()
            + self.field_accessor_constraints.len()
            + self.prepped_is.len()
            + self.prepped_matches.len();

        for _ in 0..max_iterations {
            let mut progress = false;

            // --- Resolve struct initializer constraints ---
            let mut unresolved_constraints = Vec::new();
            for mut constraint in std::mem::take(&mut self.struct_initializer_constraints) {
                // struct_id is Id(0) as placeholder; resolve by name across all scopes.
                let struct_expr_id = if constraint.struct_id == Id(0) {
                    let mut found_id = None;
                    for scope in self.scopes.values() {
                        if let Some(expr_id) = scope.name_to_id_map.get(constraint.struct_name) {
                            found_id = Some(*expr_id);
                            break;
                        }
                    }
                    match found_id {
                        Some(expr_id) => expr_id,
                        None => {
                            self.diagnostics.push(Error {
                                span: constraint.fields_span.clone(),
                                msg: format!("unknown struct: {}", constraint.struct_name),
                            });
                            continue;
                        }
                    }
                } else {
                    let scope_id = self.get_scope_id_for_entity(constraint.struct_id);
                    match self.try_get_expr_id_by_name(constraint.struct_name, scope_id) {
                        Some(expr_id) => expr_id,
                        None => {
                            self.diagnostics.push(Error {
                                span: constraint.fields_span.clone(),
                                msg: format!("unknown struct: {}", constraint.struct_name),
                            });
                            continue;
                        }
                    }
                };
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
                let initializer_id = constraint.initializer_id;
                let mut defer = None;
                let fields = constraint.fields.clone();
                let struct_id = constraint.struct_id;
                let struct_name = constraint.struct_name;
                for (field_name, field_value, field_value_span) in &fields {
                    let field = struct_fields
                        .iter()
                        .enumerate()
                        .find(|(_, x)| *x.name == **field_name);
                    let (struct_field_index, struct_field) = match field {
                        Some(field) => field,
                        None => {
                            self.diagnostics.push(Error {
                                span: *field_value_span,
                                msg: format!(
                                    "struct '{}' has no field '{}'",
                                    struct_name, field_name
                                ),
                            });
                            continue;
                        }
                    };
                    let struct_field_type = struct_field.type_id.get_type(self);
                    // Infer the value against the declared field type so that,
                    // e.g., an integer literal is treated as `f64` when the
                    // field is `f64`.
                    let value_type =
                        self.infer_type(*field_value, &struct_field_type, &substitution_context);
                    if let Type::Unresolved = value_type {
                        defer = Some(constraint);
                        break;
                    }
                    if let Some((_unified, bindings)) =
                        self.reconcile_type(&value_type, &struct_field_type, &substitution_context)
                    {
                        for (cid, tid) in bindings {
                            substitution_context.insert(cid, tid);
                        }
                        initializer_fields.insert(struct_field_index, *field_value);
                    } else {
                        // Type mismatch: emit diagnostic but still record the type for downstream consumers.
                        self.diagnostics.push(Error {
                            span: constraint.fields_span.clone(),
                            msg: format!(
                                "Expected {}, but got {} instead.",
                                self.pretty_print_type(&struct_field_type, &substitution_context),
                                self.pretty_print_type(&value_type, &substitution_context),
                            ),
                        });
                        let type_id = Type::Struct(struct_id).get_type_id(self);
                        self.resolved_types.insert(initializer_id, type_id);
                        self.struct_initializer_to_def
                            .insert(initializer_id, struct_id);
                    }
                }
                if let Some(deferred) = defer {
                    unresolved_constraints.push(deferred);
                }
                // Always store the struct initializer expression so infer_type can handle it.
                self.expr_id_to_expr_map.insert(
                    initializer_id,
                    Expr::StructInitializer(initializer_id, initializer_fields),
                );
                // Store the mapping from initializer to struct definition.
                self.struct_initializer_to_def
                    .insert(initializer_id, struct_id);
                let type_id = Type::Struct(struct_id).get_type_id(self);
                self.resolved_types.insert(initializer_id, type_id);
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
                let subject_id = constraint.subject_id;
                let member_name = constraint.member_name;

                // Defer until the subject's entity has been resolved (e.g. a
                // method-call receiver wired up in a later iteration), then
                // resolve its type by inference rather than relying on it
                // having been cached in the type maps — parameters, locals and
                // calls compute their types through `infer_type`.
                if !self.expr_id_to_expr_map.contains_key(&subject_id) {
                    remaining_accessors.insert(id, constraint);
                    continue;
                }
                let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
                match subject_type {
                    Type::Unresolved => {
                        remaining_accessors.insert(id, constraint);
                        continue;
                    }
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
                        let struct_name = struct_.name;
                        let field =
                            struct_.fields.iter().enumerate().find_map(|(i, x)| {
                                (x.name == member_name).then_some((i, x.type_id))
                            });
                        match field {
                            Some((field_index, field_type)) => {
                                self.expr_id_to_expr_map
                                    .insert(id, Expr::Field(subject_id, struct_id, field_index));
                                self.expr_id_to_type_id_map.insert(id, field_type);
                                self.resolved_types.insert(id, field_type);
                            }
                            None => {
                                self.diagnostics.push(Error {
                                    span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                                    msg: format!(
                                        "struct '{}' has no field '{}'",
                                        struct_name, member_name
                                    ),
                                });
                                self.expr_id_to_expr_map.insert(id, Expr::Error);
                            }
                        }
                    }
                    subject_type => {
                        let subject_str = self.pretty_print_type(&subject_type, &HashMap::new());
                        self.diagnostics.push(Error {
                            span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                            msg: format!(
                                "cannot access field '{}' on {}",
                                member_name, subject_str
                            ),
                        });
                        self.expr_id_to_expr_map.insert(id, Expr::Error);
                    }
                }
                progress = true;
            }
            self.field_accessor_constraints = remaining_accessors;
            if !self.field_accessor_constraints.is_empty() {
                progress = true;
            }

            // --- Resolve `is` pattern tests ---
            // Once the subject type is known, resolve the pattern (typing its
            // captures) and record the resolved `Expr::Is`. The expression's own
            // type is always `bool`.
            let mut remaining_is = Vec::new();
            for prepped in std::mem::take(&mut self.prepped_is) {
                let subject_type =
                    self.infer_type(prepped.subject_id, &Type::Unknown, &HashMap::new());
                if matches!(subject_type, Type::Unresolved) {
                    remaining_is.push(prepped);
                    continue;
                }
                let subject_type_id = subject_type.get_type_id(self);
                match self.resolve_pattern(&prepped.pattern, subject_type_id, prepped.scope_id) {
                    Some(resolved) => {
                        self.expr_id_to_expr_map
                            .insert(prepped.id, Expr::Is(prepped.subject_id, resolved));
                    }
                    None => {} // a diagnostic was already emitted
                }
                progress = true;
            }
            self.prepped_is = remaining_is;
            if !self.prepped_is.is_empty() {
                progress = true;
            }

            // --- Resolve match expressions ---
            // A match resolves once its subject type is known: the leg
            // patterns are checked against the subject's enum (typing any
            // captures), exhaustiveness is verified, and the match's own type
            // is the unification of its leg body types.
            let mut remaining_matches = Vec::new();
            for prepped in std::mem::take(&mut self.prepped_matches) {
                let subject_type =
                    self.infer_type(prepped.subject_id, &Type::Unknown, &HashMap::new());
                if matches!(subject_type, Type::Unresolved) {
                    remaining_matches.push(prepped);
                    continue;
                }
                let subject_type_id = subject_type.clone().get_type_id(self);

                let mut resolved_legs = Vec::new();
                let mut pattern_error = false;
                for (pattern, body_id) in &prepped.legs {
                    match self.resolve_pattern(pattern, subject_type_id, prepped.scope_id) {
                        Some(resolved) => resolved_legs.push((resolved, *body_id)),
                        None => pattern_error = true,
                    }
                }
                if pattern_error {
                    // Diagnostics were already emitted; drop the match.
                    progress = true;
                    continue;
                }

                // Exhaustiveness: every variant must be covered unless a
                // catch-all (`_` or a binding) is present.
                if let Type::Enum(enum_id) = subject_type {
                    let has_catch_all = resolved_legs.iter().any(|(pattern, _)| {
                        matches!(pattern, ExprPattern::Wildcard | ExprPattern::Binding(_))
                    });
                    if !has_catch_all {
                        let covered = resolved_legs
                            .iter()
                            .filter_map(|(pattern, _)| match pattern {
                                ExprPattern::Variant(_, variant_index, _) => Some(*variant_index),
                                _ => None,
                            })
                            .collect::<HashSet<_>>();
                        let missing = self
                            .enums
                            .get(&enum_id)
                            .unwrap()
                            .variants
                            .iter()
                            .enumerate()
                            .filter(|(variant_index, _)| !covered.contains(variant_index))
                            .map(|(_, variant)| format!("'{}'", variant.name))
                            .collect::<Vec<_>>();
                        if !missing.is_empty() {
                            self.diagnostics.push(Error {
                                span: prepped.span,
                                msg: format!(
                                    "match is not exhaustive: missing {}",
                                    missing.join(", ")
                                ),
                            });
                        }
                    }
                }

                // The match's type unifies the leg body types.
                let mut unified: Option<Type> = None;
                let mut deferred = false;
                for (_, body_id) in &resolved_legs {
                    let body_type = self.infer_type(*body_id, &Type::Unknown, &HashMap::new());
                    if matches!(body_type, Type::Unresolved) {
                        deferred = true;
                        break;
                    }
                    unified = Some(match unified {
                        None => body_type,
                        Some(current) => {
                            match self.reconcile_type(&current, &body_type, &HashMap::new()) {
                                Some((unified_type, _)) => unified_type,
                                None => {
                                    let expected =
                                        self.pretty_print_type(&current, &HashMap::new());
                                    let got = self.pretty_print_type(&body_type, &HashMap::new());
                                    self.diagnostics.push(Error {
                                        span: prepped.span,
                                        msg: format!(
                                            "match legs have mismatched types: expected {}, but got {} instead.",
                                            expected, got
                                        ),
                                    });
                                    current
                                }
                            }
                        }
                    });
                }
                if deferred {
                    remaining_matches.push(prepped);
                    continue;
                }
                let match_type = unified.unwrap_or(Type::Void);
                let match_type_id = match_type.get_type_id(self);
                self.resolved_types.insert(prepped.id, match_type_id);
                let legs = resolved_legs
                    .into_iter()
                    .map(|(pattern, body)| ExprMatchLeg { pattern, body })
                    .collect();
                self.expr_id_to_expr_map
                    .insert(prepped.id, Expr::Match(prepped.subject_id, legs));
                progress = true;
            }
            self.prepped_matches = remaining_matches;
            if !self.prepped_matches.is_empty() {
                progress = true;
            }

            // --- Resolve deferred method calls ---
            if !self.prepped_method_calls.is_empty() {
                let mut remaining_methods = Vec::new();
                let method_calls: Vec<_> = std::mem::take(&mut self.prepped_method_calls)
                    .into_iter()
                    .collect();
                for (
                    id,
                    subject_id,
                    member_name,
                    generic_argument_ids,
                    mut argument_ids,
                    arguments_span,
                ) in method_calls
                {
                    let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
                    match subject_type {
                        Type::Struct(_) | Type::Enum(_) => {
                            let subject_type_name = match &subject_type {
                                Type::Struct(id) => self.structs.get(id).map(|x| x.name),
                                Type::Enum(id) => self.enums.get(id).map(|x| x.name),
                                _ => None,
                            }
                            .unwrap_or("?");
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
                                        // Only a function (or intrinsic) whose
                                        // first parameter is `self` is callable
                                        // as a method.
                                        let first_parameter_id = match self
                                            .get_entity_by_id(*member_id)
                                        {
                                            Expr::Function(function_id) => {
                                                self.functions.get(function_id).and_then(
                                                    |function| function.parameters.get(0).copied(),
                                                )
                                            }
                                            Expr::ExternalFunction(external_function_id) => self
                                                .external_functions
                                                .get(external_function_id)
                                                .and_then(|external| {
                                                    external.parameters.get(0).copied()
                                                }),
                                            _ => None,
                                        };
                                        first_parameter_id.and_then(|parameter_id| {
                                            let parameter =
                                                self.parameters.get(&parameter_id).unwrap();
                                            (parameter.name == "self").then_some(*member_id)
                                        })
                                    })
                                });
                            match member_id {
                                Some(member_id) => {
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
                                None => {
                                    self.diagnostics.push(Error {
                                        span: arguments_span,
                                        msg: format!(
                                            "'{}' has no method '{}'",
                                            subject_type_name, member_name
                                        ),
                                    });
                                    self.expr_id_to_expr_map.insert(id, Expr::Error);
                                }
                            }
                            progress = true;
                        }
                        Type::Unresolved => {
                            remaining_methods.push((
                                id,
                                subject_id,
                                member_name,
                                generic_argument_ids,
                                argument_ids,
                                arguments_span,
                            ));
                        }
                        _ => {
                            self.diagnostics.push(Error {
                                span: arguments_span.clone(),
                                msg: format!("cannot call method on non-struct type"),
                            });
                            progress = true;
                        }
                    }
                }
                self.prepped_method_calls = remaining_methods;
                if !self.prepped_method_calls.is_empty() {
                    progress = true;
                }
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

                // The first value (with the annotation) grounds the variable's
                // type and must be ready. Later values — reassignments — may
                // refer to the variable itself (e.g. `i += 1`), so they are
                // checked only after the type has been grounded below.
                let first_ready = value_ids
                    .first()
                    .map(|&value_id| {
                        self.expr_id_to_expr_map.contains_key(&value_id)
                            && !matches!(
                                self.infer_type(value_id, &Type::Unknown, &HashMap::new()),
                                Type::Unresolved
                            )
                    })
                    .unwrap_or(true);
                if !first_ready {
                    remaining_vars.push(constraint);
                    continue;
                }

                let mut substitution_context = HashMap::new();
                let mut variable_type = initial_type_id.get_type(self);

                if let Some(&first_value_id) = value_ids.first() {
                    let value_type =
                        self.infer_type(first_value_id, &variable_type, &substitution_context);
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
                                span: **self.span_map.get(&first_value_id).unwrap(),
                                msg: format!(
                                    "Expected {}, but got {} instead.",
                                    expected_str, got_str
                                ),
                            });
                        }
                    }
                }

                // Ground the variable's type before checking reassignments so
                // self-referential values like `i + 1` can resolve.
                let var_type_id = variable_type.clone().get_type_id(self);
                self.variables.get_mut(&variable_id).unwrap().type_id = var_type_id;
                self.resolved_types.insert(variable_id, var_type_id);

                let mut deferred_value_ids = Vec::new();
                for &value_id in value_ids.iter().skip(1) {
                    if !self.expr_id_to_expr_map.contains_key(&value_id) {
                        deferred_value_ids.push(value_id);
                        continue;
                    }
                    let value_type =
                        self.infer_type(value_id, &variable_type, &substitution_context);
                    if matches!(value_type, Type::Unresolved) {
                        deferred_value_ids.push(value_id);
                        continue;
                    }
                    match self.reconcile_type(&value_type, &variable_type, &substitution_context) {
                        Some((_, bindings)) => {
                            for (cid, tid) in bindings {
                                substitution_context.insert(cid, tid);
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
                if !deferred_value_ids.is_empty() {
                    remaining_vars.push(VariableConstraint {
                        variable_id,
                        initial_type_id: var_type_id,
                        value_ids: deferred_value_ids,
                    });
                }
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

                // Defer until the subject's entity is resolved and its type
                // can be inferred (a local pointing at a function, a static
                // accessor resolved to a module member, etc.).
                if !self.expr_id_to_expr_map.contains_key(&subject_id) {
                    remaining_calls.push(constraint);
                    continue;
                }
                let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
                if matches!(subject_type, Type::Unresolved) {
                    remaining_calls.push(constraint);
                    continue;
                }

                // Calling a closure-typed value, e.g. `(self.fn)()` or a
                // closure stored in a variable: type check the arguments
                // against the closure's parameter types.
                if let Type::Closure(parameter_type_ids, _) = subject_type {
                    if argument_ids.len() != parameter_type_ids.len() {
                        self.diagnostics.push(Error {
                            span: arguments_span,
                            msg: format!(
                                "Expected {} {}, but got {} instead.",
                                parameter_type_ids.len(),
                                plural(parameter_type_ids.len(), "argument", "arguments"),
                                argument_ids.len()
                            ),
                        });
                        continue;
                    }
                    let substitution_context = HashMap::new();
                    let mut deferred = false;
                    for (i, parameter_type_id) in parameter_type_ids.iter().enumerate() {
                        let parameter_type = parameter_type_id.get_type(self);
                        let argument_id = *argument_ids.get(i).unwrap();
                        let argument_type =
                            self.infer_type(argument_id, &parameter_type, &substitution_context);
                        if matches!(argument_type, Type::Unresolved) {
                            remaining_calls.push(CallSubjectConstraint {
                                call_id,
                                subject_id,
                                generic_argument_ids: generic_argument_ids.clone(),
                                argument_ids: argument_ids.clone(),
                                arguments_span,
                            });
                            deferred = true;
                            break;
                        }
                        if self
                            .reconcile_type(&argument_type, &parameter_type, &substitution_context)
                            .is_none()
                        {
                            let expected =
                                self.pretty_print_type(&parameter_type, &substitution_context);
                            let got = self.pretty_print_type(&argument_type, &substitution_context);
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&argument_id).unwrap(),
                                msg: format!("Expected {}, but got {} instead.", expected, got),
                            });
                        }
                    }
                    if !deferred {
                        self.function_calls.insert(
                            call_id,
                            FunctionCall {
                                id: call_id,
                                subject_id,
                                generic_argument_ids: generic_argument_ids.clone(),
                                argument_ids: argument_ids.clone(),
                                arguments_span,
                            },
                        );
                        self.expr_id_to_expr_map
                            .insert(call_id, Expr::Call(call_id));
                        progress = true;
                    }
                    continue;
                }

                let subject_expr = self.get_entity_by_id(subject_id);
                match subject_expr {
                    Expr::Local(target_id) => {
                        let target = self.get_entity_by_id(*target_id);
                        // A variant constructor call, e.g. `Some(1)`: the
                        // arguments are checked against the variant's declared
                        // data types and the call produces the enum value.
                        if let Expr::EnumVariant(enum_id, variant_index) = target {
                            let enum_id = *enum_id;
                            let variant_index = *variant_index;
                            let data_type_ids = self.enums.get(&enum_id).unwrap().variants
                                [variant_index]
                                .data_type_ids
                                .clone();
                            if argument_ids.len() != data_type_ids.len() {
                                self.diagnostics.push(Error {
                                    span: arguments_span,
                                    msg: format!(
                                        "Expected {} {}, but got {} instead.",
                                        data_type_ids.len(),
                                        plural(data_type_ids.len(), "argument", "arguments"),
                                        argument_ids.len()
                                    ),
                                });
                                continue;
                            }
                            let substitution_context = HashMap::new();
                            let mut deferred = false;
                            for (i, data_type_id) in data_type_ids.iter().enumerate() {
                                let data_type = data_type_id.get_type(self);
                                let argument_id = *argument_ids.get(i).unwrap();
                                let argument_type =
                                    self.infer_type(argument_id, &data_type, &substitution_context);
                                if matches!(argument_type, Type::Unresolved) {
                                    remaining_calls.push(CallSubjectConstraint {
                                        call_id,
                                        subject_id,
                                        generic_argument_ids: generic_argument_ids.clone(),
                                        argument_ids: argument_ids.clone(),
                                        arguments_span,
                                    });
                                    deferred = true;
                                    break;
                                }
                                if self
                                    .reconcile_type(
                                        &argument_type,
                                        &data_type,
                                        &substitution_context,
                                    )
                                    .is_none()
                                {
                                    let expected =
                                        self.pretty_print_type(&data_type, &substitution_context);
                                    let got = self
                                        .pretty_print_type(&argument_type, &substitution_context);
                                    self.diagnostics.push(Error {
                                        span: **self.span_map.get(&argument_id).unwrap(),
                                        msg: format!(
                                            "Expected {}, but got {} instead.",
                                            expected, got
                                        ),
                                    });
                                }
                            }
                            if !deferred {
                                self.function_calls.insert(
                                    call_id,
                                    FunctionCall {
                                        id: call_id,
                                        subject_id,
                                        generic_argument_ids: generic_argument_ids.clone(),
                                        argument_ids: argument_ids.clone(),
                                        arguments_span,
                                    },
                                );
                                self.expr_id_to_expr_map
                                    .insert(call_id, Expr::Call(call_id));
                                progress = true;
                            }
                            continue;
                        }
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
                                    self.function_calls.insert(
                                        call_id,
                                        FunctionCall {
                                            id: call_id,
                                            subject_id,
                                            generic_argument_ids: generic_argument_ids.clone(),
                                            argument_ids: argument_ids.clone(),
                                            arguments_span,
                                        },
                                    );
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
                                self.function_calls.insert(
                                    call_id,
                                    FunctionCall {
                                        id: call_id,
                                        subject_id,
                                        generic_argument_ids: generic_argument_ids.clone(),
                                        argument_ids: argument_ids.clone(),
                                        arguments_span,
                                    },
                                );
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
        for prepped in &self.prepped_matches {
            self.diagnostics.push(Error {
                span: prepped.span,
                msg: "type of match expression could not be resolved".to_string(),
            });
        }

        // Clear processed constraints
        self.struct_initializer_constraints.clear();
        self.field_accessor_constraints.clear();
        self.variable_constraints.clear();
        self.call_subject_constraints.clear();
        self.prepped_matches.clear();
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
                // Built-in primitive structs read as plain types (`i32`), not
                // `struct i32`, in diagnostics.
                if self
                    .primitive_struct_ids
                    .values()
                    .any(|prim_id| prim_id == id)
                {
                    buf.push_str(&format!("type {}", struct_.name));
                } else {
                    buf.push_str(&format!("struct {}", struct_.name));
                }
            }

            Type::Trait(id) => {
                let trait_ = self.traits.get(id).unwrap();
                buf.push_str(&format!("trait {}", trait_.name));
            }

            Type::Enum(id) => {
                let enum_ = self.enums.get(id).unwrap();
                buf.push_str(&format!("enum {}", enum_.name));
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

            Type::Primitive(PrimitiveType::List(item_id)) => {
                buf.push_str("type List<");
                let item_type = item_id.get_type(self);
                let item_str = self.pretty_print_type(&item_type, substitution);
                buf.push_str(&item_str);
                buf.push('>');
            }

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
    pub generic_static_accessors: HashMap<Id, (TypeId, &'src str)>,
    pub global_scope_id: Id,
    pub implementations: Vec<Implementation<'src>>,
    // The builtin `List` intrinsics, special-cased in codegen (`new` -> `[]`,
    // `push` -> `subject.push(..)`).
    pub list_new_fn_id: Id,
    pub list_push_fn_id: Id,
    // The `std` `panic` intrinsic (if loaded); its calls lower to a `throw`.
    pub panic_fn_id: Option<Id>,
    // The source `enum bool`; the transformer lowers its variants/patterns to
    // native JS booleans rather than the array form used by other enums.
    pub bool_enum_id: Option<Id>,
    pub module_id_by_name: HashMap<&'src str, Id>,
    pub modules: IndexMap<Id, Module<'src>>,
    pub reference_count: HashMap<Id, u32>,
    pub scopes: IndexMap<Id, Scope<'src>>,
    pub span_map: HashMap<Id, &'src Span>,
    pub structs: IndexMap<Id, Struct<'src>>,
    pub type_id_to_type_map: HashMap<TypeId, Type>,
    pub variables: IndexMap<Id, Variable<'src>>,
}

/// Lexes and parses a Vilan source file into an AST, leaking the source and the
/// resulting tree so they live for the whole compilation. Used to pull the
/// `std` package's modules in from source. Returns `None` if the file can't be
/// read or fails to lex/parse.
fn load_package_module(path: &str) -> Option<&'static Spanned<NodeList<'static>>> {
    use chumsky::prelude::*;
    // The source is leaked so the parsed tree (which borrows it) can live for
    // the whole compilation. The token vector is transient — the AST holds
    // `&'static str` slices into the source, not into the tokens.
    let source: &'static str = Box::leak(std::fs::read_to_string(path).ok()?.into_boxed_str());
    let tokens = crate::lexer::lexer().parse(source).into_output()?;
    let end = source.len();
    let (root, _file_span) = crate::parser::parser()
        .map_with(|ast, e| (ast, e.span()))
        .parse(
            tokens
                .as_slice()
                .map((end..end).into(), |(token, span)| (token, span)),
        )
        .into_output()?;
    Some(Box::leak(Box::new(root)))
}

/// Builds the path to a module file in the hardcoded `std` package.
fn std_module_path(file: &str) -> String {
    format!(
        "{}/src/vilan-source/std/src/{}",
        env!("CARGO_MANIFEST_DIR"),
        file
    )
}

/// Collects the names of package modules referenced via `<root>::<module>::..`
/// in a node list's top-level `import`/`use`/`export import` statements, so the
/// loader can pull them in transitively. `root` is `pkg` when scanning a std
/// module's own siblings, or `std` when scanning the entry program for the std
/// submodules it addresses by path (e.g. `import std::option::Option`).
fn collect_module_refs<'a>(nodes: &'a NodeList<'a>, root: &str) -> Vec<&'a str> {
    let mut modules = Vec::new();
    for (node, _span) in nodes {
        let branch = match node {
            Node::Import(branch) | Node::Use(branch) => Some(branch),
            Node::Export(inner) => match &inner.0 {
                Node::Import(branch) | Node::Use(branch) => Some(branch),
                _ => None,
            },
            _ => None,
        };
        if let Some(branch) = branch {
            let mut entries = Vec::new();
            flatten_namespace_branch(branch, Vec::new(), &mut entries);
            for (path, _leaf) in entries {
                if path.first() == Some(&root) && path.len() >= 2 {
                    modules.push(path[1]);
                }
            }
        }
    }
    modules
}

pub fn analyze<'src>(nodes: &'src Spanned<NodeList<'src>>) -> Program<'src> {
    let mut analyzer = Analyzer::new();
    let global_scope = analyzer.create_scope(None);
    let global_scope_id = analyzer.push_scope(global_scope);
    // Scalar primitives are built-in structs, reachable as the bare `i32` (the
    // prelude, here the global scope) and — since the module scopes below are
    // children of the global scope — as `std::i32`.
    // Scalar number/string primitives are migrated to source (`number.vl`,
    // `string.vl`); `bool` is the source `enum bool` (captured below after
    // `boolean.vl` loads). Only `null` remains hardcoded.
    for name in ["null"] {
        let id = analyzer.register_primitive_struct(name);
        analyzer
            .mut_scope_for_scope_id(global_scope_id)
            .name_to_id_map
            .insert(name, id);
    }

    // `List` is a builtin generic container. Its element type is erased like
    // other generic nominal types; `new` and `push` are intrinsics special-cased
    // by the transformer (`List::new()` -> `[]`, `list.push(x)` -> `list.push(x)`).
    let list_struct_id = analyzer.register_primitive_struct("List");
    analyzer
        .mut_scope_for_scope_id(global_scope_id)
        .name_to_id_map
        .insert("List", list_struct_id);
    let list_type_id = Type::Struct(list_struct_id).get_type_id(&mut analyzer);
    let any_type_id = Type::Any.get_type_id(&mut analyzer);
    let void_type_id = Type::Void.get_type_id(&mut analyzer);

    // `fun new(): List`
    let list_new_fn_id = analyzer.new_entity_id();
    analyzer.external_functions.insert(
        list_new_fn_id,
        ExternalFunction {
            id: list_new_fn_id,
            name: "new",
            generic_parameter_constraint_ids: Vec::new(),
            parameters: Vec::new(),
            return_type_id: list_type_id,
            call_count: 0,
        },
    );
    analyzer
        .expr_id_to_expr_map
        .insert(list_new_fn_id, Expr::ExternalFunction(list_new_fn_id));

    // `fun push(self, item): void`
    let list_push_fn_id = analyzer.new_entity_id();
    let push_self_parameter_id = analyzer.new_entity_id();
    analyzer.parameters.insert(
        push_self_parameter_id,
        Parameter {
            id: push_self_parameter_id,
            function_id: list_push_fn_id,
            name: "self",
            type_id: list_type_id,
        },
    );
    let push_item_parameter_id = analyzer.new_entity_id();
    analyzer.parameters.insert(
        push_item_parameter_id,
        Parameter {
            id: push_item_parameter_id,
            function_id: list_push_fn_id,
            name: "item",
            type_id: any_type_id,
        },
    );
    analyzer.external_functions.insert(
        list_push_fn_id,
        ExternalFunction {
            id: list_push_fn_id,
            name: "push",
            generic_parameter_constraint_ids: Vec::new(),
            parameters: vec![push_self_parameter_id, push_item_parameter_id],
            return_type_id: void_type_id,
            call_count: 0,
        },
    );
    analyzer
        .expr_id_to_expr_map
        .insert(list_push_fn_id, Expr::ExternalFunction(list_push_fn_id));

    // Both intrinsics need a function type so calls infer their return type.
    for function_id in [list_new_fn_id, list_push_fn_id] {
        let function_type_id = analyzer.new_type_id();
        analyzer
            .type_id_to_type_map
            .insert(function_type_id, Type::Function(function_id));
        analyzer
            .expr_id_to_type_id_map
            .insert(function_id, function_type_id);
    }

    let mut list_declarations = IndexMap::new();
    list_declarations.insert("new", list_new_fn_id);
    list_declarations.insert("push", list_push_fn_id);
    analyzer.implementations.push(Implementation {
        subject: list_type_id,
        declarations: list_declarations,
    });

    // --- Load the `std` package from source ---
    // `pkg` aliases this package's sibling modules so `pkg::<module>::item`
    // resolves; module scopes are children of the global scope so they can see
    // the builtins (e.g. `str` in `io`'s `panic(message: str)`).
    let pkg_scope = analyzer.create_scope(None);
    let pkg_scope_id = analyzer.push_scope(pkg_scope);
    let pkg_module_id = analyzer.new_entity_id();
    analyzer.modules.insert(
        pkg_module_id,
        Module {
            id: pkg_module_id,
            name: "pkg",
            body: (Vec::new(), pkg_scope_id),
        },
    );
    analyzer
        .expr_id_to_expr_map
        .insert(pkg_module_id, Expr::Module(pkg_module_id));
    analyzer.module_id_by_name.insert("pkg", pkg_module_id);

    // `std` is the package root, integrated from `lib.vl`. Its re-exports bind
    // into this scope; childing it to the global scope lets `std::i32`,
    // `std::List`, ... reach the builtins.
    let std_scope = analyzer.create_scope(Some(global_scope_id));
    let std_scope_id = analyzer.push_scope(std_scope);
    let std_module_id = analyzer.new_entity_id();
    analyzer.modules.insert(
        std_module_id,
        Module {
            id: std_module_id,
            name: "std",
            body: (Vec::new(), std_scope_id),
        },
    );
    analyzer
        .expr_id_to_expr_map
        .insert(std_module_id, Expr::Module(std_module_id));
    analyzer.module_id_by_name.insert("std", std_module_id);

    // Load `lib.vl` plus every module reachable through `pkg::` references,
    // transitively. Each becomes a module registered in the `pkg` namespace;
    // bodies are walked after all are registered so cross-module references
    // resolve during `build()`.
    let lib_ast = load_package_module(&std_module_path("lib.vl"));
    let mut module_scopes: HashMap<&str, Id> = HashMap::new();
    let mut loaded: Vec<(&str, &Spanned<NodeList>, Id)> = Vec::new();
    let mut to_load: Vec<&str> = lib_ast
        .map(|ast| collect_module_refs(&ast.0, "pkg"))
        .unwrap_or_default();
    // The entry program addresses std submodules by path (`std::option::..`),
    // so its imports also seed the reachable set. Names that aren't modules
    // (e.g. the `print` in `std::print`) simply find no file and are skipped.
    to_load.extend(collect_module_refs(&nodes.0, "std"));
    // `bool` is a core primitive, so `boolean.vl` is always loaded (it is
    // dependency-free, so this pulls in nothing else).
    to_load.push("boolean");
    while let Some(name) = to_load.pop() {
        if module_scopes.contains_key(name) {
            continue;
        }
        let Some(ast) = load_package_module(&std_module_path(&format!("{name}.vl"))) else {
            continue;
        };
        let module_scope = analyzer.create_scope(Some(global_scope_id));
        let module_scope_id = analyzer.push_scope(module_scope);
        let module_id = analyzer.new_entity_id();
        analyzer.modules.insert(
            module_id,
            Module {
                id: module_id,
                name,
                body: (Vec::new(), module_scope_id),
            },
        );
        analyzer
            .expr_id_to_expr_map
            .insert(module_id, Expr::Module(module_id));
        analyzer
            .mut_scope_for_scope_id(pkg_scope_id)
            .name_to_id_map
            .insert(name, module_id);
        // Also expose the module under the `std` package root so it is
        // addressable by path from outside (`std::<module>::item`), mirroring
        // the internal `pkg::<module>` reference.
        analyzer
            .mut_scope_for_scope_id(std_scope_id)
            .name_to_id_map
            .insert(name, module_id);
        module_scopes.insert(name, module_scope_id);
        to_load.extend(collect_module_refs(&ast.0, "pkg"));
        loaded.push((name, ast, module_scope_id));
    }
    for (_name, ast, module_scope_id) in &loaded {
        analyzer.walk_expr_nodes(&ast.0, *module_scope_id);
    }
    if let Some(lib_ast) = lib_ast {
        analyzer.walk_expr_nodes(&lib_ast.0, std_scope_id);
    }
    // Remember `panic` so its calls can be typed as never and lowered to a throw.
    if let Some(io_scope_id) = module_scopes.get("io") {
        analyzer.panic_fn_id = analyzer
            .scopes
            .get(io_scope_id)
            .and_then(|scope| scope.name_to_id_map.get("panic").copied());
    }

    // Bind source-defined primitive structs into the literal registry (so number
    // and string literals infer the right type) and the global scope (so bare
    // `str`, `i32`, ... annotations resolve), replacing the hardcoded ones.
    for (primitive, module) in [
        ("str", "string"),
        ("i32", "number"),
        ("u32", "number"),
        ("f64", "number"),
        ("BigInt", "number"),
    ] {
        let id = module_scopes
            .get(module)
            .and_then(|scope_id| analyzer.scopes.get(scope_id))
            .and_then(|scope| scope.name_to_id_map.get(primitive).copied());
        if let Some(id) = id {
            analyzer.primitive_struct_ids.insert(primitive, id);
            analyzer
                .mut_scope_for_scope_id(global_scope_id)
                .name_to_id_map
                .insert(primitive, id);
        }
    }

    // Capture the source `enum bool` so boolean literals/comparisons type as it,
    // and bind it into the global scope so bare `bool` annotations resolve.
    if let Some(bool_id) = module_scopes
        .get("boolean")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("bool").copied())
    {
        analyzer.bool_enum_id = Some(bool_id);
        analyzer
            .mut_scope_for_scope_id(global_scope_id)
            .name_to_id_map
            .insert("bool", bool_id);
    }

    analyzer.walk_expr_nodes(&nodes.0, global_scope_id);
    analyzer.build();
    Program {
        closures: analyzer.closures,
        diagnostics: analyzer.diagnostics,
        entity_map: analyzer.expr_id_to_expr_map,
        entity_scope_map: analyzer.expr_id_to_scope_id_map,
        function_calls: analyzer.function_calls,
        functions: analyzer.functions,
        generic_static_accessors: analyzer.generic_static_accessors,
        global_scope_id,
        implementations: analyzer.implementations,
        list_new_fn_id,
        list_push_fn_id,
        panic_fn_id: analyzer.panic_fn_id,
        bool_enum_id: analyzer.bool_enum_id,
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
