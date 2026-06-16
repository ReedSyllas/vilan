use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::error::Error;
use crate::id::Id;
use crate::node::{
    BinaryOp, Convention, ExternBinding, GenericParameters, ImportBranch, Node, NodeIfBranch,
    NodeList, Pattern,
};
use crate::span::{Span, Spanned};
use crate::type_::{SubstitutionContext, Type, TypeId};
use crate::util::plural;

#[derive(Clone, Debug)]
pub enum Expr<'src> {
    // An assignment to a local: target accessor and the (possibly desugared,
    // e.g. `x + v` for `x += v`) value expression.
    Assignment(Id, Id),
    // `async <body>` — the body, lowered to an invoked async arrow, yielding a
    // promise. The id is the async-block closure (in `closures`).
    Async(Id),
    // `await <inner>` — resolve the inner promise; forces the enclosing function
    // async at code generation.
    Await(Id),
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
    // `for item in iterable` — the iterable, the optional element binding (None
    // for `_`), and the body. Lowers to a native JS `for...of`.
    ForEach(Id, Option<Id>, (Vec<Id>, Id)),
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
    // A unary prefix operator and its operand. Only `!` (logical not) exists.
    Unary(char, Id),
    // `&x` / `&mut x` — a view of a place (the operand). The bool is whether the
    // view is writable (`&mut`).
    Reference(Id, bool),
    // `*v` — read or write through a view (the operand).
    Dereference(Id),
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
    // An optional `if` guard, evaluated (with the pattern's captures in scope)
    // after the pattern matches; the leg only fires when it holds. An or-pattern
    // (`"y", "" =>`) is expanded to one leg per alternative, sharing the guard
    // and body.
    pub guard: Option<Id>,
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
    // A literal value test: the matched value equals this literal expression.
    Literal(Id),
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
    /// Whether the source provided a body. A trait method without one is a
    /// signature-only requirement (impls must supply it); with one it is a
    /// default method (impls may inherit it). Always true outside a trait.
    pub has_body: bool,
    pub call_count: u32,
    /// Declared `async`, or inferred async (its body awaits). Such a function
    /// compiles to a JS `async function`, and calls to it are implicitly
    /// awaited. Set by the async inference pass.
    pub is_async: bool,
}

#[derive(Debug)]
pub struct ExternalFunction<'src> {
    pub id: Id,
    pub name: &'src str,
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub parameters: Vec<Id>,
    pub return_type_id: TypeId,
    // The `@extern(..)` host binding, if any — lowers calls to a JS
    // import/call, method, or property access.
    pub extern_binding: Option<ExternBinding<'src>>,
    pub call_count: u32,
    /// Declared `async` — a promise-returning host function. Calls to it are
    /// implicitly awaited.
    pub is_async: bool,
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
    /// How the parameter receives its argument (rule 3). Recorded now; the
    /// default flip and mutability checking consume it later.
    pub convention: Convention,
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
    // A C-like enum: every variant is data-less and at least one has an explicit
    // discriminant (`enum Ordering { Less = -1, Equal = 0, Greater = 1 }`). Such
    // enums lower to their integer discriminant rather than the `[index, ..data]`
    // array form, so they compare and equality-test as plain numbers.
    pub is_numeric: bool,
}

#[derive(Debug)]
pub struct EnumVariantDeclaration<'src> {
    pub name: &'src str,
    pub data_type_ids: Vec<TypeId>,
    // The variant's integer value, used for `is_numeric` enums: the explicit
    // discriminant, or the previous variant's value plus one (C-style), from 0.
    pub discriminant: i64,
}

// A match pattern as walked, with variant names not yet resolved.
#[derive(Debug)]
enum WalkPattern<'src> {
    Wildcard,
    Binding(Id),
    // A variant path (`["Some"]`, `["Signal", "Quit"]`) and optional payload.
    Variant(Vec<&'src str>, Span, Option<Vec<WalkPattern<'src>>>),
    Tuple(Span, Vec<WalkPattern<'src>>),
    // A literal value, walked to its expression id (for type-checking + codegen).
    Literal(Id),
}

// A walked match leg: its patterns (an or-pattern when more than one), optional
// guard, and body, each scoped to the leg's captures.
#[derive(Debug)]
struct WalkLeg<'src> {
    patterns: Vec<WalkPattern<'src>>,
    guard: Option<Id>,
    body: Id,
}

// A match expression awaiting subject and pattern resolution.
#[derive(Debug)]
struct PreppedMatch<'src> {
    id: Id,
    subject_id: Id,
    scope_id: Id,
    legs: Vec<WalkLeg<'src>>,
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
    /// The traits this impl provides (`impl Point with Eq + Ord` -> [Eq, Ord]),
    /// resolved during the conformance check. Lets a method call on the subject
    /// fall back to a trait's inherited default methods.
    pub trait_ids: Vec<Id>,
}

#[derive(Debug)]
pub struct Trait<'src> {
    pub id: Id,
    pub name: &'src str,
    /// The members the trait declares, keyed by name. For a required method
    /// without a default body these point at signature-only functions.
    pub declarations: IndexMap<&'src str, Id>,
    /// Supertraits (`trait Ord with Eq + PartialOrd`), as the type ids of the
    /// `with` clause. A type implementing this trait must also satisfy these,
    /// and their members are inherited for method resolution.
    pub supertraits: Vec<TypeId>,
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
    // Index into `implementations` of the impl this check belongs to, so the
    // resolved trait id can be recorded back onto it.
    pub implementation_index: usize,
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
    // Method calls (by call id) that must re-dispatch to the receiver's concrete
    // type at codegen, by member name: a trait default called on a concrete value
    // (`Some(type)`) — Gap E — or a `self`/trait-typed call inside a default body
    // (`None`, dispatched on the type the default is being specialized for).
    trait_method_dispatch: HashMap<Id, (Option<TypeId>, &'src str)>,
    // All trait-bound type ids of a generic parameter (`T: A + B` -> [A, B]),
    // keyed by the parameter's constraint id (its first bound, which is its
    // `Type::Generic` identity). Stored unresolved (bounds resolve in `build()`,
    // not during the walk); member resolution on a `T`-typed value searches every
    // bound.
    generic_bounds: HashMap<TypeId, Vec<TypeId>>,
    // For an impl whose subject is a generic application (`impl Option<(type T,
    // type U)>`), the impl body scope -> (subject type id, the subject's walked
    // generic arguments). Lets `self`'s variant patterns substitute the subject
    // enum's declared parameters for these args.
    impl_subject_args: HashMap<Id, (TypeId, Vec<TypeId>)>,
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
    // A fresh element-type inference slot (an `Unknown` type id) per `List::new()`
    // call, keyed by the call id so the slot stays stable across re-inference.
    // `push` unifies it with the pushed value's type, so a built-up list's
    // element is inferred (`List::new(); push(p: Point)` -> `List<Point>`).
    list_element_slots: HashMap<Id, TypeId>,
    // Pending element-slot unifications from `push` calls: (element slot, the
    // pushed argument's expression id). Resolved in the constraint loop.
    prepped_slot_unifications: Vec<(TypeId, Id)>,
    // Assignments awaiting local resolution: (target accessor id, value id).
    prepped_assignments: Vec<(Id, Id)>,
    prepped_field_accessors: Vec<(Id, Id, &'src str)>,
    prepped_imports: Vec<(Vec<&'src str>, &'src str, Id, Span)>,
    prepped_locals: Vec<(Id, &'src str)>,
    prepped_is: Vec<PreppedIs<'src>>,
    prepped_matches: Vec<PreppedMatch<'src>>,
    prepped_method_calls: Vec<(Id, Id, &'src str, Vec<TypeId>, Vec<Id>, Span)>,
    // `for x in iterable` expressions, as (for-each id, iterable id), resolved
    // after typing to decide native `for...of` vs the Iterator-protocol loop.
    prepped_for_each: Vec<(Id, Id)>,
    // `for x in iterable` element bindings, as (item variable id, iterable id),
    // resolved in the constraint loop: the item takes the iterable's element
    // type (`List<i32>` -> `i32`), so the body can use it concretely.
    prepped_for_each_items: Vec<(Id, Id)>,
    // Method calls whose arguments need checking against the method's parameters,
    // as (member id, explicit argument ids). A wired method call isn't checked by
    // the free-call machinery, so this is a dedicated deferred pass (no subject
    // re-resolution — that recurses); it also drives bidirectional closure-arg
    // inference. The method's first parameter is `self`, so args align at +1.
    prepped_method_arg_checks: Vec<(Id, Vec<Id>)>,
    // For-each loops whose iterable is a custom iterator: the resolved `next`
    // method id, so codegen emits a `next()`/`Some`-matching loop instead.
    for_each_next: HashMap<Id, Id>,
    // Arithmetic binary expressions (`a + b`), as (binary id, op, lhs id),
    // resolved after typing to decide native JS arithmetic vs an operator-trait
    // method call (`Add::add`, ...).
    prepped_binary_ops: Vec<(Id, BinaryOp, Id)>,
    // Arithmetic binary expressions whose left operand's type implements the
    // matching operator trait: the resolved method id, so codegen emits
    // `add(lhs, rhs)` instead of `lhs + rhs`.
    binary_op_dispatch: HashMap<Id, Id>,
    // Method calls (by call id) on a generic impl whose generic parameters bind
    // to concrete types from the receiver (`xs.sum()` on `List<i32>` binds the
    // impl's `T` to `i32`): the resulting substitution, so codegen emits a
    // monomorphized instance of the method body (e.g. `T::default()` -> `0`).
    method_call_substitution: HashMap<Id, SubstitutionContext>,
    prepped_static_accessors: Vec<(Id, TypeId, &'src str)>,
    prepped_struct_initializers:
        Vec<(Id, &'src str, Vec<TypeId>, Vec<(&'src str, Id, Span)>, Span)>,
    prepped_trait_impls: Vec<TraitImplCheck<'src>>,
    // Deferred named type references: (target type id, name, scope, span, the
    // walked generic argument type ids). The arguments parameterize the resolved
    // nominal type (`Option<i32>` -> `Enum(option_id, [i32])`); empty for a bare
    // name or a generic parameter.
    prepped_type_locals: Vec<(TypeId, &'src str, Id, Span, Vec<TypeId>)>,
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
    // The `std::promise` `Promise<T>` struct, if loaded. `async e` types as
    // `Promise<type of e>`, and `await p` unwraps a `Promise<T>` to `T`.
    promise_struct_id: Option<Id>,
}

static EMPTY_SPAN: Span = Span {
    start: 0,
    end: 0,
    context: (),
};

// Flattens an `import`/`use` tree into (path, leaf-name) pairs, e.g.
// `a::{ b, c::d }` becomes `([a], b)` and `([a, c], d)`.
/// Whether `op` is an arithmetic operator that a type can overload by
/// implementing the corresponding `std::operators` trait.
fn is_overloadable_operator(op: BinaryOp) -> bool {
    operator_trait_method(op).is_some()
}

/// The `(trait name, method name)` an arithmetic operator dispatches to, e.g.
/// `+` -> `("Add", "add")`. Returns `None` for comparison/logical operators,
/// which are not overloadable.
fn operator_trait_method(op: BinaryOp) -> Option<(&'static str, &'static str)> {
    match op {
        BinaryOp::Add => Some(("Add", "add")),
        BinaryOp::Sub => Some(("Sub", "sub")),
        BinaryOp::Mul => Some(("Mul", "mul")),
        BinaryOp::Div => Some(("Div", "div")),
        _ => None,
    }
}

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
            trait_method_dispatch: HashMap::new(),
            generic_bounds: HashMap::new(),
            impl_subject_args: HashMap::new(),
            implementations: Vec::new(),
            module_id_by_name: HashMap::new(),
            modules: IndexMap::new(),
            parameters: IndexMap::new(),
            primitive_struct_ids: HashMap::new(),
            bool_enum_id: None,
            list_element_slots: HashMap::new(),
            prepped_slot_unifications: Vec::new(),
            prepped_assignments: Vec::new(),
            prepped_field_accessors: Vec::new(),
            prepped_imports: Vec::new(),
            prepped_locals: Vec::new(),
            prepped_is: Vec::new(),
            prepped_matches: Vec::new(),
            prepped_method_calls: Vec::new(),
            prepped_for_each: Vec::new(),
            prepped_for_each_items: Vec::new(),
            prepped_method_arg_checks: Vec::new(),
            for_each_next: HashMap::new(),
            prepped_binary_ops: Vec::new(),
            binary_op_dispatch: HashMap::new(),
            method_call_substitution: HashMap::new(),
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
            promise_struct_id: None,
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

    /// Whether `member_id` is callable as a method — a function (or intrinsic)
    /// whose first parameter is named `self`.
    fn is_self_method(&self, member_id: Id) -> bool {
        let first_parameter_id = match self.get_entity_by_id(member_id) {
            Expr::Function(function_id) => self
                .functions
                .get(function_id)
                .and_then(|function| function.parameters.first().copied()),
            Expr::ExternalFunction(external_function_id) => self
                .external_functions
                .get(external_function_id)
                .and_then(|external| external.parameters.first().copied()),
            _ => None,
        };
        first_parameter_id
            .and_then(|parameter_id| self.parameters.get(&parameter_id))
            .map(|parameter| parameter.name == "self")
            .unwrap_or(false)
    }

    /// Resolves a method `member_name` callable on a concrete `subject_type`
    /// (a struct or enum) by searching its implementations.
    fn method_member_in_impls(&self, subject_type: &Type, member_name: &str) -> Option<Id> {
        self.method_member_impl_subject(subject_type, member_name)
            .map(|(member_id, _)| member_id)
    }

    /// Like `method_member_in_impls`, but also returns the matching impl's
    /// subject type id. Reconciling that subject against the concrete receiver
    /// binds the impl's generic parameters (`impl List<T>` against `List<i32>`
    /// binds `T = i32`), which monomorphizes the method body.
    fn method_member_impl_subject(
        &self,
        subject_type: &Type,
        member_name: &str,
    ) -> Option<(Id, TypeId)> {
        self.implementations
            .iter()
            .filter(|implementation| {
                self.compare_type(
                    subject_type,
                    &implementation.subject.get_type(self),
                    &HashMap::new(),
                )
            })
            .find_map(|implementation| {
                let member_id = implementation
                    .declarations
                    .get(member_name)
                    .copied()
                    .filter(|member_id| self.is_self_method(*member_id))?;
                Some((member_id, implementation.subject))
            })
    }

    /// Resolves the operator method for `op` on a value of `subject_type` — the
    /// `add`/`sub`/`mul`/`div` declared by an `impl Subject with Add/...`. Used to
    /// dispatch `a + b` to `Add::add(a, b)` when the left operand's type
    /// overloads the operator; returns `None` (so codegen keeps native JS
    /// arithmetic) for numbers and any type without the matching operator impl.
    fn operator_method(&self, op: BinaryOp, subject_type: &Type) -> Option<Id> {
        let (trait_name, method_name) = operator_trait_method(op)?;
        self.implementations
            .iter()
            .filter(|implementation| {
                self.compare_type(
                    subject_type,
                    &implementation.subject.get_type(self),
                    &HashMap::new(),
                )
            })
            .filter(|implementation| {
                implementation.trait_ids.iter().any(|trait_id| {
                    self.traits
                        .get(trait_id)
                        .map(|trait_| trait_.name == trait_name)
                        .unwrap_or(false)
                })
            })
            .find_map(|implementation| implementation.declarations.get(method_name).copied())
    }

    /// Resolves a method `member_name` declared by `trait_id` — a default method
    /// or a signature-only requirement — callable on a `Self`-typed receiver.
    /// Used for abstract receivers: `Self` in a trait default method, a
    /// trait-bounded generic, or a trait-typed value. Searches the trait and its
    /// supertraits, so e.g. a `T: Ord` value can call `eq` (from `PartialEq`).
    fn method_member_in_trait(&self, trait_id: Id, member_name: &str) -> Option<Id> {
        self.trait_with_supertraits(trait_id)
            .into_iter()
            .find_map(|id| {
                self.traits
                    .get(&id)
                    .and_then(|trait_| trait_.declarations.get(member_name).copied())
                    .filter(|member_id| self.is_self_method(*member_id))
            })
    }

    /// `trait_id` plus its transitive supertraits (each supertrait's `TypeId`
    /// resolved to a trait id). A trait's full interface — for method resolution
    /// and impl conformance — includes everything its supertraits declare.
    fn trait_with_supertraits(&self, trait_id: Id) -> Vec<Id> {
        let mut result = Vec::new();
        let mut stack = vec![trait_id];
        while let Some(id) = stack.pop() {
            if result.contains(&id) {
                continue;
            }
            result.push(id);
            if let Some(trait_) = self.traits.get(&id) {
                for supertrait_type_id in &trait_.supertraits {
                    if let Type::Trait(super_id) = supertrait_type_id.get_type(self) {
                        stack.push(super_id);
                    }
                }
            }
        }
        result
    }

    /// Resolves `member_name` as an inherited trait *default* method on a
    /// concrete `subject_type` — one the type's `impl ... with Trait` doesn't
    /// itself declare but a (super)trait provides with a body (Gap E). A
    /// non-default requirement is never inherited (conformance forces the impl
    /// to declare it, so `method_member_in_impls` finds it first).
    fn method_member_in_inherited_defaults(
        &self,
        subject_type: &Type,
        member_name: &str,
    ) -> Option<Id> {
        self.implementations
            .iter()
            .filter(|implementation| {
                self.compare_type(
                    subject_type,
                    &implementation.subject.get_type(self),
                    &HashMap::new(),
                )
            })
            .flat_map(|implementation| implementation.trait_ids.iter().copied())
            .find_map(|trait_id| {
                let member_id = self.method_member_in_trait(trait_id, member_name)?;
                self.member_has_default_body(member_id).then_some(member_id)
            })
    }

    /// Whether a trait member has a source-provided body (a default method), so
    /// an `impl` of the trait may inherit it rather than supply its own.
    fn member_has_default_body(&self, member_id: Id) -> bool {
        match self.get_entity_by_id(member_id) {
            Expr::Function(function_id) => self
                .functions
                .get(function_id)
                .map(|function| function.has_body)
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Wires a resolved method call: prepends the receiver as the first argument
    /// and records the `FunctionCall` behind an `Expr::Call`.
    fn wire_method_call(
        &mut self,
        id: Id,
        subject_id: Id,
        member_id: Id,
        generic_argument_ids: Vec<TypeId>,
        mut argument_ids: Vec<Id>,
        arguments_span: Span,
    ) {
        let member_local_id = self.new_entity_id();
        self.expr_id_to_expr_map
            .insert(member_local_id, Expr::Local(member_id));
        argument_ids.insert(0, subject_id);
        self.function_calls.insert(
            id,
            FunctionCall {
                id,
                subject_id: member_local_id,
                generic_argument_ids: generic_argument_ids.clone(),
                argument_ids: argument_ids.clone(),
                arguments_span,
            },
        );
        self.expr_id_to_expr_map.insert(id, Expr::Call(id));
        let _ = (generic_argument_ids, arguments_span);
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
        Type::Struct(self.primitive_struct_ids[name], Vec::new())
    }

    /// The boolean type — the source-defined `enum bool`. Falls back to
    /// `Unresolved` if `boolean.vl` has not been captured yet, so the constraint
    /// solver defers rather than panicking.
    fn bool_type(&self) -> Type {
        match self.bool_enum_id {
            Some(id) => Type::Enum(id, Vec::new()),
            None => Type::Unresolved,
        }
    }

    /// A scalar primitive (`i32`, `str`, ...) — backed by a JS value type, so
    /// assigning it never aliases and never needs a copy. Distinct from the
    /// *aggregate* primitives (`List`, `Context`), which lower to mutable JS
    /// arrays and do.
    fn is_scalar_primitive(&self, id: Id) -> bool {
        ["str", "i32", "u32", "f64", "BigInt", "null"]
            .iter()
            .any(|name| self.primitive_struct_ids.get(name) == Some(&id))
    }

    /// Whether a type lowers to a mutable JS aggregate (a struct, `List`,
    /// `Context`, or a tuple) — the types that alias under assignment and so
    /// need a semantic copy (rule 1). Scalars and `bool` do not.
    fn is_cloneable_aggregate(&self, type_: &Type) -> bool {
        match type_ {
            Type::Struct(id, _) => !self.is_scalar_primitive(*id),
            Type::Tuple(_) => true,
            _ => false,
        }
    }

    /// The resolved type of an expression, if known — the same two-map lookup
    /// `infer_type_path` begins with.
    fn type_of_expr(&self, expr_id: Id) -> Option<Type> {
        if let Some(type_id) = self.expr_id_to_type_id_map.get(&expr_id) {
            return Some(type_id.get_type(self));
        }
        self.resolved_types
            .get(&expr_id)
            .map(|type_id| type_id.get_type(self))
    }

    /// Whether an expression reads existing aggregate storage (a binding or a
    /// field) rather than producing a fresh value (a literal, constructor, or
    /// call). Only a place can alias, so only a place needs a copy.
    fn is_place_expr(&self, expr_id: Id) -> bool {
        matches!(
            self.expr_id_to_expr_map.get(&expr_id),
            Some(Expr::Local(_)) | Some(Expr::Field(_, _, _))
        )
    }

    /// Rule 3: if a place is a field/deref chain rooted in a readonly (bare or
    /// `&`) parameter, its name — used to reject mutation through it. `None` when
    /// the root is mutable (a `mut` local, or an `own` / `&mut` parameter) or is
    /// not a parameter at all. Bare parameters are readonly by default (the
    /// position-default-convention flip); `&mut` / `own` opt back into mutation.
    fn readonly_root_parameter(&self, expr_id: Id) -> Option<&'src str> {
        match self.expr_id_to_expr_map.get(&expr_id)? {
            Expr::Field(subject_id, _, _) => self.readonly_root_parameter(*subject_id),
            Expr::Dereference(operand_id) => self.readonly_root_parameter(*operand_id),
            Expr::Local(binding_id) => {
                let parameter = self.parameters.get(binding_id)?;
                matches!(parameter.convention, Convention::Bare | Convention::Ref)
                    .then_some(parameter.name)
            }
            _ => None,
        }
    }

    /// Rule 3 (position-default conventions): a write rooted in a readonly (bare
    /// or `&`) parameter is rejected — the author must declare it `&mut`. Runs
    /// after `build()`, once field accessors have resolved to `Expr::Field`, so
    /// the whole place chain is walkable.
    fn check_readonly_mutation(&mut self) {
        let assignment_targets: Vec<Id> = self
            .expr_id_to_expr_map
            .values()
            .filter_map(|expr| match expr {
                Expr::Assignment(target_id, _) => Some(*target_id),
                _ => None,
            })
            .collect();
        for target_id in assignment_targets {
            if let Some(name) = self.readonly_root_parameter(target_id) {
                self.diagnostics.push(Error {
                    span: **self.span_map.get(&target_id).unwrap_or(&&EMPTY_SPAN),
                    msg: format!(
                        "cannot mutate through readonly parameter '{name}'; declare it `&mut {name}` to allow mutation."
                    ),
                });
            }
        }
    }

    /// Rule 1 (value semantics): the value expressions that must be deep-copied
    /// at code generation, because they bind or assign an aggregate *place* that
    /// would otherwise alias its source under JS reference semantics. Fresh
    /// values (constructors, literals, calls) own their result and are left
    /// alone. Rule 2 (elision) then removes the copies whose source is dead (see
    /// `is_elidable_copy`).
    fn compute_clone_sites(&self) -> HashSet<Id> {
        let repeatable = self.collect_repeatable_interiors();
        let mut sites = HashSet::new();
        for expr in self.expr_id_to_expr_map.values() {
            let (value_id, value_type) = match expr {
                Expr::Variable(variable_id) => match self.variables.get(variable_id) {
                    Some(variable) => match variable.initial {
                        Some(value_id) => (value_id, variable.type_id.get_type(self)),
                        None => continue,
                    },
                    None => continue,
                },
                Expr::Assignment(_target_id, value_id) => match self.type_of_expr(*value_id) {
                    Some(value_type) => (*value_id, value_type),
                    None => continue,
                },
                _ => continue,
            };
            if self.is_place_expr(value_id)
                && self.is_cloneable_aggregate(&value_type)
                && !self.is_elidable_copy(value_id, &repeatable)
            {
                sites.insert(value_id);
            }
        }
        sites
    }

    /// Rule 2 (elision): whether a copy of an aggregate place can be downgraded
    /// to a move because the aliasing can never be observed. Sound, not complete
    /// — we only elide when the source is a local read *exactly once* (so there
    /// is no later read, and no closure capture, which would be a second read)
    /// and that read is not inside a loop or closure, where it could repeat and
    /// the alias would persist into the next iteration. A parameter is never
    /// elided: it aliases the caller's value, which outlives the call.
    fn is_elidable_copy(&self, value_id: Id, repeatable: &HashSet<Id>) -> bool {
        if repeatable.contains(&value_id) {
            return false;
        }
        let Some(Expr::Local(binding_id)) = self.expr_id_to_expr_map.get(&value_id) else {
            // Only a simple binding alias is elided; a field/element source
            // (`mut b = a.field`) is conservatively always copied.
            return false;
        };
        self.variables.contains_key(binding_id)
            && self.reference_count.get(binding_id).copied() == Some(1)
    }

    /// Entity ids inside a loop or closure body — code that may run a different
    /// number of times than its enclosing scope, so a copy there cannot be
    /// elided (the alias would survive into the next repetition). Every function,
    /// module, and closure body is walked; closures are also roots (at depth 1)
    /// so a copy inside any closure is treated as repeatable.
    fn collect_repeatable_interiors(&self) -> HashSet<Id> {
        let mut interior = HashSet::new();
        let mut visited = HashSet::new();
        for function in self.functions.values() {
            if function.has_body {
                for statement in &function.body.0 {
                    self.mark_repeatable(*statement, 0, &mut interior, &mut visited);
                }
                self.mark_repeatable(function.body.1, 0, &mut interior, &mut visited);
            }
        }
        for module in self.modules.values() {
            for statement in &module.body.0 {
                self.mark_repeatable(*statement, 0, &mut interior, &mut visited);
            }
        }
        for closure in self.closures.values() {
            self.mark_repeatable(closure.return_, 1, &mut interior, &mut visited);
        }
        interior
    }

    /// Walks the expression tree from `id`, recording every id reached at
    /// `depth > 0` (inside a loop or closure) in `interior`. Mirrors the call
    /// graph's traversal; `visited` guards against shared sub-expressions.
    fn mark_repeatable(
        &self,
        id: Id,
        depth: u32,
        interior: &mut HashSet<Id>,
        visited: &mut HashSet<Id>,
    ) {
        if !visited.insert(id) {
            return;
        }
        if depth > 0 {
            interior.insert(id);
        }
        let Some(expr) = self.expr_id_to_expr_map.get(&id) else {
            return;
        };
        match expr {
            Expr::Variable(variable_id) => {
                if let Some(initial) = self
                    .variables
                    .get(variable_id)
                    .and_then(|variable| variable.initial)
                {
                    self.mark_repeatable(initial, depth, interior, visited);
                }
            }
            Expr::Closure(closure_id) | Expr::Async(closure_id) => {
                if let Some(closure) = self.closures.get(closure_id) {
                    self.mark_repeatable(closure.return_, depth + 1, interior, visited);
                }
            }
            Expr::Field(subject_id, _, _) => {
                self.mark_repeatable(*subject_id, depth, interior, visited)
            }
            Expr::FunctionReturn(value_id) => {
                self.mark_repeatable(*value_id, depth, interior, visited)
            }
            Expr::Binary(_, lhs, rhs) => {
                self.mark_repeatable(*lhs, depth, interior, visited);
                self.mark_repeatable(*rhs, depth, interior, visited);
            }
            Expr::Unary(_, operand) | Expr::Reference(operand, _) | Expr::Dereference(operand) => {
                self.mark_repeatable(*operand, depth, interior, visited)
            }
            Expr::Assignment(target_id, value_id) => {
                self.mark_repeatable(*target_id, depth, interior, visited);
                self.mark_repeatable(*value_id, depth, interior, visited);
            }
            Expr::Call(call_id) => {
                if let Some(function_call) = self.function_calls.get(call_id) {
                    self.mark_repeatable(function_call.subject_id, depth, interior, visited);
                    for argument_id in &function_call.argument_ids {
                        self.mark_repeatable(*argument_id, depth, interior, visited);
                    }
                }
            }
            Expr::Await(inner) => self.mark_repeatable(*inner, depth, interior, visited),
            Expr::Block((statements, tail)) => {
                for statement in statements {
                    self.mark_repeatable(*statement, depth, interior, visited);
                }
                self.mark_repeatable(*tail, depth, interior, visited);
            }
            Expr::For(condition, (statements, tail)) => {
                if let Some(condition) = condition {
                    self.mark_repeatable(*condition, depth + 1, interior, visited);
                }
                for statement in statements {
                    self.mark_repeatable(*statement, depth + 1, interior, visited);
                }
                self.mark_repeatable(*tail, depth + 1, interior, visited);
            }
            Expr::ForEach(iterable, _item, (statements, tail)) => {
                self.mark_repeatable(*iterable, depth, interior, visited);
                for statement in statements {
                    self.mark_repeatable(*statement, depth + 1, interior, visited);
                }
                self.mark_repeatable(*tail, depth + 1, interior, visited);
            }
            Expr::If(branch) => self.mark_repeatable_if(branch, depth, interior, visited),
            Expr::Is(subject_id, _pattern) => {
                self.mark_repeatable(*subject_id, depth, interior, visited)
            }
            Expr::Match(subject_id, legs) => {
                self.mark_repeatable(*subject_id, depth, interior, visited);
                for leg in legs {
                    if let Some(guard) = leg.guard {
                        self.mark_repeatable(guard, depth, interior, visited);
                    }
                    self.mark_repeatable(leg.body, depth, interior, visited);
                }
            }
            Expr::List(ids) | Expr::Tuple(ids) => {
                for id in ids {
                    self.mark_repeatable(*id, depth, interior, visited);
                }
            }
            Expr::StructInitializer(_, fields) => {
                for value_id in fields.values() {
                    self.mark_repeatable(*value_id, depth, interior, visited);
                }
            }
            _ => {}
        }
    }

    fn mark_repeatable_if(
        &self,
        branch: &ExprIfBranch,
        depth: u32,
        interior: &mut HashSet<Id>,
        visited: &mut HashSet<Id>,
    ) {
        match branch {
            ExprIfBranch::If(condition, (statements, tail), else_branch) => {
                self.mark_repeatable(*condition, depth, interior, visited);
                for statement in statements {
                    self.mark_repeatable(*statement, depth, interior, visited);
                }
                self.mark_repeatable(*tail, depth, interior, visited);
                if let Some(else_branch) = else_branch {
                    self.mark_repeatable_if(else_branch, depth, interior, visited);
                }
            }
            ExprIfBranch::Else((statements, tail)) => {
                for statement in statements {
                    self.mark_repeatable(*statement, depth, interior, visited);
                }
                self.mark_repeatable(*tail, depth, interior, visited);
            }
        }
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

    /// Resolves a name in type position, walking outward to the nearest binding
    /// that denotes a *type* (a struct/enum/trait/module/generic, or `Self`),
    /// skipping value bindings. So in `random.vl`'s `external fun i32(low: i32)`,
    /// the parameter type `i32` resolves to the global `i32` struct rather than
    /// the enclosing `i32` function that shares its name.
    fn try_get_type_id_by_name(&self, name: &str, scope_id: Id) -> Option<Id> {
        let mut current = Some(scope_id);
        while let Some(scope_id) = current {
            let scope = self.scopes.get(&scope_id)?;
            if let Some(entity_id) = scope.name_to_id_map.get(name).copied() {
                let is_type = match self.expr_id_to_expr_map.get(&entity_id) {
                    Some(
                        Expr::Struct(_)
                        | Expr::Enum(_)
                        | Expr::Trait(_)
                        | Expr::Module(_)
                        | Expr::Generic(_),
                    ) => true,
                    Some(_) => false,
                    // No expression but a type id (e.g. the implicit `Self`).
                    None => self.expr_id_to_type_id_map.contains_key(&entity_id),
                };
                if is_type {
                    return Some(entity_id);
                }
            }
            current = scope.parent_id;
        }
        None
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
                // A default (`B = Self`) supplies the type used when no argument
                // is given: the parameter name resolves to the default type
                // rather than to a fresh, unbounded generic.
                if let Some(default) = &parameter.default {
                    let default_type_id = self.walk_type_node(default, scope_id);
                    self.register_defaulted_parameter(parameter.name, default_type_id, scope_id);
                    generic_parameter_constraint_ids.push(default_type_id);
                    continue;
                }
                let constraint_type_id =
                    self.register_binder(parameter.name, &parameter.bounds, scope_id);
                generic_parameter_constraint_ids.push(constraint_type_id);
            }
        }
        generic_parameter_constraint_ids
    }

    /// Registers one generic binder (a name with optional `: A + B` bounds) into
    /// `scope_id`, returning its constraint id (its `Type::Generic` identity).
    /// The first bound is the constraint; all bounds are recorded so member
    /// resolution on a value of this type can search each.
    fn register_binder(
        &mut self,
        name: &'src str,
        bounds: &[Spanned<Node<'src>>],
        scope_id: Id,
    ) -> TypeId {
        let bound_type_ids: Vec<TypeId> = bounds
            .iter()
            .map(|bound| self.walk_type_node(bound, scope_id))
            .collect();
        let constraint_type_id = bound_type_ids
            .first()
            .copied()
            .unwrap_or_else(|| Type::Any.get_type_id(self));
        self.register_generic_parameter(name, constraint_type_id, scope_id);
        // Bounds resolve in `build()`, so they're stored unresolved; only needed
        // when there is more than one (a single bound is recoverable from the
        // constraint id itself).
        if bound_type_ids.len() > 1 {
            self.generic_bounds
                .insert(constraint_type_id, bound_type_ids);
        }
        constraint_type_id
    }

    /// Registers every `type X` binder in an impl subject pattern as one of the
    /// impl's generic parameters — at the top level (`impl type T`), in generic
    /// arguments (`impl Option<type T>`), in tuples (`impl Option<(type T, type
    /// U)>`), and nested (`impl Option<Result<type T, type E>>`).
    fn register_subject_binders(&mut self, node: &'src Spanned<Node<'src>>, scope_id: Id) {
        match &node.0 {
            Node::TypeBinder(name, bounds) => {
                self.register_binder(name, bounds, scope_id);
            }
            Node::AccessorWithGenerics(_, generic_arguments) => {
                for argument in &generic_arguments.0 {
                    self.register_subject_binders(argument, scope_id);
                }
            }
            Node::Tuple(items) => {
                for item in items {
                    self.register_subject_binders(item, scope_id);
                }
            }
            Node::ClosureType(parameters, return_type) => {
                for parameter in &parameters.0 {
                    self.register_subject_binders(&parameter.1, scope_id);
                }
                if let Some(return_type) = return_type {
                    self.register_subject_binders(return_type, scope_id);
                }
            }
            _ => {}
        }
    }

    /// The substitution mapping a subject enum's declared generic parameters to
    /// the impl's concrete subject arguments, for `self`'s patterns inside an
    /// `impl Enum<args>` — e.g. inside `impl Option<(T, U)>`, `Some`'s declared
    /// payload (`Option`'s `T`) maps to the tuple `(T, U)`. Searches up the scope
    /// chain for the enclosing impl; `None` unless the matched enum is its
    /// subject (so a different enum's payloads are left untouched).
    fn impl_subject_substitution(&self, scope_id: Id, enum_id: Id) -> Option<SubstitutionContext> {
        let mut current = Some(scope_id);
        while let Some(scope_id) = current {
            if let Some((subject_type_id, arguments)) = self.impl_subject_args.get(&scope_id) {
                if !matches!(subject_type_id.get_type(self), Type::Enum(id, _) if id == enum_id) {
                    return None;
                }
                let declared = &self.enums.get(&enum_id)?.generic_parameter_constraint_ids;
                return Some(
                    declared
                        .iter()
                        .copied()
                        .zip(arguments.iter().copied())
                        .collect(),
                );
            }
            current = self.scopes.get(&scope_id).and_then(|scope| scope.parent_id);
        }
        None
    }

    /// The substitution mapping an enum's declared generic parameters to a
    /// matched value's concrete type arguments — so `Some(let x)` on a value of
    /// type `Option<Car>` binds `x: Car` (the variant payload `T` -> `Car`).
    /// `None` when the type isn't this enum or carries no (non-erased) arguments.
    fn enum_type_substitution(&self, enum_id: Id, type_id: TypeId) -> Option<SubstitutionContext> {
        let arguments = match type_id.get_type(self) {
            Type::Enum(id, arguments) if id == enum_id && !arguments.is_empty() => arguments,
            _ => return None,
        };
        let declared = &self.enums.get(&enum_id)?.generic_parameter_constraint_ids;
        Some(declared.iter().copied().zip(arguments).collect())
    }

    /// The trait ids a generic parameter is bound by (`T: A + B` -> `[A, B]`),
    /// resolved at call time. A multi-bound parameter's bounds are recorded in
    /// `generic_bounds`; a single bound is recoverable from the constraint id
    /// itself. Empty if the parameter is unconstrained or its bounds are
    /// unresolved/non-trait. Must be called in `build()`, once types resolve.
    fn generic_bound_trait_ids(&self, constraint_id: TypeId) -> Vec<Id> {
        let bound_type_ids = self
            .generic_bounds
            .get(&constraint_id)
            .cloned()
            .unwrap_or_else(|| vec![constraint_id]);
        bound_type_ids
            .iter()
            .filter_map(|type_id| match type_id.get_type(self) {
                Type::Trait(trait_id) => Some(trait_id),
                _ => None,
            })
            .collect()
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

    /// Registers a generic parameter that has a default (`B = Self`). The name
    /// resolves to the default's type, so a use of the parameter with no
    /// explicit argument falls back to it. Marked `Expr::Generic` (like any
    /// generic parameter) so it's still excluded from the scope's declarations.
    fn register_defaulted_parameter(
        &mut self,
        name: &'src str,
        default_type_id: TypeId,
        scope_id: Id,
    ) {
        let expr_id = self.new_entity_id();
        self.expr_id_to_expr_map
            .insert(expr_id, Expr::Generic(default_type_id));
        self.expr_id_to_type_id_map.insert(expr_id, default_type_id);
        self.expr_id_to_scope_id_map.insert(expr_id, scope_id);
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
            // `type X` binders only appear in type position (impl subjects).
            Node::TypeBinder(..) => Some(Expr::Error),
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
            // `for x in iterable` lowers to a native JS `for...of` (see
            // `Expr::ForEach`); it iterates a JS-iterable (e.g. a `List`, which is
            // an array). Iterating a custom `Iterator` (the trait protocol) is not
            // supported yet — that needs trait dispatch.
            Node::ForIn(variable, iterable, body) => {
                let iterable_id = self.walk_expr_node(iterable, scope_id);
                let body_scope_id = self.create_owned_scope(Some(scope_id)).id;
                // The element binding's type is the iterable's element type when
                // recoverable, else unknown (a `List` value erases its element
                // type). `_` introduces no binding.
                let item_id = (*variable != "_").then(|| {
                    let variable_id = self.new_entity_id();
                    // The binding starts `Unknown` (so a method call on it defers
                    // rather than erroring) and is resolved to the iterable's
                    // element type in the constraint loop, falling back to `any`
                    // for an iterable whose element type can't be recovered.
                    let element_type_id = Type::Unknown.get_type_id(self);
                    self.variables.insert(
                        variable_id,
                        Variable {
                            id: variable_id,
                            name: variable,
                            initial: None,
                            type_id: element_type_id,
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
                    self.prepped_for_each_items.push((variable_id, iterable_id));
                    variable_id
                });
                let ids = self.walk_expr_nodes(&body.0.0, body_scope_id);
                let expr_id = self.walk_expr_node(&body.0.1, body_scope_id);
                // Decide native `for...of` vs the Iterator-protocol loop once the
                // iterable's type is known (in `build`).
                self.prepped_for_each.push((id, iterable_id));
                Some(Expr::ForEach(iterable_id, item_id, (ids, expr_id)))
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
            Node::Unary(operator, operand) => {
                let operand_id = self.walk_expr_node(operand, scope_id);
                Some(Expr::Unary(*operator, operand_id))
            }
            // `&x` / `&mut x` — a view of a place. For aggregates a view is the
            // value's own reference, so it types and lowers as the operand;
            // mutability tracking and primitive-local boxing come later.
            Node::Reference(mutable, operand) => {
                let operand_id = self.walk_expr_node(operand, scope_id);
                Some(Expr::Reference(operand_id, *mutable))
            }
            // `*v` — read/write through a view; types and lowers as the operand.
            Node::Dereference(operand) => {
                let operand_id = self.walk_expr_node(operand, scope_id);
                Some(Expr::Dereference(operand_id))
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
                                // A bare `self` parameter (incl. `&self` /
                                // `&mut self`) takes the enclosing `Self` type.
                                None if x.0 == "self" => self
                                    .try_get_expr_id_by_name("Self", scope_id)
                                    .and_then(|self_id| {
                                        self.expr_id_to_type_id_map.get(&self_id).copied()
                                    })
                                    .unwrap_or_else(|| Type::Unknown.get_type_id(self)),
                                None => Type::Unknown.get_type_id(self),
                            },
                            convention: x.2,
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
                            extern_binding: function.extern_binding.clone(),
                            call_count: 0,
                            is_async: function.is_async,
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
                            has_body: function.body.is_some(),
                            call_count: 0,
                            is_async: function.is_async,
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
                if is_overloadable_operator(*op) {
                    self.prepped_binary_ops.push((id, *op, lhs_id));
                }
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
            Node::Assign(target, op, value) => {
                let value_id = self.walk_expr_node(value, scope_id);
                // The target is an lvalue node — a local (`Accessor`) or a field
                // place (`MemberAccessor`). Walking it yields an `Expr::Local`
                // or, once resolved, an `Expr::Field`; the transformer renders
                // either as the left side of a JS assignment.
                let target_id = self.walk_expr_node(target, scope_id);
                // A compound assignment like `x += v` desugars to `x = x + v`;
                // the left operand re-reads the same place.
                let stored_value_id = match op {
                    Some(op) => {
                        let lhs_id = self.walk_expr_node(target, scope_id);
                        let binary_id = self.new_entity_id();
                        self.expr_id_to_expr_map
                            .insert(binary_id, Expr::Binary(*op, lhs_id, value_id));
                        self.expr_id_to_scope_id_map.insert(binary_id, scope_id);
                        self.span_map.insert(binary_id, &node.1);
                        if is_overloadable_operator(*op) {
                            self.prepped_binary_ops.push((binary_id, *op, lhs_id));
                        }
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
                // C-style discriminants: each unspecified variant continues from
                // the previous value plus one, starting at 0. The enum is numeric
                // only if every variant is data-less and one is explicit.
                let mut next_discriminant: i64 = 0;
                let mut all_data_less = true;
                let mut any_explicit_discriminant = false;
                for (variant_index, variant) in variants.0.iter().enumerate() {
                    let variant_name = variant.0.0;
                    let data_type_ids: Vec<TypeId> = variant
                        .0
                        .1
                        .iter()
                        .map(|data_type| self.walk_type_node(data_type, body_scope_id))
                        .collect();
                    all_data_less &= data_type_ids.is_empty();
                    let explicit_discriminant = variant.0.2;
                    any_explicit_discriminant |= explicit_discriminant.is_some();
                    let discriminant = explicit_discriminant.unwrap_or(next_discriminant);
                    next_discriminant = discriminant + 1;
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
                        discriminant,
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
                        is_numeric: all_data_less && any_explicit_discriminant,
                    },
                );
                Some(Expr::Enum(id))
            }
            Node::Match(subject, legs) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                let mut walked_legs = Vec::new();
                for (patterns, guard, body) in &legs.0 {
                    // Each leg scopes its captures (and guard) to its own body.
                    let leg_scope_id = self.create_owned_scope(Some(scope_id)).id;
                    let walked_patterns = patterns
                        .iter()
                        .map(|pattern| self.walk_pattern(pattern, leg_scope_id))
                        .collect();
                    let guard_id = guard
                        .as_ref()
                        .map(|guard| self.walk_expr_node(guard, leg_scope_id));
                    let body_id = self.walk_expr_node(body, leg_scope_id);
                    walked_legs.push(WalkLeg {
                        patterns: walked_patterns,
                        guard: guard_id,
                        body: body_id,
                    });
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
            Node::Impl(subject, traits, body) => {
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                // The impl's generic parameters are the `type X` binders in the
                // subject pattern (anywhere: `impl List<type T>`, `impl
                // Option<(type T, type U)>`, or a blanket `impl type T`). Register
                // them before walking the subject so they resolve.
                self.register_subject_binders(subject, body_scope_id);
                let subject_type_id = self.walk_type_node(subject, body_scope_id);
                // Within an `impl`, `Self` refers to the subject type.
                self.register_self_type(body_scope_id, subject_type_id);
                // Record the subject's generic arguments (the `<...>` on the head)
                // so `self`'s variant patterns substitute the enum/struct's
                // declared parameters for these args — e.g. `Some` on a
                // `Option<(T, U)>` subject has payload `(T, U)`, not the abstract
                // `T` of `enum Option<T>`.
                if let Node::AccessorWithGenerics(_, generic_arguments) = &subject.0 {
                    let argument_type_ids: Vec<TypeId> = generic_arguments
                        .0
                        .iter()
                        .map(|argument| self.walk_type_node(argument, body_scope_id))
                        .collect();
                    self.impl_subject_args
                        .insert(body_scope_id, (subject_type_id, argument_type_ids));
                }
                let subject = subject_type_id;
                self.walk_expr_nodes(&body.0, body_scope_id);
                let declarations = self.collect_declarations(body_scope_id);
                let implementation_index = self.implementations.len();
                // `impl Subject with A + B` must satisfy each trait; record a
                // conformance check per trait to run once declarations are known.
                // The check also resolves the trait id back onto the impl.
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
                            implementation_index,
                        });
                    }
                }
                self.implementations.push(Implementation {
                    subject,
                    declarations,
                    trait_ids: Vec::new(),
                });

                Some(Expr::Impl(id))
            }
            Node::Trait(name, generic_parameters, supertraits, body) => {
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                self.register_generic_parameters(generic_parameters, body_scope_id);
                // Inside a trait, `Self` is the trait itself (abstractly): a
                // `self`-typed receiver in a default method resolves its method
                // calls against this trait's own declarations.
                let self_type_id = Type::Trait(id).get_type_id(self);
                self.register_self_type(body_scope_id, self_type_id);
                // Supertraits are resolved in the body scope so their generic
                // arguments (`PartialEq<B>`) see the trait's parameters.
                let supertraits = supertraits
                    .iter()
                    .map(|supertrait| self.walk_type_node(supertrait, body_scope_id))
                    .collect();
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
                        supertraits,
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
                            convention: x.2,
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
            // `async <body>` lowers to an immediately-invoked, no-parameter
            // async closure. The closure is its own node (in `closures`), so the
            // async inference pass treats it as a boundary: its awaits make the
            // closure async, not the enclosing function.
            Node::Async(body) => {
                let closure_id = self.new_entity_id();
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let return_id = self.walk_expr_node(body, body_scope_id);
                self.closures.insert(
                    closure_id,
                    Closure {
                        id: closure_id,
                        parameters: Vec::new(),
                        return_: return_id,
                    },
                );
                self.expr_id_to_expr_map
                    .insert(closure_id, Expr::Closure(closure_id));
                self.span_map.insert(closure_id, &node.1);
                self.expr_id_to_scope_id_map
                    .insert(closure_id, body_scope_id);
                // The type (`Promise<T>`) is inferred lazily in `infer_type_path`.
                Some(Expr::Async(closure_id))
            }
            // `await <inner>` — its type (the unwrapped `T`) is inferred lazily.
            Node::Await(inner) => {
                let inner_id = self.walk_expr_node(inner, scope_id);
                Some(Expr::Await(inner_id))
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
            Pattern::Variant(path, payload) => WalkPattern::Variant(
                path.clone(),
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
            Pattern::Literal(literal) => {
                WalkPattern::Literal(self.walk_expr_node(literal, scope_id))
            }
        }
    }

    /// Resolves a variant pattern's path to its entity: a bare name (`Some`,
    /// or `true`/`false` against a `bool` subject), or a qualified path
    /// (`Signal::Quit`) descended through the leading namespaces (an enum's
    /// variants or a module's members).
    fn resolve_variant_path(
        &mut self,
        path: &[&'src str],
        lookup_scope_id: Id,
        expected_type_id: TypeId,
    ) -> Option<Id> {
        if let [name] = path {
            if let Some(entity) = self.try_get_expr_id_by_name(name, lookup_scope_id) {
                return Some(entity);
            }
            // `true`/`false` are keywords, not names in scope; against a `bool`
            // subject resolve them directly against the `bool` enum's variants so
            // `match flag { true => .., false => .. }` needs no imports.
            if matches!(
                expected_type_id.get_type(self),
                Type::Enum(id, _) if Some(id) == self.bool_enum_id
            ) {
                return self.bool_enum_id.and_then(|bool_id| {
                    let variants_scope_id = self.enums.get(&bool_id)?.variants_scope_id;
                    self.scopes
                        .get(&variants_scope_id)?
                        .name_to_id_map
                        .get(name)
                        .copied()
                });
            }
            return None;
        }
        // A qualified path: resolve the head, then descend each segment through
        // an enum's variant namespace or a module's scope.
        let mut segments = path.iter();
        let head = segments.next()?;
        let mut current = self.try_get_expr_id_by_name(head, lookup_scope_id)?;
        for segment in segments {
            let scope_id = match self.expr_id_to_expr_map.get(&current)? {
                Expr::Enum(enum_id) => self.enums.get(enum_id)?.variants_scope_id,
                Expr::Module(module_id) => self.modules.get(module_id)?.body.1,
                _ => return None,
            };
            current = self
                .scopes
                .get(&scope_id)?
                .name_to_id_map
                .get(*segment)
                .copied()?;
        }
        Some(current)
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
            WalkPattern::Variant(path, span, payload) => {
                let span = *span;
                let name = path.join("::");
                let entity =
                    match self.resolve_variant_path(path, lookup_scope_id, expected_type_id) {
                        Some(entity) => entity,
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
                    Type::Enum(expected_enum_id, _) if expected_enum_id == enum_id => {}
                    Type::Unknown | Type::Any | Type::Generic(_) => {}
                    Type::Enum(_, _) => {
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
                let mut data_type_ids = self.enums.get(&enum_id).unwrap().variants[variant_index]
                    .data_type_ids
                    .clone();
                // Substitute the variant's declared payload types for the matched
                // enum's concrete type arguments, so `Some(let x)` on `Option<Car>`
                // binds `x: Car` (not the abstract `T`). The arguments come from
                // the matched value's type, or — when `self`'s type is the still
                // abstract enum inside an `impl Enum<args>` — from the impl subject.
                let substitution = self
                    .enum_type_substitution(enum_id, expected_type_id)
                    .or_else(|| self.impl_subject_substitution(lookup_scope_id, enum_id));
                if let Some(substitution) = substitution {
                    data_type_ids = data_type_ids
                        .iter()
                        .map(|data_type_id| {
                            let substituted =
                                self.substitute_type(&data_type_id.get_type(self), &substitution);
                            substituted.get_type_id(self)
                        })
                        .collect();
                }
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
            WalkPattern::Literal(literal_id) => {
                // The literal's type must be compatible with the matched value's.
                let literal_id = *literal_id;
                let literal_type = self.infer_type(literal_id, &Type::Unknown, &HashMap::new());
                let subject_type = expected_type_id.get_type(self);
                if !matches!(subject_type, Type::Unknown | Type::Any | Type::Unresolved)
                    && !self.compare_type(&subject_type, &literal_type, &HashMap::new())
                {
                    let expected = self.pretty_print_type(&subject_type, &HashMap::new());
                    let got = self.pretty_print_type(&literal_type, &HashMap::new());
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&literal_id).unwrap_or(&&EMPTY_SPAN),
                        msg: format!("literal pattern of type {} cannot match {}", got, expected),
                    });
                    return None;
                }
                Some(ExprPattern::Literal(literal_id))
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
                        .push((type_id, name, scope_id, node.1, Vec::new()));
                    None
                }
            },
            // `Option<i32>` resolves to the nominal type carrying its walked
            // arguments (`Enum(option_id, [i32])`); the arguments parameterize the
            // type, declare implicit impl generics, and drive monomorphization.
            Node::AccessorWithGenerics(name, generic_arguments) => {
                let argument_type_ids: Vec<TypeId> = generic_arguments
                    .0
                    .iter()
                    .map(|argument| self.walk_type_node(argument, scope_id))
                    .collect();
                self.prepped_type_locals
                    .push((type_id, name, scope_id, node.1, argument_type_ids));
                None
            }
            // A `type X` binder resolves to the generic it was registered as (by
            // `register_subject_binders` before the subject is walked).
            Node::TypeBinder(name, _bounds) => {
                self.prepped_type_locals
                    .push((type_id, name, scope_id, node.1, Vec::new()));
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
            // `&T` / `&mut T` carries the inner type for now (identity); a
            // parameter captures the `&`/`&mut` separately as its convention.
            Node::Reference(_, inner) => return self.walk_type_node(inner, scope_id),
            x => unimplemented!("unhandled type node: {:?}", x),
        };

        if let Some(type_) = type_ {
            self.type_id_to_type_map.insert(type_id, type_);
        }

        type_id
    }

    /// The element type of an iterable, when recoverable: a `List<T>` yields
    /// `T`. Returns `None` for an erased `List` (no arguments) or a non-list
    /// iterable (e.g. a custom iterator), whose element the caller treats as
    /// `any`.
    fn iterable_element_type(&self, iterable_type: &Type) -> Option<Type> {
        match iterable_type {
            Type::Struct(id, arguments)
                if Some(*id) == self.primitive_struct_ids.get("List").copied() =>
            {
                arguments
                    .first()
                    .map(|element_type_id| element_type_id.get_type(self))
            }
            _ => None,
        }
    }

    /// If `type_` is a `List` whose element is still an unresolved inference slot
    /// (an `Unknown` type id from `List::new()`), returns that slot's type id.
    fn list_element_slot(&self, type_: &Type) -> Option<TypeId> {
        match type_ {
            Type::Struct(id, arguments) if self.is_slot_container(*id) && arguments.len() == 1 => {
                let slot = arguments[0];
                matches!(slot.get_type(self), Type::Unknown).then_some(slot)
            }
            _ => None,
        }
    }

    /// Whether a struct is an element-slot container — `List` or `Context` —
    /// whose single type argument is inferred from a method call (`List::push`
    /// fills `List<T>`; `Context::run`'s value fills `Context<T>`).
    fn is_slot_container(&self, id: Id) -> bool {
        self.primitive_struct_ids.get("List") == Some(&id)
            || self.primitive_struct_ids.get("Context") == Some(&id)
    }

    /// If `type_` is a `List` whose element is an unbound generic (i.e. the
    /// result of `List::new()`), replaces the element with a fresh inference
    /// slot stable for this call id, so the element can be unified from later
    /// `push` calls. Otherwise returns the type unchanged.
    fn freshen_list_element_slots(&mut self, type_: Type, call_id: Id) -> Type {
        match type_ {
            Type::Struct(id, arguments)
                if self.is_slot_container(id)
                    && arguments.len() == 1
                    && matches!(arguments[0].get_type(self), Type::Generic(_)) =>
            {
                let slot = match self.list_element_slots.get(&call_id).copied() {
                    Some(slot) => slot,
                    None => {
                        let slot = Type::Unknown.get_type_id(self);
                        self.list_element_slots.insert(call_id, slot);
                        slot
                    }
                };
                Type::Struct(id, vec![slot])
            }
            other => other,
        }
    }

    /// Infers each closure argument of a method call against the method's
    /// corresponding parameter type, so an unannotated closure param is filled
    /// bidirectionally (`builder.on_start(|s| ..)` types `s` from `|Server|
    /// void`). `argument_ids` are the explicit args (no receiver); the method's
    /// first parameter is `self`, so they align at offset 1.
    fn infer_closure_args_against_params(&mut self, member_id: Id, argument_ids: &[Id]) {
        let parameter_ids = match self.expr_id_to_expr_map.get(&member_id) {
            Some(Expr::Function(function_id)) => self
                .functions
                .get(function_id)
                .map(|f| f.parameters.clone()),
            Some(Expr::ExternalFunction(function_id)) => self
                .external_functions
                .get(function_id)
                .map(|f| f.parameters.clone()),
            _ => None,
        };
        let Some(parameter_ids) = parameter_ids else {
            return;
        };
        for (index, argument_id) in argument_ids.iter().enumerate() {
            if !matches!(
                self.expr_id_to_expr_map.get(argument_id),
                Some(Expr::Closure(_))
            ) {
                continue;
            }
            // `+ 1` skips the method's `self` parameter.
            let Some(parameter_id) = parameter_ids.get(index + 1) else {
                continue;
            };
            let parameter_type = self
                .parameters
                .get(parameter_id)
                .map(|parameter| parameter.type_id.get_type(self));
            if let Some(parameter_type) = parameter_type {
                self.infer_type(*argument_id, &parameter_type, &HashMap::new());
            }
        }
    }

    /// Whether `expr_id` is a closure parameter whose type is still `Unknown`
    /// (an unannotated `|x| ..` awaiting bidirectional inference), following
    /// `Local` indirections to the `Parameter` entity.
    fn is_unknown_closure_parameter(&self, expr_id: Id) -> bool {
        let mut id = expr_id;
        loop {
            match self.expr_id_to_expr_map.get(&id) {
                Some(Expr::Local(target_id)) => id = *target_id,
                Some(Expr::Parameter(parameter_id)) => {
                    return self.parameters.get(parameter_id).is_some_and(|parameter| {
                        self.closures.contains_key(&parameter.function_id)
                            && matches!(parameter.type_id.get_type(self), Type::Unknown)
                    });
                }
                _ => return false,
            }
        }
    }

    /// The variant index a constructor subject refers to (`Some` -> 0), following
    /// `Local` indirections to the `EnumVariant` entity.
    fn enum_variant_index(&self, subject_id: Id) -> Option<usize> {
        let mut id = subject_id;
        loop {
            match self.expr_id_to_expr_map.get(&id)? {
                Expr::Local(target_id) => id = *target_id,
                Expr::EnumVariant(_, variant_index) => return Some(*variant_index),
                _ => return None,
            }
        }
    }

    /// Infers a generic enum's type arguments from a variant constructor's
    /// arguments: the variant's declared payload types mention the enum's
    /// parameters, so reconciling them against the actual argument types binds
    /// each parameter (`Some(3)` — payload `T`, argument `i32` — gives `[i32]`,
    /// i.e. `Option<i32>`). Returns `None` when the enum is non-generic, the
    /// variant index is unknown, an argument is still unresolved, or some
    /// parameter stays unbound (e.g. `None`) — leaving the arguments erased,
    /// which matches any instantiation.
    fn infer_enum_constructor_arguments(
        &mut self,
        subject_id: Id,
        enum_id: Id,
        argument_ids: &[Id],
        substitution_context: &SubstitutionContext,
        exprs_seen: &mut HashSet<Id>,
    ) -> Option<Vec<TypeId>> {
        let variant_index = self.enum_variant_index(subject_id)?;
        let enum_ = self.enums.get(&enum_id)?;
        let generic_parameters = enum_.generic_parameter_constraint_ids.clone();
        if generic_parameters.is_empty() {
            return None;
        }
        let data_type_ids = enum_.variants.get(variant_index)?.data_type_ids.clone();
        let mut bindings: SubstitutionContext = HashMap::new();
        for (data_type_id, argument_id) in data_type_ids.iter().zip(argument_ids.iter()) {
            let data_type = data_type_id.get_type(self);
            let argument_type =
                self.infer_type_inner(*argument_id, &data_type, substitution_context, exprs_seen);
            if matches!(argument_type, Type::Unresolved) {
                return None;
            }
            if let Some((_, new_bindings)) =
                self.reconcile_type(&data_type, &argument_type, &bindings)
            {
                bindings.extend(new_bindings);
            }
        }
        // Only commit when every parameter is bound; a partial inference keeps
        // the arguments erased rather than inventing placeholders.
        generic_parameters
            .iter()
            .map(|constraint_id| bindings.get(constraint_id).copied())
            .collect()
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
        // `exprs_seen` guards against infinite recursion through a genuine cycle
        // (an expression whose type depends on itself). It tracks the current
        // recursion *path*, not every expression ever visited: the id is removed
        // again once this call returns, so a node shared by two sibling branches
        // (a DAG, e.g. the same `Some` variant in `(Some(x), Some(y))`) is not
        // wrongly treated as a cycle on the second visit.
        if exprs_seen.contains(&expr_id) {
            return Type::Unresolved;
        }
        exprs_seen.insert(expr_id);
        let inferred = self.infer_type_path(expr_id, constraint, substitution_context, exprs_seen);
        exprs_seen.remove(&expr_id);
        inferred
    }

    fn infer_type_path(
        &mut self,
        expr_id: Id,
        constraint: &Type,
        substitution_context: &SubstitutionContext,
        exprs_seen: &mut HashSet<Id>,
    ) -> Type {
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
            // `async <body>` is a `Promise<T>`, T the type of the body. If the
            // expected type is itself `Promise<U>`, the body is checked against U.
            Expr::Async(closure_id) => {
                let body_id = self.closures.get(closure_id).map(|closure| closure.return_);
                let inner_constraint = match &constraint {
                    Type::Struct(id, arguments) if Some(*id) == self.promise_struct_id => arguments
                        .first()
                        .map(|type_id| type_id.get_type(self))
                        .unwrap_or(Type::Unknown),
                    _ => Type::Unknown,
                };
                let body_type = body_id
                    .map(|body_id| {
                        self.infer_type_inner(
                            body_id,
                            &inner_constraint,
                            substitution_context,
                            exprs_seen,
                        )
                    })
                    .unwrap_or(Type::Unknown);
                // Defer while the body type is still settling, so the wrapped
                // `Promise<unresolved>` isn't compared against and rejected.
                if matches!(body_type, Type::Unresolved) {
                    return Type::Unresolved;
                }
                match self.promise_struct_id {
                    Some(promise_id) => {
                        let body_type_id = body_type.get_type_id(self);
                        Type::Struct(promise_id, vec![body_type_id])
                    }
                    None => Type::Any,
                }
            }
            // `await <inner>` unwraps a `Promise<T>` to `T` (and is the identity
            // on a non-promise).
            Expr::Await(inner_id) => {
                let inner = self.infer_type_inner(
                    *inner_id,
                    &Type::Unknown,
                    substitution_context,
                    exprs_seen,
                );
                match &inner {
                    Type::Unresolved => Type::Unresolved,
                    Type::Struct(id, arguments) if Some(*id) == self.promise_struct_id => arguments
                        .first()
                        .map(|type_id| type_id.get_type(self))
                        .unwrap_or(Type::Any),
                    _ => inner,
                }
            }
            // `!x` (logical not) is a boolean.
            Expr::Unary('!', _) => self.bool_type(),
            Expr::Unary(_, operand_id) => {
                self.infer_type_inner(*operand_id, &constraint, substitution_context, exprs_seen)
            }
            // A view (`&x` / `&mut x`) and a deref (`*v`) both carry the operand's
            // type for now — a view of `T` reads and writes as a `T`. A distinct
            // view type arrives with the mutability checker.
            Expr::Reference(operand_id, _) | Expr::Dereference(operand_id) => {
                self.infer_type_inner(*operand_id, &constraint, substitution_context, exprs_seen)
            }
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
                                Type::Struct(id, _) if *id == self.primitive_struct_ids["f64"] => {
                                    "f64"
                                }
                                Type::Struct(id, _) if *id == self.primitive_struct_ids["u32"] => {
                                    "u32"
                                }
                                Type::Struct(id, _) if *id == self.primitive_struct_ids["i32"] => {
                                    "i32"
                                }
                                Type::Struct(id, _)
                                    if *id == self.primitive_struct_ids["BigInt"] =>
                                {
                                    "BigInt"
                                }
                                _ => "i32",
                            }
                        }
                    }
                };
                self.primitive_struct_type(name)
            }
            Expr::List(item_ids) => {
                // A list literal is the `List` struct parameterized by its
                // unified element type (`[1, 2]` -> `List<i32>`); an empty list
                // erases the element (matches any `List<T>`).
                let item_ids = item_ids.clone();
                let mut element_type = Type::Unknown;
                for item_id in &item_ids {
                    let item_type = self.infer_type_inner(
                        *item_id,
                        &element_type,
                        substitution_context,
                        exprs_seen,
                    );
                    if matches!(item_type, Type::Unresolved) {
                        return Type::Unresolved;
                    }
                    element_type = match self.reconcile_type(
                        &element_type,
                        &item_type,
                        substitution_context,
                    ) {
                        Some((unified, _)) => unified,
                        None => element_type,
                    };
                }
                match self.primitive_struct_ids.get("List").copied() {
                    Some(list_id) => {
                        let arguments = if matches!(element_type, Type::Unknown) {
                            Vec::new()
                        } else {
                            vec![element_type.get_type_id(self)]
                        };
                        Type::Struct(list_id, arguments)
                    }
                    None => Type::Unknown,
                }
            }
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
            Expr::Struct(struct_id) => Type::Struct(*struct_id, Vec::new()),
            Expr::Enum(enum_id) => Type::Enum(*enum_id, Vec::new()),
            // A bare variant reference is a value of the enum (e.g. `None`); a
            // variant with data acts as a constructor whose call also yields
            // the enum.
            Expr::EnumVariant(enum_id, _) => Type::Enum(*enum_id, Vec::new()),
            Expr::Trait(trait_id) => Type::Trait(*trait_id),
            Expr::Module(module_id) => Type::Module(*module_id),
            Expr::Call(id) => {
                let id = *id;
                // The call may not have been wired up yet (its `FunctionCall`
                // is recorded once the subject resolves). Defer until it is.
                let (subject_id, generic_argument_ids, argument_ids) =
                    match self.function_calls.get(&id) {
                        Some(function_call) => (
                            function_call.subject_id,
                            function_call.generic_argument_ids.clone(),
                            function_call.argument_ids.clone(),
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
                    // value of the enum, with the enum's type arguments inferred
                    // from the constructor arguments (`Some(3)` -> `Option<i32>`).
                    Type::Enum(enum_id, arguments) => {
                        if arguments.is_empty() {
                            let inferred = self.infer_enum_constructor_arguments(
                                subject_id,
                                enum_id,
                                &argument_ids,
                                substitution_context,
                                exprs_seen,
                            );
                            Type::Enum(enum_id, inferred.unwrap_or(arguments))
                        } else {
                            Type::Enum(enum_id, arguments)
                        }
                    }
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
                                f.parameters.first().copied(),
                            )
                        });
                        let Some((
                            generic_constraint_ids,
                            return_type_id,
                            body_return_id,
                            self_parameter_id,
                        )) = function
                        else {
                            // An external function: use its declared return type
                            // (giving `List::new()` a fresh element slot).
                            let return_type = self
                                .external_functions
                                .get(&function_id)
                                .map(|f| f.return_type_id.get_type(self))
                                .unwrap_or(Type::Void);
                            return self.freshen_list_element_slots(return_type, id);
                        };
                        let mut substitution_context = substitution_context.clone();
                        for (i, constraint_id) in generic_constraint_ids.iter().enumerate() {
                            if let Some(argument_id) = generic_argument_ids.get(i) {
                                substitution_context.insert(*constraint_id, *argument_id);
                            }
                        }
                        let return_type = match return_type_id {
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
                        };
                        let return_type = self.freshen_list_element_slots(return_type, id);
                        // Specialize a `Self` return. When a method's declared
                        // return type is the same as its `self` parameter's type
                        // — i.e. it returns `Self` — the call yields the
                        // receiver's actual type. So a `Self`-returning trait
                        // default called on a concrete value gives that concrete
                        // type, not the abstract `Type::Trait` Self stands for.
                        let returns_self = match (self_parameter_id, return_type_id) {
                            (Some(self_parameter_id), Some(return_type_id)) => {
                                let self_type = self
                                    .parameters
                                    .get(&self_parameter_id)
                                    .map(|parameter| parameter.type_id.get_type(self));
                                self_type == Some(return_type_id.get_type(self))
                            }
                            _ => false,
                        };
                        if returns_self {
                            if let Some(receiver_id) = argument_ids.first().copied() {
                                let receiver_type = self.infer_type_inner(
                                    receiver_id,
                                    &Type::Unknown,
                                    &substitution_context,
                                    exprs_seen,
                                );
                                if matches!(receiver_type, Type::Struct(_, _) | Type::Enum(_, _)) {
                                    return receiver_type;
                                }
                            }
                        }
                        return_type
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
                    Type::Struct(struct_def_id, Vec::new())
                } else {
                    Type::Struct(*initializer_id, Vec::new())
                }
            }
            Expr::Generic(type_id) => type_id.get_type(self),
            // Comparisons and logical `&&` produce a `bool`; arithmetic produces
            // the operand type (taken from the left-hand side).
            Expr::Binary(
                BinaryOp::Eq
                | BinaryOp::NotEq
                | BinaryOp::Lt
                | BinaryOp::Gt
                | BinaryOp::LtEq
                | BinaryOp::GtEq
                | BinaryOp::And,
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
            // A value `if` (an `if`/`else` chain with a final `else`) has the
            // type of its branches — take the first branch's trailing expression
            // (the branches must agree). Without a final `else` it is a statement,
            // so it is void.
            Expr::If(branch) => {
                fn has_final_else(branch: &ExprIfBranch) -> bool {
                    match branch {
                        ExprIfBranch::If(_, _, Some(next)) => has_final_else(next),
                        ExprIfBranch::If(_, _, None) => false,
                        ExprIfBranch::Else(_) => true,
                    }
                }
                match branch {
                    ExprIfBranch::If(_, (_, trailing), _) if has_final_else(branch) => {
                        let trailing = *trailing;
                        self.infer_type_inner(
                            trailing,
                            &constraint,
                            substitution_context,
                            exprs_seen,
                        )
                    }
                    _ => Type::Void,
                }
            }
            Expr::Closure(closure_id) => {
                let closure = self.closures.get(closure_id).unwrap();
                let parameter_ids = closure.parameters.clone();
                let return_expr_id = closure.return_;
                // Bidirectional inference: when the expected type is a closure of
                // matching arity, fill any unannotated (`Unknown`) parameter from
                // it — so `|res|` passed where `|Res| void` is expected types
                // `res` as `Res`.
                if let Type::Closure(expected_parameter_ids, _) = &constraint {
                    if expected_parameter_ids.len() == parameter_ids.len() {
                        let expected = expected_parameter_ids.clone();
                        for (parameter_id, expected_type_id) in parameter_ids.iter().zip(expected) {
                            let is_unknown = self
                                .parameters
                                .get(parameter_id)
                                .is_some_and(|p| matches!(p.type_id.get_type(self), Type::Unknown));
                            let expected_known = !matches!(
                                expected_type_id.get_type(self),
                                Type::Unknown | Type::Unresolved
                            );
                            if is_unknown && expected_known {
                                if let Some(parameter) = self.parameters.get_mut(parameter_id) {
                                    parameter.type_id = expected_type_id;
                                }
                            }
                        }
                    }
                }
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
            // Same nominal type: unify argument-wise, keeping the instantiated
            // side when the other is erased. Reconciling the arguments collects
            // the generic bindings that drive element-type inference (e.g.
            // `List<T>` against `List<i32>` binds `T = i32`).
            (Type::Enum(l_id, l_arguments), Type::Enum(r_id, r_arguments)) if l_id == r_id => {
                let (arguments, bindings) =
                    self.reconcile_argument_types(l_arguments, r_arguments, substitution_context)?;
                (Type::Enum(*l_id, arguments), bindings)
            }
            (Type::Struct(l_id, l_arguments), Type::Struct(r_id, r_arguments)) if l_id == r_id => {
                let (arguments, bindings) =
                    self.reconcile_argument_types(l_arguments, r_arguments, substitution_context)?;
                (Type::Struct(*l_id, arguments), bindings)
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

    /// Reconciles two nominal types' argument lists. An erased (empty) side
    /// yields the other; otherwise the arguments unify pairwise, accumulating
    /// the generic bindings discovered along the way.
    fn reconcile_argument_types(
        &mut self,
        left: &[TypeId],
        right: &[TypeId],
        substitution_context: &SubstitutionContext,
    ) -> Option<(Vec<TypeId>, Vec<(TypeId, TypeId)>)> {
        if left.is_empty() {
            return Some((right.to_vec(), Vec::new()));
        }
        if right.is_empty() {
            return Some((left.to_vec(), Vec::new()));
        }
        if left.len() != right.len() {
            return None;
        }
        let mut arguments = Vec::with_capacity(left.len());
        let mut all_bindings = Vec::new();
        for (left_id, right_id) in left.iter().zip(right.iter()) {
            let left_type = left_id.get_type(self);
            let right_type = right_id.get_type(self);
            let (argument, bindings) =
                self.reconcile_type(&left_type, &right_type, substitution_context)?;
            all_bindings.extend(bindings);
            arguments.push(argument.get_type_id(self));
        }
        Some((arguments, all_bindings))
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
            // Same nominal type: compatible when the arguments are (a side with
            // no arguments is an erased/abstract `List`/`Option`, compatible with
            // any instantiation).
            (Type::Enum(l_id, l_arguments), Type::Enum(r_id, r_arguments)) if l_id == r_id => {
                self.compare_argument_types(l_arguments, r_arguments, substitution_context)
            }
            (Type::Struct(l_id, l_arguments), Type::Struct(r_id, r_arguments)) if l_id == r_id => {
                self.compare_argument_types(l_arguments, r_arguments, substitution_context)
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

    /// Compares two nominal types' argument lists. A side with no arguments is
    /// erased (an abstract `List`/`Option`) and matches any instantiation;
    /// otherwise the arguments must be pairwise compatible.
    fn compare_argument_types(
        &self,
        left: &[TypeId],
        right: &[TypeId],
        substitution_context: &SubstitutionContext,
    ) -> bool {
        if left.is_empty() || right.is_empty() {
            return true;
        }
        left.len() == right.len()
            && left.iter().zip(right.iter()).all(|(left_id, right_id)| {
                let left_type = left_id.get_type(self);
                let right_type = right_id.get_type(self);
                // A generic argument (an impl's type parameter, e.g. the `T` of
                // `impl List<T: Add>`) is a hole to be bound, not a constraint to
                // satisfy structurally — so it matches any concrete argument. The
                // bound is enforced separately by member resolution.
                matches!(left_type, Type::Generic(_))
                    || matches!(right_type, Type::Generic(_))
                    || self.compare_type(&left_type, &right_type, substitution_context)
            })
    }

    /// Resolves any generic type parameters in `type_` using the substitution
    /// context, e.g. turning the return type `T` of `default<T>` into `Id` for
    /// a call `default<Id>()`.
    fn substitute_type(
        &mut self,
        type_: &Type,
        substitution_context: &SubstitutionContext,
    ) -> Type {
        match type_ {
            Type::Generic(constraint_id) => substitution_context
                .get(constraint_id)
                .map(|type_id| {
                    let resolved = type_id.get_type(self);
                    self.substitute_type(&resolved, substitution_context)
                })
                .unwrap_or_else(|| type_.clone()),
            // A nominal type substitutes its arguments (`Option<T>` -> `Option<i32>`
            // when `T` is bound).
            Type::Enum(id, arguments) => {
                let arguments = arguments.clone();
                Type::Enum(
                    *id,
                    self.substitute_argument_types(&arguments, substitution_context),
                )
            }
            Type::Struct(id, arguments) => {
                let arguments = arguments.clone();
                Type::Struct(
                    *id,
                    self.substitute_argument_types(&arguments, substitution_context),
                )
            }
            _ => type_.clone(),
        }
    }

    /// Substitutes each argument type id through the context, re-interning the
    /// results. Used for the arguments of a nominal `Enum`/`Struct` type.
    fn substitute_argument_types(
        &mut self,
        arguments: &[TypeId],
        substitution_context: &SubstitutionContext,
    ) -> Vec<TypeId> {
        arguments
            .iter()
            .map(|argument| {
                let argument_type = argument.get_type(self);
                let substituted = self.substitute_type(&argument_type, substitution_context);
                substituted.get_type_id(self)
            })
            .collect()
    }

    /// Attempts to resolve one `import`/`export import` and bind it into its
    /// scope, returning whether it bound. A re-export can name an item another,
    /// not-yet-resolved re-export will bind (`lib` re-exports from `io` via a
    /// chain of relay modules), so a single failure is not final — the caller
    /// retries to a fixpoint. `report` emits the diagnostic for a genuine
    /// failure; during the fixpoint passes it is `false` (a miss just defers).
    fn resolve_import(
        &mut self,
        path: &[&'src str],
        name: &'src str,
        scope_id: Id,
        span: Span,
        report: bool,
    ) -> bool {
        // A `self` leaf re-binds the namespace it sits in under its own name
        // (e.g. `Option::{ self }` binds `Option`); otherwise the leaf is the
        // final path segment and binds under its own name.
        let (segments, bind_name): (Vec<&str>, &str) = if name == "self" {
            match path.last().copied() {
                Some(last) => (path.to_vec(), last),
                None => {
                    if report {
                        self.diagnostics.push(Error {
                            span,
                            msg: "`self` import has no enclosing namespace".to_string(),
                        });
                    }
                    return false;
                }
            }
        } else {
            let mut segments = path.to_vec();
            segments.push(name);
            (segments, name)
        };
        let mut segments = segments.into_iter();
        let root = segments.next().unwrap();
        let module_id = match self.module_id_by_name.get(root) {
            Some(module_id) => *module_id,
            None => {
                if report {
                    self.diagnostics.push(Error {
                        span,
                        msg: format!("cannot find module '{}' to import", root),
                    });
                }
                return false;
            }
        };
        let mut target_id = module_id;
        let mut namespace_scope_id = self.modules.get(&module_id).unwrap().body.1;
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
                    if report {
                        self.diagnostics.push(Error {
                            span,
                            msg: format!("cannot find '{}' in the imported path", part),
                        });
                    }
                    return false;
                }
            }
        }
        let scope = self.mut_scope_for_scope_id(scope_id);
        scope.name_to_id_map.insert(bind_name, target_id);
        true
    }

    fn build(&mut self) {
        // Resolve imports/re-exports to a fixpoint: a re-export may name an item
        // bound by another re-export resolved in a later pass (a chain of relay
        // modules), so keep retrying the unresolved ones until a pass binds
        // nothing new, then report whatever genuinely could not be found.
        let mut remaining = self.prepped_imports.clone();
        loop {
            let before = remaining.len();
            remaining.retain(|(path, name, scope_id, span)| {
                !self.resolve_import(path, name, *scope_id, *span, false)
            });
            if remaining.len() == before || remaining.is_empty() {
                break;
            }
        }
        for (path, name, scope_id, span) in remaining {
            self.resolve_import(&path, name, scope_id, span, true);
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

        for (type_id, name, scope_id, span, argument_type_ids) in self.prepped_type_locals.clone() {
            // Prefer a type binding (so a value sharing the name doesn't shadow
            // it in type position), falling back to any binding for diagnostics.
            let resolved = self
                .try_get_type_id_by_name(name, scope_id)
                .or_else(|| self.try_get_expr_id_by_name(name, scope_id));
            match resolved {
                Some(subject_id) => {
                    let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
                    // Attach the written generic arguments to the nominal type
                    // (`Option<i32>` -> `Enum(option_id, [i32])`). A bare name
                    // keeps whatever the reference resolved to.
                    let subject_type = match (subject_type, argument_type_ids.is_empty()) {
                        (Type::Enum(id, _), false) => Type::Enum(id, argument_type_ids),
                        (Type::Struct(id, _), false) => Type::Struct(id, argument_type_ids),
                        (other, _) => other,
                    };
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
                Type::Struct(struct_id, _) => {
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
                Type::Struct(_, _) | Type::Trait(_) | Type::Enum(_, _) => {
                    let variant_id = match &subject_type {
                        Type::Enum(enum_id, _) => {
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
                Type::Generic(constraint_id) => {
                    let bound_trait_ids = self.generic_bound_trait_ids(constraint_id);
                    if bound_trait_ids.is_empty() {
                        self.diagnostics.push(Error {
                            span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                            msg: format!(
                                "cannot access '{}' on an unconstrained type parameter",
                                member_name
                            ),
                        });
                    } else {
                        // Record the accessor so codegen can monomorphize it to
                        // the concrete type's member at each call site.
                        self.generic_static_accessors
                            .insert(id, (constraint_id, member_name));
                        // Search every bound trait (`T: A + B`) for the member.
                        let member_id = bound_trait_ids.iter().find_map(|trait_id| {
                            self.traits
                                .get(trait_id)
                                .and_then(|trait_| trait_.declarations.get(member_name).copied())
                        });
                        match member_id {
                            Some(member_id) => {
                                let rc = self.reference_count.entry(member_id).or_insert(0);
                                *rc += 1;
                                self.expr_id_to_expr_map.insert(id, Expr::Local(member_id));
                            }
                            None => {
                                let bounds = bound_trait_ids
                                    .iter()
                                    .filter_map(|trait_id| {
                                        self.traits.get(trait_id).map(|t| t.name)
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" + ");
                                self.diagnostics.push(Error {
                                    span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                                    msg: format!(
                                        "no bound of this type parameter ({}) has a member '{}'",
                                        bounds, member_name
                                    ),
                                });
                            }
                        }
                    }
                }
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
            // Required members are the trait's signature-only declarations; a
            // member with a default body is inherited, so an impl need not
            // provide it.
            if !self.traits.contains_key(&trait_id) {
                self.diagnostics.push(Error {
                    span: check.span,
                    msg: format!("'{}' is not a trait", check.trait_name),
                });
                continue;
            }
            // Record the trait on its impl so method calls on the subject can
            // fall back to the trait's inherited default methods.
            if let Some(implementation) = self.implementations.get_mut(check.implementation_index) {
                implementation.trait_ids.push(trait_id);
            }
            // Required members are the signature-only declarations of the trait
            // AND its supertraits (a member with a default body is inherited, so
            // an impl need not provide it). Implementing `X with Ord` thus
            // requires the members of `Ord` plus `Eq`/`PartialOrd`/`PartialEq`.
            let required: Vec<&'src str> = self
                .trait_with_supertraits(trait_id)
                .into_iter()
                .filter_map(|id| self.traits.get(&id))
                .flat_map(|trait_| {
                    trait_
                        .declarations
                        .iter()
                        .filter(|(_, member_id)| !self.member_has_default_body(**member_id))
                        .map(|(name, _)| *name)
                })
                .collect();
            let subject_name = match check.subject_type_id.get_type(self) {
                Type::Struct(struct_id, _) => self
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
        // A true fixpoint: each pass resolves at least one deferred item along a
        // live dependency chain (its blocked dependents resolve on later passes)
        // and sets `progress`; resolved types never revert. The loop exits the
        // moment a pass resolves nothing — so it is order-independent: whatever
        // can resolve eventually does, regardless of which pass reaches it. The
        // bound below is only a safety net against a non-converging bug; it is
        // never the reason a well-typed program resolves, so it just has to
        // exceed any real chain. Each resolution consumes a distinct deferred
        // item, and the items (including the slot unifications generated
        // mid-solve while resolving `push`/`run`) are bounded by the entity
        // count, so twice it is ample.
        let max_iterations = 2 * self.entity_id as usize + 16;

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
                        let type_id = Type::Struct(struct_id, Vec::new()).get_type_id(self);
                        self.resolved_types.insert(initializer_id, type_id);
                        self.struct_initializer_to_def
                            .insert(initializer_id, struct_id);
                    }
                }
                if let Some(deferred) = defer {
                    unresolved_constraints.push(deferred);
                } else {
                    // The constraint fully resolved this pass.
                    progress = true;
                }
                // Always store the struct initializer expression so infer_type can handle it.
                self.expr_id_to_expr_map.insert(
                    initializer_id,
                    Expr::StructInitializer(initializer_id, initializer_fields),
                );
                // Store the mapping from initializer to struct definition.
                self.struct_initializer_to_def
                    .insert(initializer_id, struct_id);
                let type_id = Type::Struct(struct_id, Vec::new()).get_type_id(self);
                self.resolved_types.insert(initializer_id, type_id);
            }
            self.struct_initializer_constraints = unresolved_constraints;

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
                    Type::Struct(struct_id, _) => {
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

                // Resolve each leg's patterns (an or-pattern has several) and its
                // optional guard.
                let mut resolved_legs: Vec<(Vec<ExprPattern>, Option<Id>, Id)> = Vec::new();
                let mut pattern_error = false;
                let mut guard_deferred = false;
                for leg in &prepped.legs {
                    let mut resolved_patterns = Vec::new();
                    for pattern in &leg.patterns {
                        match self.resolve_pattern(pattern, subject_type_id, prepped.scope_id) {
                            Some(resolved) => resolved_patterns.push(resolved),
                            None => pattern_error = true,
                        }
                    }
                    if let Some(guard_id) = leg.guard {
                        // A guard must be a resolved `bool`.
                        let guard_type = self.infer_type(guard_id, &Type::Unknown, &HashMap::new());
                        if matches!(guard_type, Type::Unresolved) {
                            guard_deferred = true;
                        } else if !self.compare_type(
                            &guard_type,
                            &self.bool_type(),
                            &HashMap::new(),
                        ) {
                            let got = self.pretty_print_type(&guard_type, &HashMap::new());
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&guard_id).unwrap_or(&&EMPTY_SPAN),
                                msg: format!("match guard must be a bool, but got {}", got),
                            });
                            pattern_error = true;
                        }
                    }
                    resolved_legs.push((resolved_patterns, leg.guard, leg.body));
                }
                if guard_deferred {
                    remaining_matches.push(prepped);
                    continue;
                }
                if pattern_error {
                    // Diagnostics were already emitted; drop the match.
                    progress = true;
                    continue;
                }

                // Exhaustiveness: a leg is an irrefutable catch-all when it is
                // unguarded and a pattern matches anything (`_`, a binding, or a
                // tuple destructure).
                let has_catch_all = resolved_legs.iter().any(|(patterns, guard, _)| {
                    guard.is_none()
                        && patterns.iter().any(|pattern| {
                            matches!(
                                pattern,
                                ExprPattern::Wildcard
                                    | ExprPattern::Binding(_)
                                    | ExprPattern::Tuple(_)
                            )
                        })
                });
                match &subject_type {
                    Type::Enum(enum_id, _) if !has_catch_all => {
                        // Each unguarded variant pattern (in any leg) covers its
                        // variant.
                        let covered = resolved_legs
                            .iter()
                            .filter(|(_, guard, _)| guard.is_none())
                            .flat_map(|(patterns, _, _)| patterns)
                            .filter_map(|pattern| match pattern {
                                ExprPattern::Variant(_, variant_index, _) => Some(*variant_index),
                                _ => None,
                            })
                            .collect::<HashSet<_>>();
                        let missing = self
                            .enums
                            .get(enum_id)
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
                    // A non-enum subject (e.g. a `str` matched with literals) has
                    // an unbounded domain, so it needs an explicit catch-all. Tuples
                    // and not-yet-known types are exempt.
                    Type::Tuple(_) | Type::Unknown | Type::Any | Type::Generic(_) => {}
                    _ if !has_catch_all => {
                        self.diagnostics.push(Error {
                            span: prepped.span,
                            msg: "match is not exhaustive: add a catch-all `_` leg".to_string(),
                        });
                    }
                    _ => {}
                }

                // The match's type unifies the leg body types.
                let mut unified: Option<Type> = None;
                let mut deferred = false;
                for (_, _, body_id) in &resolved_legs {
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
                // Expand each or-pattern leg into one leg per alternative, all
                // sharing the guard and body.
                let legs = resolved_legs
                    .into_iter()
                    .flat_map(|(patterns, guard, body)| {
                        patterns.into_iter().map(move |pattern| ExprMatchLeg {
                            pattern,
                            guard,
                            body,
                        })
                    })
                    .collect();
                self.expr_id_to_expr_map
                    .insert(prepped.id, Expr::Match(prepped.subject_id, legs));
                progress = true;
            }
            self.prepped_matches = remaining_matches;

            // --- Resolve deferred method calls ---
            if !self.prepped_method_calls.is_empty() {
                // The outcome of resolving one method call's receiver to a callable
                // member (kept separate from acting on it so the lookup can borrow
                // `self` immutably and the wiring mutably).
                enum MethodLookup {
                    Found(Id),
                    NoMethod,
                    Defer,
                    NotCallable,
                }
                let mut remaining_methods = Vec::new();
                let method_calls: Vec<_> = std::mem::take(&mut self.prepped_method_calls)
                    .into_iter()
                    .collect();
                for (
                    id,
                    subject_id,
                    member_name,
                    generic_argument_ids,
                    argument_ids,
                    arguments_span,
                ) in method_calls
                {
                    let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
                    // Resolve the method against the receiver's implementations
                    // for a concrete struct/enum, or against a trait's declarations
                    // for an abstract receiver: `Self` in a trait default method
                    // (`Type::Trait`), a trait-bounded generic (`Type::Generic`
                    // whose constraint is a trait), or a trait-typed value.
                    let found = |member_id: Option<Id>| match member_id {
                        Some(member_id) => MethodLookup::Found(member_id),
                        None => MethodLookup::NoMethod,
                    };
                    let lookup = match &subject_type {
                        Type::Struct(_, _) | Type::Enum(_, _) => {
                            match self.method_member_impl_subject(&subject_type, member_name) {
                                Some((member_id, impl_subject_id)) => {
                                    // Bind the impl's generic parameters from the
                                    // receiver (`List<i32>` against the impl's
                                    // `List<T>` binds `T = i32`) so the method body
                                    // monomorphizes.
                                    let impl_subject = impl_subject_id.get_type(self);
                                    if let Some((_, bindings)) = self.reconcile_type(
                                        &impl_subject,
                                        &subject_type,
                                        &HashMap::new(),
                                    ) {
                                        if !bindings.is_empty() {
                                            self.method_call_substitution
                                                .insert(id, bindings.into_iter().collect());
                                        }
                                    }
                                    // A method that fills a container's inference
                                    // slot — `list.push(value)` or
                                    // `context.run(value, ..)` — unifies the slot
                                    // with the value (the first argument).
                                    if member_name == "push" || member_name == "run" {
                                        if let (Some(slot), Some(argument_id)) = (
                                            self.list_element_slot(&subject_type),
                                            argument_ids.first(),
                                        ) {
                                            self.prepped_slot_unifications
                                                .push((slot, *argument_id));
                                        }
                                    }
                                    MethodLookup::Found(member_id)
                                }
                                // Gap E: fall back to an inherited trait default,
                                // re-dispatched to this concrete type at codegen.
                                None => match self
                                    .method_member_in_inherited_defaults(&subject_type, member_name)
                                {
                                    Some(member_id) => {
                                        let receiver_type_id =
                                            subject_type.clone().get_type_id(self);
                                        self.trait_method_dispatch
                                            .insert(id, (Some(receiver_type_id), member_name));
                                        MethodLookup::Found(member_id)
                                    }
                                    None => MethodLookup::NoMethod,
                                },
                            }
                        }
                        Type::Trait(trait_id) => {
                            let member = self.method_member_in_trait(*trait_id, member_name);
                            // Inside a trait default body `self`/`Self` is
                            // `Type::Trait`; record the call so codegen re-dispatches
                            // it to whatever concrete type the default is
                            // specialized for.
                            if member.is_some() {
                                self.trait_method_dispatch.insert(id, (None, member_name));
                            }
                            found(member)
                        }
                        Type::Generic(constraint_id) => {
                            let bound_trait_ids = self.generic_bound_trait_ids(*constraint_id);
                            if bound_trait_ids.is_empty() {
                                match constraint_id.get_type(self) {
                                    Type::Unresolved => MethodLookup::Defer,
                                    _ => MethodLookup::NotCallable,
                                }
                            } else {
                                // Search every bound trait (`T: A + B`) for the method.
                                found(bound_trait_ids.iter().find_map(|trait_id| {
                                    self.method_member_in_trait(*trait_id, member_name)
                                }))
                            }
                        }
                        Type::Unresolved => MethodLookup::Defer,
                        // An unannotated closure parameter awaiting bidirectional
                        // inference (e.g. `|res| { res.method() }` where `res`'s
                        // type is filled in from the expected closure type) defers
                        // rather than erroring; once its type lands the call
                        // resolves. (Only closure params, so an uncalled closure's
                        // method-less param doesn't churn the loop.)
                        Type::Unknown if self.is_unknown_closure_parameter(subject_id) => {
                            MethodLookup::Defer
                        }
                        _ => MethodLookup::NotCallable,
                    };
                    match lookup {
                        MethodLookup::Found(member_id) => {
                            // Drive bidirectional inference of any closure
                            // arguments against the method's parameter types (a
                            // wired method call isn't arg-checked like a free
                            // call, so `builder.on_start(|s| ..)` would otherwise
                            // leave `s` untyped), and defer a full argument
                            // type-check against the parameters.
                            self.infer_closure_args_against_params(member_id, &argument_ids);
                            self.prepped_method_arg_checks
                                .push((member_id, argument_ids.clone()));
                            self.wire_method_call(
                                id,
                                subject_id,
                                member_id,
                                generic_argument_ids,
                                argument_ids,
                                arguments_span,
                            );
                            progress = true;
                        }
                        MethodLookup::NoMethod => {
                            let type_str = self.pretty_print_type(&subject_type, &HashMap::new());
                            self.diagnostics.push(Error {
                                span: arguments_span,
                                msg: format!("{} has no method '{}'", type_str, member_name),
                            });
                            self.expr_id_to_expr_map.insert(id, Expr::Error);
                            progress = true;
                        }
                        MethodLookup::Defer => {
                            remaining_methods.push((
                                id,
                                subject_id,
                                member_name,
                                generic_argument_ids,
                                argument_ids,
                                arguments_span,
                            ));
                        }
                        MethodLookup::NotCallable => {
                            let type_str = self.pretty_print_type(&subject_type, &HashMap::new());
                            self.diagnostics.push(Error {
                                span: arguments_span,
                                msg: format!(
                                    "cannot call method '{}' on {}",
                                    member_name, type_str
                                ),
                            });
                            progress = true;
                        }
                    }
                }
                self.prepped_method_calls = remaining_methods;
            }

            // --- Unify `List` element slots from `push` calls ---
            // `list.push(value)` writes the list's element inference slot
            // (`List::new()`'s fresh `Unknown`) with the pushed value's type, so a
            // built-up list's element is inferred.
            if !self.prepped_slot_unifications.is_empty() {
                let mut remaining = Vec::new();
                for (slot, argument_id) in std::mem::take(&mut self.prepped_slot_unifications) {
                    if !matches!(slot.get_type(self), Type::Unknown) {
                        // Already unified by an earlier push.
                        continue;
                    }
                    let argument_type =
                        self.infer_type(argument_id, &Type::Unknown, &HashMap::new());
                    if matches!(argument_type, Type::Unresolved) {
                        remaining.push((slot, argument_id));
                        continue;
                    }
                    if !matches!(argument_type, Type::Unknown) {
                        self.type_id_to_type_map.insert(slot, argument_type);
                        progress = true;
                    }
                }
                self.prepped_slot_unifications = remaining;
            }

            // --- Resolve `for x in iterable` element bindings ---
            // Once the iterable's type is known, the item takes its element type
            // (`List<i32>` -> `i32`), falling back to `any` when the element type
            // can't be recovered (an erased `List`, a custom iterator). Done in
            // the loop so a method call on the item — deferred while the item is
            // still `Unknown` — resolves once the element type lands.
            if !self.prepped_for_each_items.is_empty() {
                let mut remaining_items = Vec::new();
                for (item_id, iterable_id) in std::mem::take(&mut self.prepped_for_each_items) {
                    let iterable_type =
                        self.infer_type(iterable_id, &Type::Unknown, &HashMap::new());
                    let element_type = self.iterable_element_type(&iterable_type);
                    // Defer while the iterable or its element is still unresolved
                    // (an element slot a later `push` may yet fill); a post-loop
                    // pass commits whatever remains to `any`.
                    if matches!(iterable_type, Type::Unresolved)
                        || matches!(element_type, Some(Type::Unknown | Type::Unresolved))
                    {
                        remaining_items.push((item_id, iterable_id));
                        continue;
                    }
                    let element_type_id = element_type.unwrap_or(Type::Any).get_type_id(self);
                    if let Some(variable) = self.variables.get_mut(&item_id) {
                        variable.type_id = element_type_id;
                    }
                    self.resolved_types.insert(item_id, element_type_id);
                    progress = true;
                }
                self.prepped_for_each_items = remaining_items;
            }

            // --- Type-check method-call arguments against parameters ---
            // Deferred until every argument resolves; then each is reconciled
            // against the method's parameter type. (Only argument checking — no
            // subject re-resolution, which would recurse.)
            if !self.prepped_method_arg_checks.is_empty() {
                let mut remaining = Vec::new();
                'checks: for (member_id, argument_ids) in
                    std::mem::take(&mut self.prepped_method_arg_checks)
                {
                    let parameter_ids = match self.expr_id_to_expr_map.get(&member_id) {
                        Some(Expr::Function(function_id)) => self
                            .functions
                            .get(function_id)
                            .map(|f| f.parameters.clone()),
                        Some(Expr::ExternalFunction(function_id)) => self
                            .external_functions
                            .get(function_id)
                            .map(|f| f.parameters.clone()),
                        _ => None,
                    };
                    let Some(parameter_ids) = parameter_ids else {
                        continue;
                    };
                    // Infer every argument first; defer the whole check until they
                    // all resolve, so errors aren't reported against partial types.
                    let mut argument_types = Vec::with_capacity(argument_ids.len());
                    for argument_id in &argument_ids {
                        // `+ 1` skips the method's `self` parameter.
                        let parameter_type = parameter_ids
                            .get(argument_types.len() + 1)
                            .and_then(|parameter_id| self.parameters.get(parameter_id))
                            .map(|parameter| parameter.type_id.get_type(self))
                            .unwrap_or(Type::Unknown);
                        let argument_type =
                            self.infer_type(*argument_id, &parameter_type, &HashMap::new());
                        if matches!(argument_type, Type::Unresolved) {
                            remaining.push((member_id, argument_ids.clone()));
                            continue 'checks;
                        }
                        argument_types.push(argument_type);
                    }
                    for (index, argument_type) in argument_types.into_iter().enumerate() {
                        let argument_id = argument_ids[index];
                        let Some(parameter_type) = parameter_ids
                            .get(index + 1)
                            .and_then(|parameter_id| self.parameters.get(parameter_id))
                            .map(|parameter| parameter.type_id.get_type(self))
                        else {
                            continue;
                        };
                        if self
                            .reconcile_type(&argument_type, &parameter_type, &HashMap::new())
                            .is_none()
                        {
                            let expected = self.pretty_print_type(&parameter_type, &HashMap::new());
                            let got = self.pretty_print_type(&argument_type, &HashMap::new());
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&argument_id).unwrap_or(&&EMPTY_SPAN),
                                msg: format!("Expected {}, but got {} instead.", expected, got),
                            });
                        }
                    }
                    progress = true;
                }
                self.prepped_method_arg_checks = remaining;
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
            }
            self.call_subject_constraints = remaining_calls;

            // A true fixpoint: a pass that resolves nothing is stuck — every
            // resolution above sets `progress`, so unresolved work that *could*
            // make progress would have. Whatever remains is reported below.
            if !progress {
                break;
            }
        }

        // Commit any `for x in iterable` bindings still deferred (their element
        // slot never resolved — an empty, never-pushed list): the item is `any`.
        for (item_id, iterable_id) in std::mem::take(&mut self.prepped_for_each_items) {
            let iterable_type = self.infer_type(iterable_id, &Type::Unknown, &HashMap::new());
            let element_type = self
                .iterable_element_type(&iterable_type)
                .filter(|element| !matches!(element, Type::Unknown | Type::Unresolved))
                .unwrap_or(Type::Any);
            let element_type_id = element_type.get_type_id(self);
            if let Some(variable) = self.variables.get_mut(&item_id) {
                variable.type_id = element_type_id;
            }
            self.resolved_types.insert(item_id, element_type_id);
        }

        // --- Resolve `for x in iterable` to the Iterator protocol where it
        // applies --- a concrete iterable with a `next` method (a custom
        // iterator) iterates by calling `next()` until `None`; anything else
        // (e.g. a `List`) stays a native `for...of`.
        for (for_each_id, iterable_id) in std::mem::take(&mut self.prepped_for_each) {
            let iterable_type = self.infer_type(iterable_id, &Type::Unknown, &HashMap::new());
            if matches!(iterable_type, Type::Struct(_, _) | Type::Enum(_, _)) {
                if let Some(next_id) = self.method_member_in_impls(&iterable_type, "next") {
                    self.for_each_next.insert(for_each_id, next_id);
                }
            }
        }

        // --- Resolve operator overloading --- an arithmetic `a <op> b` whose
        // left operand's type implements the matching operator trait
        // (`impl T with Add` for `+`, ...) dispatches to that method; everything
        // else (numbers, strings) keeps native JS arithmetic.
        for (binary_id, op, lhs_id) in std::mem::take(&mut self.prepped_binary_ops) {
            let lhs_type = self.infer_type(lhs_id, &Type::Unknown, &HashMap::new());
            if matches!(lhs_type, Type::Struct(_, _) | Type::Enum(_, _)) {
                if let Some(method_id) = self.operator_method(op, &lhs_type) {
                    self.binary_op_dispatch.insert(binary_id, method_id);
                }
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
        let unresolved_matches: Vec<(Id, Span)> = self
            .prepped_matches
            .iter()
            .map(|prepped| (prepped.subject_id, prepped.span))
            .collect();
        for (subject_id, span) in unresolved_matches {
            let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
            let subject_str = self.pretty_print_type(&subject_type, &HashMap::new());
            self.diagnostics.push(Error {
                span,
                msg: format!(
                    "type of match expression could not be resolved (subject: {})",
                    subject_str
                ),
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

    /// Appends `<A, B>` to `buf` for a nominal type's arguments (nothing when
    /// there are none), so `Option<i32>` reads as `enum Option<i32>`.
    fn push_type_arguments(
        &self,
        buf: &mut String,
        arguments: &[TypeId],
        substitution: &SubstitutionContext,
    ) {
        if arguments.is_empty() {
            return;
        }
        buf.push('<');
        for (index, argument) in arguments.iter().enumerate() {
            if index > 0 {
                buf.push_str(", ");
            }
            let argument_type = argument.get_type(self);
            self.pretty_print_type_inner(&argument_type, substitution, buf, 0);
        }
        buf.push('>');
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

            Type::Struct(id, arguments) => {
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
                self.push_type_arguments(buf, arguments, substitution);
            }

            Type::Trait(id) => {
                let trait_ = self.traits.get(id).unwrap();
                buf.push_str(&format!("trait {}", trait_.name));
            }

            Type::Enum(id, arguments) => {
                let enum_ = self.enums.get(id).unwrap();
                buf.push_str(&format!("enum {}", enum_.name));
                self.push_type_arguments(buf, arguments, substitution);
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

/// An `external` std function with a built-in JS lowering.
#[derive(Debug, Clone, Copy)]
pub enum Intrinsic {
    // `scan(): str` — read a line of stdin (runtime helper).
    Scan,
    // `str.trim()` -> native `.trim()`.
    StrTrim,
    // `str.to_lowercase_ascii()` -> native `.toLowerCase()`.
    StrToLowercaseAscii,
    // `str.parse_i32(): Option<i32>` -> a runtime helper returning the enum form.
    ParseI32,
    // `random::i32(low, high): i32` -> a runtime helper over `Math.random`.
    RandomI32,
}

#[derive(Debug)]
pub struct Program<'src> {
    pub closures: IndexMap<Id, Closure>,
    pub diagnostics: Vec<Error>,
    pub enums: IndexMap<Id, Enum<'src>>,
    pub entity_map: HashMap<Id, Expr<'src>>,
    pub entity_scope_map: HashMap<Id, Id>,
    pub function_calls: IndexMap<Id, FunctionCall>,
    pub functions: IndexMap<Id, Function<'src>>,
    pub external_functions: IndexMap<Id, ExternalFunction<'src>>,
    pub traits: IndexMap<Id, Trait<'src>>,
    pub generic_static_accessors: HashMap<Id, (TypeId, &'src str)>,
    pub trait_method_dispatch: HashMap<Id, (Option<TypeId>, &'src str)>,
    pub for_each_next: HashMap<Id, Id>,
    pub binary_op_dispatch: HashMap<Id, Id>,
    pub method_call_substitution: HashMap<Id, SubstitutionContext>,
    pub global_scope_id: Id,
    pub implementations: Vec<Implementation<'src>>,
    // The source `List` intrinsics (`list.vl`), special-cased in codegen
    // (`new` -> `[]`, `push` -> `subject.push(..)`). `None` only if `list.vl`
    // failed to load.
    pub list_new_fn_id: Option<Id>,
    pub list_push_fn_id: Option<Id>,
    // The `std` `panic` intrinsic (if loaded); its calls lower to a `throw`.
    pub panic_fn_id: Option<Id>,
    // The `std::context` `Context` intrinsics (if `context.vl` loaded): the
    // `new`/`run`/`get` method ids. The context threading pass keys off these to
    // find context bindings and their `run`/`get` sites; the transformer lowers
    // `Context::new()` to an opaque value.
    pub context_new_fn_id: Option<Id>,
    pub context_run_fn_id: Option<Id>,
    pub context_get_fn_id: Option<Id>,
    // `external` std functions the transformer lowers to native JS or a runtime
    // helper (`str.trim()`, `scan()`, `random::i32(..)`, ...), keyed by fn id.
    pub intrinsics: HashMap<Id, Intrinsic>,
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
    // The next unused entity id. Post-analysis passes (the context threading
    // pass) mint fresh entities — synthetic parameters and references — from
    // here without colliding with analyzed ones.
    pub next_entity_id: u32,
    // Functions and closures that are async (declared `async`, or inferred — its
    // body awaits, directly or by calling an async function). Filled by the
    // async inference pass; the transformer emits these as `async` and awaits
    // calls to them.
    pub async_functions: HashSet<Id>,
    // Rule 1 (value semantics): value expressions that the transformer wraps in
    // a deep copy because they bind/assign an aggregate place that would
    // otherwise alias its source. Filled by `compute_clone_sites`.
    pub clone_sites: HashSet<Id>,
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
            for (path, leaf) in entries {
                if path.first() != Some(&root) {
                    continue;
                }
                // `import std::option::..` -> the module is the segment after the
                // root; a bare `import std::random` -> the module is the leaf.
                if path.len() >= 2 {
                    modules.push(path[1]);
                } else {
                    modules.push(leaf);
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
    // Every primitive is now migrated to source and captured after its module
    // loads, reachable as the bare name (the prelude, here the global scope) and
    // — since the module scopes below are children of the global scope — as
    // `std::<name>`: scalars `str`/`i32`/... (`string.vl`/`number.vl`), `bool`
    // (`boolean.vl`), `List` (`list.vl`), and `null` (`null.vl`).

    // `List` is the built-in growable array. It is migrated to source
    // (`list.vl`, an `external struct` with `external fun new`/`push`); the
    // struct id and the `new`/`push` intrinsic ids are captured below after the
    // module loads, and the transformer lowers them to `[]` / `.push`.
    let mut list_new_fn_id: Option<Id> = None;
    let mut list_push_fn_id: Option<Id> = None;

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
    // `bool`, `List`, and `null` are core primitives, so their (dependency-free)
    // modules are always loaded even when not imported.
    to_load.push("boolean");
    to_load.push("list");
    to_load.push("null");
    to_load.push("promise");
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
        ("null", "null"),
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

    // Bind the source `List` struct into the global scope (so bare `List`
    // resolves in user code). Its `new`/`push` intrinsic ids are captured after
    // `build()`, once impl subjects resolve.
    let list_struct_id = module_scopes
        .get("list")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("List").copied());
    if let Some(list_struct_id) = list_struct_id {
        analyzer.primitive_struct_ids.insert("List", list_struct_id);
        analyzer
            .mut_scope_for_scope_id(global_scope_id)
            .name_to_id_map
            .insert("List", list_struct_id);
    }

    // The `std::context` `Context` struct, if `context.vl` loaded. Its
    // `new`/`run`/`get` method ids are captured after `build()`, once impl
    // subjects resolve. `Context` is reached only by path (`std::context::..`),
    // so it isn't bound into the global scope.
    let context_struct_id = module_scopes
        .get("context")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("Context").copied());
    // Register `Context` as an element-slot container so `Context<T>`'s value
    // type is inferred from `run` (mirroring `List` + `push`).
    if let Some(context_struct_id) = context_struct_id {
        analyzer
            .primitive_struct_ids
            .insert("Context", context_struct_id);
    }

    // The `std::promise` `Promise<T>` struct, so `async`/`await` type precisely.
    analyzer.promise_struct_id = module_scopes
        .get("promise")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("Promise").copied());
    // Bind `Promise` into the global scope so a bare `Promise<T>` annotation
    // resolves (alongside `std::promise::Promise` by path).
    if let Some(promise_struct_id) = analyzer.promise_struct_id {
        analyzer
            .mut_scope_for_scope_id(global_scope_id)
            .name_to_id_map
            .insert("Promise", promise_struct_id);
    }

    analyzer.walk_expr_nodes(&nodes.0, global_scope_id);
    analyzer.build();
    analyzer.check_readonly_mutation();

    // Find `Context`'s `new`/`run`/`get` intrinsics (the context threading pass
    // keys off them) now that impl subjects have resolved.
    let mut context_new_fn_id: Option<Id> = None;
    let mut context_run_fn_id: Option<Id> = None;
    let mut context_get_fn_id: Option<Id> = None;
    if let Some(context_struct_id) = context_struct_id {
        for implementation in &analyzer.implementations {
            let subject_is_context = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == context_struct_id
            );
            if subject_is_context {
                context_new_fn_id = implementation
                    .declarations
                    .get("new")
                    .copied()
                    .or(context_new_fn_id);
                context_run_fn_id = implementation
                    .declarations
                    .get("run")
                    .copied()
                    .or(context_run_fn_id);
                context_get_fn_id = implementation
                    .declarations
                    .get("get")
                    .copied()
                    .or(context_get_fn_id);
            }
        }
    }

    // Find `List`'s `new`/`push` (special-cased by the transformer to `[]` /
    // `.push`) now that impl subjects have resolved.
    if let Some(list_struct_id) = list_struct_id {
        for implementation in &analyzer.implementations {
            // Match the `List` impl by nominal id, ignoring the subject's type
            // arguments (`impl List<type T>` has subject `List<Generic>`).
            let subject_is_list = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == list_struct_id
            );
            if subject_is_list {
                list_new_fn_id = implementation
                    .declarations
                    .get("new")
                    .copied()
                    .or(list_new_fn_id);
                list_push_fn_id = implementation
                    .declarations
                    .get("push")
                    .copied()
                    .or(list_push_fn_id);
            }
        }
    }

    // Capture the external std functions with built-in JS lowerings: `str`'s
    // methods (across every `impl str` block), and the module-level `scan` /
    // `random::i32`.
    let mut intrinsics: HashMap<Id, Intrinsic> = HashMap::new();
    if let Some(str_struct_id) = analyzer.primitive_struct_ids.get("str").copied() {
        for implementation in &analyzer.implementations {
            let subject_is_str = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == str_struct_id
            );
            if subject_is_str {
                for (name, intrinsic) in [
                    ("trim", Intrinsic::StrTrim),
                    ("to_lowercase_ascii", Intrinsic::StrToLowercaseAscii),
                    ("parse_i32", Intrinsic::ParseI32),
                ] {
                    if let Some(id) = implementation.declarations.get(name).copied() {
                        intrinsics.insert(id, intrinsic);
                    }
                }
            }
        }
    }
    let module_member = |module: &str, name: &str| {
        module_scopes
            .get(module)
            .and_then(|scope_id| analyzer.scopes.get(scope_id))
            .and_then(|scope| scope.name_to_id_map.get(name).copied())
    };
    if let Some(scan_id) = module_member("io", "scan") {
        intrinsics.insert(scan_id, Intrinsic::Scan);
    }
    if let Some(random_id) = module_member("random", "i32") {
        intrinsics.insert(random_id, Intrinsic::RandomI32);
    }

    let clone_sites = analyzer.compute_clone_sites();

    Program {
        closures: analyzer.closures,
        diagnostics: analyzer.diagnostics,
        enums: analyzer.enums,
        entity_map: analyzer.expr_id_to_expr_map,
        entity_scope_map: analyzer.expr_id_to_scope_id_map,
        function_calls: analyzer.function_calls,
        functions: analyzer.functions,
        external_functions: analyzer.external_functions,
        traits: analyzer.traits,
        generic_static_accessors: analyzer.generic_static_accessors,
        trait_method_dispatch: analyzer.trait_method_dispatch,
        for_each_next: analyzer.for_each_next,
        binary_op_dispatch: analyzer.binary_op_dispatch,
        method_call_substitution: analyzer.method_call_substitution,
        intrinsics,
        global_scope_id,
        implementations: analyzer.implementations,
        list_new_fn_id,
        list_push_fn_id,
        panic_fn_id: analyzer.panic_fn_id,
        context_new_fn_id,
        context_run_fn_id,
        context_get_fn_id,
        bool_enum_id: analyzer.bool_enum_id,
        module_id_by_name: analyzer.module_id_by_name,
        modules: analyzer.modules,
        reference_count: analyzer.reference_count,
        scopes: analyzer.scopes,
        span_map: analyzer.span_map,
        structs: analyzer.structs,
        type_id_to_type_map: analyzer.type_id_to_type_map,
        variables: analyzer.variables,
        next_entity_id: analyzer.entity_id,
        async_functions: HashSet::new(),
        clone_sites,
    }
}
