use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::error::Error;
use crate::id::Id;
use crate::node::{
    BinaryOp, Convention, ExternBinding, GenericParameters, ImportBranch, Node, NodeIfBranch,
    NodeList, Pattern,
};
use crate::span::{Span, Spanned};
use crate::target::{Platform, Target};
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
    // A tuple comprehension `(x in xs = e)`: the element binder, the source tuple
    // expr, and the body expr. Types as a mapped tuple and unrolls per element.
    TupleComprehension(Id, Id, Id),
    // An enum declaration.
    Enum(Id),
    // A reference to one variant of an enum: the enum and the variant index.
    // `let (a, b) = value` — destructure `value` (an expr id) by an irrefutable
    // pattern, declaring its bindings. Lowered to a temp + element bindings.
    Destructure(Id, ExprPattern),
    EnumVariant(Id, usize),
    Error,
    ExternalFunction(Id),
    Field(Id, Id, usize),
    // `subject[index]` — a `List` element place: subject and index expr ids. Its
    // type is the list's element type (resolved like a field accessor).
    Index(Id, Id),
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
    // A tuple destructure. Each element carries its flat-storage width (a nested
    // tuple element occupies more than one slot), so the transformer reads each
    // at its flat offset and reslices a multi-slot capture.
    Tuple(Vec<(ExprPattern, usize)>),
    // A literal value test: the matched value equals this literal expression.
    Literal(Id),
}

#[derive(Debug)]
pub struct Function<'src> {
    pub id: Id,
    pub name: &'src str,
    /// The span of the function's name in the source (for go-to-definition and
    /// rename), distinct from the whole-declaration span in `span_map`.
    pub name_span: Span,
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
    /// Set when the signature has a `borrows <param>` clause: the returned view
    /// is a projection of an argument, so it is permitted to escape (rule 3's
    /// sanctioned case) rather than being rejected by `check_view_escape`.
    pub borrows: bool,
    /// Whether the (view) return type is `&mut` rather than `&` — so a binding of
    /// the call (`let v = obj.slot()`) is writable through `*v`.
    pub returns_mut_view: bool,
}

#[derive(Debug)]
pub struct ExternalFunction<'src> {
    pub id: Id,
    pub name: &'src str,
    /// The span of the name in the source (for go-to-definition / rename).
    pub name_span: Span,
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
    /// The span of the binding's name (for go-to-definition / rename).
    pub name_span: Span,
    pub initial: Option<Id>,
    pub type_id: TypeId,
    pub mutable: bool,
}

#[derive(Debug)]
pub struct Struct<'src> {
    pub id: Id,
    pub name: &'src str,
    /// The span of the struct's name (for go-to-definition / rename).
    pub name_span: Span,
    pub generic_parameter_constraint_ids: Vec<TypeId>,
    pub fields: Vec<Field<'src>>,
}

#[derive(Debug, Clone)]
pub struct Field<'src> {
    pub name: &'src str,
    /// The span of just the field's name in the source (for go-to-definition and
    /// rename). Derived from the start of the field declaration.
    pub name_span: Span,
    pub type_id: TypeId,
}

#[derive(Debug)]
pub struct Enum<'src> {
    pub id: Id,
    pub name: &'src str,
    /// The span of the enum's name (for go-to-definition / rename).
    pub name_span: Span,
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
    Variant(
        Vec<&'src str>,
        Span,
        SourceId,
        Option<Vec<WalkPattern<'src>>>,
    ),
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

// A `let (a, b) = value` destructuring binding awaiting its value's type, so the
// pattern's bindings can be typed from the value's (tuple) element types. The
// bindings are already walked into `scope_id`.
#[derive(Debug)]
struct DestructureConstraint<'src> {
    id: Id,
    value_id: Id,
    type_id: Option<TypeId>,
    scope_id: Id,
    pattern: WalkPattern<'src>,
    // A parameter binder's value is an `Unknown`-typed parameter until later
    // inference fills it (a closure passed where `|(A, B)| ..` is expected), so
    // defer while it is still `Unknown` rather than typing bindings as unknown.
    // A `let`'s value is ready immediately, so its `Unknown` is a real error.
    defer_until_known: bool,
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
    /// Each provided trait's generic arguments, in the impl's own generic terms
    /// (`impl Signal<type T> with Readable<T>` -> `[(readable_id, [T])]`). Used to
    /// recover a parameterized trait's arguments for a concrete subject.
    pub trait_args: Vec<(Id, Vec<TypeId>)>,
}

#[derive(Debug)]
pub struct Trait<'src> {
    pub id: Id,
    pub name: &'src str,
    /// The span of the trait's name (for go-to-definition / rename).
    pub name_span: Span,
    /// The trait's own generic parameters' constraint ids (`trait Get<T>` -> the
    /// id of `T`), so a parameterized-trait method call can substitute them with
    /// the trait's concrete arguments.
    pub generic_parameter_constraint_ids: Vec<TypeId>,
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
    // The trait's generic arguments (`with Readable<T>` -> `[T]`), in impl terms.
    pub trait_arguments: Vec<TypeId>,
    pub scope_id: Id,
    pub declarations: IndexMap<&'src str, Id>,
    pub span: Span,
    // The file the `with <trait>` clause is in, for the type-reference index.
    pub source_id: SourceId,
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
    // Destructures for tuple parameters (`|(a, b)| ..`), run before the body.
    pub parameter_destructures: Vec<Id>,
    pub return_: Id,
}

/// A constraint that a struct initializer's field value must
/// match the corresponding struct field type.
#[derive(Debug, Clone)]
pub struct StructInitializerConstraint<'src> {
    pub initializer_id: Id,
    pub struct_id: Id,
    /// The lexical scope the initializer appears in, so the struct name resolves
    /// from there (walking parents) rather than scanning every scope by name.
    pub scope_id: Id,
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

/// A deferred type-inference task, resolved by `Analyzer::try_resolve`. This is
/// the unified replacement for the ~25 per-kind `prepped_*`/`*_constraints`
/// worklists the fixpoint drains today: one queue, resolved in a single explicit
/// order (and, in a later increment, dependency-driven). Variants are migrated
/// over from the old worklists one kind at a time; `priority` reproduces the
/// original inter-section order so each migration stays corpus-identical.
#[derive(Debug)]
enum Constraint<'src> {
    /// `subject[index]` — resolves to the subject `List`'s element type.
    Subscript {
        id: Id,
        subject_id: Id,
        index_id: Id,
    },
    /// `subject is Pattern` — resolves the pattern (typing its captures) once the
    /// subject type is known; the expression itself is `bool`.
    Is(PreppedIs<'src>),
    /// `(x in xs => e)` — once the source `xs` resolves to a mapped tuple, type the
    /// binder `x` as its element so the body `e` checks; the expression is itself a
    /// mapped tuple. Resolved before method calls so a method on `x` sees its type.
    Comprehension {
        id: Id,
        binder_id: Id,
        source_id: Id,
        body_id: Id,
    },
    /// `subject.field` — resolves to the named field's type once the subject
    /// resolves to a struct.
    FieldAccessor(FieldAccessorConstraint<'src>),
    /// `Struct { field = value, .. }` — checks the fields against the struct
    /// definition, infers its type arguments from the values, and records the
    /// initializer once every field value's type is known.
    StructInitializer(StructInitializerConstraint<'src>),
    /// `match subject { .. }` — once the subject type is known, resolves the leg
    /// patterns and guards, checks exhaustiveness, and types the match as the
    /// unification of its leg bodies.
    Match(PreppedMatch<'src>),
    /// `let v = value` (plus any reassignments) — grounds the variable's type
    /// from its first value, then checks the reassignments against it.
    Variable(VariableConstraint),
    /// `let (a, b) = value` — types the pattern's bindings from the value's
    /// (tuple) element types once the value resolves.
    Destructure(DestructureConstraint<'src>),
    /// `receiver.method(args)` — resolves the method against the receiver's type
    /// (impl, trait, or bound generic), binds generics, wires the call, and spawns
    /// a `MethodArgCheck` (and a `SlotUnification` for `push`/`run`).
    MethodCall {
        id: Id,
        subject_id: Id,
        member_name: &'src str,
        generic_argument_ids: Vec<TypeId>,
        argument_ids: Vec<Id>,
        arguments_span: Span,
    },
    /// Unify a container's element inference slot with a pushed value's type
    /// (`list.push(value)`), spawned while resolving a `push`/`run` method call.
    SlotUnification { slot: TypeId, argument_id: Id },
    /// Type-check a wired method call's arguments against the method's parameters,
    /// spawned once the call is wired (deferred until every argument resolves).
    MethodArgCheck {
        member_id: Id,
        argument_ids: Vec<Id>,
        arguments_span: Span,
    },
    /// `for item in iterable` — types the loop item from the iterable's element
    /// type once the iterable resolves (committed to `any` post-fixpoint if it
    /// never does — an empty, never-pushed list).
    ForEachItem { item_id: Id, iterable_id: Id },
    /// `subject(args)` — once the call subject resolves (a function, a closure
    /// value, or an enum variant constructor), checks the arguments and wires the
    /// call, recording any generic bindings inferred from the arguments.
    CallSubject(CallSubjectConstraint),
}

impl Constraint<'_> {
    /// The position this kind resolved at in the original `build()` fixpoint. The
    /// queue is processed in ascending priority, so a migrated section runs where
    /// it always did relative to the not-yet-migrated ones.
    fn priority(&self) -> u8 {
        match self {
            Constraint::StructInitializer(_) => 1,
            // Before method calls (6), so a method on a comprehension binder sees
            // the binder's element type.
            Constraint::Comprehension { .. } => 1,
            Constraint::FieldAccessor(_) => 2,
            Constraint::Subscript { .. } => 3,
            Constraint::Is(_) => 4,
            Constraint::Match(_) => 5,
            Constraint::MethodCall { .. } => 6,
            Constraint::SlotUnification { .. } => 7,
            Constraint::ForEachItem { .. } => 8,
            Constraint::MethodArgCheck { .. } => 9,
            Constraint::Variable(_) => 10,
            Constraint::Destructure(_) => 10,
            Constraint::CallSubject(_) => 11,
        }
    }
}

/// The outcome of resolving a [`Constraint`]. `Failed` has already recorded its
/// diagnostic. `Resolved` and `Failed` are both progress — the task is done and
/// dropped from the queue; `Deferred` re-queues it for a later pass.
enum Resolution {
    Resolved,
    Deferred,
    Failed,
}

impl<'src> StructInitializerConstraint<'src> {
    fn from_walk(
        initializer_id: Id,
        scope_id: Id,
        name: &'src str,
        generic_argument_ids: Vec<TypeId>,
        e_fields: Vec<(&'src str, Id, Span)>,
        fields_span: Span,
    ) -> Self {
        Self {
            initializer_id,
            struct_id: Id(0),
            scope_id,
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
    closures: IndexMap<Id, Closure>,
    diagnostics: Vec<Error>,
    entity_id: u32,
    enums: IndexMap<Id, Enum<'src>>,
    expr_id_to_expr_map: HashMap<Id, Expr<'src>>,
    expr_id_to_scope_id_map: HashMap<Id, Id>,
    expr_id_to_type_id_map: HashMap<Id, TypeId>,
    // The span of the member identifier in a field access or method call (`.x`),
    // keyed by the access expr id — the precise use-site span for rename/nav.
    member_name_spans: HashMap<Id, Span>,
    // The file currently being walked, so type references (which aren't entities)
    // can be tagged with their source for the language server.
    current_source_id: SourceId,
    // Each named type reference in a type position (`Option`, `i32`, a trait
    // bound, ...): its file, name span, the definition it resolves to (when one
    // exists), and its type id (rendered to a hover label after `build`, once
    // all referenced types are resolved). Drives go-to-definition / hover.
    type_references: Vec<(SourceId, Span, Option<Id>, TypeId)>,
    external_functions: IndexMap<Id, ExternalFunction<'src>>,
    // `subject[index]` subscripts awaiting type resolution: index expr id ->
    // The unified constraint queue (replacing the per-kind `prepped_*` worklists,
    // one kind migrated at a time). Drained and re-queued by `resolve_constraints`
    // each fixpoint pass.
    constraints: Vec<Constraint<'src>>,
    function_calls: IndexMap<Id, FunctionCall>,
    functions: IndexMap<Id, Function<'src>>,
    generic_constraint_names: HashMap<TypeId, &'src str>,
    // Static accessors whose subject is a generic parameter (e.g. `T::default`),
    // Calls/accessors the analyzer can't pin to a concrete callee, so codegen
    // re-resolves them per monomorphization (see `GenericDispatch`). One channel
    // for all three shapes — `T::member()` and `value.method()` on a generic, and
    // a trait method re-dispatched to a concrete (or `self`) type. Keyed by the
    // accessor id for the static form (= the call's `subject_id`), else the call
    // id; the keys never collide.
    generic_dispatch: HashMap<Id, GenericDispatch<'src>>,
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
    // Assignments awaiting local resolution: (target accessor id, value id).
    prepped_assignments: Vec<(Id, Id)>,
    // The compiler-synthesized re-read of the target in a compound assignment
    // (`x += v` desugars to `x = x + v`). When the target is a view, this re-read
    // must also read *through* it (transparent references R5), so it is tracked
    // to be deref-wrapped alongside the target — distinct from a user-written
    // value-position read, which keeps its explicit `*` (R6).
    compound_reread_ids: HashSet<Id>,
    prepped_field_accessors: Vec<(Id, Id, &'src str)>,
    prepped_imports: Vec<(Vec<(&'src str, Span)>, &'src str, Id, Span, Span, SourceId)>,
    prepped_locals: Vec<(Id, &'src str)>,
    // Comprehension binders whose element type isn't set yet — a method call on
    // one defers (like an unknown closure parameter) rather than erroring.
    untyped_comprehension_binders: HashSet<Id>,
    // `for x in iterable` expressions, as (for-each id, iterable id), resolved
    // after typing to decide native `for...of` vs the Iterator-protocol loop.
    prepped_for_each: Vec<(Id, Id)>,
    // `for x in iterable` element bindings, as (item variable id, iterable id),
    // resolved in the constraint loop: the item takes the iterable's element
    // type (`List<i32>` -> `i32`), so the body can use it concretely.
    // Method calls whose arguments need checking against the method's parameters,
    // as (member id, explicit argument ids). A wired method call isn't checked by
    // the free-call machinery, so this is a dedicated deferred pass (no subject
    // re-resolution — that recurses); it also drives bidirectional closure-arg
    // inference. The method's first parameter is `self`, so args align at +1. The
    // `Span` is the call's argument list, for the arity diagnostic.
    // For-each loops whose iterable is a custom iterator: the resolved `next`
    // method id, so codegen emits a `next()`/`Some`-matching loop instead.
    for_each_next: HashMap<Id, Id>,
    // `for e in &mut list` / `for e in &list` — the loop binding is a view of each
    // element rather than a copy. Maps the binding id to whether the view is
    // writable (`&mut`). Drives the indexed-loop lowering + view classification.
    for_each_views: HashMap<Id, bool>,
    // `match opt { Some(let v) => .. }` where `opt` is a call returning a view
    // wrapped in an enum payload (`fun get(..): Option<&mut i32> { Some(&mut
    // self.x) }`, or `Option<&mut Node>` for an aggregate). The capture `v` binds
    // the view directly (a scalar `(base, key)` pair, or an aggregate's JS
    // reference), so it is a view binding. Maps it to `(mutable, scalar)`:
    // `mutable` is `&mut` vs `&`; `scalar` distinguishes a `*v`-deref pair from an
    // aggregate accessed via `.field`. Computed by `compute_wrapped_view_captures`.
    wrapped_view_captures: HashMap<Id, (bool, bool)>,
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
    prepped_trait_impls: Vec<TraitImplCheck<'src>>,
    // Deferred named type references: (target type id, name, scope, span, the
    // walked generic argument type ids). The arguments parameterize the resolved
    // nominal type (`Option<i32>` -> `Enum(option_id, [i32])`); empty for a bare
    // name or a generic parameter.
    prepped_type_locals: Vec<(TypeId, &'src str, Id, Span, Vec<TypeId>, SourceId)>,
    prepped_uses: Vec<(Vec<(&'src str, Span)>, &'src str, Id, Span, Span, SourceId)>,
    prepped_type_static_accessors: Vec<(TypeId, TypeId, &'src str, Span)>,
    reference_count: HashMap<Id, u32>,
    resolved_types: HashMap<Id, TypeId>,
    scope_id: u32,
    scopes: IndexMap<Id, Scope<'src>>,
    span_map: HashMap<Id, &'src Span>,
    struct_initializer_to_def: HashMap<Id, Id>, // initializer_id -> struct definition id
    structs: IndexMap<Id, Struct<'src>>,
    traits: IndexMap<Id, Trait<'src>>,
    type_id_to_type_map: HashMap<TypeId, Type>,
    type_id: u32,
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
        // `!=` dispatches to `eq` too; the transformer negates it. (Resolving the
        // `ne` default here would miss impls that only provide `eq`.)
        BinaryOp::Eq | BinaryOp::NotEq => Some(("PartialEq", "eq")),
        _ => None,
    }
}

fn flatten_namespace_branch<'src>(
    branch: &ImportBranch<'src>,
    path: Vec<(&'src str, Span)>,
    entries: &mut Vec<(Vec<(&'src str, Span)>, &'src str, Span)>,
) {
    match branch {
        ImportBranch::Path(name, span, child_branch) => match child_branch {
            None => entries.push((path, name, *span)),
            Some(child) => {
                let mut path = path;
                path.push((name, *span));
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
            closures: IndexMap::new(),
            diagnostics: Vec::new(),
            entity_id: 0,
            enums: IndexMap::new(),
            expr_id_to_expr_map: HashMap::new(),
            expr_id_to_scope_id_map: HashMap::new(),
            expr_id_to_type_id_map: HashMap::new(),
            member_name_spans: HashMap::new(),
            current_source_id: SourceId(0),
            type_references: Vec::new(),
            external_functions: IndexMap::new(),
            constraints: Vec::new(),
            function_calls: IndexMap::new(),
            functions: IndexMap::new(),
            generic_constraint_names: HashMap::new(),
            generic_dispatch: HashMap::new(),
            generic_bounds: HashMap::new(),
            impl_subject_args: HashMap::new(),
            implementations: Vec::new(),
            module_id_by_name: HashMap::new(),
            modules: IndexMap::new(),
            parameters: IndexMap::new(),
            primitive_struct_ids: HashMap::new(),
            bool_enum_id: None,
            list_element_slots: HashMap::new(),
            prepped_assignments: Vec::new(),
            compound_reread_ids: HashSet::new(),
            prepped_field_accessors: Vec::new(),
            prepped_imports: Vec::new(),
            prepped_locals: Vec::new(),
            untyped_comprehension_binders: HashSet::new(),
            prepped_for_each: Vec::new(),
            for_each_next: HashMap::new(),
            for_each_views: HashMap::new(),
            wrapped_view_captures: HashMap::new(),
            prepped_binary_ops: Vec::new(),
            binary_op_dispatch: HashMap::new(),
            method_call_substitution: HashMap::new(),
            prepped_static_accessors: Vec::new(),
            prepped_trait_impls: Vec::new(),
            prepped_type_locals: Vec::new(),
            prepped_type_static_accessors: Vec::new(),
            prepped_uses: Vec::new(),
            reference_count: HashMap::new(),
            resolved_types: HashMap::new(),
            scope_id: 0,
            scopes: IndexMap::new(),
            span_map: HashMap::new(),
            struct_initializer_to_def: HashMap::new(),
            structs: IndexMap::new(),
            traits: IndexMap::new(),
            type_id_to_type_map: HashMap::new(),
            type_id: 0,
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
        // `unwrap_or_else(|| panic!(..))`, not `expect(format!(..))`: the latter
        // builds the message on *every* call, not just the failing one. This is a
        // hot accessor, so eagerly formatting even `id` — let alone the whole
        // `expr_id_to_expr_map` it used to dump with `{:#?}` — made resolution
        // quadratic in program size.
        self.expr_id_to_expr_map
            .get(&id)
            .unwrap_or_else(|| panic!("failed to get entity for id: {id:?}"))
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

    /// Whether a concrete `subject_type` implements `trait_id` — has an
    /// `impl Subject with Trait`. Lets a concrete value satisfy a trait-typed
    /// parameter (e.g. a `Self`-defaulted generic that resolved to the trait).
    fn type_implements_trait(&self, subject_type: &Type, trait_id: Id) -> bool {
        self.implementations.iter().any(|implementation| {
            implementation.trait_ids.contains(&trait_id)
                && self.compare_type(
                    subject_type,
                    &implementation.subject.get_type(self),
                    &HashMap::new(),
                )
        })
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

    /// Whether an operator (`==`, `<`, `+`, ...) on this type lowers to a native JS
    /// operator: the scalar primitives, `bool`, and numeric (C-like) enums (which
    /// lower to their integer discriminant). Such a type needs no trait dispatch
    /// for its operators, and a missing operator impl is not an error for it —
    /// dispatching would recurse anyway, since the impl body uses the same operator.
    fn is_native_operator_type(&self, type_: &Type) -> bool {
        match type_ {
            Type::Struct(id, _) => ["i32", "u32", "f64", "BigInt", "str"]
                .iter()
                .any(|name| self.primitive_struct_ids.get(*name).copied() == Some(*id)),
            Type::Enum(id, _) => {
                self.bool_enum_id == Some(*id)
                    || self.enums.get(id).is_some_and(|enum_| enum_.is_numeric)
            }
            _ => false,
        }
    }

    /// Resolves the operator method for `op` on `subject_type` — the `add`/`sub`/
    /// `mul`/`div`/`eq` declared by an `impl Subject with Add`/`PartialEq`/... —
    /// returning `(method id, impl subject type)`. The subject lets the caller bind
    /// the impl's generics from the operand (`Option<Point>` against the impl's
    /// `Option<T>`) so the method monomorphizes. `None` for native-operator types
    /// (kept as native JS) and any type without the matching impl.
    fn operator_method(&self, op: BinaryOp, subject_type: &Type) -> Option<(Id, TypeId)> {
        if self.is_native_operator_type(subject_type) {
            return None;
        }
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
            .find_map(|implementation| {
                implementation
                    .declarations
                    .get(method_name)
                    .copied()
                    .map(|method_id| (method_id, implementation.subject))
            })
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
                    if let Type::Trait(super_id, _) = supertrait_type_id.get_type(self) {
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
            .unwrap_or_else(|| panic!("failed to get scope for id: {}", scope_id.0))
    }

    fn get_scope_id_for_entity(&mut self, entity_id: Id) -> Id {
        self.expr_id_to_scope_id_map
            .get(&entity_id)
            .copied()
            .unwrap_or_else(|| panic!("failed to get scope of entity: {}", entity_id.0))
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
        // Each call mints a fresh id; types are intentionally *not* interned.
        // Interning (a `Type -> TypeId` reverse map deduping e.g. every `i32`) is a
        // deferred, low-value optimization with a sharp edge: inference resolves a
        // type *in place* by mutating `type_id_to_type_map[id]` — an `Unknown` slot
        // becoming concrete, a deferred accessor id resolving — so any mutated id
        // must stay unshared. A correct interner would have to exclude `Unknown` /
        // `Unresolved` (and anything else later mutated) and require `Type: Hash + Eq`.
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

    /// The type of a place expression. Falls back to the binding's type for a
    /// `Local` reference, whose own expr id usually carries no type entry (the
    /// type lives on the variable/parameter it names).
    fn place_value_type(&self, expr_id: Id) -> Option<Type> {
        if let Some(value_type) = self.type_of_expr(expr_id) {
            return Some(value_type);
        }
        match self.expr_id_to_expr_map.get(&expr_id)? {
            Expr::Local(binding_id) => {
                if let Some(variable) = self.variables.get(binding_id) {
                    Some(variable.type_id.get_type(self))
                } else {
                    self.parameters
                        .get(binding_id)
                        .map(|parameter| parameter.type_id.get_type(self))
                }
            }
            _ => None,
        }
    }

    /// Whether an expression reads existing aggregate storage (a binding or a
    /// field) rather than producing a fresh value (a literal, constructor, or
    /// call). Only a place can alias, so only a place needs a copy.
    fn is_place_expr(&self, expr_id: Id) -> bool {
        matches!(
            self.expr_id_to_expr_map.get(&expr_id),
            Some(Expr::Local(_)) | Some(Expr::Field(_, _, _)) | Some(Expr::Index(_, _))
        )
    }

    /// If a place is a field/deref chain rooted in something immutable, its name
    /// and the fix hint (`&mut x` for a readonly parameter, `mut` for an
    /// immutable `let` local). `None` when the root is mutable — a `mut` local,
    /// or an `own` / `&mut` parameter. A bare parameter is readonly by default
    /// (the position-default-convention flip).
    fn readonly_root(&self, expr_id: Id) -> Option<(&'src str, &'static str)> {
        match self.expr_id_to_expr_map.get(&expr_id)? {
            Expr::Field(subject_id, _, _) => self.readonly_root(*subject_id),
            Expr::Index(subject_id, _) => self.readonly_root(*subject_id),
            Expr::Dereference(operand_id) => self.readonly_root(*operand_id),
            Expr::Local(binding_id) => {
                if let Some(parameter) = self.parameters.get(binding_id) {
                    matches!(parameter.convention, Convention::Bare | Convention::Ref)
                        .then_some((parameter.name, "`&mut`"))
                } else if let Some(variable) = self.variables.get(binding_id) {
                    // A binding holding a view is governed by the view's
                    // mutability, not the `let`/`mut` of the binding: `let v =
                    // &mut c` is writable, `let v = &c` is readonly.
                    match self.view_binding_mutability(*binding_id) {
                        Some(true) => None,
                        Some(false) => Some((variable.name, "`&mut`")),
                        None => (!variable.mutable).then_some((variable.name, "`mut`")),
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Whether a binding holds a view, and if so whether it is writable: `&mut`
    /// → `Some(true)`, `&` → `Some(false)`, an owned value → `None`. Follows
    /// copies between locals (`let w = v`).
    fn view_binding_mutability(&self, binding_id: Id) -> Option<bool> {
        // A `for e in &mut list` binding, or a `Some(let v)` capture over a
        // wrapped-view call, has no initial; its writability is the view's.
        if let Some(&mutable) = self.for_each_views.get(&binding_id) {
            return Some(mutable);
        }
        if let Some(&(mutable, _scalar)) = self.wrapped_view_captures.get(&binding_id) {
            return Some(mutable);
        }
        let initial = self.variables.get(&binding_id)?.initial?;
        match self.expr_id_to_expr_map.get(&initial)? {
            Expr::Reference(_, mutable) => Some(*mutable),
            Expr::Local(source_id) => self.view_binding_mutability(*source_id),
            // `let v = obj.slot()` borrows a view; its writability is the borrows
            // function's return convention (`&mut` vs `&`).
            Expr::Call(call_id) => {
                let function_call = self.function_calls.get(call_id)?;
                match self.expr_id_to_expr_map.get(&function_call.subject_id)? {
                    Expr::Local(function_id) => {
                        let function = self.functions.get(function_id)?;
                        function.borrows.then_some(function.returns_mut_view)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether a call resolves to a `borrows` function (one that returns a view —
    /// scalar *or* aggregate). The dual of `call_returns_scalar_view`, which only
    /// admits the scalar `(base, key)` shape.
    fn call_returns_view(&self, call_id: Id) -> bool {
        let Some(function_call) = self.function_calls.get(&call_id) else {
            return false;
        };
        matches!(
            self.expr_id_to_expr_map.get(&function_call.subject_id),
            Some(Expr::Local(function_id))
                if self.functions.get(function_id).is_some_and(|function| function.borrows)
        )
    }

    /// Whether a binding or parameter holds a view: a view binding (a `&`/`&mut`
    /// reference, a copy of one, a `borrows`-call result, or a `for e in &mut` /
    /// `Option<&mut T>` capture — all unified by `view_binding_mutability`), or a
    /// `&`/`&mut` parameter.
    fn binding_or_param_is_view(&self, binding_id: Id) -> bool {
        self.view_binding_mutability(binding_id).is_some()
            || self.parameters.get(&binding_id).is_some_and(|parameter| {
                matches!(parameter.convention, Convention::Ref | Convention::RefMut)
            })
    }

    /// Whether an l-value denotes a view, so a bare assignment to it writes
    /// *through* to the referent (transparent-references rule R5): a view binding
    /// or `&`/`&mut` parameter, a `borrows`-returning call, or a `&[mut] place`
    /// reference. A field/index projection (`view.field`) is *not* a view — it is
    /// an ordinary place reached by auto-derefing the view, so it is excluded.
    fn assignment_target_is_view(&self, expr_id: Id) -> bool {
        match self.expr_id_to_expr_map.get(&expr_id) {
            Some(Expr::Reference(_, _)) => true,
            Some(Expr::Local(binding_id)) => self.binding_or_param_is_view(*binding_id),
            Some(Expr::Call(call_id)) => self.call_returns_view(*call_id),
            _ => false,
        }
    }

    /// Transparent-references rule R5: a bare assignment to a view writes
    /// *through* it. Wrap a view-typed assignment target (and, for a compound
    /// assignment, the synthesized re-read of that target) in a synthetic
    /// `Dereference` so it lowers exactly like the explicit `*target`
    /// write-through path — unifying `x = v` with the former `*x = v` at zero cost
    /// to codegen (so the migration off `*` is byte-identical). Runs after
    /// `infer_borrows`, so `borrows`-call targets are recognized.
    fn rewrite_view_assignment_targets(&mut self) {
        let assignments: Vec<(Id, Id, Id)> = self
            .expr_id_to_expr_map
            .iter()
            .filter_map(|(id, expr)| match expr {
                Expr::Assignment(target, value) => Some((*id, *target, *value)),
                _ => None,
            })
            .collect();
        for (assignment_id, target_id, value_id) in assignments {
            // The target: a bare view writes *through*, so deref it.
            if !matches!(
                self.expr_id_to_expr_map.get(&target_id),
                Some(Expr::Dereference(_))
            ) && self.assignment_target_is_view(target_id)
            {
                let deref_id = self.wrap_in_deref(target_id);
                self.expr_id_to_expr_map
                    .insert(assignment_id, Expr::Assignment(deref_id, value_id));
            }
            // A compound assignment (`x op= v` → `x = x + v`) re-reads the target;
            // when that target is a view, the synthesized re-read must read
            // *through* it too. (A user-written value-position read is not in
            // `compound_reread_ids`, so it keeps needing an explicit `*` — R6.)
            let compound = match self.expr_id_to_expr_map.get(&value_id) {
                Some(Expr::Binary(op, lhs_id, rhs_id)) => Some((*op, *lhs_id, *rhs_id)),
                _ => None,
            };
            if let Some((op, lhs_id, rhs_id)) = compound
                && self.compound_reread_ids.contains(&lhs_id)
                && !matches!(
                    self.expr_id_to_expr_map.get(&lhs_id),
                    Some(Expr::Dereference(_))
                )
                && self.assignment_target_is_view(lhs_id)
            {
                let deref_id = self.wrap_in_deref(lhs_id);
                self.expr_id_to_expr_map
                    .insert(value_id, Expr::Binary(op, deref_id, rhs_id));
            }
        }
    }

    /// Mint a synthetic `Dereference` of `operand_id`, carrying its span / scope /
    /// type so diagnostics and codegen treat it like an explicit `*operand`.
    fn wrap_in_deref(&mut self, operand_id: Id) -> Id {
        let deref_id = self.new_entity_id();
        self.expr_id_to_expr_map
            .insert(deref_id, Expr::Dereference(operand_id));
        if let Some(span) = self.span_map.get(&operand_id).copied() {
            self.span_map.insert(deref_id, span);
        }
        if let Some(scope_id) = self.expr_id_to_scope_id_map.get(&operand_id).copied() {
            self.expr_id_to_scope_id_map.insert(deref_id, scope_id);
        }
        if let Some(type_id) = self.expr_id_to_type_id_map.get(&operand_id).copied() {
            self.expr_id_to_type_id_map.insert(deref_id, type_id);
        }
        deref_id
    }

    /// Whether an expression evaluates to a *view* (rule 3, second-class): a
    /// `&`/`&mut` reference, a binding that holds one, or a `&`/`&mut` parameter.
    /// (A bare parameter is conceptually a readonly view too, but is excluded
    /// here — flagging every returned bare parameter is deferred.)
    fn is_view_expr(&self, expr_id: Id, view_bindings: &HashSet<Id>) -> bool {
        match self.expr_id_to_expr_map.get(&expr_id) {
            Some(Expr::Reference(_, _)) => true,
            Some(Expr::Local(binding_id)) => {
                view_bindings.contains(binding_id)
                    || self.parameters.get(binding_id).is_some_and(|parameter| {
                        matches!(parameter.convention, Convention::Ref | Convention::RefMut)
                    })
            }
            _ => false,
        }
    }

    /// Local bindings that hold a view (`let v = &x`, or `let w = v` where `v`
    /// is one) — a greatest fixpoint, since a view can be copied between locals.
    fn compute_view_bindings(&self) -> HashSet<Id> {
        // A `for e in &mut list` binding, or a `Some(let v)` capture over a
        // wrapped-view call, holds a view with no `initial` to follow.
        let mut view_bindings: HashSet<Id> = self.for_each_views.keys().copied().collect();
        view_bindings.extend(self.wrapped_view_captures.keys().copied());
        loop {
            let mut changed = false;
            for variable in self.variables.values() {
                if view_bindings.contains(&variable.id) {
                    continue;
                }
                if let Some(initial) = variable.initial {
                    if self.is_view_expr(initial, &view_bindings) {
                        view_bindings.insert(variable.id);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        view_bindings
    }

    /// Whether a call is an enum-variant constructor (`Some(x)`, `Ok(x)`) rather
    /// than an ordinary function call — its arguments become the payload of a
    /// value-semantics enum, so a view among them escapes. The callee resolves to
    /// an `EnumVariant`, directly or through a `Local` name reference.
    fn call_is_variant_constructor(&self, call_id: Id) -> bool {
        let Some(function_call) = self.function_calls.get(&call_id) else {
            return false;
        };
        let mut subject = function_call.subject_id;
        if let Some(Expr::Local(target)) = self.expr_id_to_expr_map.get(&subject) {
            subject = *target;
        }
        matches!(
            self.expr_id_to_expr_map.get(&subject),
            Some(Expr::EnumVariant(..))
        )
    }

    /// Rule 3 (second-class): a view may not escape its scope — it cannot be
    /// returned, stored in a struct field, placed in a collection, or carried in
    /// an enum payload. (Passing a view as an argument, or binding it to a local,
    /// is fine.) This is what removes lifetimes: a view structurally cannot
    /// outlive its target. Returning a borrow derived from an argument is
    /// Phase 5 — infer the `borrows` effect. A function whose returned view
    /// projects a `&`/`&mut` parameter borrows that parameter: the caller's
    /// argument outlives the call, so the view is sound, whether or not the
    /// signature spells `borrows` out. (A returned view of a *local* still
    /// dangles, and `derives_from_view_param` excludes it, so it stays rejected
    /// by `check_view_escape`.) An inferred function is then indistinguishable
    /// from an explicit `borrows` one for escape, scalar-view lowering, and
    /// binding writability — they all read `Function.borrows`.
    ///
    /// Runs before `check_view_escape` (and the scalar-view-call analysis) so the
    /// flipped flag is visible to every consumer. The passing corpus is
    /// unaffected: a function that returns such a view today *must* already say
    /// `borrows` or it fails to compile, so this only newly admits programs that
    /// previously errored.
    fn infer_borrows(&mut self) {
        let view_bindings = self.compute_view_bindings();
        let capturing: HashSet<Id> = self
            .closures
            .keys()
            .copied()
            .filter(|closure_id| self.closure_captures_view_param(*closure_id))
            .collect();
        let inferred: Vec<Id> = self
            .functions
            .values()
            .filter(|function| {
                function.has_body
                    && !function.borrows
                    && self.escapes_as_view(function.body.1, &view_bindings, &capturing)
                    && self.derives_from_view_param(function.body.1)
            })
            .map(|function| function.id)
            .collect();
        for function_id in inferred {
            if let Some(function) = self.functions.get_mut(&function_id) {
                function.borrows = true;
            }
        }
    }

    /// recovered later by `borrows` (Phase 5).
    fn check_view_escape(&mut self) {
        let view_bindings = self.compute_view_bindings();
        // A closure that captures a view is itself second-class (P4c): it cannot
        // escape either, or the captured view would outlive its target.
        let closure_ids: Vec<Id> = self.closures.keys().copied().collect();
        let capturing: HashSet<Id> = closure_ids
            .into_iter()
            .filter(|closure_id| self.closure_captures_view_param(*closure_id))
            .collect();
        // Variant constructors that *are* a function's `Some(&mut self.x)` return
        // (including each branch of a conditional `if live { Some(..) } else { None
        // }`): the wrapped view projects a parameter, so the pair is a sanctioned
        // borrow the caller unwraps via `match`, not an escape.
        let function_ids: Vec<Id> = self.functions.keys().copied().collect();
        let sanctioned_wrapped: HashSet<Id> = function_ids
            .into_iter()
            .flat_map(|function_id| self.wrapped_view_return_calls(function_id))
            .collect();
        let mut escapes: Vec<Id> = Vec::new();
        for expr in self.expr_id_to_expr_map.values() {
            match expr {
                Expr::FunctionReturn(value_id)
                    if self.escapes_as_view(*value_id, &view_bindings, &capturing) =>
                {
                    escapes.push(*value_id);
                }
                Expr::StructInitializer(_, fields) => escapes.extend(
                    fields
                        .values()
                        .copied()
                        .filter(|id| self.escapes_as_view(*id, &view_bindings, &capturing)),
                ),
                Expr::List(ids) | Expr::Tuple(ids) => escapes.extend(
                    ids.iter()
                        .copied()
                        .filter(|id| self.escapes_as_view(*id, &view_bindings, &capturing)),
                ),
                // A view passed to an enum-variant constructor (`Some(&mut x)`) is
                // stored in the payload by value, so it escapes — the same as a
                // struct field. (A view passed to an ordinary function is fine; the
                // callee only borrows it for the call, so those calls are skipped.)
                Expr::Call(call_id)
                    if self.call_is_variant_constructor(*call_id)
                        && !sanctioned_wrapped.contains(call_id) =>
                {
                    if let Some(function_call) = self.function_calls.get(call_id) {
                        escapes.extend(
                            function_call
                                .argument_ids
                                .iter()
                                .copied()
                                .filter(|id| self.escapes_as_view(*id, &view_bindings, &capturing)),
                        );
                    }
                }
                _ => {}
            }
        }
        // A function or closure body whose trailing expression escapes a view
        // returns it implicitly. A `borrows` function may return one — but only a
        // projection of a (view) parameter, whose target the caller keeps alive;
        // a view of a local still dangles and is rejected.
        for function in self.functions.values() {
            if function.has_body
                && self.escapes_as_view(function.body.1, &view_bindings, &capturing)
                && !(function.borrows && self.derives_from_view_param(function.body.1))
            {
                escapes.push(function.body.1);
            }
        }
        let closure_returns: Vec<Id> = self.closures.values().map(|c| c.return_).collect();
        for return_id in closure_returns {
            if self.escapes_as_view(return_id, &view_bindings, &capturing) {
                escapes.push(return_id);
            }
        }
        for expr_id in escapes {
            self.diagnostics.push(Error {
                span: **self.span_map.get(&expr_id).unwrap_or(&&EMPTY_SPAN),
                msg: "a view cannot escape its scope: it may not be returned, stored in a field, placed in a collection, or carried in an enum payload. Return an owned value or a handle instead.".to_string(),
            });
        }
    }

    /// Whether an expression escapes a view when placed in a return / field /
    /// collection position: a view itself, or a closure that captures one.
    fn escapes_as_view(
        &self,
        expr_id: Id,
        view_bindings: &HashSet<Id>,
        capturing: &HashSet<Id>,
    ) -> bool {
        if self.is_view_expr(expr_id, view_bindings) {
            return true;
        }
        matches!(
            self.expr_id_to_expr_map.get(&expr_id),
            Some(Expr::Closure(closure_id)) | Some(Expr::Async(closure_id))
                if capturing.contains(closure_id)
        )
    }

    /// Whether an expression (a `borrows` function's returned view) projects a
    /// `&`/`&mut` parameter — the caller's argument, which outlives the call, so
    /// the returned view is sound. A view of a local would dangle, so it isn't.
    fn derives_from_view_param(&self, expr_id: Id) -> bool {
        let mut found = false;
        let mut visited = HashSet::new();
        self.scan_view_param_ref(expr_id, &mut found, &mut visited);
        found
    }

    /// Whether a closure body references a `&` / `&mut` parameter — necessarily
    /// one captured from an enclosing function, since closure parameters are
    /// always bare. (Capturing an outer view *binding* is deferred.)
    fn closure_captures_view_param(&self, closure_id: Id) -> bool {
        let Some(closure) = self.closures.get(&closure_id) else {
            return false;
        };
        let mut captured = false;
        let mut visited = HashSet::new();
        self.scan_view_param_ref(closure.return_, &mut captured, &mut visited);
        captured
    }

    fn scan_view_param_ref(&self, expr_id: Id, captured: &mut bool, visited: &mut HashSet<Id>) {
        if *captured || !visited.insert(expr_id) {
            return;
        }
        let Some(expr) = self.expr_id_to_expr_map.get(&expr_id).cloned() else {
            return;
        };
        match expr {
            Expr::Local(binding_id) => {
                if self.parameters.get(&binding_id).is_some_and(|parameter| {
                    matches!(parameter.convention, Convention::Ref | Convention::RefMut)
                }) {
                    *captured = true;
                }
            }
            // Nested closures are their own nodes; their captures are checked
            // separately.
            Expr::Closure(_) | Expr::Async(_) => {}
            Expr::Variable(variable_id) => {
                if let Some(initial) = self.variables.get(&variable_id).and_then(|v| v.initial) {
                    self.scan_view_param_ref(initial, captured, visited);
                }
            }
            Expr::Reference(operand, _) | Expr::Dereference(operand) | Expr::Unary(_, operand) => {
                self.scan_view_param_ref(operand, captured, visited)
            }
            Expr::Binary(_, lhs, rhs) => {
                self.scan_view_param_ref(lhs, captured, visited);
                self.scan_view_param_ref(rhs, captured, visited);
            }
            Expr::Assignment(target, value) => {
                self.scan_view_param_ref(target, captured, visited);
                self.scan_view_param_ref(value, captured, visited);
            }
            Expr::Field(subject, _, _) => self.scan_view_param_ref(subject, captured, visited),
            Expr::Index(subject, index) => {
                self.scan_view_param_ref(subject, captured, visited);
                self.scan_view_param_ref(index, captured, visited);
            }
            Expr::FunctionReturn(value) | Expr::Await(value) => {
                self.scan_view_param_ref(value, captured, visited)
            }
            Expr::Call(call_id) => {
                if let Some(function_call) = self.function_calls.get(&call_id) {
                    for argument in function_call.argument_ids.clone() {
                        self.scan_view_param_ref(argument, captured, visited);
                    }
                }
            }
            Expr::Block((statements, tail)) => {
                for statement in statements {
                    self.scan_view_param_ref(statement, captured, visited);
                }
                self.scan_view_param_ref(tail, captured, visited);
            }
            Expr::For(condition, (statements, tail)) => {
                if let Some(condition) = condition {
                    self.scan_view_param_ref(condition, captured, visited);
                }
                for statement in statements {
                    self.scan_view_param_ref(statement, captured, visited);
                }
                self.scan_view_param_ref(tail, captured, visited);
            }
            Expr::ForEach(iterable, _, (statements, tail)) => {
                self.scan_view_param_ref(iterable, captured, visited);
                for statement in statements {
                    self.scan_view_param_ref(statement, captured, visited);
                }
                self.scan_view_param_ref(tail, captured, visited);
            }
            Expr::Match(subject, legs) => {
                self.scan_view_param_ref(subject, captured, visited);
                for leg in legs {
                    if let Some(guard) = leg.guard {
                        self.scan_view_param_ref(guard, captured, visited);
                    }
                    self.scan_view_param_ref(leg.body, captured, visited);
                }
            }
            Expr::List(ids) | Expr::Tuple(ids) => {
                for id in ids {
                    self.scan_view_param_ref(id, captured, visited);
                }
            }
            Expr::StructInitializer(_, fields) => {
                for value in fields.values() {
                    self.scan_view_param_ref(*value, captured, visited);
                }
            }
            // `If` is the one branch form left; walk it explicitly.
            Expr::If(branch) => self.scan_view_param_ref_if(&branch, captured, visited),
            _ => {}
        }
    }

    fn scan_view_param_ref_if(
        &self,
        branch: &ExprIfBranch,
        captured: &mut bool,
        visited: &mut HashSet<Id>,
    ) {
        match branch {
            ExprIfBranch::If(condition, (statements, tail), else_branch) => {
                self.scan_view_param_ref(*condition, captured, visited);
                for statement in statements {
                    self.scan_view_param_ref(*statement, captured, visited);
                }
                self.scan_view_param_ref(*tail, captured, visited);
                if let Some(else_branch) = else_branch {
                    self.scan_view_param_ref_if(else_branch, captured, visited);
                }
            }
            ExprIfBranch::Else((statements, tail)) => {
                for statement in statements {
                    self.scan_view_param_ref(*statement, captured, visited);
                }
                self.scan_view_param_ref(*tail, captured, visited);
            }
        }
    }

    /// The root binding of a place expression (`x.field.field` / `*v` → `x`).
    fn place_root(&self, expr_id: Id) -> Option<Id> {
        match self.expr_id_to_expr_map.get(&expr_id)? {
            Expr::Local(binding_id) => Some(*binding_id),
            Expr::Field(subject_id, _, _) => self.place_root(*subject_id),
            Expr::Index(subject_id, _) => self.place_root(*subject_id),
            Expr::Dereference(operand_id) => self.place_root(*operand_id),
            _ => None,
        }
    }

    /// For each local view binding, the *local* root it borrows (`let v = &x` →
    /// `x`; `let w = v` → `v`'s root). A fixpoint, since views copy between
    /// locals. Views of parameters borrow outside the function, so they have no
    /// local root and are omitted.
    fn compute_view_origins(&self) -> HashMap<Id, Id> {
        let mut origins: HashMap<Id, Id> = HashMap::new();
        loop {
            let mut changed = false;
            for variable in self.variables.values() {
                if origins.contains_key(&variable.id) {
                    continue;
                }
                let Some(initial) = variable.initial else {
                    continue;
                };
                let root = match self.expr_id_to_expr_map.get(&initial) {
                    Some(Expr::Reference(operand_id, _)) => self.place_root(*operand_id),
                    Some(Expr::Local(source_id)) => origins.get(source_id).copied(),
                    _ => None,
                };
                if let Some(root) = root {
                    origins.insert(variable.id, root);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        origins
    }

    /// Slice 4 (primitive-local view boxing): scalar locals that have a view
    /// taken of them. A JS number isn't addressable, so a viewed scalar local is
    /// boxed into a one-slot cell `[value]`; its reads/writes go through `[0]`
    /// and `&`/`&mut` of it yields the cell. Aggregates (already JS objects) and
    /// field/element views need no boxing.
    fn compute_boxed_locals(&self) -> HashSet<Id> {
        let mut boxed = HashSet::new();
        for expr in self.expr_id_to_expr_map.values() {
            if let Expr::Reference(operand, _) = expr {
                if let Some(root) = self.place_root(*operand) {
                    let is_scalar_local = self.variables.get(&root).is_some_and(|variable| {
                        matches!(variable.type_id.get_type(self), Type::Struct(id, _) if self.is_scalar_primitive(id))
                    });
                    if is_scalar_local {
                        boxed.insert(root);
                    }
                }
            }
        }
        boxed
    }

    /// Whether a place is a scalar primitive — the case that needs a `(base, key)`
    /// view, since a JS number/string isn't addressable on its own.
    fn place_is_scalar(&self, expr_id: Id) -> bool {
        matches!(
            self.place_value_type(expr_id),
            Some(Type::Struct(id, _)) if self.is_scalar_primitive(id)
        )
    }

    /// Whether a function returns a scalar `(base, key)` view: it has a `borrows`
    /// clause and its return type's pointee is a scalar (`&mut i32` collapses to
    /// `i32`, so the scalar check is on the collapsed `return_type_id`).
    fn function_returns_scalar_view(&self, function_id: Id) -> bool {
        self.functions.get(&function_id).is_some_and(|function| {
            function.borrows
                && matches!(
                    function.return_type_id.map(|type_id| type_id.get_type(self)),
                    Some(Type::Struct(id, _)) if self.is_scalar_primitive(id)
                )
        })
    }

    /// The tail-position leaf expressions an expression can evaluate to: an
    /// `if`/`match`/block contributes each branch's tail, anything else is itself.
    /// Used to look through a conditional wrapped-view return (`if live { Some(&mut
    /// self.items[i]) } else { None }`).
    fn collect_tail_leaves(&self, expr_id: Id, leaves: &mut Vec<Id>) {
        match self.expr_id_to_expr_map.get(&expr_id) {
            Some(Expr::If(branch)) => self.collect_tail_leaves_if(branch, leaves),
            Some(Expr::Match(_, legs)) => {
                for leg in legs {
                    self.collect_tail_leaves(leg.body, leaves);
                }
            }
            Some(Expr::Block((_, tail))) => self.collect_tail_leaves(*tail, leaves),
            _ => leaves.push(expr_id),
        }
    }

    fn collect_tail_leaves_if(&self, branch: &ExprIfBranch, leaves: &mut Vec<Id>) {
        match branch {
            ExprIfBranch::If(_, (_, tail), else_branch) => {
                self.collect_tail_leaves(*tail, leaves);
                if let Some(else_branch) = else_branch {
                    self.collect_tail_leaves_if(else_branch, leaves);
                }
            }
            ExprIfBranch::Else((_, tail)) => self.collect_tail_leaves(*tail, leaves),
        }
    }

    /// If `leaf_id` is `Some(&[mut] place-projecting-a-parameter)`, the view's
    /// `(mutable, scalar)` and the constructor call's id. `scalar` distinguishes a
    /// `(base, key)` pair (`&mut self.value: i32`) from an aggregate view that is
    /// just the projected reference (`&mut self.node: Node`).
    fn leaf_wrapped_view(&self, leaf_id: Id) -> Option<(bool, bool, Id)> {
        let Some(Expr::Call(call_id)) = self.expr_id_to_expr_map.get(&leaf_id) else {
            return None;
        };
        if !self.call_is_variant_constructor(*call_id) {
            return None;
        }
        // A single view payload that projects a parameter: `Some(&mut self.value)`.
        let [argument_id] = self.function_calls.get(call_id)?.argument_ids.as_slice() else {
            return None;
        };
        match self.expr_id_to_expr_map.get(argument_id) {
            Some(Expr::Reference(operand, mutable))
                if self.derives_from_view_param(*argument_id) =>
            {
                Some((*mutable, self.place_is_scalar(*operand), *call_id))
            }
            _ => None,
        }
    }

    /// If a function returns a view wrapped in a single-payload enum variant
    /// (`fun get(&mut self): Option<&mut i32> { Some(&mut self.x) }`, the aggregate
    /// `Option<&mut Node>`, or the conditional `{ if live { Some(&mut self.items[i])
    /// } else { None } }` behind a real `Arena::get`), the view's `(mutable,
    /// scalar)`. Every wrapped-view tail leaf must project a parameter and agree on
    /// `(mutable, scalar)`; the other leaves (`None`) carry no view. `None` if no
    /// leaf returns such a view, or they disagree. (A leaf returning a
    /// non-projecting view is left for `check_view_escape` to reject — it isn't in
    /// the sanctioned set below.)
    fn function_returns_wrapped_view(&self, function_id: Id) -> Option<(bool, bool)> {
        let function = self.functions.get(&function_id)?;
        if !function.has_body {
            return None;
        }
        let mut leaves = Vec::new();
        self.collect_tail_leaves(function.body.1, &mut leaves);
        let mut shape = None;
        for leaf in leaves {
            if let Some((mutable, scalar, _)) = self.leaf_wrapped_view(leaf) {
                match shape {
                    None => shape = Some((mutable, scalar)),
                    Some(previous) if previous != (mutable, scalar) => return None,
                    _ => {}
                }
            }
        }
        shape
    }

    /// The variant-constructor call ids of a function's sanctioned wrapped-view
    /// tail leaves — the `Some(&mut ..)` sites `check_view_escape` must not reject.
    fn wrapped_view_return_calls(&self, function_id: Id) -> Vec<Id> {
        if self.function_returns_wrapped_view(function_id).is_none() {
            return Vec::new();
        }
        let Some(function) = self.functions.get(&function_id) else {
            return Vec::new();
        };
        let mut leaves = Vec::new();
        self.collect_tail_leaves(function.body.1, &mut leaves);
        leaves
            .into_iter()
            .filter_map(|leaf| self.leaf_wrapped_view(leaf).map(|(_, _, call_id)| call_id))
            .collect()
    }

    /// If a call resolves to a function returning a wrapped view, that view's
    /// `(mutable, scalar)` (see `function_returns_wrapped_view`).
    fn call_returns_wrapped_view(&self, call_id: Id) -> Option<(bool, bool)> {
        let function_call = self.function_calls.get(&call_id)?;
        match self.expr_id_to_expr_map.get(&function_call.subject_id)? {
            Expr::Local(function_id) => self.function_returns_wrapped_view(*function_id),
            _ => None,
        }
    }

    /// Match captures (`Some(let v)`) whose subject is a call returning a wrapped
    /// view: the capture binds the view directly (the `(base, key)` pair, or an
    /// aggregate's reference), so it is a view binding. Maps each such capture to
    /// the view's `(mutable, scalar)`.
    fn compute_wrapped_view_captures(&self) -> HashMap<Id, (bool, bool)> {
        let mut captures = HashMap::new();
        for expr in self.expr_id_to_expr_map.values() {
            let Expr::Match(subject_id, legs) = expr else {
                continue;
            };
            let Some(Expr::Call(call_id)) = self.expr_id_to_expr_map.get(subject_id) else {
                continue;
            };
            let Some(shape) = self.call_returns_wrapped_view(*call_id) else {
                continue;
            };
            for leg in legs {
                // `Some(let v)`: a variant pattern with exactly one binding payload.
                if let ExprPattern::Variant(_, _, sub_patterns) = &leg.pattern
                    && let [ExprPattern::Binding(capture_id)] = sub_patterns.as_slice()
                {
                    captures.insert(*capture_id, shape);
                }
            }
        }
        captures
    }

    /// Whether a call resolves to a `borrows` function returning a scalar view —
    /// so `*call` / a binding of it derefs through `(base, key)`.
    fn call_returns_scalar_view(&self, call_id: Id) -> bool {
        let Some(function_call) = self.function_calls.get(&call_id) else {
            return false;
        };
        matches!(
            self.expr_id_to_expr_map.get(&function_call.subject_id),
            Some(Expr::Local(function_id)) if self.function_returns_scalar_view(*function_id)
        )
    }

    /// Call exprs that resolve to a `borrows` function returning a scalar view, so
    /// `*call` reads/writes through `call[0][call[1]]`.
    fn compute_scalar_view_calls(&self) -> HashSet<Id> {
        self.function_calls
            .keys()
            .copied()
            .filter(|call_id| self.call_returns_scalar_view(*call_id))
            .collect()
    }

    /// `Reference` exprs (`&place` / `&mut place`) whose target is a scalar — the
    /// ones that lower to a `[base, key]` pair (a boxed local's cell at slot 0, or
    /// a struct's field slot) rather than to the aggregate's own JS reference.
    fn compute_scalar_view_refs(&self) -> HashSet<Id> {
        self.expr_id_to_expr_map
            .iter()
            .filter_map(|(expr_id, expr)| match expr {
                Expr::Reference(operand, _) if self.place_is_scalar(*operand) => Some(*expr_id),
                _ => None,
            })
            .collect()
    }

    /// Bindings/parameters whose deref reads/writes a scalar slot through a
    /// `(base, key)` view: a view of a scalar place (a boxed local *or* a scalar
    /// field), copied between locals (`let w = v`), or a scalar `&`/`&mut`
    /// parameter (which receives such a pair from its caller).
    fn compute_primitive_views(&self) -> HashSet<Id> {
        let mut views: HashSet<Id> = HashSet::new();
        // A `Some(let v)` capture over a wrapped-*scalar*-view call binds the
        // `(base, key)` pair directly, so `*v` derefs through it. (An aggregate
        // wrapped-view capture is the reference itself, accessed via `.field` —
        // not a `(base, key)` pair — so it is excluded here.)
        views.extend(
            self.wrapped_view_captures
                .iter()
                .filter(|(_, shape)| shape.1)
                .map(|(capture_id, _)| *capture_id),
        );
        // A `for e in &mut list` over a scalar element binds `e` as a `(base, key)`
        // pair into the list, so `*e` derefs through it.
        for binding_id in self.for_each_views.keys() {
            let is_scalar = self.variables.get(binding_id).is_some_and(|variable| {
                matches!(variable.type_id.get_type(self), Type::Struct(id, _) if self.is_scalar_primitive(id))
            });
            if is_scalar {
                views.insert(*binding_id);
            }
        }
        loop {
            let mut changed = false;
            for variable in self.variables.values() {
                if views.contains(&variable.id) {
                    continue;
                }
                let Some(initial) = variable.initial else {
                    continue;
                };
                let is_scalar = match self.expr_id_to_expr_map.get(&initial) {
                    Some(Expr::Reference(operand, _)) => self.place_is_scalar(*operand),
                    Some(Expr::Local(source)) => views.contains(source),
                    // `let v = obj.slot()` — a `borrows` call returning a scalar view.
                    Some(Expr::Call(call_id)) => self.call_returns_scalar_view(*call_id),
                    _ => false,
                };
                if is_scalar {
                    views.insert(variable.id);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for parameter in self.parameters.values() {
            let is_scalar_view =
                matches!(parameter.convention, Convention::Ref | Convention::RefMut)
                    && matches!(
                        parameter.type_id.get_type(self),
                        Type::Struct(id, _) if self.is_scalar_primitive(id)
                    );
            if is_scalar_view {
                views.insert(parameter.id);
            }
        }
        views
    }

    /// Rule 4 (no invalidating mutation under a live view): a binding may not be
    /// reassigned while a view into it is live. The live range is lexical — a
    /// view is live from its declaration to the end of its block. (Resize / move
    /// / drop invalidation, and index-into-container views, are deferred.)
    fn check_invalidation(&mut self) {
        let view_origins = self.compute_view_origins();
        if view_origins.is_empty() {
            return;
        }
        let bodies: Vec<(Vec<Id>, Id)> = self
            .functions
            .values()
            .filter(|function| function.has_body)
            .map(|function| (function.body.0.clone(), function.body.1))
            .collect();
        let closure_returns: Vec<Id> = self.closures.values().map(|c| c.return_).collect();
        let mut violations: Vec<(Id, Id)> = Vec::new();
        for (statements, tail) in &bodies {
            let mut live = HashSet::new();
            self.scan_invalidation_block(
                statements,
                *tail,
                &view_origins,
                &mut live,
                &mut violations,
            );
        }
        for return_id in closure_returns {
            let mut live = HashSet::new();
            self.scan_invalidation_block(&[], return_id, &view_origins, &mut live, &mut violations);
        }
        for (reassignment_id, root_id) in violations {
            let name = self
                .variables
                .get(&root_id)
                .map(|v| v.name)
                .unwrap_or("value");
            self.diagnostics.push(Error {
                span: **self.span_map.get(&reassignment_id).unwrap_or(&&EMPTY_SPAN),
                msg: format!(
                    "cannot reassign '{name}' while a view into it is live (rule 4: no invalidating mutation under a live view)."
                ),
            });
        }
    }

    /// Whether `ancestor` is a strict (proper) ancestor scope of `descendant` —
    /// i.e. `descendant` is lexically nested inside `ancestor`, so it ends first.
    fn scope_is_strict_ancestor(&self, ancestor: Id, descendant: Id) -> bool {
        let mut current = self
            .scopes
            .get(&descendant)
            .and_then(|scope| scope.parent_id);
        while let Some(scope_id) = current {
            if scope_id == ancestor {
                return true;
            }
            current = self.scopes.get(&scope_id).and_then(|scope| scope.parent_id);
        }
        false
    }

    /// Rule 3 (lifetime): a view binding may not be reseated to view a place that
    /// goes out of scope before the binding does — `mut axis = &mut a; { mut local
    /// = …; axis = &mut local; } axis.value = 99` leaves `axis` dangling once the
    /// inner block ends. Caught lexically: a reseat `view = &place` is rejected
    /// when `place`'s root local is declared in a strictly inner scope, so it dies
    /// first. Reseating to another view binding (`axis = parent`), or to a
    /// same/outer-scope place, is fine. Conservative: it fires even when the
    /// binding isn't read after the inner scope — reseating an outer view to a
    /// block-local place serves no purpose anyway.
    fn check_reseat_escape(&mut self) {
        let view_bindings = self.compute_view_bindings();
        if view_bindings.is_empty() {
            return;
        }
        let mut violations: Vec<(Id, &'src str)> = Vec::new();
        for (assignment_id, expr) in &self.expr_id_to_expr_map {
            let Expr::Assignment(target_id, value_id) = expr else {
                continue;
            };
            // A reseat targets the view binding itself (`view = …`), not `*view`.
            let Some(Expr::Local(binding)) = self.expr_id_to_expr_map.get(target_id) else {
                continue;
            };
            if !view_bindings.contains(binding) {
                continue;
            }
            // …reseated to a freshly-taken view of a place (`&mut local`).
            let Some(Expr::Reference(operand, _)) = self.expr_id_to_expr_map.get(value_id) else {
                continue;
            };
            let Some(root) = self.place_root(*operand) else {
                continue;
            };
            let (Some(&binding_scope), Some(&root_scope)) = (
                self.expr_id_to_scope_id_map.get(binding),
                self.expr_id_to_scope_id_map.get(&root),
            ) else {
                continue;
            };
            if self.scope_is_strict_ancestor(binding_scope, root_scope) {
                let name = self.variables.get(&root).map(|v| v.name).unwrap_or("value");
                violations.push((*assignment_id, name));
            }
        }
        for (reseat_id, name) in violations {
            self.diagnostics.push(Error {
                span: **self.span_map.get(&reseat_id).unwrap_or(&&EMPTY_SPAN),
                msg: format!(
                    "cannot reseat a view to '{name}', which goes out of scope before the view; the view would dangle. Reseat to a place that outlives the view, or use a handle."
                ),
            });
        }
    }

    /// Scans a block's statements in order, tracking live view bindings.
    /// `live` carries views from enclosing blocks in; views declared here die at
    /// the block's end.
    fn scan_invalidation_block(
        &self,
        statements: &[Id],
        tail: Id,
        view_origins: &HashMap<Id, Id>,
        live: &mut HashSet<Id>,
        violations: &mut Vec<(Id, Id)>,
    ) {
        let outer = live.clone();
        for statement in statements {
            self.scan_invalidation(*statement, view_origins, live, violations);
        }
        self.scan_invalidation(tail, view_origins, live, violations);
        live.retain(|view| outer.contains(view));
    }

    fn scan_invalidation(
        &self,
        expr_id: Id,
        view_origins: &HashMap<Id, Id>,
        live: &mut HashSet<Id>,
        violations: &mut Vec<(Id, Id)>,
    ) {
        let Some(expr) = self.expr_id_to_expr_map.get(&expr_id).cloned() else {
            return;
        };
        match expr {
            Expr::Variable(variable_id) => {
                if let Some(initial) = self.variables.get(&variable_id).and_then(|v| v.initial) {
                    self.scan_invalidation(initial, view_origins, live, violations);
                }
                if view_origins.contains_key(&variable_id) {
                    live.insert(variable_id);
                }
            }
            Expr::Assignment(target_id, value_id) => {
                self.scan_invalidation(value_id, view_origins, live, violations);
                // Reassigning a whole binding invalidates views into it.
                if let Some(Expr::Local(root_id)) = self.expr_id_to_expr_map.get(&target_id) {
                    if live
                        .iter()
                        .any(|view| view_origins.get(view) == Some(root_id))
                    {
                        violations.push((target_id, *root_id));
                    }
                }
                self.scan_invalidation(target_id, view_origins, live, violations);
            }
            Expr::Block((statements, tail)) => {
                self.scan_invalidation_block(&statements, tail, view_origins, live, violations);
            }
            Expr::For(condition, (statements, tail)) => {
                if let Some(condition) = condition {
                    self.scan_invalidation(condition, view_origins, live, violations);
                }
                self.scan_invalidation_block(&statements, tail, view_origins, live, violations);
            }
            Expr::ForEach(iterable, _, (statements, tail)) => {
                self.scan_invalidation(iterable, view_origins, live, violations);
                self.scan_invalidation_block(&statements, tail, view_origins, live, violations);
            }
            Expr::If(branch) => self.scan_invalidation_if(&branch, view_origins, live, violations),
            Expr::Match(subject_id, legs) => {
                self.scan_invalidation(subject_id, view_origins, live, violations);
                for leg in legs {
                    if let Some(guard) = leg.guard {
                        self.scan_invalidation(guard, view_origins, live, violations);
                    }
                    self.scan_invalidation_block(&[], leg.body, view_origins, live, violations);
                }
            }
            Expr::Reference(operand, _) | Expr::Dereference(operand) | Expr::Unary(_, operand) => {
                self.scan_invalidation(operand, view_origins, live, violations);
            }
            Expr::Binary(_, lhs, rhs) => {
                self.scan_invalidation(lhs, view_origins, live, violations);
                self.scan_invalidation(rhs, view_origins, live, violations);
            }
            Expr::Field(subject, _, _) => {
                self.scan_invalidation(subject, view_origins, live, violations)
            }
            Expr::Index(subject, index) => {
                self.scan_invalidation(subject, view_origins, live, violations);
                self.scan_invalidation(index, view_origins, live, violations);
            }
            Expr::FunctionReturn(value) | Expr::Await(value) => {
                self.scan_invalidation(value, view_origins, live, violations)
            }
            Expr::Call(call_id) => {
                if let Some(function_call) = self.function_calls.get(&call_id) {
                    for argument in function_call.argument_ids.clone() {
                        self.scan_invalidation(argument, view_origins, live, violations);
                    }
                }
            }
            Expr::List(ids) | Expr::Tuple(ids) => {
                for id in ids {
                    self.scan_invalidation(id, view_origins, live, violations);
                }
            }
            Expr::StructInitializer(_, fields) => {
                for value in fields.values() {
                    self.scan_invalidation(*value, view_origins, live, violations);
                }
            }
            _ => {}
        }
    }

    fn scan_invalidation_if(
        &self,
        branch: &ExprIfBranch,
        view_origins: &HashMap<Id, Id>,
        live: &mut HashSet<Id>,
        violations: &mut Vec<(Id, Id)>,
    ) {
        match branch {
            ExprIfBranch::If(condition, (statements, tail), else_branch) => {
                self.scan_invalidation(*condition, view_origins, live, violations);
                self.scan_invalidation_block(statements, *tail, view_origins, live, violations);
                if let Some(else_branch) = else_branch {
                    self.scan_invalidation_if(else_branch, view_origins, live, violations);
                }
            }
            ExprIfBranch::Else((statements, tail)) => {
                self.scan_invalidation_block(statements, *tail, view_origins, live, violations);
            }
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
            if let Some((name, fix)) = self.readonly_root(target_id) {
                let advice = if fix == "`&mut`" {
                    format!("declare it `&mut {name}`")
                } else {
                    "declare it `mut`".to_string()
                };
                self.diagnostics.push(Error {
                    span: **self.span_map.get(&target_id).unwrap_or(&&EMPTY_SPAN),
                    msg: format!("cannot mutate immutable '{name}'; {advice} to allow mutation."),
                });
            }
        }
    }

    /// Rule 3: an argument passed to a `&mut` parameter must be a mutable place.
    /// In particular the receiver of a `&mut self` method (argument 0) cannot be
    /// rooted in a readonly parameter — `self.cars.push(..)` inside a bare-`self`
    /// method is rejected, directing the author to `&mut self`. Only direct
    /// calls (`subject -> Local(callee)`) are resolved; dispatched/generic
    /// callees are conservatively skipped.
    fn check_mutable_arguments(&mut self) {
        let call_ids: Vec<Id> = self
            .expr_id_to_expr_map
            .values()
            .filter_map(|expr| match expr {
                Expr::Call(call_id) => Some(*call_id),
                _ => None,
            })
            .collect();
        for call_id in call_ids {
            let Some(function_call) = self.function_calls.get(&call_id) else {
                continue;
            };
            let subject_id = function_call.subject_id;
            let argument_ids = function_call.argument_ids.clone();
            let callee_id = match self.expr_id_to_expr_map.get(&subject_id) {
                Some(Expr::Local(callee_id)) => *callee_id,
                _ => continue,
            };
            let parameter_ids = self
                .functions
                .get(&callee_id)
                .map(|function| function.parameters.clone())
                .or_else(|| {
                    self.external_functions
                        .get(&callee_id)
                        .map(|external| external.parameters.clone())
                });
            let Some(parameter_ids) = parameter_ids else {
                continue;
            };
            for (parameter_id, argument_id) in parameter_ids.iter().zip(argument_ids.iter()) {
                let writable = self
                    .parameters
                    .get(parameter_id)
                    .is_some_and(|parameter| parameter.convention == Convention::RefMut);
                if writable {
                    if let Some((name, fix)) = self.readonly_root(*argument_id) {
                        let advice = if fix == "`&mut`" {
                            format!("declare it `&mut {name}`")
                        } else {
                            "declare it `mut`".to_string()
                        };
                        self.diagnostics.push(Error {
                            span: **self.span_map.get(argument_id).unwrap_or(&&EMPTY_SPAN),
                            msg: format!(
                                "cannot mutate immutable '{name}'; {advice} to allow mutation."
                            ),
                        });
                    }
                }
            }
        }
    }

    /// Forming a writable view `&mut place` requires the place to be mutable —
    /// you cannot get a `&mut` to an immutable `let` local or through a readonly
    /// (bare / `&`) parameter. Complements `check_mutable_arguments`, which only
    /// sees bare-place arguments; an *explicit* `&mut a` argument (or any other
    /// `&mut a`, e.g. `let v = &mut a`) is caught here.
    fn check_mutable_references(&mut self) {
        let references: Vec<(Id, Id)> = self
            .expr_id_to_expr_map
            .iter()
            .filter_map(|(reference_id, expr)| match expr {
                Expr::Reference(operand_id, true) => Some((*reference_id, *operand_id)),
                _ => None,
            })
            .collect();
        for (reference_id, operand_id) in references {
            if let Some((name, fix)) = self.readonly_root(operand_id) {
                let advice = if fix == "`&mut`" {
                    format!("declare it `&mut {name}`")
                } else {
                    "declare it `mut`".to_string()
                };
                self.diagnostics.push(Error {
                    span: **self.span_map.get(&reference_id).unwrap_or(&&EMPTY_SPAN),
                    msg: format!(
                        "cannot take a writable view of immutable '{name}'; {advice} to allow mutation."
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
        // An aggregate place passed to an `own` parameter is copied: the callee
        // owns its value, so mutating it must not affect the caller. Resolved by
        // the direct `subject -> Local(callee)` path, with the same elision.
        for expr in self.expr_id_to_expr_map.values() {
            let Expr::Call(call_id) = expr else {
                continue;
            };
            let Some(function_call) = self.function_calls.get(call_id) else {
                continue;
            };
            let callee_id = match self.expr_id_to_expr_map.get(&function_call.subject_id) {
                Some(Expr::Local(callee_id)) => *callee_id,
                _ => continue,
            };
            let parameter_ids = self
                .functions
                .get(&callee_id)
                .map(|function| function.parameters.clone())
                .or_else(|| {
                    self.external_functions
                        .get(&callee_id)
                        .map(|external| external.parameters.clone())
                });
            let Some(parameter_ids) = parameter_ids else {
                continue;
            };
            for (parameter_id, argument_id) in parameter_ids.iter().zip(&function_call.argument_ids)
            {
                let is_own = self
                    .parameters
                    .get(parameter_id)
                    .is_some_and(|parameter| parameter.convention == Convention::Own);
                if is_own
                    && self.is_place_expr(*argument_id)
                    && self
                        .place_value_type(*argument_id)
                        .is_some_and(|value_type| self.is_cloneable_aggregate(&value_type))
                    && !self.is_elidable_copy(*argument_id, &repeatable)
                {
                    sites.insert(*argument_id);
                }
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
            Expr::Index(subject_id, index_id) => {
                self.mark_repeatable(*subject_id, depth, interior, visited);
                self.mark_repeatable(*index_id, depth, interior, visited);
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
    /// skipping value bindings — so a parameter type that shares its name with a
    /// value in scope (e.g. a function) still resolves to the type, not the value.
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
        generic_parameters: &'src Option<GenericParameters<'src>>,
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
                let constraint_type_id = self.register_binder(
                    parameter.name,
                    &parameter.name_span,
                    &parameter.bounds,
                    scope_id,
                );
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
        name_span: &'src Span,
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
        self.register_generic_parameter(name, name_span, constraint_type_id, scope_id);
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
                self.register_binder(name, &node.1, bounds, scope_id);
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
                Type::Trait(trait_id, _) => Some(trait_id),
                _ => None,
            })
            .collect()
    }

    /// Registers a single generic parameter named `name` (bound by the
    /// constraint type) into `scope_id`.
    fn register_generic_parameter(
        &mut self,
        name: &'src str,
        name_span: &'src Span,
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
        // The binder's name span makes a use of the parameter go-to-definable.
        self.span_map.insert(expr_id, name_span);
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
            Node::Index(subject, index) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                let index_id = self.walk_expr_node(index, scope_id);
                self.constraints.push(Constraint::Subscript {
                    id,
                    subject_id,
                    index_id,
                });
                None
            }
            Node::MemberAccessor(subject, member) => {
                let subject_id = self.walk_expr_node(subject, scope_id);
                match &member.0 {
                    Node::Accessor(name) => {
                        self.member_name_spans.insert(id, member.1);
                        self.constraints
                            .push(Constraint::FieldAccessor(FieldAccessorConstraint {
                                id,
                                subject_id,
                                member_name: name,
                            }));
                    }
                    Node::Number(name, _, _) => {
                        self.prepped_field_accessors.push((id, subject_id, *name));
                    }
                    Node::Call(call_subject, call_generic_arguments, call_arguments) => {
                        match &call_subject.0 {
                            Node::Accessor(name) => {
                                self.member_name_spans.insert(id, call_subject.1);
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
                                self.constraints.push(Constraint::MethodCall {
                                    id,
                                    subject_id,
                                    member_name: name,
                                    generic_argument_ids,
                                    argument_ids,
                                    arguments_span: call_arguments.1,
                                });
                            }
                            _ => {
                                self.diagnostics.push(Error {
                                    span: call_subject.1,
                                    msg: "expected a method name after `.`".to_string(),
                                });
                                self.expr_id_to_expr_map.insert(id, Expr::Error);
                            }
                        }
                    }
                    _ => {
                        self.diagnostics.push(Error {
                            span: member.1,
                            msg: "expected a field or method name after `.`".to_string(),
                        });
                        self.expr_id_to_expr_map.insert(id, Expr::Error);
                    }
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
                for (path, name, leaf_span) in entries {
                    self.prepped_imports.push((
                        path,
                        name,
                        scope_id,
                        node.1,
                        leaf_span,
                        self.current_source_id,
                    ));
                }
                None
            }
            Node::Use(root_branch) => {
                let mut entries = Vec::new();
                flatten_namespace_branch(root_branch, Vec::new(), &mut entries);
                for (path, name, leaf_span) in entries {
                    self.prepped_uses.push((
                        path,
                        name,
                        scope_id,
                        node.1,
                        leaf_span,
                        self.current_source_id,
                    ));
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
                    // The binding name follows `for ` in the loop header.
                    let header = node.1.into_range();
                    let name_span: Span =
                        (header.start + 4..header.start + 4 + variable.len()).into();
                    self.variables.insert(
                        variable_id,
                        Variable {
                            id: variable_id,
                            name: variable,
                            name_span,
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
                    // `for e in &mut list` / `&list` — the iterable is a view, so
                    // each binding is a view of the element (write-through), not a
                    // copy. The element type still resolves below (a view is
                    // identity-typed).
                    if let Some(Expr::Reference(_, mutable)) =
                        self.expr_id_to_expr_map.get(&iterable_id)
                    {
                        self.for_each_views.insert(variable_id, *mutable);
                    }
                    self.constraints.push(Constraint::ForEachItem {
                        item_id: variable_id,
                        iterable_id,
                    });
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
                let walk_pattern = self.walk_pattern(&pattern.0, &pattern.1, scope_id, true);
                self.constraints.push(Constraint::Is(PreppedIs {
                    id,
                    subject_id,
                    scope_id,
                    pattern: walk_pattern,
                }));
                None
            }
            // Re-export visibility is not tracked yet; walking the inner
            // statement is enough to bind it into the current scope.
            Node::Export(inner) => {
                self.walk_expr_node(inner, scope_id);
                None
            }
            // `@derive(..)` is transparent: walk the wrapped item; the synthesized
            // trait impls are appended separately (see `derive_impl_source`).
            Node::Derive(_derives, inner) => {
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
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                // A tuple parameter (`fun f((a, b): T)`) desugars to a synthetic
                // positional parameter plus a destructure run before the body.
                let mut parameter_destructures = Vec::new();
                let parameters = function
                    .parameters
                    .0
                    .iter()
                    .map(|parameter| {
                        self.walk_parameter(
                            parameter,
                            id,
                            body_scope_id,
                            body_scope_id,
                            &mut parameter_destructures,
                        )
                    })
                    .collect::<Vec<_>>();
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
                            name_span: function.name.1,
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
                            // Parameter destructures run first, before the body.
                            let mut ids = parameter_destructures;
                            ids.extend(self.walk_expr_nodes(&body.0.0, body_scope_id));
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
                            name_span: function.name.1,
                            generic_parameter_constraint_ids,
                            parameters,
                            return_type_id,
                            body: (ids, expr_id, body_scope_id),
                            has_body: function.body.is_some(),
                            call_count: 0,
                            is_async: function.is_async,
                            borrows: function.borrows.is_some(),
                            returns_mut_view: matches!(
                                function.return_type.as_deref().map(|spanned| &spanned.0),
                                Some(Node::Reference(true, _))
                            ),
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
                self.constraints
                    .push(Constraint::CallSubject(CallSubjectConstraint::from_walk(
                        id,
                        subject_id,
                        generic_argument_ids,
                        argument_ids,
                        arguments.1,
                    )));
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
                let name_span = name.1;
                let name = name.0;
                // `_` eats the value: the binding is never referenceable.
                if name != "_" {
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
                        name_span,
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
                self.constraints
                    .push(Constraint::Variable(VariableConstraint::from_walk(
                        id, type_id, value_ids,
                    )));
                Some(Expr::Variable(id))
            }
            Node::LetDestructure(pattern, type_, value, mutable) => {
                // Walk the value, then the binder pattern (which registers its
                // bindings in scope as `Unknown`-typed variables); a `Destructure`
                // constraint types them from the value's element types once it
                // resolves. The whole `let` lowers to the recorded
                // `Expr::Destructure`, so the walk inserts nothing now.
                let value_id = value
                    .as_ref()
                    .map(|value| self.walk_expr_node(value, scope_id));
                let walk_pattern = self.walk_pattern(&pattern.0, &pattern.1, scope_id, false);
                if *mutable {
                    self.set_pattern_bindings_mutable(&walk_pattern);
                }
                let type_id = type_.as_ref().map(|x| self.walk_type_node(x, scope_id));
                match value_id {
                    Some(value_id) => {
                        self.constraints
                            .push(Constraint::Destructure(DestructureConstraint {
                                id,
                                value_id,
                                type_id,
                                scope_id,
                                pattern: walk_pattern,
                                defer_until_known: false,
                            }));
                        None
                    }
                    None => {
                        self.diagnostics.push(Error {
                            span: node.1,
                            msg: "a destructuring `let` requires a value".to_string(),
                        });
                        Some(Expr::Error)
                    }
                }
            }
            Node::Assign(target, op, value) => {
                // R6 (transparent references): `*` is value extraction — an
                // rvalue — and may not be an assignment target. A view is written
                // *through* directly (`x = v`), so `*x = v` is rejected.
                if matches!(&target.0, Node::Dereference(_)) {
                    self.diagnostics.push(Error {
                        span: target.1,
                        msg: "cannot assign through `*`: a view is written through directly — write `x = …`, not `*x = …`".to_string(),
                    });
                }
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
                        // Remember this synthesized re-read so a view target's
                        // re-read is also deref-wrapped (R5).
                        self.compound_reread_ids.insert(lhs_id);
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
                let name_span = name.1;
                let name = name.0;
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
                    // The field's name sits at the start of its declaration span.
                    let field_range = child.1.into_range();
                    let name_span: Span =
                        (field_range.start..field_range.start + name.len()).into();
                    let type_id = child
                        .0
                        .1
                        .as_ref()
                        .map(|x| self.walk_type_node(x, body_scope_id))
                        .unwrap_or(Type::Unknown.get_type_id(self));
                    fields.push(Field {
                        name,
                        name_span,
                        type_id,
                    });
                }
                self.structs.insert(
                    id,
                    Struct {
                        id,
                        name,
                        name_span,
                        generic_parameter_constraint_ids,
                        fields,
                    },
                );
                Some(Expr::Struct(id))
            }
            Node::Enum(name, generic_parameters, variants) => {
                let name_span = name.1;
                let name = name.0;
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
                        name_span,
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
                        .map(|pattern| {
                            self.walk_pattern(&pattern.0, &pattern.1, leg_scope_id, true)
                        })
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
                self.constraints.push(Constraint::Match(PreppedMatch {
                    id,
                    subject_id,
                    scope_id,
                    legs: walked_legs,
                    span: node.1,
                }));
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
                self.constraints.push(Constraint::StructInitializer(
                    StructInitializerConstraint::from_walk(
                        id,
                        scope_id,
                        name,
                        generic_argument_ids,
                        e_fields,
                        fields.1,
                    ),
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
                    let (trait_name, trait_arguments) = match &trait_.0 {
                        Node::Accessor(name) => (Some(*name), Vec::new()),
                        // `with Readable<T>` — capture the trait's arguments in the
                        // impl's generic terms (resolved in the impl body scope).
                        Node::AccessorWithGenerics(name, generic_arguments) => {
                            let argument_type_ids = generic_arguments
                                .0
                                .iter()
                                .map(|argument| self.walk_type_node(argument, body_scope_id))
                                .collect();
                            (Some(*name), argument_type_ids)
                        }
                        _ => (None, Vec::new()),
                    };
                    if let Some(trait_name) = trait_name {
                        self.prepped_trait_impls.push(TraitImplCheck {
                            subject_type_id: subject,
                            trait_name,
                            trait_arguments,
                            scope_id,
                            declarations: declarations.clone(),
                            span: trait_.1,
                            source_id: self.current_source_id,
                            implementation_index,
                        });
                    }
                }
                self.implementations.push(Implementation {
                    subject,
                    declarations,
                    trait_ids: Vec::new(),
                    trait_args: Vec::new(),
                });

                Some(Expr::Impl(id))
            }
            Node::Trait(name, generic_parameters, supertraits, body) => {
                let name_span = name.1;
                let name = name.0;
                let scope = self.mut_scope_for_scope_id(scope_id);
                scope.name_to_id_map.insert(name, id);
                self.reference_count.entry(id).or_insert(0);
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let generic_parameter_constraint_ids =
                    self.register_generic_parameters(generic_parameters, body_scope_id);
                // Inside a trait, `Self` is the trait itself (abstractly): a
                // `self`-typed receiver in a default method resolves its method
                // calls against this trait's own declarations.
                let self_type_id = Type::Trait(id, Vec::new()).get_type_id(self);
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
                        name_span,
                        generic_parameter_constraint_ids,
                        declarations,
                        supertraits,
                    },
                );
                Some(Expr::Trait(id))
            }
            Node::TupleComprehension {
                binder,
                binder_span,
                source,
                body,
            } => {
                let source_id = self.walk_expr_node(source, scope_id);
                // The element binder scopes to the body. Its type (the source's
                // element type) is set when the comprehension resolves.
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                let binder_id = self.new_entity_id();
                let unknown_type_id = Type::Unknown.get_type_id(self);
                self.variables.insert(
                    binder_id,
                    Variable {
                        id: binder_id,
                        name: binder,
                        name_span: *binder_span,
                        initial: None,
                        type_id: unknown_type_id,
                        mutable: false,
                    },
                );
                self.expr_id_to_expr_map
                    .insert(binder_id, Expr::Variable(binder_id));
                self.expr_id_to_scope_id_map
                    .insert(binder_id, body_scope_id);
                self.span_map.insert(binder_id, binder_span);
                self.reference_count.entry(binder_id).or_insert(0);
                if *binder != "_" {
                    self.mut_scope_for_scope_id(body_scope_id)
                        .name_to_id_map
                        .insert(binder, binder_id);
                }
                // A method on the binder defers until its type is set (below).
                self.untyped_comprehension_binders.insert(binder_id);
                let body_id = self.walk_expr_node(body, body_scope_id);
                // The binder's type is set when the source resolves (before method
                // calls); the constraint records the `Expr::TupleComprehension`.
                self.constraints.push(Constraint::Comprehension {
                    id,
                    binder_id,
                    source_id,
                    body_id,
                });
                None
            }
            Node::Closure(closure) => {
                let body_scope = self.create_scope(Some(scope_id));
                let body_scope_id = self.push_scope(body_scope);
                // A tuple parameter (`|(a, b)| ..`) desugars to a synthetic
                // positional parameter plus a destructure run before the body.
                let mut parameter_destructures = Vec::new();
                let parameters = closure
                    .parameters
                    .0
                    .iter()
                    .map(|parameter| {
                        self.walk_parameter(
                            parameter,
                            id,
                            body_scope_id,
                            scope_id,
                            &mut parameter_destructures,
                        )
                    })
                    .collect::<Vec<_>>();
                let expr_id = self.walk_expr_node(&closure.return_value, body_scope_id);
                self.closures.insert(
                    id,
                    Closure {
                        id,
                        parameters,
                        parameter_destructures,
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
                        parameter_destructures: Vec::new(),
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
            Node::ClosureType(_, _) => {
                self.diagnostics.push(Error {
                    span: node.1,
                    msg: "a closure type is not valid here (expected an expression)".to_string(),
                });
                Some(Expr::Error)
            }
            Node::MappedType { .. } => {
                self.diagnostics.push(Error {
                    span: node.1,
                    msg: "a mapped tuple type is not valid here (expected an expression)"
                        .to_string(),
                });
                Some(Expr::Error)
            }
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
    /// Marks every binding in a walked binder pattern mutable — for `mut (a, b) =
    /// ..`, where the `mut` applies to all bindings (the pattern parser can't see
    /// it, so it walks them immutable).
    fn set_pattern_bindings_mutable(&mut self, pattern: &WalkPattern<'src>) {
        match pattern {
            WalkPattern::Binding(capture_id) => {
                if let Some(variable) = self.variables.get_mut(capture_id) {
                    variable.mutable = true;
                }
            }
            WalkPattern::Tuple(_, patterns) => {
                for sub_pattern in patterns {
                    self.set_pattern_bindings_mutable(sub_pattern);
                }
            }
            _ => {}
        }
    }

    /// Walks one function/closure parameter into a positional parameter entity.
    /// A plain binder (`x`, `self`) becomes a named, referenceable parameter; a
    /// tuple binder (`(a, b)`) becomes a synthetic, non-referenceable positional
    /// parameter plus a `Destructure` prelude (collected into `destructures`)
    /// that binds its elements in the body scope — exactly like a destructuring
    /// `let` whose value is that parameter.
    fn walk_parameter(
        &mut self,
        parameter: &'src crate::node::Parameter<'src>,
        function_id: Id,
        body_scope_id: Id,
        type_scope_id: Id,
        destructures: &mut Vec<Id>,
    ) -> Id {
        let (pattern, parameter_type, convention, span) = parameter;
        let parameter_id = self.new_entity_id();
        let type_id = match (pattern, parameter_type) {
            (_, Some(type_node)) => self.walk_type_node(type_node, type_scope_id),
            // A bare `self` (incl. `&self` / `&mut self`) takes the enclosing
            // `Self` type.
            (Pattern::Binding(name, _), None) if *name == "self" => self
                .try_get_expr_id_by_name("Self", type_scope_id)
                .and_then(|self_id| self.expr_id_to_type_id_map.get(&self_id).copied())
                .unwrap_or_else(|| Type::Unknown.get_type_id(self)),
            (_, None) => Type::Unknown.get_type_id(self),
        };
        // A tuple binder is not referenceable by name; `_` keeps it positional.
        let name = match pattern {
            Pattern::Binding(name, _) => *name,
            _ => "_",
        };
        // `_` eats the argument: it stays positional but is never referenceable.
        if name != "_" {
            let scope = self.mut_scope_for_scope_id(body_scope_id);
            scope.name_to_id_map.insert(name, parameter_id);
        }
        self.parameters.insert(
            parameter_id,
            Parameter {
                id: parameter_id,
                function_id,
                name,
                type_id,
                convention: *convention,
            },
        );
        self.expr_id_to_expr_map
            .insert(parameter_id, Expr::Parameter(parameter_id));
        // The name span makes the parameter go-to-definable and hoverable, and a
        // referenceable cursor target.
        self.span_map.insert(parameter_id, span);
        // A tuple binder: bind its elements from the synthetic parameter, lowering
        // to a `Destructure` prelude run before the body. The destructure reads a
        // *reference* to the parameter (an `Expr::Local`), since the parameter
        // entity itself is a declaration that emits no value.
        if !matches!(pattern, Pattern::Binding(..)) {
            let walked = self.walk_pattern(pattern, span, body_scope_id, false);
            let reference_id = self.new_entity_id();
            self.expr_id_to_expr_map
                .insert(reference_id, Expr::Local(parameter_id));
            self.expr_id_to_scope_id_map
                .insert(reference_id, body_scope_id);
            self.span_map.insert(reference_id, span);
            let destructure_id = self.new_entity_id();
            self.span_map.insert(destructure_id, span);
            self.expr_id_to_scope_id_map
                .insert(destructure_id, body_scope_id);
            self.constraints
                .push(Constraint::Destructure(DestructureConstraint {
                    id: destructure_id,
                    value_id: reference_id,
                    type_id: None,
                    scope_id: body_scope_id,
                    pattern: walked,
                    defer_until_known: true,
                }));
            destructures.push(destructure_id);
        }
        parameter_id
    }

    fn walk_pattern(
        &mut self,
        pattern: &'src Pattern<'src>,
        span: &'src Span,
        scope_id: Id,
        // `match` bindings spell `let`/`mut` before each capture; binder elements
        // (a `let`/parameter tuple) don't. Drives the name-span keyword strip.
        keyword_prefixed: bool,
    ) -> WalkPattern<'src> {
        match pattern {
            Pattern::Wildcard => WalkPattern::Wildcard,
            Pattern::Binding(name, mutable) => {
                let name = *name;
                let capture_id = self.new_entity_id();
                let unknown_type_id = Type::Unknown.get_type_id(self);
                // In a `match` binding the span covers the `let `/`mut ` keyword
                // before the capture, so strip it; a binder element (a `let`/parameter
                // tuple) carries the bare identifier span and needs no adjustment.
                let header = span.into_range();
                let prefix = if keyword_prefixed { "let ".len() } else { 0 };
                let name_span: Span =
                    (header.start + prefix..header.start + prefix + name.len()).into();
                self.variables.insert(
                    capture_id,
                    Variable {
                        id: capture_id,
                        name,
                        name_span,
                        initial: None,
                        type_id: unknown_type_id,
                        mutable: *mutable,
                    },
                );
                self.expr_id_to_expr_map
                    .insert(capture_id, Expr::Variable(capture_id));
                self.expr_id_to_scope_id_map.insert(capture_id, scope_id);
                self.span_map.insert(capture_id, span);
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
                *span,
                self.current_source_id,
                payload.as_ref().map(|patterns| {
                    patterns
                        .iter()
                        .map(|sub_pattern| {
                            self.walk_pattern(
                                &sub_pattern.0,
                                &sub_pattern.1,
                                scope_id,
                                keyword_prefixed,
                            )
                        })
                        .collect()
                }),
            ),
            Pattern::Tuple(patterns) => WalkPattern::Tuple(
                *span,
                patterns
                    .iter()
                    .map(|sub_pattern| {
                        self.walk_pattern(
                            &sub_pattern.0,
                            &sub_pattern.1,
                            scope_id,
                            keyword_prefixed,
                        )
                    })
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
    /// The number of flat slots a value of this type occupies once tuples are
    /// flattened (matching the transformer's `flat_width`): a tuple is the sum of
    /// its elements', anything else is one. Computed from the type known at
    /// pattern resolution — correct for every concrete pattern; a still-generic
    /// element counts as one slot (it can only be destructured as a tuple once a
    /// tuple bound makes its arity known).
    fn tuple_flat_width(&self, type_id: TypeId) -> usize {
        match type_id.get_type(self) {
            Type::Tuple(element_ids) => element_ids
                .iter()
                .map(|id| self.tuple_flat_width(*id))
                .sum(),
            _ => 1,
        }
    }

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
            WalkPattern::Variant(path, span, source_id, payload) => {
                let span = *span;
                let source_id = *source_id;
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
                // Record each `::`-separated segment for the language server: the
                // variant name (the last segment) navigates to the variant, and
                // the enum name before it (in a qualified `Enum::Variant`) to the
                // enum. A payload capture inside the pattern keeps its own span.
                let enum_type_id = Type::Enum(enum_id, Vec::new()).get_type_id(self);
                let last = path.len().saturating_sub(1);
                let mut offset = span.into_range().start;
                for (index, segment) in path.iter().enumerate() {
                    let segment_span: Span = (offset..offset + segment.len()).into();
                    if index == last {
                        self.type_references.push((
                            source_id,
                            segment_span,
                            Some(entity),
                            enum_type_id,
                        ));
                    } else if index + 1 == last {
                        self.type_references.push((
                            source_id,
                            segment_span,
                            Some(enum_id),
                            enum_type_id,
                        ));
                    }
                    offset += segment.len() + "::".len();
                }
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
                            msg: format!(
                                "cannot match an enum variant against type {}",
                                subject_str
                            ),
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
                // Element types come from the matched tuple type when known (a
                // concrete-source mapped type expands to one); otherwise each
                // element resolves against `Unknown`.
                let expected = expected_type_id.get_type(self);
                let element_type_ids = match self.expand_mapped(expected) {
                    Type::Tuple(ids) if ids.len() == patterns.len() => ids,
                    _ => {
                        let unknown = Type::Unknown.get_type_id(self);
                        vec![unknown; patterns.len()]
                    }
                };
                let _ = span;
                let mut resolved = Vec::new();
                for (sub_pattern, element_type_id) in patterns.iter().zip(element_type_ids) {
                    // The element's flat width (a nested tuple spans several slots),
                    // resolved here while the matched type is known.
                    let width = self.tuple_flat_width(element_type_id);
                    resolved.push((
                        self.resolve_pattern(sub_pattern, element_type_id, lookup_scope_id)?,
                        width,
                    ));
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
                        msg: format!(
                            "literal pattern of type {} cannot match type {}",
                            got, expected
                        ),
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
                    self.prepped_type_locals.push((
                        type_id,
                        name,
                        scope_id,
                        node.1,
                        Vec::new(),
                        self.current_source_id,
                    ));
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
                self.prepped_type_locals.push((
                    type_id,
                    name,
                    scope_id,
                    node.1,
                    argument_type_ids,
                    self.current_source_id,
                ));
                None
            }
            // A `type X` binder resolves to the generic it was registered as (by
            // `register_subject_binders` before the subject is walked).
            Node::TypeBinder(name, _bounds) => {
                self.prepped_type_locals.push((
                    type_id,
                    name,
                    scope_id,
                    node.1,
                    Vec::new(),
                    self.current_source_id,
                ));
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
            // A mapped tuple type `(U in T: F<U>)`. Walk the source in this scope;
            // bind `U` in a child scope and walk the template there. Expand now if
            // the source is already a concrete tuple, else stay symbolic until it
            // is (at the call / monomorphization).
            Node::MappedType {
                binder,
                binder_span: _,
                source,
                template,
            } => {
                let source_id = self.walk_type_node(source, scope_id);
                let mapping_scope_id = self.create_owned_scope(Some(scope_id)).id;
                // The binder `U` is a synthetic type-level generic; its name span
                // isn't needed (it isn't a go-to-definition target).
                let binder_id = self.register_binder(binder, &EMPTY_SPAN, &[], mapping_scope_id);
                let template_id = self.walk_type_node(template, mapping_scope_id);
                // Stay symbolic: the source/template ids may be deferred (resolved
                // only in `build()`), so expansion happens lazily on consumption.
                Some(Type::Mapped(binder_id, source_id, template_id))
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
    /// The iterator-protocol method a `for` loop drives an iterable with:
    /// `next_mut` for a `for e in &mut container` (each binding a writable view),
    /// otherwise `next` (a copying iterator, or a readonly view). A built-in
    /// `List`/`Set` view loop ignores this — it lowers to an indexed loop.
    fn for_each_next_method(&self, item_id: Option<Id>) -> &'static str {
        match item_id.and_then(|id| self.for_each_views.get(&id)) {
            Some(true) => "next_mut",
            _ => "next",
        }
    }

    fn iterable_element_type(&self, iterable_type: &Type, next_method: &str) -> Option<Type> {
        match iterable_type {
            // `List<T>` and `Set<T>` both iterate their single element type `T`
            // (a JS array / `Set` are natively iterable, yielding elements).
            Type::Struct(id, arguments)
                if Some(*id) == self.primitive_struct_ids.get("List").copied()
                    || Some(*id) == self.primitive_struct_ids.get("Set").copied() =>
            {
                arguments
                    .first()
                    .map(|element_type_id| element_type_id.get_type(self))
            }
            // A custom iterator (e.g. `Range`): its element is the payload of
            // `next(self): Option<T>` (or `next_mut(&mut self): Option<&mut T>` for
            // a `&mut` view loop), so the binding gets type `T`.
            Type::Struct(_, _) | Type::Enum(_, _) => {
                let next_id = self.method_member_in_impls(iterable_type, next_method)?;
                let Some(Expr::Function(function_id)) = self.expr_id_to_expr_map.get(&next_id)
                else {
                    return None;
                };
                let return_type = self
                    .functions
                    .get(function_id)?
                    .return_type_id?
                    .get_type(self);
                match return_type {
                    Type::Enum(enum_id, arguments)
                        if self.enums.get(&enum_id).map(|enumeration| enumeration.name)
                            == Some("Option") =>
                    {
                        arguments.first().map(|element| element.get_type(self))
                    }
                    _ => None,
                }
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
    /// A resolved method/function entity's parameter ids and its *own* generic
    /// constraint ids (the `<U>` it declares, not any inherited from an enclosing
    /// impl). `None` if `member_id` isn't a function.
    fn method_signature(&self, member_id: Id) -> Option<(Vec<Id>, Vec<TypeId>)> {
        match self.expr_id_to_expr_map.get(&member_id) {
            Some(Expr::Function(function_id)) => self.functions.get(function_id).map(|function| {
                (
                    function.parameters.clone(),
                    function.generic_parameter_constraint_ids.clone(),
                )
            }),
            Some(Expr::ExternalFunction(function_id)) => {
                self.external_functions.get(function_id).map(|function| {
                    (
                        function.parameters.clone(),
                        function.generic_parameter_constraint_ids.clone(),
                    )
                })
            }
            _ => None,
        }
    }

    /// Binds a method's own generics from its arguments, parameter-first (so the
    /// bindings key on the callee). With `skip_closures`, only non-closure
    /// arguments are used — run first so a closure parameter `|T| ..` is typed with
    /// `T` already known (e.g. `bind_each`'s `|todo| ..`, where `T` is the element
    /// of `Source<List<T>>`); a second pass over all arguments then binds generics
    /// fixed by a closure's return (`derive<U>`'s `U`).
    fn bind_method_own_generics(
        &mut self,
        member_id: Id,
        argument_ids: &[Id],
        skip_closures: bool,
        substitution: &mut SubstitutionContext,
    ) {
        let Some((parameter_ids, own_generics)) = self.method_signature(member_id) else {
            return;
        };
        if own_generics.is_empty() {
            return;
        }
        for (index, argument_id) in argument_ids.iter().enumerate() {
            if skip_closures
                && matches!(
                    self.expr_id_to_expr_map.get(argument_id),
                    Some(Expr::Closure(_))
                )
            {
                continue;
            }
            // `+ 1` skips the method's `self` parameter.
            let Some(parameter_id) = parameter_ids.get(index + 1) else {
                continue;
            };
            let Some(parameter_type) = self
                .parameters
                .get(parameter_id)
                .map(|parameter| parameter.type_id.get_type(self))
            else {
                continue;
            };
            let argument_type = self.infer_type(*argument_id, &parameter_type, substitution);
            if matches!(argument_type, Type::Unresolved) {
                continue;
            }
            if let Some((_, bindings)) =
                self.reconcile_type(&parameter_type, &argument_type, substitution)
            {
                for (constraint_id, type_id) in bindings {
                    if own_generics.contains(&constraint_id) {
                        substitution.insert(constraint_id, type_id);
                    }
                }
            }
        }
    }

    fn infer_closure_args_against_params(
        &mut self,
        member_id: Id,
        argument_ids: &[Id],
        substitution: &SubstitutionContext,
    ) {
        let Some((parameter_ids, _)) = self.method_signature(member_id) else {
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
                // Substitute the receiver's bindings into the parameter type so a
                // generic `|T| U` is matched as `|i32| U`, typing the closure's
                // parameter concretely rather than as the abstract `T`.
                let parameter_type = self.substitute_type(&parameter_type, substitution);
                self.infer_type(*argument_id, &parameter_type, substitution);
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

    /// Whether an expression refers to a comprehension binder whose element type
    /// isn't set yet — following `Local`/`Variable` indirections to the binder.
    fn is_untyped_comprehension_binder(&self, expr_id: Id) -> bool {
        let mut id = expr_id;
        loop {
            if self.untyped_comprehension_binders.contains(&id) {
                return true;
            }
            match self.expr_id_to_expr_map.get(&id) {
                Some(Expr::Local(target_id)) | Some(Expr::Variable(target_id))
                    if *target_id != id =>
                {
                    id = *target_id
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
            Expr::Trait(trait_id) => Type::Trait(*trait_id, Vec::new()),
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
                            // Substitute the receiver's impl bindings into a generic
                            // external return type — `shared.read(): T` on a
                            // `Shared<Counter>` yields `Counter`, not abstract `T`.
                            let return_type = if let Some(method_substitution) =
                                self.method_call_substitution.get(&id).cloned()
                            {
                                let mut context = substitution_context.clone();
                                for (constraint_id, type_id) in method_substitution {
                                    context.insert(constraint_id, type_id);
                                }
                                self.substitute_type(&return_type, &context)
                            } else {
                                return_type
                            };
                            return self.freshen_list_element_slots(return_type, id);
                        };
                        let mut substitution_context = substitution_context.clone();
                        // A method on a concrete generic instance (`box.unwrap()`
                        // where `box: Box2<Node>`) binds the impl's type parameters
                        // from the receiver; apply them so a return type mentioning
                        // them (`T`, `Option<T>`) resolves to the concrete argument.
                        if let Some(method_substitution) =
                            self.method_call_substitution.get(&id).cloned()
                        {
                            for (constraint_id, type_id) in method_substitution {
                                substitution_context.insert(constraint_id, type_id);
                            }
                        }
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
                        // A generic parameter fixed only by the return type — no
                        // argument binds it — is inferred by unifying the return
                        // type against the call's expected type, and recorded so
                        // the transformer monomorphizes the call. Without this a
                        // `let n: i32 = make()` for `fun make<T>(): T` (or a static
                        // method on a generic container, `List::from_json(t):
                        // List<T>`) leaves `T` unbound and `T::member()` dangling.
                        // (Argument-bound generics are recorded during call-subject
                        // resolution; this fills the return-type-only gap.)
                        if !matches!(constraint, Type::Unknown | Type::Unresolved) {
                            let mut return_generics = Vec::new();
                            self.collect_generics(&return_type, 0, &mut return_generics);
                            if !return_generics.is_empty()
                                && let Some((_, bindings)) = self.reconcile_type(
                                    &return_type,
                                    &constraint,
                                    &substitution_context,
                                )
                            {
                                for (constraint_id, type_id) in bindings {
                                    if return_generics.contains(&constraint_id) {
                                        self.method_call_substitution
                                            .entry(id)
                                            .or_default()
                                            .insert(constraint_id, type_id);
                                    }
                                }
                            }
                        }
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
                    // Not callable. The user-facing "cannot call a non-function
                    // value" diagnostic is emitted once during call-subject
                    // resolution; this runs on every inference pass, so just yield
                    // an unknown type rather than re-erroring (or panicking).
                    _ => Type::Unknown,
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
                | BinaryOp::And
                | BinaryOp::Or,
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
            Expr::TupleComprehension(binder_id, source_id, body_id) => {
                let (binder_id, source_id, body_id) = (*binder_id, *source_id, *body_id);
                // The source must be a mapped tuple `(U in T: F<U>)` (its element
                // type is the template `F<U>`); type the binder as that element,
                // then the result is `(U in T: <body type>)`.
                let source_type = self.infer_type_inner(
                    source_id,
                    &Type::Unknown,
                    substitution_context,
                    exprs_seen,
                );
                match source_type {
                    Type::Unresolved => Type::Unresolved,
                    Type::Mapped(element_binder, element_source, element_template) => {
                        if let Some(variable) = self.variables.get_mut(&binder_id) {
                            variable.type_id = element_template;
                        }
                        let body_type = self.infer_type_inner(
                            body_id,
                            &Type::Unknown,
                            substitution_context,
                            exprs_seen,
                        );
                        match body_type {
                            Type::Unresolved => Type::Unresolved,
                            body_type => {
                                let body_type_id = body_type.get_type_id(self);
                                Type::Mapped(element_binder, element_source, body_type_id)
                            }
                        }
                    }
                    _ => Type::Unknown,
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
                                // Resolve the expected parameter type through the
                                // active substitution, so a generic method param
                                // (`|T| U`) types the closure parameter with the
                                // concrete receiver binding (`T = Point`) rather
                                // than the abstract `T`.
                                let resolved = match expected_type_id.get_type(self) {
                                    Type::Generic(constraint_id) => substitution_context
                                        .get(&constraint_id)
                                        .copied()
                                        .unwrap_or(expected_type_id),
                                    _ => expected_type_id,
                                };
                                if let Some(parameter) = self.parameters.get_mut(parameter_id) {
                                    parameter.type_id = resolved;
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

    /// Whether `type_id` resolves to `Generic(constraint_id)` itself — a
    /// self-mapping a substitution can hold (`T -> T`). Recursing on it loops.
    fn is_self_generic(&self, type_id: TypeId, constraint_id: TypeId) -> bool {
        matches!(self.type_id_to_type_map.get(&type_id), Some(Type::Generic(c)) if *c == constraint_id)
    }

    /// Inverts a mapped tuple `(U in T: F<U>)` against a concrete argument tuple:
    /// for each argument slot, reconcile it against the template `F<U>` to recover
    /// what `U` (the single hole) bound to, then bind the source generic `T` to the
    /// tuple of those. The unified type is the concrete argument tuple.
    fn invert_mapped(
        &mut self,
        binder_id: TypeId,
        source_id: TypeId,
        template_id: TypeId,
        argument_element_ids: &[TypeId],
        substitution_context: &SubstitutionContext,
    ) -> Option<(Type, Vec<(TypeId, TypeId)>)> {
        // The source must be an unbound generic `T`; a concrete source would have
        // already expanded to a `Tuple`, leaving nothing to infer.
        let source_constraint = match source_id.get_type(self) {
            Type::Generic(constraint_id) => constraint_id,
            _ => return None,
        };
        let template = template_id.get_type(self);
        let mut inner_ids = Vec::with_capacity(argument_element_ids.len());
        for element_id in argument_element_ids {
            let element = element_id.get_type(self);
            let (_, bindings) = self.reconcile_type(&element, &template, substitution_context)?;
            // The template's single hole is `binder_id`; recover its binding.
            let inner = bindings
                .iter()
                .rev()
                .find(|(constraint_id, _)| *constraint_id == binder_id)
                .map(|(_, type_id)| *type_id)?;
            inner_ids.push(inner);
        }
        let tuple_type_id = Type::Tuple(inner_ids).get_type_id(self);
        Some((
            Type::Tuple(argument_element_ids.to_vec()),
            vec![(source_constraint, tuple_type_id)],
        ))
    }

    /// The generic arguments a concrete type provides for a trait it implements
    /// (`Source<i32>` for `Readable` -> `[i32]`): match the type against the
    /// providing impl's subject to bind the impl's generics, then substitute the
    /// impl's recorded trait arguments through that binding.
    fn trait_args_for(&mut self, concrete: &Type, trait_id: Id) -> Option<Vec<TypeId>> {
        let candidates: Vec<(TypeId, Vec<TypeId>)> = self
            .implementations
            .iter()
            .filter_map(|implementation| {
                implementation
                    .trait_args
                    .iter()
                    .find(|(provided, _)| *provided == trait_id)
                    .map(|(_, arguments)| (implementation.subject, arguments.clone()))
            })
            .collect();
        for (subject_id, arguments) in candidates {
            let subject = subject_id.get_type(self);
            if let Some((_, bindings)) =
                self.reconcile_type(concrete, &subject, &SubstitutionContext::new())
            {
                let context: SubstitutionContext = bindings.into_iter().collect();
                let resolved = arguments
                    .iter()
                    .map(|argument| {
                        let argument_type = argument.get_type(self);
                        self.substitute_type(&argument_type, &context)
                            .get_type_id(self)
                    })
                    .collect();
                return Some(resolved);
            }
        }
        None
    }

    /// Expands a mapped tuple type whose source is concrete (`(U in (A,B): F<U>)`
    /// -> `(F<A>, F<B>)`); a mapped type over a still-abstract source, or any
    /// other type, is returned unchanged.
    fn expand_mapped(&mut self, type_: Type) -> Type {
        if matches!(type_, Type::Mapped(..)) {
            self.substitute_type(&type_, &SubstitutionContext::new())
        } else {
            type_
        }
    }

    fn reconcile_type(
        &mut self,
        a: &Type,
        b: &Type,
        substitution_context: &SubstitutionContext,
    ) -> Option<(Type, Vec<(TypeId, TypeId)>)> {
        let _guard = crate::util::RecursionGuard::enter()?;
        // A concrete-source mapped type expands to its tuple and reconciles
        // structurally; an abstract-source one stays mapped for the inversion arms.
        let a_owned;
        let a = if matches!(a, Type::Mapped(..)) {
            a_owned = self.expand_mapped(a.clone());
            &a_owned
        } else {
            a
        };
        let b_owned;
        let b = if matches!(b, Type::Mapped(..)) {
            b_owned = self.expand_mapped(b.clone());
            &b_owned
        } else {
            b
        };
        Some(match (a, b) {
            (Type::Any, _) | (_, Type::Unknown) => (a.clone(), Vec::new()),
            (_, Type::Any) | (Type::Unknown, _) => (b.clone(), Vec::new()),
            (Type::Unresolved, _) | (_, Type::Unresolved) => {
                return None;
            }
            // A bound generic reconciles its resolved type against the other side.
            // A self-mapping (`T -> T`, which reconciling an impl's own parameter
            // against itself records into the context) must NOT recurse on the
            // same generic, or reconciliation loops forever — the same guard
            // `substitute_type` already applies.
            (Type::Generic(constraint_id), _) => match substitution_context.get(constraint_id) {
                Some(resolved_id) if !self.is_self_generic(*resolved_id, *constraint_id) => {
                    let resolved = resolved_id.get_type(self);
                    let (unified, mut bindings) =
                        self.reconcile_type(&resolved, b, substitution_context)?;
                    bindings.push((*constraint_id, b.clone().get_type_id(self)));
                    (unified, bindings)
                }
                _ => {
                    let bindings = vec![(*constraint_id, b.clone().get_type_id(self))];
                    (a.clone(), bindings)
                }
            },
            (_, Type::Generic(constraint_id)) => match substitution_context.get(constraint_id) {
                Some(resolved_id) if !self.is_self_generic(*resolved_id, *constraint_id) => {
                    let resolved = resolved_id.get_type(self);
                    let (unified, mut bindings) =
                        self.reconcile_type(a, &resolved, substitution_context)?;
                    bindings.push((*constraint_id, a.clone().get_type_id(self)));
                    (unified, bindings)
                }
                _ => {
                    let bindings = vec![(*constraint_id, a.clone().get_type_id(self))];
                    (b.clone(), bindings)
                }
            },
            // A mapped tuple parameter `(U in T: F<U>)` reconciled against a
            // concrete argument tuple: invert the template per element to infer the
            // source tuple `T` (`combine((Source<A>, Source<B>))` binds `T = (A, B)`).
            (Type::Mapped(binder_id, source_id, template_id), Type::Tuple(elements)) => self
                .invert_mapped(
                    *binder_id,
                    *source_id,
                    *template_id,
                    elements,
                    substitution_context,
                )?,
            (Type::Tuple(elements), Type::Mapped(binder_id, source_id, template_id)) => self
                .invert_mapped(
                    *binder_id,
                    *source_id,
                    *template_id,
                    elements,
                    substitution_context,
                )?,
            // A concrete value satisfies a trait-typed parameter when it
            // implements that trait — e.g. a `Counter` (which `impl`s `Combine`)
            // passed where a `Combine` is expected, including a `Self`-defaulted
            // generic that resolved to the trait.
            (Type::Struct(..) | Type::Enum(..), Type::Trait(trait_id, template_arguments)) => {
                if !self.type_implements_trait(a, *trait_id) {
                    return None;
                }
                // A parameterized trait (`Readable<U>`) binds its arguments from the
                // concrete impl: recover the type's trait arguments and reconcile
                // them against the template, so `Signal<A>` against `Readable<U>`
                // binds `U = A`.
                let mut bindings = Vec::new();
                if !template_arguments.is_empty() {
                    if let Some(concrete_arguments) = self.trait_args_for(a, *trait_id) {
                        for (template_argument, concrete_argument) in
                            template_arguments.clone().iter().zip(concrete_arguments)
                        {
                            let template = template_argument.get_type(self);
                            let concrete = concrete_argument.get_type(self);
                            if let Some((_, mut argument_bindings)) =
                                self.reconcile_type(&concrete, &template, substitution_context)
                            {
                                bindings.append(&mut argument_bindings);
                            }
                        }
                    }
                }
                (a.clone(), bindings)
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
            // A concrete type satisfies a trait-typed slot (a generic bound like
            // `T: PartialEq`, or a trait-typed parameter) when it implements the
            // trait — so a conditional impl `impl Option<T: PartialEq>` matches a
            // concrete `Option<i32>`. (Mirrors the same arm in `reconcile_type`.)
            (Type::Struct(..) | Type::Enum(..), Type::Trait(trait_id, _)) => {
                self.type_implements_trait(a, *trait_id)
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
            // Same trait: compatible when their arguments are (an erased side —
            // `Iterator` written without `<T>` — matches any instantiation).
            (Type::Trait(l_id, l_arguments), Type::Trait(r_id, r_arguments)) if l_id == r_id => {
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
        let Some(_guard) = crate::util::RecursionGuard::enter() else {
            return type_.clone();
        };
        match type_ {
            Type::Generic(constraint_id) => substitution_context
                .get(constraint_id)
                .map(|type_id| {
                    let resolved = type_id.get_type(self);
                    // Guard against a self-mapping (`T -> T`), which the receiver's
                    // own impl parameter can produce, so substitution terminates.
                    if matches!(&resolved, Type::Generic(c) if c == constraint_id) {
                        resolved
                    } else {
                        self.substitute_type(&resolved, substitution_context)
                    }
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
            // A parameterized trait substitutes its arguments (`Readable<U>` ->
            // `Readable<A>` under `U = A`), so a mapped trait template instantiates.
            Type::Trait(id, arguments) => {
                let arguments = arguments.clone();
                Type::Trait(
                    *id,
                    self.substitute_argument_types(&arguments, substitution_context),
                )
            }
            // A closure type substitutes its parameter and return types, so a
            // generic method parameter `|T| U` becomes `|i32| U` under `T = i32` —
            // without this an unannotated closure argument's parameter stays the
            // abstract `T`.
            Type::Closure(parameters, return_type_id) => {
                let parameters = parameters.clone();
                let return_type = return_type_id.get_type(self);
                let parameters = self.substitute_argument_types(&parameters, substitution_context);
                let return_type = self
                    .substitute_type(&return_type, substitution_context)
                    .get_type_id(self);
                Type::Closure(parameters, return_type)
            }
            Type::Tuple(element_ids) => {
                let element_ids = element_ids.clone();
                Type::Tuple(self.substitute_argument_types(&element_ids, substitution_context))
            }
            // A mapped tuple substitutes its source; once that is a concrete tuple
            // it expands element-wise (`F[U := X]` per element `X`), otherwise it
            // stays a mapped type over the substituted source.
            Type::Mapped(binder_id, source_id, template_id) => {
                let (binder_id, source_id, template_id) = (*binder_id, *source_id, *template_id);
                let source = source_id.get_type(self);
                match self.substitute_type(&source, substitution_context) {
                    Type::Tuple(element_ids) => {
                        let template = template_id.get_type(self);
                        let slots = element_ids
                            .iter()
                            .map(|element_id| {
                                let mut context = substitution_context.clone();
                                context.insert(binder_id, *element_id);
                                self.substitute_type(&template, &context).get_type_id(self)
                            })
                            .collect();
                        Type::Tuple(slots)
                    }
                    other => Type::Mapped(binder_id, other.get_type_id(self), template_id),
                }
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
    /// Record a name→definition reference for the language server (drives
    /// go-to-definition and hover): `span` is where the name appears, `target_id`
    /// the entity it refers to. A label is rendered from the entity's kind.
    fn record_reference(&mut self, source_id: SourceId, span: Span, target_id: Id) {
        let label_type = match self.expr_id_to_expr_map.get(&target_id) {
            Some(Expr::Enum(id)) | Some(Expr::EnumVariant(id, _)) => Type::Enum(*id, Vec::new()),
            Some(Expr::Struct(id)) => Type::Struct(*id, Vec::new()),
            Some(Expr::Trait(id)) => Type::Trait(*id, Vec::new()),
            Some(Expr::Module(id)) => Type::Module(*id),
            Some(Expr::Function(id)) | Some(Expr::ExternalFunction(id)) => Type::Function(*id),
            _ => Type::Unknown,
        };
        let label_type_id = label_type.get_type_id(self);
        self.type_references
            .push((source_id, span, Some(target_id), label_type_id));
    }

    fn resolve_import(
        &mut self,
        path: &[(&'src str, Span)],
        name: &'src str,
        scope_id: Id,
        span: Span,
        report: bool,
        leaf_span: Span,
        source_id: SourceId,
    ) -> bool {
        // The segments to walk, each with its source span. A `self` leaf re-binds
        // the namespace it sits in (e.g. `Option::{ self }` binds `Option`);
        // otherwise the leaf is the final segment and binds under its own name.
        let (segments, bind_name): (Vec<(&str, Span)>, &str) = if name == "self" {
            match path.last() {
                Some((last, _)) => (path.to_vec(), *last),
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
            segments.push((name, leaf_span));
            (segments, name)
        };
        let mut segments = segments.into_iter();
        let (root, root_span) = segments.next().unwrap();
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
        self.record_reference(source_id, root_span, module_id);
        let mut target_id = module_id;
        let mut namespace_scope_id = self.modules.get(&module_id).unwrap().body.1;
        for (part, part_span) in segments {
            match self.try_get_expr_id_by_name(part, namespace_scope_id) {
                Some(id) => {
                    target_id = id;
                    self.record_reference(source_id, part_span, id);
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
        // A `self` leaf's own span points at the namespace it re-binds.
        if name == "self" {
            self.record_reference(source_id, leaf_span, target_id);
        }
        let scope = self.mut_scope_for_scope_id(scope_id);
        scope.name_to_id_map.insert(bind_name, target_id);
        true
    }

    /// Drains the constraint queue once, attempting each task in priority order
    /// (the queue is kept sorted, so this preserves the original section order)
    /// and re-queueing those that defer. Returns whether any task resolved or
    /// failed — i.e. whether this pass made progress.
    fn resolve_constraints(&mut self) -> bool {
        let mut progress = false;
        // Re-sort each pass so any task a prior pass spawned (e.g. a slot
        // unification produced while resolving `push`) falls into priority order.
        // The sort is stable — tasks of one kind keep their original source order
        // — and the queue is already near-sorted, so it stays cheap.
        let mut queue = std::mem::take(&mut self.constraints);
        queue.sort_by_key(|constraint| constraint.priority());
        let mut deferred = Vec::new();
        for constraint in queue {
            match self.try_resolve(&constraint) {
                Resolution::Resolved | Resolution::Failed => progress = true,
                Resolution::Deferred => deferred.push(constraint),
            }
        }
        // Tasks spawned mid-pass landed back in `self.constraints`; keep them
        // (after the deferrals) to be sorted into place on the next pass.
        deferred.append(&mut self.constraints);
        self.constraints = deferred;
        progress
    }

    /// Dispatches a constraint to its per-kind resolver.
    fn try_resolve(&mut self, constraint: &Constraint<'src>) -> Resolution {
        match constraint {
            Constraint::Subscript {
                id,
                subject_id,
                index_id,
            } => self.resolve_subscript(*id, *subject_id, *index_id),
            Constraint::Is(prepped) => self.resolve_is(prepped),
            Constraint::FieldAccessor(constraint) => self.resolve_field_accessor(constraint),
            Constraint::StructInitializer(constraint) => {
                self.resolve_struct_initializer(constraint)
            }
            Constraint::Match(prepped) => self.resolve_match(prepped),
            Constraint::Variable(constraint) => self.resolve_variable(constraint),
            Constraint::Destructure(constraint) => self.resolve_destructure(constraint),
            Constraint::Comprehension {
                id,
                binder_id,
                source_id,
                body_id,
            } => self.resolve_comprehension(*id, *binder_id, *source_id, *body_id),
            Constraint::MethodCall {
                id,
                subject_id,
                member_name,
                generic_argument_ids,
                argument_ids,
                arguments_span,
            } => self.resolve_method_call(
                *id,
                *subject_id,
                member_name,
                generic_argument_ids,
                argument_ids,
                *arguments_span,
            ),
            Constraint::SlotUnification { slot, argument_id } => {
                self.resolve_slot_unification(*slot, *argument_id)
            }
            Constraint::MethodArgCheck {
                member_id,
                argument_ids,
                arguments_span,
            } => self.resolve_method_arg_check(*member_id, argument_ids, *arguments_span),
            Constraint::ForEachItem {
                item_id,
                iterable_id,
            } => self.resolve_for_each_item(*item_id, *iterable_id),
            Constraint::CallSubject(constraint) => self.resolve_call_subject(constraint),
        }
    }

    /// Records a resolved call: a `FunctionCall` plus the `Expr::Call` entity.
    fn wire_call(
        &mut self,
        call_id: Id,
        subject_id: Id,
        generic_argument_ids: &[TypeId],
        argument_ids: &[Id],
        arguments_span: Span,
    ) {
        self.function_calls.insert(
            call_id,
            FunctionCall {
                id: call_id,
                subject_id,
                generic_argument_ids: generic_argument_ids.to_vec(),
                argument_ids: argument_ids.to_vec(),
                arguments_span,
            },
        );
        self.expr_id_to_expr_map
            .insert(call_id, Expr::Call(call_id));
    }

    /// `subject(args)`: once the subject resolves, dispatch on what it is — a
    /// closure value, an enum variant constructor, or a function reference —
    /// type-check the arguments, and wire the call. Defers while the subject or an
    /// argument is unresolved (or an argument is an unknown closure parameter).
    fn resolve_call_subject(&mut self, constraint: &CallSubjectConstraint) -> Resolution {
        let call_id = constraint.call_id;
        let subject_id = constraint.subject_id;
        let generic_argument_ids = &constraint.generic_argument_ids;
        let argument_ids = &constraint.argument_ids;
        let arguments_span = constraint.arguments_span;

        // Defer until the subject's entity is resolved and its type can be
        // inferred (a local pointing at a function, a static accessor, etc.).
        if !self.expr_id_to_expr_map.contains_key(&subject_id) {
            return Resolution::Deferred;
        }
        let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
        if matches!(subject_type, Type::Unresolved) {
            return Resolution::Deferred;
        }

        // Calling a closure-typed value, e.g. `(self.fn)()`: type-check the
        // arguments against the closure's parameter types.
        if let Type::Closure(parameter_type_ids, _) = &subject_type {
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
                return Resolution::Failed;
            }
            let substitution_context = HashMap::new();
            for (index, parameter_type_id) in parameter_type_ids.clone().iter().enumerate() {
                let parameter_type = parameter_type_id.get_type(self);
                let argument_id = *argument_ids.get(index).unwrap();
                let argument_type =
                    self.infer_type(argument_id, &parameter_type, &substitution_context);
                if matches!(argument_type, Type::Unresolved) {
                    return Resolution::Deferred;
                }
                if self
                    .reconcile_type(&argument_type, &parameter_type, &substitution_context)
                    .is_none()
                {
                    let expected = self.pretty_print_type(&parameter_type, &substitution_context);
                    let got = self.pretty_print_type(&argument_type, &substitution_context);
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&argument_id).unwrap(),
                        msg: format!("Expected {}, but got {} instead.", expected, got),
                    });
                }
            }
            self.wire_call(
                call_id,
                subject_id,
                generic_argument_ids,
                argument_ids,
                arguments_span,
            );
            return Resolution::Resolved;
        }

        let subject_expr = self.get_entity_by_id(subject_id).clone();
        match subject_expr {
            Expr::Local(target_id) => {
                let target = self.get_entity_by_id(target_id).clone();
                // A variant constructor call, e.g. `Some(1)`: the arguments are
                // checked against the variant's declared data types.
                if let Expr::EnumVariant(enum_id, variant_index) = target {
                    let data_type_ids = self.enums.get(&enum_id).unwrap().variants[variant_index]
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
                        return Resolution::Failed;
                    }
                    let substitution_context = HashMap::new();
                    for (index, data_type_id) in data_type_ids.iter().enumerate() {
                        let data_type = data_type_id.get_type(self);
                        let argument_id = *argument_ids.get(index).unwrap();
                        let argument_type =
                            self.infer_type(argument_id, &data_type, &substitution_context);
                        if matches!(argument_type, Type::Unresolved) {
                            return Resolution::Deferred;
                        }
                        if self
                            .reconcile_type(&argument_type, &data_type, &substitution_context)
                            .is_none()
                        {
                            let expected =
                                self.pretty_print_type(&data_type, &substitution_context);
                            let got = self.pretty_print_type(&argument_type, &substitution_context);
                            self.diagnostics.push(Error {
                                span: **self.span_map.get(&argument_id).unwrap(),
                                msg: format!("Expected {}, but got {} instead.", expected, got),
                            });
                        }
                    }
                    self.wire_call(
                        call_id,
                        subject_id,
                        generic_argument_ids,
                        argument_ids,
                        arguments_span,
                    );
                    return Resolution::Resolved;
                }
                let function_data = match &target {
                    Expr::Function(function_id) => {
                        self.functions.get(function_id).map(|function| {
                            (
                                function.parameters.clone(),
                                function.generic_parameter_constraint_ids.clone(),
                            )
                        })
                    }
                    Expr::ExternalFunction(external_function_id) => self
                        .external_functions
                        .get(external_function_id)
                        .map(|function| {
                            (
                                function.parameters.clone(),
                                function.generic_parameter_constraint_ids.clone(),
                            )
                        }),
                    _ => None,
                };

                if let Some((parameters, generic_parameter_constraint_ids)) = function_data {
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
                        return Resolution::Failed;
                    }
                    let mut substitution_context = HashMap::new();
                    for (index, generic_argument_id) in generic_argument_ids.iter().enumerate() {
                        if let Some(generic_constraint) =
                            generic_parameter_constraint_ids.get(index)
                        {
                            substitution_context.insert(*generic_constraint, *generic_argument_id);
                        }
                    }
                    for (index, parameter_id) in parameters.iter().enumerate() {
                        let parameter = self.parameters.get(parameter_id).unwrap();
                        let parameter_type = parameter.type_id.get_type(self);
                        let argument_id = *argument_ids.get(index).unwrap();
                        let argument_type =
                            self.infer_type(argument_id, &parameter_type, &substitution_context);
                        // Defer while an argument is unresolved, or is a closure
                        // parameter still awaiting its type (`count.derive(|n|
                        // format(n))` — `n` is typed only once `derive` resolves).
                        if matches!(argument_type, Type::Unresolved)
                            || self.is_unknown_closure_parameter(argument_id)
                        {
                            return Resolution::Deferred;
                        }
                        // Reconcile parameter-first so the bindings key on the
                        // *callee's* generics (passing a `T`-typed value to
                        // `f<U>(u: U)` must bind `U = T`, not `T = U`).
                        match self.reconcile_type(
                            &parameter_type,
                            &argument_type,
                            &substitution_context,
                        ) {
                            Some((_unified, bindings)) => {
                                for (constraint_id, type_id) in bindings {
                                    substitution_context.insert(constraint_id, type_id);
                                }
                            }
                            None => {
                                let expected =
                                    self.pretty_print_type(&parameter_type, &substitution_context);
                                let got =
                                    self.pretty_print_type(&argument_type, &substitution_context);
                                self.diagnostics.push(Error {
                                    span: **self.span_map.get(&argument_id).unwrap(),
                                    msg: format!("Expected {}, but got {} instead.", expected, got),
                                });
                            }
                        }
                    }
                    self.wire_call(
                        call_id,
                        subject_id,
                        generic_argument_ids,
                        argument_ids,
                        arguments_span,
                    );
                    // Record generic bindings inferred from the arguments so the
                    // transformer can monomorphize the call (e.g. `range(0, 9)`
                    // binds `T = i32`, `Box::new(5)` binds the impl's `T`). Key off
                    // the inferred bindings, not the function's own generic list.
                    if !substitution_context.is_empty() {
                        self.method_call_substitution
                            .insert(call_id, substitution_context);
                    }
                    Resolution::Resolved
                } else if !matches!(target, Expr::Error) {
                    // The subject resolved to a non-callable value (a struct or
                    // module name, not a function or variant) — e.g. `Point(1, 2)`.
                    let struct_name = match &target {
                        Expr::Struct(struct_id) => self.structs.get(struct_id).map(|s| s.name),
                        _ => None,
                    };
                    self.diagnostics.push(Error {
                        span: arguments_span,
                        msg: match struct_name {
                            Some(name) => format!(
                                "cannot call '{name}': it is a struct, not a function — construct it with `{{ .. }}` or `::new(..)`"
                            ),
                            None => "cannot call a non-function value".to_string(),
                        },
                    });
                    Resolution::Failed
                } else {
                    // The subject is already an error; leave it to its own diagnostic.
                    Resolution::Failed
                }
            }
            // A direct function reference.
            Expr::Function(_) | Expr::ExternalFunction(_) => {
                self.wire_call(
                    call_id,
                    subject_id,
                    generic_argument_ids,
                    argument_ids,
                    arguments_span,
                );
                Resolution::Resolved
            }
            _ => {
                self.diagnostics.push(Error {
                    span: arguments_span,
                    msg: "cannot call a non-function value".to_string(),
                });
                Resolution::Failed
            }
        }
    }

    /// `for item in iterable`: once the iterable's type is known, the item takes
    /// its element type. Defers while the iterable (or its element slot, which a
    /// later `push` may fill) is unresolved.
    fn resolve_for_each_item(&mut self, item_id: Id, iterable_id: Id) -> Resolution {
        let iterable_type = self.infer_type(iterable_id, &Type::Unknown, &HashMap::new());
        let next_method = self.for_each_next_method(Some(item_id));
        let element_type = self.iterable_element_type(&iterable_type, next_method);
        if matches!(iterable_type, Type::Unresolved)
            || matches!(element_type, Some(Type::Unknown | Type::Unresolved))
        {
            return Resolution::Deferred;
        }
        let element_type_id = element_type.unwrap_or(Type::Any).get_type_id(self);
        if let Some(variable) = self.variables.get_mut(&item_id) {
            variable.type_id = element_type_id;
        }
        self.resolved_types.insert(item_id, element_type_id);
        Resolution::Resolved
    }

    /// `receiver.method(args)`: resolve the method against the receiver type
    /// (concrete impl, trait default, or bound generic), bind the impl's and the
    /// method's own generics, wire the call, and spawn a `MethodArgCheck` (plus a
    /// `SlotUnification` for `push`/`run`). Defers while the receiver is
    /// unresolved or an unknown closure parameter.
    fn resolve_method_call(
        &mut self,
        id: Id,
        subject_id: Id,
        member_name: &'src str,
        generic_argument_ids: &[TypeId],
        argument_ids: &[Id],
        arguments_span: Span,
    ) -> Resolution {
        // The outcome of resolving the receiver to a callable member (kept
        // separate from acting on it so the lookup can borrow `self` immutably and
        // the wiring mutably).
        enum MethodLookup {
            Found(Id),
            NoMethod,
            Defer,
            NotCallable,
        }
        let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
        let found = |member_id: Option<Id>| match member_id {
            Some(member_id) => MethodLookup::Found(member_id),
            None => MethodLookup::NoMethod,
        };
        let lookup = match &subject_type {
            Type::Struct(_, _) | Type::Enum(_, _) => {
                match self.method_member_impl_subject(&subject_type, member_name) {
                    Some((member_id, impl_subject_id)) => {
                        // Bind the impl's generic parameters from the receiver
                        // (`List<i32>` against the impl's `List<T>` binds `T = i32`)
                        // so the method body monomorphizes.
                        let impl_subject = impl_subject_id.get_type(self);
                        if let Some((_, bindings)) =
                            self.reconcile_type(&impl_subject, &subject_type, &HashMap::new())
                            && !bindings.is_empty()
                        {
                            self.method_call_substitution
                                .insert(id, bindings.into_iter().collect());
                        }
                        // A method that fills a container's inference slot —
                        // `list.push(value)` or `context.run(value, ..)` — unifies
                        // the slot with the value (the first argument).
                        if member_name == "push" || member_name == "run" {
                            if let (Some(slot), Some(argument_id)) =
                                (self.list_element_slot(&subject_type), argument_ids.first())
                            {
                                self.constraints.push(Constraint::SlotUnification {
                                    slot,
                                    argument_id: *argument_id,
                                });
                            }
                        }
                        MethodLookup::Found(member_id)
                    }
                    // Gap E: fall back to an inherited trait default, re-dispatched
                    // to this concrete type at codegen.
                    None => {
                        match self.method_member_in_inherited_defaults(&subject_type, member_name) {
                            Some(member_id) => {
                                let receiver_type_id = subject_type.clone().get_type_id(self);
                                self.generic_dispatch.insert(
                                    id,
                                    GenericDispatch::OnType(Some(receiver_type_id), member_name),
                                );
                                MethodLookup::Found(member_id)
                            }
                            None => MethodLookup::NoMethod,
                        }
                    }
                }
            }
            Type::Trait(trait_id, trait_arguments) => {
                let trait_id = *trait_id;
                let trait_arguments = trait_arguments.clone();
                let member = self.method_member_in_trait(trait_id, member_name);
                // Inside a trait default body `self`/`Self` is `Type::Trait`; record
                // the call so codegen re-dispatches it to whatever concrete type the
                // default is specialized for.
                if member.is_some() {
                    self.generic_dispatch
                        .insert(id, GenericDispatch::OnType(None, member_name));
                    // A parameterized trait substitutes its generic parameters with
                    // the concrete arguments, so the method's signature (`got(): T`)
                    // types against them (`Get<i32>::got` -> `i32`).
                    if !trait_arguments.is_empty() {
                        let parameter_ids = self
                            .traits
                            .get(&trait_id)
                            .map(|trait_| trait_.generic_parameter_constraint_ids.clone())
                            .unwrap_or_default();
                        let substitution: SubstitutionContext =
                            parameter_ids.into_iter().zip(trait_arguments).collect();
                        if !substitution.is_empty() {
                            self.method_call_substitution.insert(id, substitution);
                        }
                    }
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
                    let member = bound_trait_ids
                        .iter()
                        .find_map(|trait_id| self.method_member_in_trait(*trait_id, member_name));
                    // The member may be an abstract (bodyless) required method, so
                    // record the call for codegen to re-dispatch to the concrete
                    // type `T` is bound to at each monomorphization.
                    if member.is_some() {
                        self.generic_dispatch.insert(
                            id,
                            GenericDispatch::OnConstraint(*constraint_id, member_name),
                        );
                    }
                    found(member)
                }
            }
            Type::Unresolved => MethodLookup::Defer,
            // An unannotated closure parameter awaiting bidirectional inference
            // defers rather than erroring; once its type lands the call resolves.
            Type::Unknown if self.is_unknown_closure_parameter(subject_id) => MethodLookup::Defer,
            // A comprehension binder whose element type isn't set yet defers, too.
            Type::Unknown if self.is_untyped_comprehension_binder(subject_id) => {
                MethodLookup::Defer
            }
            _ => MethodLookup::NotCallable,
        };
        match lookup {
            MethodLookup::Found(member_id) => {
                // Drive bidirectional inference of any closure arguments against the
                // method's parameter types, and defer a full argument type-check.
                let mut substitution = self
                    .method_call_substitution
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                // Bind the method's own generics from the non-closure arguments
                // first, so a closure parameter `|T| ..` is typed with `T` known.
                self.bind_method_own_generics(member_id, argument_ids, true, &mut substitution);
                self.infer_closure_args_against_params(member_id, argument_ids, &substitution);
                // Then bind generics fixed by a closure's return (`derive<U>`'s `U`),
                // now that the closures are typed.
                self.bind_method_own_generics(member_id, argument_ids, false, &mut substitution);
                if !substitution.is_empty() {
                    self.method_call_substitution.insert(id, substitution);
                }
                self.constraints.push(Constraint::MethodArgCheck {
                    member_id,
                    argument_ids: argument_ids.to_vec(),
                    arguments_span,
                });
                self.wire_method_call(
                    id,
                    subject_id,
                    member_id,
                    generic_argument_ids.to_vec(),
                    argument_ids.to_vec(),
                    arguments_span,
                );
                Resolution::Resolved
            }
            MethodLookup::NoMethod => {
                let type_str = self.pretty_print_type(&subject_type, &HashMap::new());
                self.diagnostics.push(Error {
                    span: arguments_span,
                    msg: format!("{} has no method '{}'", type_str, member_name),
                });
                self.expr_id_to_expr_map.insert(id, Expr::Error);
                Resolution::Failed
            }
            MethodLookup::Defer => Resolution::Deferred,
            MethodLookup::NotCallable => {
                let type_str = self.pretty_print_type(&subject_type, &HashMap::new());
                self.diagnostics.push(Error {
                    span: arguments_span,
                    msg: format!("cannot call method '{}' on {}", member_name, type_str),
                });
                Resolution::Failed
            }
        }
    }

    /// Unify a container's element slot with a pushed value's type. A slot already
    /// filled (by an earlier push) is a no-op; defers while the value is unresolved.
    fn resolve_slot_unification(&mut self, slot: TypeId, argument_id: Id) -> Resolution {
        if !matches!(slot.get_type(self), Type::Unknown) {
            // Already unified by an earlier push.
            return Resolution::Resolved;
        }
        let argument_type = self.infer_type(argument_id, &Type::Unknown, &HashMap::new());
        if matches!(argument_type, Type::Unresolved) {
            return Resolution::Deferred;
        }
        if !matches!(argument_type, Type::Unknown) {
            self.type_id_to_type_map.insert(slot, argument_type);
        }
        Resolution::Resolved
    }

    /// Type-check a wired method call's arguments against the method's parameters
    /// (`self` is parameter 0, so arguments align at offset 1). Defers until every
    /// argument resolves, so errors aren't reported against partial types.
    fn resolve_method_arg_check(
        &mut self,
        member_id: Id,
        argument_ids: &[Id],
        arguments_span: Span,
    ) -> Resolution {
        let Some((parameter_ids, _)) = self.method_signature(member_id) else {
            return Resolution::Resolved;
        };
        let expected = parameter_ids.len().saturating_sub(1);
        if argument_ids.len() != expected {
            self.diagnostics.push(Error {
                span: arguments_span,
                msg: format!(
                    "Expected {} {}, but got {} instead.",
                    expected,
                    plural(expected, "argument", "arguments"),
                    argument_ids.len()
                ),
            });
            return Resolution::Failed;
        }
        // Infer every argument first; defer until they all resolve.
        let mut argument_types = Vec::with_capacity(argument_ids.len());
        for argument_id in argument_ids {
            // `+ 1` skips the method's `self` parameter.
            let parameter_type = parameter_ids
                .get(argument_types.len() + 1)
                .and_then(|parameter_id| self.parameters.get(parameter_id))
                .map(|parameter| parameter.type_id.get_type(self))
                .unwrap_or(Type::Unknown);
            let argument_type = self.infer_type(*argument_id, &parameter_type, &HashMap::new());
            if matches!(argument_type, Type::Unresolved) {
                return Resolution::Deferred;
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
        Resolution::Resolved
    }

    /// `let (a, b) = value`: once the value's type is known, resolve the pattern
    /// against it (an explicit annotation takes precedence), typing each binding
    /// from the corresponding tuple element. Records the `Expr::Destructure` the
    /// transformer lowers.
    /// `(x in xs => e)`: once the source resolves to a mapped tuple, type the
    /// binder as its element template so the body checks, and record the
    /// comprehension expression (it itself types as a mapped tuple via `infer_type`).
    fn resolve_comprehension(
        &mut self,
        id: Id,
        binder_id: Id,
        source_id: Id,
        body_id: Id,
    ) -> Resolution {
        let source_type = self.infer_type(source_id, &Type::Unknown, &HashMap::new());
        match self.expand_mapped(source_type) {
            Type::Unresolved | Type::Unknown => Resolution::Deferred,
            Type::Mapped(_, _, element_template) => {
                if let Some(variable) = self.variables.get_mut(&binder_id) {
                    variable.type_id = element_template;
                }
                self.untyped_comprehension_binders.remove(&binder_id);
                self.expr_id_to_expr_map
                    .insert(id, Expr::TupleComprehension(binder_id, source_id, body_id));
                Resolution::Resolved
            }
            // A concrete tuple source isn't supported yet (heterogeneous elements
            // have no single binder type); only mapped sources, which combine uses.
            other => {
                let got = self.pretty_print_type(&other, &HashMap::new());
                self.diagnostics.push(Error {
                    span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                    msg: format!(
                        "a tuple comprehension's source must be a mapped tuple, got {got}"
                    ),
                });
                Resolution::Failed
            }
        }
    }

    fn resolve_destructure(&mut self, constraint: &DestructureConstraint<'src>) -> Resolution {
        let value_type = self.infer_type(constraint.value_id, &Type::Unknown, &HashMap::new());
        let not_ready = matches!(value_type, Type::Unresolved)
            || (constraint.defer_until_known && matches!(value_type, Type::Unknown));
        if not_ready {
            return Resolution::Deferred;
        }
        let expected_type_id = constraint
            .type_id
            .unwrap_or_else(|| value_type.get_type_id(self));
        match self.resolve_pattern(&constraint.pattern, expected_type_id, constraint.scope_id) {
            Some(resolved) => {
                self.expr_id_to_expr_map.insert(
                    constraint.id,
                    Expr::Destructure(constraint.value_id, resolved),
                );
                Resolution::Resolved
            }
            None => Resolution::Failed,
        }
    }

    /// `let v = value` (plus reassignments): ground the variable's type from its
    /// first value (which must be ready), then reconcile the reassignments. A
    /// reassignment that isn't ready re-queues a fresh `Variable` task carrying
    /// the grounded type and just the still-pending values.
    fn resolve_variable(&mut self, constraint: &VariableConstraint) -> Resolution {
        let variable_id = constraint.variable_id;
        let initial_type_id = constraint.initial_type_id;
        let value_ids = &constraint.value_ids;

        // The first value (with the annotation) grounds the variable's type and
        // must be ready. Later values — reassignments — may refer to the variable
        // itself (e.g. `i += 1`), so they are checked only after grounding.
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
            return Resolution::Deferred;
        }

        let mut substitution_context = HashMap::new();
        let mut variable_type = initial_type_id.get_type(self);

        if let Some(&first_value_id) = value_ids.first() {
            let value_type = self.infer_type(first_value_id, &variable_type, &substitution_context);
            match self.reconcile_type(&value_type, &variable_type, &substitution_context) {
                Some((unified, bindings)) => {
                    for (constraint_id, type_id) in bindings {
                        substitution_context.insert(constraint_id, type_id);
                    }
                    if let Type::Unknown = variable_type {
                        variable_type = unified;
                    }
                }
                None => {
                    let expected_str =
                        self.pretty_print_type(&variable_type, &substitution_context);
                    let got_str = self.pretty_print_type(&value_type, &substitution_context);
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&first_value_id).unwrap(),
                        msg: format!("Expected {}, but got {} instead.", expected_str, got_str),
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
            let value_type = self.infer_type(value_id, &variable_type, &substitution_context);
            if matches!(value_type, Type::Unresolved) {
                deferred_value_ids.push(value_id);
                continue;
            }
            match self.reconcile_type(&value_type, &variable_type, &substitution_context) {
                Some((_, bindings)) => {
                    for (constraint_id, type_id) in bindings {
                        substitution_context.insert(constraint_id, type_id);
                    }
                }
                None => {
                    let expected_str =
                        self.pretty_print_type(&variable_type, &substitution_context);
                    let got_str = self.pretty_print_type(&value_type, &substitution_context);
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&value_id).unwrap(),
                        msg: format!("Expected {}, but got {} instead.", expected_str, got_str),
                    });
                }
            }
        }
        if !deferred_value_ids.is_empty() {
            // Re-queue the still-pending reassignments against the now-grounded
            // type (a fresh task, processed next pass at this kind's priority).
            self.constraints
                .push(Constraint::Variable(VariableConstraint {
                    variable_id,
                    initial_type_id: var_type_id,
                    value_ids: deferred_value_ids,
                }));
        }
        Resolution::Resolved
    }

    /// `match subject { .. }`: once the subject type is known, resolve each leg's
    /// patterns (typing captures) and guard, check exhaustiveness, and type the
    /// match as the unification of its leg bodies. Defers while the subject, a
    /// guard, or a leg body is unresolved.
    fn resolve_match(&mut self, prepped: &PreppedMatch<'src>) -> Resolution {
        let subject_type = self.infer_type(prepped.subject_id, &Type::Unknown, &HashMap::new());
        if matches!(subject_type, Type::Unresolved) {
            return Resolution::Deferred;
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
                } else if !self.compare_type(&guard_type, &self.bool_type(), &HashMap::new()) {
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
            return Resolution::Deferred;
        }
        if pattern_error {
            // Diagnostics were already emitted; drop the match.
            return Resolution::Failed;
        }

        // Exhaustiveness: a leg is an irrefutable catch-all when it is unguarded
        // and a pattern matches anything (`_`, a binding, or a tuple destructure).
        let has_catch_all = resolved_legs.iter().any(|(patterns, guard, _)| {
            guard.is_none()
                && patterns.iter().any(|pattern| {
                    matches!(
                        pattern,
                        ExprPattern::Wildcard | ExprPattern::Binding(_) | ExprPattern::Tuple(_)
                    )
                })
        });
        match &subject_type {
            Type::Enum(enum_id, _) if !has_catch_all => {
                // Each unguarded variant pattern (in any leg) covers its variant.
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
                        msg: format!("match is not exhaustive: missing {}", missing.join(", ")),
                    });
                }
            }
            // A non-enum subject (e.g. a `str` matched with literals) has an
            // unbounded domain, so it needs an explicit catch-all. Tuples and
            // not-yet-known types are exempt.
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
        for (_, _, body_id) in &resolved_legs {
            let body_type = self.infer_type(*body_id, &Type::Unknown, &HashMap::new());
            if matches!(body_type, Type::Unresolved) {
                return Resolution::Deferred;
            }
            unified = Some(match unified {
                None => body_type,
                Some(current) => match self.reconcile_type(&current, &body_type, &HashMap::new()) {
                    Some((unified_type, _)) => unified_type,
                    None => {
                        let expected = self.pretty_print_type(&current, &HashMap::new());
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
                },
            });
        }
        let match_type = unified.unwrap_or(Type::Void);
        let match_type_id = match_type.get_type_id(self);
        self.resolved_types.insert(prepped.id, match_type_id);
        // Expand each or-pattern leg into one leg per alternative, all sharing the
        // guard and body.
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
        Resolution::Resolved
    }

    /// `Struct { field = value, .. }`: resolve the struct by name (lexically),
    /// check field count, infer each value against its declared field type
    /// (binding the struct's type arguments), and record the initializer. Defers
    /// while any field value is unresolved. The initializer expression and type
    /// are stored on every attempt (the partial store is overwritten on re-run),
    /// matching the original "always store" behaviour.
    fn resolve_struct_initializer(
        &mut self,
        constraint: &StructInitializerConstraint<'src>,
    ) -> Resolution {
        let struct_id =
            match self.try_get_expr_id_by_name(constraint.struct_name, constraint.scope_id) {
                Some(expr_id) => expr_id,
                None => {
                    self.diagnostics.push(Error {
                        span: constraint.fields_span.clone(),
                        msg: format!("unknown struct: {}", constraint.struct_name),
                    });
                    return Resolution::Failed;
                }
            };
        let struct_ = match self.structs.get(&struct_id) {
            Some(struct_) => struct_,
            None => {
                self.diagnostics.push(Error {
                    span: constraint.fields_span.clone(),
                    msg: format!("cannot initialize a non-struct: {}", constraint.struct_name),
                });
                return Resolution::Failed;
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
            return Resolution::Failed;
        }
        let initializer_id = constraint.initializer_id;
        let struct_name = constraint.struct_name;
        let mut initializer_fields = IndexMap::new();
        let mut substitution_context = HashMap::new();
        for (index, generic_argument_id) in constraint.generic_argument_ids.iter().enumerate() {
            if let Some(generic_constraint) = generic_param_ids.get(index) {
                substitution_context.insert(*generic_constraint, *generic_argument_id);
            }
        }
        let mut deferred = false;
        for (field_name, field_value, field_value_span) in &constraint.fields {
            let field = struct_fields
                .iter()
                .enumerate()
                .find(|(_, field)| *field.name == **field_name);
            let (struct_field_index, struct_field) = match field {
                Some(field) => field,
                None => {
                    self.diagnostics.push(Error {
                        span: *field_value_span,
                        msg: format!("struct '{}' has no field '{}'", struct_name, field_name),
                    });
                    continue;
                }
            };
            let struct_field_type = struct_field.type_id.get_type(self);
            // Infer the value against the declared field type so that, e.g., an
            // integer literal is treated as `f64` when the field is `f64`.
            let value_type =
                self.infer_type(*field_value, &struct_field_type, &substitution_context);
            if let Type::Unresolved = value_type {
                deferred = true;
                break;
            }
            if let Some((_unified, bindings)) =
                self.reconcile_type(&value_type, &struct_field_type, &substitution_context)
            {
                for (constraint_id, type_id) in bindings {
                    substitution_context.insert(constraint_id, type_id);
                }
                initializer_fields.insert(struct_field_index, *field_value);
            } else {
                // Type mismatch: emit a diagnostic but still record the type for
                // downstream consumers.
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
        // Always store the initializer expression so `infer_type` can handle it
        // (a partial store while deferred is overwritten on the resolving run).
        self.expr_id_to_expr_map.insert(
            initializer_id,
            Expr::StructInitializer(initializer_id, initializer_fields),
        );
        self.struct_initializer_to_def
            .insert(initializer_id, struct_id);
        // Fill the struct's type arguments from the bindings inferred above
        // (`Box { value = 5 }` -> `Box<i32>`), so methods called on the value
        // monomorphize against the concrete element type. A parameter no field
        // constrains stays abstract.
        let type_arguments = generic_param_ids
            .iter()
            .map(|constraint_id| {
                substitution_context
                    .get(constraint_id)
                    .copied()
                    .unwrap_or(*constraint_id)
            })
            .collect();
        let type_id = Type::Struct(struct_id, type_arguments).get_type_id(self);
        self.resolved_types.insert(initializer_id, type_id);
        if deferred {
            Resolution::Deferred
        } else {
            Resolution::Resolved
        }
    }

    /// `subject.field`: once the subject resolves to a struct, the accessor's type
    /// is the named field's. Defers while the subject is unresolved (or an
    /// unknown closure parameter awaiting bidirectional inference).
    fn resolve_field_accessor(&mut self, constraint: &FieldAccessorConstraint<'src>) -> Resolution {
        let id = constraint.id;
        let subject_id = constraint.subject_id;
        let member_name = constraint.member_name;
        if !self.expr_id_to_expr_map.contains_key(&subject_id) {
            return Resolution::Deferred;
        }
        let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
        match subject_type {
            Type::Unresolved => Resolution::Deferred,
            Type::Struct(struct_id, _) => {
                let struct_ = match self.structs.get(&struct_id) {
                    Some(struct_) => struct_,
                    None => {
                        self.diagnostics.push(Error {
                            span: **self.span_map.get(&id).unwrap(),
                            msg: format!("subject is not a struct: {}", struct_id.0),
                        });
                        return Resolution::Failed;
                    }
                };
                let struct_name = struct_.name;
                let field = struct_
                    .fields
                    .iter()
                    .enumerate()
                    .find_map(|(index, field)| {
                        (field.name == member_name).then_some((index, field.type_id))
                    });
                match field {
                    Some((field_index, field_type)) => {
                        self.expr_id_to_expr_map
                            .insert(id, Expr::Field(subject_id, struct_id, field_index));
                        self.expr_id_to_type_id_map.insert(id, field_type);
                        self.resolved_types.insert(id, field_type);
                        Resolution::Resolved
                    }
                    None => {
                        self.diagnostics.push(Error {
                            span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                            msg: format!("struct '{}' has no field '{}'", struct_name, member_name),
                        });
                        self.expr_id_to_expr_map.insert(id, Expr::Error);
                        Resolution::Failed
                    }
                }
            }
            subject_type => {
                // A closure parameter still awaiting bidirectional inference —
                // typed when the enclosing method call resolves (a later pass,
                // since method calls solve after field accessors). Defer.
                if self.is_unknown_closure_parameter(subject_id) {
                    return Resolution::Deferred;
                }
                let subject_str = self.pretty_print_type(&subject_type, &HashMap::new());
                self.diagnostics.push(Error {
                    span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                    msg: format!(
                        "cannot access field '{}' on type {}",
                        member_name, subject_str
                    ),
                });
                self.expr_id_to_expr_map.insert(id, Expr::Error);
                Resolution::Failed
            }
        }
    }

    /// `subject is Pattern`: once the subject type is known, resolve the pattern
    /// (typing its captures) and record the `Expr::Is`. `None` from the pattern
    /// means a diagnostic was already emitted.
    fn resolve_is(&mut self, prepped: &PreppedIs<'src>) -> Resolution {
        let subject_type = self.infer_type(prepped.subject_id, &Type::Unknown, &HashMap::new());
        if matches!(subject_type, Type::Unresolved) {
            return Resolution::Deferred;
        }
        let subject_type_id = subject_type.get_type_id(self);
        match self.resolve_pattern(&prepped.pattern, subject_type_id, prepped.scope_id) {
            Some(resolved) => {
                self.expr_id_to_expr_map
                    .insert(prepped.id, Expr::Is(prepped.subject_id, resolved));
                Resolution::Resolved
            }
            None => Resolution::Failed,
        }
    }

    /// `subject[index]`: once the subject's `List<T>` type is known, the
    /// subscript's type is the element `T`; records the resolved `Expr::Index`.
    fn resolve_subscript(&mut self, id: Id, subject_id: Id, index_id: Id) -> Resolution {
        if !self.expr_id_to_expr_map.contains_key(&subject_id) {
            return Resolution::Deferred;
        }
        let subject_type = self.infer_type(subject_id, &Type::Unknown, &HashMap::new());
        let list_id = self.primitive_struct_ids.get("List").copied();
        match subject_type {
            Type::Unresolved => Resolution::Deferred,
            Type::Struct(struct_id, arguments)
                if Some(struct_id) == list_id && arguments.len() == 1 =>
            {
                let element_type = arguments[0];
                self.expr_id_to_expr_map
                    .insert(id, Expr::Index(subject_id, index_id));
                self.expr_id_to_type_id_map.insert(id, element_type);
                self.resolved_types.insert(id, element_type);
                Resolution::Resolved
            }
            subject_type => {
                let subject_str = self.pretty_print_type(&subject_type, &HashMap::new());
                self.diagnostics.push(Error {
                    span: **self.span_map.get(&id).unwrap_or(&&EMPTY_SPAN),
                    msg: format!("cannot index {} (only a `List` is indexable)", subject_str),
                });
                self.expr_id_to_expr_map.insert(id, Expr::Error);
                Resolution::Failed
            }
        }
    }

    fn build(&mut self) {
        // Resolve imports/re-exports to a fixpoint: a re-export may name an item
        // bound by another re-export resolved in a later pass (a chain of relay
        // modules), so keep retrying the unresolved ones until a pass binds
        // nothing new, then report whatever genuinely could not be found.
        let mut remaining = self.prepped_imports.clone();
        loop {
            let before = remaining.len();
            remaining.retain(|(path, name, scope_id, span, leaf_span, source_id)| {
                !self.resolve_import(path, name, *scope_id, *span, false, *leaf_span, *source_id)
            });
            if remaining.len() == before || remaining.is_empty() {
                break;
            }
        }
        for (path, name, scope_id, span, leaf_span, source_id) in remaining {
            self.resolve_import(&path, name, scope_id, span, true, leaf_span, source_id);
        }

        // --- Resolve `use` statements ---
        // `use Namespace::{ a, b }` binds items out of a namespace — a module
        // or an enum (whose namespace holds its variants) — into the scope the
        // statement appears in.
        for (path, name, scope_id, span, leaf_span, source_id) in
            std::mem::take(&mut self.prepped_uses)
        {
            let mut segments = path
                .iter()
                .copied()
                .chain(std::iter::once((name, leaf_span)));
            let (root, root_span) = segments.next().unwrap();
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
            self.record_reference(source_id, root_span, current);
            let mut resolved = true;
            for (segment, segment_span) in segments {
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
                self.record_reference(source_id, segment_span, current);
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
            // A view binding or `&[mut]` parameter is written *through* — the
            // assignment mutates the referent, not the binding — so its
            // writability is enforced by `check_readonly_mutation` (after
            // `borrows` is inferred) and the assigned value refines the referent,
            // not this binding's own type. Skip both checks here.
            if self.binding_or_param_is_view(variable_id) {
                continue;
            }
            // An ordinary immutable local is rejected by `check_readonly_mutation`
            // (which knows about views); here we only flag a target that is not a
            // variable at all (e.g. a by-value parameter) and feed the assigned
            // value into the variable's type inference.
            if self.variables.get(&variable_id).is_none() {
                self.diagnostics.push(Error {
                    span: **self.span_map.get(&target_id).unwrap_or(&&EMPTY_SPAN),
                    msg: "cannot assign to this expression".to_string(),
                });
                continue;
            }
            if let Some(Constraint::Variable(constraint)) =
                self.constraints.iter_mut().find(|constraint| {
                    matches!(constraint, Constraint::Variable(constraint) if constraint.variable_id == variable_id)
                })
            {
                constraint.value_ids.push(value_id);
            }
        }

        for (type_id, name, scope_id, span, argument_type_ids, source_id) in
            self.prepped_type_locals.clone()
        {
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
                        // A parameterized trait bound/template (`Into<bool>`,
                        // `Readable<U>`) keeps its arguments for impl selection.
                        (Type::Trait(id, _), false) => Type::Trait(id, argument_type_ids),
                        (other, _) => other,
                    };
                    // Record the reference for the language server: the type name
                    // span, the definition it points at, and a hover label. A
                    // generic resolves to its binder entity (`subject_id`), which
                    // carries the `<T>` name span.
                    let definition_id = match &subject_type {
                        Type::Struct(id, _) | Type::Enum(id, _) | Type::Trait(id, _) => Some(*id),
                        Type::Generic(_) => Some(subject_id),
                        _ => None,
                    };
                    // Store the type id; its label is rendered after `build`,
                    // when all referenced types are resolved. Rendering here could
                    // hit a not-yet-resolved type id and panic.
                    self.type_references
                        .push((source_id, span, definition_id, type_id));
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
                        msg: format!(
                            "cannot access field '{}' on type {}",
                            member_name, subject_str
                        ),
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
                Type::Struct(_, _) | Type::Trait(_, _) | Type::Enum(_, _) => {
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
                        self.generic_dispatch.insert(
                            id,
                            GenericDispatch::OnConstraint(constraint_id, member_name),
                        );
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
                implementation
                    .trait_args
                    .push((trait_id, check.trait_arguments.clone()));
            }
            // Record the `with <trait>` reference for go-to-definition / hover.
            let trait_type_id = Type::Trait(trait_id, Vec::new()).get_type_id(self);
            self.type_references
                .push((check.source_id, check.span, Some(trait_id), trait_type_id));
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
        // A true fixpoint: each pass resolves the constraints whose dependencies
        // have landed (their blocked dependents resolve on later passes), in
        // priority order; resolved types never revert. The loop exits the moment a
        // pass resolves nothing — so it is order-independent: whatever can resolve
        // eventually does, regardless of which pass reaches it. The bound is only a
        // safety net against a non-converging bug, never the reason a well-typed
        // program resolves, so it just has to exceed any real dependency chain.
        // Each resolution consumes a distinct queued task, and the tasks (including
        // the slot unifications spawned mid-solve while resolving `push`/`run`) are
        // bounded by the entity count, so twice it is ample.
        let max_iterations = 2 * self.entity_id as usize + 16;

        for _ in 0..max_iterations {
            if !self.resolve_constraints() {
                break;
            }
        }

        // Commit any `for x in iterable` bindings still deferred (their element
        // slot never resolved — an empty, never-pushed list): the item is `any`.
        let deferred_for_each: Vec<(Id, Id)> = self
            .constraints
            .iter()
            .filter_map(|constraint| match constraint {
                Constraint::ForEachItem {
                    item_id,
                    iterable_id,
                } => Some((*item_id, *iterable_id)),
                _ => None,
            })
            .collect();
        for (item_id, iterable_id) in deferred_for_each {
            let iterable_type = self.infer_type(iterable_id, &Type::Unknown, &HashMap::new());
            let next_method = self.for_each_next_method(Some(item_id));
            let element_type = self
                .iterable_element_type(&iterable_type, next_method)
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
                // `for e in &mut container` drives a `next_mut(&mut self): Option<&mut
                // T>` iterator (each binding a writable view); a plain `for x in
                // container` drives `next`. A built-in `List`/`Set` has neither and
                // falls through to the indexed/native loop.
                let item_id = match self.expr_id_to_expr_map.get(&for_each_id) {
                    Some(Expr::ForEach(_, item_id, _)) => *item_id,
                    _ => None,
                };
                let next_method = self.for_each_next_method(item_id);
                if let Some(next_id) = self.method_member_in_impls(&iterable_type, next_method) {
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
            // A generic-bounded operand (`x == y` where `x: T: PartialEq`, e.g. the
            // element compare inside `Option<T>::eq`) dispatches to the operator
            // trait method, re-resolved to T's concrete impl at each monomorphization
            // — the operator analogue of an instance method call on a generic
            // receiver, recorded in the same `generic_dispatch` channel.
            if let Type::Generic(constraint_id) = lhs_type {
                if let Some((_, method_name)) = operator_trait_method(op) {
                    let bound_trait_ids = self.generic_bound_trait_ids(constraint_id);
                    let provides = bound_trait_ids.iter().any(|trait_id| {
                        self.method_member_in_trait(*trait_id, method_name)
                            .is_some()
                    });
                    if provides {
                        self.generic_dispatch.insert(
                            binary_id,
                            GenericDispatch::OnConstraint(constraint_id, method_name),
                        );
                    }
                }
                continue;
            }
            if !matches!(lhs_type, Type::Struct(_, _) | Type::Enum(_, _)) {
                continue;
            }
            if let Some((method_id, impl_subject_id)) = self.operator_method(op, &lhs_type) {
                self.binary_op_dispatch.insert(binary_id, method_id);
                // Monomorphize the operator method against the operand's type args
                // (`Option<Point> ==` binds the impl's `Option<T>` to `T = Point`) so
                // a generic element comparison inside the body — e.g. `Option<T>::eq`'s
                // `x == y` — dispatches to the concrete impl. Only needed when a bound
                // type is a non-native aggregate; for all-native bindings (e.g.
                // `Option<i32>`) the generic emission already lowers `==` to native JS.
                let impl_subject = impl_subject_id.get_type(self);
                if let Some((_, bindings)) =
                    self.reconcile_type(&impl_subject, &lhs_type, &HashMap::new())
                {
                    let needs_specialization = bindings.iter().any(|(_, concrete)| {
                        !self.is_native_operator_type(&concrete.get_type(self))
                    });
                    if needs_specialization {
                        self.method_call_substitution
                            .insert(binary_id, bindings.into_iter().collect());
                    }
                }
            } else if let Some((trait_name, method_name)) = operator_trait_method(op) {
                // The operator maps to a trait but this operand doesn't provide it.
                // Native-operator types (the scalars, `bool`, numeric enums) use
                // native JS, so skip them; any other type genuinely needs the impl.
                if !self.is_native_operator_type(&lhs_type) {
                    let type_name = match &lhs_type {
                        Type::Struct(id, _) => self.structs.get(id).map(|s| s.name),
                        Type::Enum(id, _) => self.enums.get(id).map(|e| e.name),
                        _ => None,
                    }
                    .unwrap_or("value");
                    self.diagnostics.push(Error {
                        span: **self.span_map.get(&binary_id).unwrap_or(&&EMPTY_SPAN),
                        msg: format!(
                            "type '{type_name}' does not implement the `{trait_name}` operator; \
                             add `impl {trait_name} for {type_name}` with the `{method_name}` method"
                        ),
                    });
                }
            }
        }

        // --- Post-solve diagnostics ---
        // Unresolved tasks still on the queue could not be typed (priority order
        // — struct initializers before field accessors — matches the original
        // per-list diagnostic order).
        for constraint in &self.constraints {
            match constraint {
                Constraint::StructInitializer(constraint) => self.diagnostics.push(Error {
                    span: constraint.fields_span,
                    msg: "type of struct initializer could not be resolved".to_string(),
                }),
                Constraint::FieldAccessor(constraint) => self.diagnostics.push(Error {
                    span: **(self.span_map.get(&constraint.id).unwrap_or(&&EMPTY_SPAN)),
                    msg: "type of accessor subject could not be resolved".to_string(),
                }),
                Constraint::Variable(constraint) => self.diagnostics.push(Error {
                    span: **(self
                        .span_map
                        .get(&constraint.variable_id)
                        .unwrap_or(&&EMPTY_SPAN)),
                    msg: format!(
                        "type of variable '{}' could not be resolved",
                        self.variables
                            .get(&constraint.variable_id)
                            .map(|variable| variable.name)
                            .unwrap_or("unknown")
                    ),
                }),
                Constraint::CallSubject(constraint) => self.diagnostics.push(Error {
                    span: constraint.arguments_span,
                    msg: "type of function call arguments could not be resolved".to_string(),
                }),
                _ => {}
            }
        }
        let unresolved_matches: Vec<(Id, Span)> = self
            .constraints
            .iter()
            .filter_map(|constraint| match constraint {
                Constraint::Match(prepped) => Some((prepped.subject_id, prepped.span)),
                _ => None,
            })
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
    }

    /// Pretty-prints a type for diagnostics, resolving generic names
    /// with their substitution context when available.
    fn pretty_print_type(&self, type_: &Type, substitution: &SubstitutionContext) -> String {
        let mut visiting = Vec::new();
        self.pretty_print_type_at(type_, substitution, 0, &mut visiting)
    }

    /// Render a type at a given recursion depth. `visiting` holds the function
    /// ids currently being expanded, so a self-referential signature stops at the
    /// cycle instead of recursing forever — e.g. a function whose parameter type
    /// shares its name and resolves back to the function. `depth` is a coarse
    /// backstop for any other cycle.
    fn pretty_print_type_at(
        &self,
        type_: &Type,
        substitution: &SubstitutionContext,
        depth: usize,
        visiting: &mut Vec<Id>,
    ) -> String {
        let mut buf = String::new();
        self.pretty_print_type_inner(type_, substitution, &mut buf, depth, visiting);
        buf
    }

    /// Appends `<A, B>` to `buf` for a nominal type's arguments (nothing when
    /// there are none), so `Option<i32>` reads as `enum Option<i32>`.
    fn push_type_arguments(
        &self,
        buf: &mut String,
        arguments: &[TypeId],
        substitution: &SubstitutionContext,
        depth: usize,
        visiting: &mut Vec<Id>,
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
            self.pretty_print_type_inner(&argument_type, substitution, buf, depth + 1, visiting);
        }
        buf.push('>');
    }

    /// The generics a function signature exposes, in display order: the
    /// function's own parameters first (positional), then any inherited from an
    /// enclosing impl that appear in the signature (paired with `true`, shown
    /// `?`). A generic the `substitution` has bound to a concrete type is omitted
    /// — it renders concretely, not as a parameter.
    fn signature_generics(
        &self,
        own: &[TypeId],
        parameter_types: &[Type],
        return_type: Option<&Type>,
        substitution: &SubstitutionContext,
    ) -> Vec<(TypeId, bool)> {
        let mut found = Vec::new();
        for parameter_type in parameter_types {
            self.collect_generics(parameter_type, 0, &mut found);
        }
        if let Some(return_type) = return_type {
            self.collect_generics(return_type, 0, &mut found);
        }
        let mut generics: Vec<(TypeId, bool)> = Vec::new();
        let mut seen: HashSet<TypeId> = HashSet::new();
        for constraint_id in own {
            if !substitution.contains_key(constraint_id) && seen.insert(*constraint_id) {
                generics.push((*constraint_id, false));
            }
        }
        for constraint_id in found {
            if !own.contains(&constraint_id)
                && !substitution.contains_key(&constraint_id)
                && seen.insert(constraint_id)
            {
                generics.push((constraint_id, true));
            }
        }
        generics
    }

    /// Collects the generic constraint ids appearing in `type_` — through nominal
    /// arguments, closures, and tuples — in first-seen order.
    fn collect_generics(&self, type_: &Type, depth: usize, out: &mut Vec<TypeId>) {
        if depth > 24 {
            return;
        }
        match type_ {
            Type::Generic(constraint_id) => {
                if !out.contains(constraint_id) {
                    out.push(*constraint_id);
                }
            }
            Type::Struct(_, arguments) | Type::Enum(_, arguments) => {
                for argument in arguments {
                    self.collect_generics(&argument.get_type(self), depth + 1, out);
                }
            }
            Type::Closure(parameters, return_id) => {
                for parameter in parameters {
                    self.collect_generics(&parameter.get_type(self), depth + 1, out);
                }
                self.collect_generics(&return_id.get_type(self), depth + 1, out);
            }
            Type::Tuple(items) => {
                for item in items {
                    self.collect_generics(&item.get_type(self), depth + 1, out);
                }
            }
            _ => {}
        }
    }

    fn pretty_print_type_inner(
        &self,
        type_: &Type,
        substitution: &SubstitutionContext,
        buf: &mut String,
        depth: usize,
        visiting: &mut Vec<Id>,
    ) {
        const MAX_DEPTH: usize = 24;
        if depth > MAX_DEPTH {
            buf.push('…');
            return;
        }
        match type_ {
            // Type names render bare (Rust/TypeScript style) — `i32`, not `type
            // i32`; `Option<i32>`, not `enum Option<type i32>`. A diagnostic that
            // needs the word "type" adds it in its own message.
            Type::Any => buf.push_str("any"),
            Type::Unknown => buf.push_str("unknown"),
            Type::Unresolved => buf.push_str("unresolved"),
            Type::Void => buf.push_str("void"),

            Type::Generic(constraint_id) => {
                // A generic resolved to a concrete type (in a monomorphization)
                // renders as that type; otherwise as its name (`T`). Its bound is
                // shown only where it's declared — the signature's `<…>` list.
                if let Some(concrete_id) = substitution.get(constraint_id) {
                    let concrete = concrete_id.get_type(self);
                    self.pretty_print_type_inner(&concrete, substitution, buf, depth + 1, visiting);
                } else {
                    let generic_name = self
                        .generic_constraint_names
                        .get(constraint_id)
                        .copied()
                        .unwrap_or("?");
                    buf.push_str(generic_name);
                }
            }

            Type::Function(id) => {
                // The id may name a regular or an `external` function; both render
                // as `fn name<generics>(paramType, ..): return`.
                let signature = self
                    .functions
                    .get(id)
                    .map(|func| {
                        (
                            func.name,
                            &func.parameters,
                            &func.generic_parameter_constraint_ids,
                            func.return_type_id,
                        )
                    })
                    .or_else(|| {
                        self.external_functions.get(id).map(|external| {
                            (
                                external.name,
                                &external.parameters,
                                &external.generic_parameter_constraint_ids,
                                Some(external.return_type_id),
                            )
                        })
                    });
                let Some((name, parameter_ids, own_generics, return_type_id)) = signature else {
                    buf.push_str("fn");
                    return;
                };
                // A function whose own signature refers back to it would recurse
                // forever — e.g. a parameter type that shares the function's name
                // and resolves back to it. If it's already on the path, render the
                // name alone.
                if visiting.contains(id) {
                    buf.push_str(&format!("fn {}", name));
                    return;
                }
                visiting.push(*id);
                buf.push_str(&format!("fn {}", name));

                // `<U, T?>` — the generics the signature uses: the function's own
                // parameters first, then any inherited from an enclosing impl
                // (marked `?`, addressable by name). Each carries its bound when it
                // isn't the open `any`. Generics bound to a concrete type by the
                // `substitution` are omitted (they render concretely below).
                let parameter_types: Vec<Type> = parameter_ids
                    .iter()
                    .filter_map(|parameter_id| self.parameters.get(parameter_id))
                    .map(|parameter| parameter.type_id.get_type(self))
                    .collect();
                let return_type = return_type_id.map(|type_id| type_id.get_type(self));
                let generics = self.signature_generics(
                    own_generics,
                    &parameter_types,
                    return_type.as_ref(),
                    substitution,
                );
                if !generics.is_empty() {
                    buf.push('<');
                    for (index, (constraint_id, inherited)) in generics.iter().enumerate() {
                        if index > 0 {
                            buf.push_str(", ");
                        }
                        let generic_name = self
                            .generic_constraint_names
                            .get(constraint_id)
                            .copied()
                            .unwrap_or("?");
                        buf.push_str(generic_name);
                        if *inherited {
                            buf.push('?');
                        }
                        let bound = constraint_id.get_type(self);
                        if !matches!(bound, Type::Any) {
                            buf.push_str(" of ");
                            self.pretty_print_type_inner(
                                &bound,
                                substitution,
                                buf,
                                depth + 1,
                                visiting,
                            );
                        }
                    }
                    buf.push('>');
                }

                buf.push('(');
                for (index, parameter_type) in parameter_types.iter().enumerate() {
                    if index > 0 {
                        buf.push_str(", ");
                    }
                    self.pretty_print_type_inner(
                        parameter_type,
                        substitution,
                        buf,
                        depth + 1,
                        visiting,
                    );
                }
                buf.push(')');

                // The return type, unless it's `void` (as Rust omits `-> ()`).
                if let Some(return_type) = return_type
                    && !matches!(return_type, Type::Void)
                {
                    buf.push_str(": ");
                    self.pretty_print_type_inner(
                        &return_type,
                        substitution,
                        buf,
                        depth + 1,
                        visiting,
                    );
                }
                visiting.pop();
            }

            Type::Struct(id, arguments) => {
                let Some(struct_) = self.structs.get(id) else {
                    buf.push('?');
                    return;
                };
                buf.push_str(struct_.name);
                self.push_type_arguments(buf, arguments, substitution, depth, visiting);
            }

            Type::Trait(id, trait_arguments) => {
                if let Some(trait_) = self.traits.get(id) {
                    buf.push_str(trait_.name);
                    self.push_type_arguments(buf, trait_arguments, substitution, depth, visiting);
                } else {
                    buf.push('?');
                }
            }

            Type::Enum(id, arguments) => {
                let Some(enum_) = self.enums.get(id) else {
                    buf.push('?');
                    return;
                };
                buf.push_str(enum_.name);
                self.push_type_arguments(buf, arguments, substitution, depth, visiting);
            }

            Type::Module(id) => {
                if let Some(module) = self.modules.get(id) {
                    buf.push_str(&format!("module {}", module.name));
                } else {
                    buf.push_str("module");
                }
            }

            Type::Closure(parameters, return_id) => {
                buf.push_str("|");
                for (i, parameter_id) in parameters.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    let parameter_type = parameter_id.get_type(self);
                    buf.push_str(&self.pretty_print_type_at(
                        &parameter_type,
                        substitution,
                        depth + 1,
                        visiting,
                    ));
                }
                buf.push_str("| ");
                let return_type = return_id.get_type(self);
                buf.push_str(&self.pretty_print_type_at(
                    &return_type,
                    substitution,
                    depth + 1,
                    visiting,
                ));
            }

            Type::Tuple(items) => {
                buf.push('(');
                for (i, item_id) in items.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    let item_type = item_id.get_type(self);
                    let item_str =
                        self.pretty_print_type_at(&item_type, substitution, depth + 1, visiting);
                    buf.push_str(&item_str);
                }
                buf.push(')');
            }
            Type::Mapped(binder_id, source_id, template_id) => {
                let binder = self
                    .generic_constraint_names
                    .get(binder_id)
                    .copied()
                    .unwrap_or("?");
                let source = source_id.get_type(self);
                let template = template_id.get_type(self);
                let source_str =
                    self.pretty_print_type_at(&source, substitution, depth + 1, visiting);
                let template_str =
                    self.pretty_print_type_at(&template, substitution, depth + 1, visiting);
                buf.push_str(&format!("({binder} in {source_str}: {template_str})"));
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

/// A call or accessor that dispatches generically: the analyzer can't pin the
/// concrete callee, so it records how codegen should re-resolve it at each
/// monomorphization. Unifies the static-accessor, generic-method, and
/// trait-method dispatch records into one channel (`generic_dispatch`).
#[derive(Debug, Clone, Copy)]
pub enum GenericDispatch<'src> {
    /// `T::member()`, `value.method()`, or `a op b` on a generic-bounded operand:
    /// resolve the constraint through the active substitution, then dispatch to
    /// the concrete type's member.
    OnConstraint(TypeId, &'src str),
    /// A trait method re-dispatched to a concrete type — an inherited default
    /// called on a concrete value (`Some(type)`), or a `self`/trait-typed call
    /// inside a default body (`None`, dispatched on the type being specialized).
    OnType(Option<TypeId>, &'src str),
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
    // `str.to_uppercase()` -> native `.toUpperCase()`.
    StrToUppercase,
    // `str.len(): i32` -> native `.length` (property read).
    StrLen,
    // `str.contains(needle): bool` -> native `.includes(..)`.
    StrContains,
    // `str.starts_with(prefix): bool` -> native `.startsWith(..)`.
    StrStartsWith,
    // `str.ends_with(suffix): bool` -> native `.endsWith(..)`.
    StrEndsWith,
    // `str.replace(from, to): str` -> native `.replaceAll(..)`.
    StrReplace,
    // `str.repeat(count): str` -> native `.repeat(..)`.
    StrRepeat,
    // `str.split(sep): List<str>` -> native `.split(..)` (returns a JS array).
    StrSplit,
    // `str.substring(start, end): str` -> native `.substring(..)`.
    StrSubstring,
    // `str.parse_i32(): Option<i32>` -> a runtime helper returning the enum form.
    ParseI32,
    // `str.parse_f64(): Option<f64>` -> a runtime helper returning the enum form.
    ParseF64,
    // `random::range_i32`/`range_u32` -> an integer range helper over `Math.random`.
    RandomInt,
    // `random::range_f64` -> a float range helper over `Math.random`.
    RandomFloat,
    // `process::args(): List<str>` -> the script's arguments (`process.argv` tail).
    Args,
    // `process::env(key): Option<str>` -> a runtime helper returning the enum form.
    Env,
    // `List.len(): i32` -> native `.length` (property read).
    ListLen,
    // `List.get(i): Option<T>` -> a bounds-checked runtime helper (Option form).
    ListGet,
    // `List.pop(): Option<T>` -> a runtime helper that removes the last element.
    ListPop,
    // `Shared::new(value)` -> a `{ v: value }` cell (a JS object, so `__clone`
    // shares it by reference instead of deep-copying — that is what makes a
    // `Shared` co-owned rather than snapshotted).
    SharedNew,
    // `Shared.clone()` -> the same cell (identity): just the receiver.
    SharedClone,
    // `Shared.read()`/`write()` -> the cell's value, `self.v`.
    SharedValue,
    // `Set::new(): Set<T>` -> `new Set()`.
    SetNew,
    // `Set.insert(value)` -> native `.add(value)`.
    SetInsert,
    // `Set.contains(value): bool` -> native `.has(value)`.
    SetContains,
    // `Set.remove(value)` -> native `.delete(value)`.
    SetRemove,
    // `Set.len(): i32` -> native `.size` (property read).
    SetLen,
    // `Map::new(): Map<K, V>` -> `new Map()`.
    MapNew,
    // `Map.insert(key, value)` -> native `.set(key, value)`.
    MapInsert,
    // `Map.get(key): Option<V>` -> a runtime helper returning the Option form.
    MapGet,
    // `Map.contains_key(key): bool` -> native `.has(key)`.
    MapContainsKey,
    // `Map.remove(key)` -> native `.delete(key)`.
    MapRemove,
    // `Map.len(): i32` -> native `.size` (property read).
    MapLen,
    // `Map.keys(): List<K>` -> a runtime helper snapshotting the keys to an array.
    MapKeys,
    // `Map.values(): List<V>` -> a runtime helper snapshotting the values.
    MapValues,
    // `JsonValue.field(name): JsonValue` -> native `self[name]` (a dynamic
    // property read on a `JSON.parse` result, for which there is no host fn).
    JsonField,
    // `JsonValue.tag(): str` -> the externally-tagged enum discriminator via a
    // runtime helper (`typeof`/`Object.keys`).
    JsonTag,
    // `JsonValue.elements(): List<JsonValue>` -> the receiver itself: a parsed
    // JSON array already IS a JS array, so this is an identity (typed) projection.
    JsonElements,
    // `JsonValue.is_null(): bool` -> `self === null` (JSON `null` parses to JS
    // `null`); the `Option::None` discriminator.
    JsonIsNull,
    // `dom::query_selector_all(selector): List<Element>` -> the matches as a real
    // array, `Array.from(document.querySelectorAll(selector))` (querySelectorAll
    // yields a NodeList, which a `List` would otherwise mishandle).
    QuerySelectorAll,
}

/// Identifies a source file within a compiled `Program` — an index into
/// `Program.sources`. `SourceId(0)` is always the entry file; the rest are
/// `std` package modules pulled in during analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u32);

/// The half-open entity-id range `[start, end)` produced while walking one
/// source file. Since entity ids are minted monotonically and each file is
/// walked by a single top-level pass, these ranges map an entity back to the
/// file it came from (see `Program::source_of`).
#[derive(Debug, Clone, Copy)]
pub struct SourceRange {
    pub start: u32,
    pub end: u32,
    pub source: SourceId,
}

#[derive(Debug)]
pub struct Program<'src> {
    /// The host the emitted JS targets (Node / browser) — gates the platform std
    /// layer at load time and host-binding emission in the transformer.
    pub target: Target,
    pub closures: IndexMap<Id, Closure>,
    pub diagnostics: Vec<Error>,
    pub enums: IndexMap<Id, Enum<'src>>,
    pub entity_map: HashMap<Id, Expr<'src>>,
    pub entity_scope_map: HashMap<Id, Id>,
    pub function_calls: IndexMap<Id, FunctionCall>,
    pub functions: IndexMap<Id, Function<'src>>,
    pub external_functions: IndexMap<Id, ExternalFunction<'src>>,
    pub traits: IndexMap<Id, Trait<'src>>,
    pub generic_dispatch: HashMap<Id, GenericDispatch<'src>>,
    pub for_each_next: HashMap<Id, Id>,
    // `for e in &mut list` loop bindings → whether the element view is `&mut`.
    pub for_each_views: HashMap<Id, bool>,
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
    // helper (`str.trim()`, `scan()`, `random::range_i32(..)`, ...), keyed by fn id.
    // (The per-type `range_*` are forwarded to by the `Random` trait impls.)
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
    pub parameters: IndexMap<Id, Parameter<'src>>,
    // The source files that make up this program: `sources[0]` is the entry
    // file, the rest are `std` modules. `source_ranges` maps each entity id to
    // its file (see `source_of`); both drive cross-file navigation in the LSP.
    pub sources: Vec<PathBuf>,
    pub source_ranges: Vec<SourceRange>,
    // Use-site identifier spans for field accesses / method calls (`.x`), keyed
    // by the access expr id — drives rename and go-to-definition on members.
    pub member_name_spans: HashMap<Id, Span>,
    // Maps a struct initializer expr id to the struct definition it constructs,
    // so go-to-definition on `Point { .. }` reaches the `struct` declaration.
    pub struct_initializer_to_def: HashMap<Id, Id>,
    // Named type references in type position: `(file, name span, definition id,
    // label)`. Type names aren't entities, so this drives go-to-definition and
    // hover on them (e.g. `Option`, `i32`, a trait bound).
    pub type_references: Vec<(SourceId, Span, Option<Id>, String)>,
    // A human-readable type label for every typed expression (e.g. `struct
    // Point`, `type i32`, `enum Option<i32>`), pre-rendered during analysis for
    // language-server hover. Keyed by expr id; `expr_id_to_type_id_map` wins
    // over `resolved_types` where both apply (matching `type_of_expr`).
    pub expr_types: HashMap<Id, String>,
    // The resolved type id of every typed expression and binding (same merge as
    // `expr_types`). The transformer reads this to compute a tuple's flat-storage
    // layout — which elements are themselves tuples (and so are spread, not nested)
    // — resolving any generic element through the active monomorphization.
    pub expr_type_ids: HashMap<Id, TypeId>,
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
    // Slice 4: scalar locals boxed into a `[value]` cell because a view is taken
    // of them; their reads/writes lower through `[0]`.
    pub boxed_locals: HashSet<Id>,
    // View bindings/params holding a scalar `(base, key)` view; `*v` lowers to
    // `v[0][v[1]]` (covers both a boxed local and a scalar field).
    pub primitive_views: HashSet<Id>,
    // `&place`/`&mut place` exprs whose target is scalar, so the view lowers to a
    // `[base, key]` pair rather than the aggregate's own reference.
    pub scalar_view_refs: HashSet<Id>,
    // Call exprs resolving to a `borrows` function that returns a scalar view, so
    // `*call` derefs through `call[0][call[1]]`.
    pub scalar_view_calls: HashSet<Id>,
}

impl<'src> Program<'src> {
    /// The source file an entity originated from, by locating the walk range
    /// that produced its id. `None` for synthetic entities minted outside any
    /// file walk (e.g. during post-analysis passes).
    pub fn source_of(&self, id: Id) -> Option<SourceId> {
        self.source_ranges
            .iter()
            .find(|range| id.0 >= range.start && id.0 < range.end)
            .map(|range| range.source)
    }

    /// The filesystem path of a source file.
    pub fn source_path(&self, source: SourceId) -> Option<&Path> {
        self.sources.get(source.0 as usize).map(PathBuf::as_path)
    }
}

/// Lexes and parses a Vilan source file into an AST, leaking the source and the
/// resulting tree so they live for the whole compilation. Used to pull the
/// `std` package's modules in from source. Returns `None` if the file can't be
/// read or fails to lex/parse.
fn load_package_module(path: &str) -> Option<&'static Spanned<NodeList<'static>>> {
    use chumsky::prelude::*;
    use std::collections::HashMap;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::{Mutex, OnceLock};

    // A process-global, content-addressed parse cache: a hash of the file's
    // contents maps to its leaked AST. Every analysis re-loads the `std` modules
    // (and the language server re-analyzes on each keystroke), but their source
    // rarely changes — so reuse the parse, and leak the source + tree only once
    // per distinct content instead of once per analysis.
    static CACHE: OnceLock<Mutex<HashMap<u64, &'static crate::span::Spanned<NodeList<'static>>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let source = std::fs::read_to_string(path).ok()?;
    let key = {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(ast) = cache.lock().unwrap().get(&key) {
        return Some(*ast);
    }

    // Cache miss: lex and parse. The source is leaked so the parsed tree (which
    // borrows it) can live for the whole compilation. The token vector is
    // transient — the AST holds `&'static str` slices into the source.
    let source: &'static str = Box::leak(source.into_boxed_str());
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
    let ast = Box::leak(Box::new(root));
    cache.lock().unwrap().insert(key, ast);
    Some(ast)
}

/// The synthesized trait-impl source for one `@derive(..)` item. Only structs
/// with a field body are handled; each supported trait name emits its impl,
/// built from the struct's field names. Unknown trait names are skipped (the
/// missing-impl error surfaces naturally at the use site).
/// Renders a (field) type node back to source for use in generated code — a
/// name (`i32`/`Point`) or a generic application (`List<i32>`). Other forms fall
/// back to `_`, which surfaces a clear error at the generated use site.
fn render_type(node: &Node<'_>) -> String {
    match node {
        Node::Accessor(name) => name.to_string(),
        Node::AccessorWithGenerics(name, arguments) => {
            let arguments = arguments
                .0
                .iter()
                .map(|argument| render_type(&argument.0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name}<{arguments}>")
        }
        _ => "_".to_string(),
    }
}

/// The synthesized trait impls for a `@derive(..)` enum. Each derive is built
/// from the variants (their names and payload arities) via a `match`. `Default`
/// is skipped — an enum has no unambiguous default variant. (Generic enums are
/// not yet handled; the missing impl surfaces naturally at the use site.)
fn derive_enum_impls(
    derives: &[&str],
    enum_name: &str,
    variants: &Spanned<Vec<Spanned<crate::node::EnumVariant<'_>>>>,
) -> String {
    // (variant name, payload type names — its arity is the length).
    let variants: Vec<(&str, Vec<String>)> = variants
        .0
        .iter()
        .map(|variant| {
            let payload_types = variant
                .0
                .1
                .iter()
                .map(|payload| render_type(&payload.0))
                .collect();
            (variant.0.0, payload_types)
        })
        .collect();
    let mut out = String::new();
    for derive in derives {
        match *derive {
            "PartialEq" => {
                // `match (self, other) { (E::V(let s0,..), E::V(let o0,..)) => s0
                // == o0 && .., (E::W, E::W) => true, _ => false }`.
                let mut arms = String::new();
                for (name, payload_types) in &variants {
                    let arity = payload_types.len();
                    if arity == 0 {
                        arms.push_str(&format!(
                            "\t\t\t({enum_name}::{name}, {enum_name}::{name}) => true,\n"
                        ));
                    } else {
                        let bind = |prefix: char| {
                            (0..arity)
                                .map(|i| format!("let {prefix}{i}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let comparison = (0..arity)
                            .map(|i| format!("s{i} == o{i}"))
                            .collect::<Vec<_>>()
                            .join(" && ");
                        arms.push_str(&format!(
                            "\t\t\t({enum_name}::{name}({}), {enum_name}::{name}({})) => {comparison},\n",
                            bind('s'),
                            bind('o'),
                        ));
                    }
                }
                arms.push_str("\t\t\t_ => false,\n");
                out.push_str(&format!(
                    "impl {enum_name} with PartialEq {{\n\
                     \tfun eq(self, other: {enum_name}): bool {{\n\
                     \t\tmatch (self, other) {{\n{arms}\t\t}}\n\
                     \t}}\n\
                     }}\n"
                ));
            }
            "Debug" => {
                // `match self { E::V(let p0,..) => "V(" + p0.debug() + ", " + .. +
                // ")", E::W => "W" }`.
                let mut arms = String::new();
                for (name, payload_types) in &variants {
                    let arity = payload_types.len();
                    if arity == 0 {
                        arms.push_str(&format!("\t\t\t{enum_name}::{name} => \"{name}\",\n"));
                    } else {
                        let binds = (0..arity)
                            .map(|i| format!("let p{i}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let parts = (0..arity)
                            .map(|i| format!("p{i}.debug()"))
                            .collect::<Vec<_>>()
                            .join(" + \", \" + ");
                        arms.push_str(&format!(
                            "\t\t\t{enum_name}::{name}({binds}) => \"{name}(\" + {parts} + \")\",\n"
                        ));
                    }
                }
                out.push_str(&format!(
                    "impl {enum_name} with Debug {{\n\
                     \tfun debug(self): str {{\n\
                     \t\tmatch self {{\n{arms}\t\t}}\n\
                     \t}}\n\
                     }}\n"
                ));
            }
            "Json" => {
                // Externally tagged: no payload -> `"V"`; one -> `{"V":<p>}`;
                // many -> `{"V":[<p0>,<p1>]}`.
                let mut arms = String::new();
                for (name, payload_types) in &variants {
                    let arity = payload_types.len();
                    if arity == 0 {
                        arms.push_str(&format!(
                            "\t\t\t{enum_name}::{name} => \"\\\"{name}\\\"\",\n"
                        ));
                    } else if arity == 1 {
                        arms.push_str(&format!(
                            "\t\t\t{enum_name}::{name}(let p0) => \"{{\\\"{name}\\\":\" + p0.to_json() + \"}}\",\n"
                        ));
                    } else {
                        let binds = (0..arity)
                            .map(|i| format!("let p{i}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let parts = (0..arity)
                            .map(|i| format!("p{i}.to_json()"))
                            .collect::<Vec<_>>()
                            .join(" + \",\" + ");
                        arms.push_str(&format!(
                            "\t\t\t{enum_name}::{name}({binds}) => \"{{\\\"{name}\\\":[\" + {parts} + \"]}}\",\n"
                        ));
                    }
                }
                out.push_str(&format!(
                    "impl {enum_name} with Json {{\n\
                     \tfun to_json(self): str {{\n\
                     \t\tmatch self {{\n{arms}\t\t}}\n\
                     \t}}\n\
                     }}\n"
                ));
                // The reverse direction: read the externally-tagged discriminator,
                // then rebuild that variant from the host value. A no-payload tag is
                // the bare string; a single payload is `value.field(tag)`; several
                // are positional elements of the tagged array. Each payload is
                // coerced via its own type's `from_json_value`.
                let mut arms = String::new();
                for (name, payload_types) in &variants {
                    let arity = payload_types.len();
                    if arity == 0 {
                        arms.push_str(&format!("\t\t\t\"{name}\" => {enum_name}::{name},\n"));
                    } else if arity == 1 {
                        let payload_type = &payload_types[0];
                        arms.push_str(&format!(
                            "\t\t\t\"{name}\" => {enum_name}::{name}({payload_type}::from_json_value(value.field(\"{name}\"))),\n"
                        ));
                    } else {
                        let elements = payload_types
                            .iter()
                            .enumerate()
                            .map(|(index, payload_type)| {
                                format!(
                                    "{payload_type}::from_json_value(value.field(\"{name}\").field(\"{index}\"))"
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        arms.push_str(&format!(
                            "\t\t\t\"{name}\" => {enum_name}::{name}({elements}),\n"
                        ));
                    }
                }
                arms.push_str(&format!(
                    "\t\t\t_ => panic(\"unknown variant in JSON for enum {enum_name}\"),\n"
                ));
                out.push_str(&format!(
                    "impl {enum_name} with FromJson {{\n\
                     \tfun from_json(text: str): {enum_name} {{\n\
                     \t\t{enum_name}::from_json_value(parse_json_value(text))\n\
                     \t}}\n\
                     \tfun from_json_value(value: JsonValue): {enum_name} {{\n\
                     \t\tmatch value.tag() {{\n{arms}\t\t}}\n\
                     \t}}\n\
                     }}\n"
                ));
            }
            _ => {}
        }
    }
    out
}

fn derive_impl_source(derives: &[&str], item: &Spanned<Node<'_>>) -> String {
    if let Node::Enum(name, _generics, variants) = &item.0 {
        return derive_enum_impls(derives, name.0, variants);
    }
    let Node::Struct(name, _generics, _external, Some(fields)) = &item.0 else {
        return String::new();
    };
    let struct_name = name.0;
    let fields: Vec<(&str, String)> = fields
        .0
        .iter()
        .map(|field| {
            let field_name = field.0.0;
            let field_type = field
                .0
                .1
                .as_ref()
                .map(|type_| render_type(&type_.0))
                .unwrap_or_else(|| "_".to_string());
            (field_name, field_type)
        })
        .collect();
    let mut out = String::new();
    for derive in derives {
        match *derive {
            "PartialEq" => {
                // `self.a == other.a && self.b == other.b` (a field-less struct is
                // always equal).
                let comparison = if fields.is_empty() {
                    "true".to_string()
                } else {
                    fields
                        .iter()
                        .map(|(field, _)| format!("self.{field} == other.{field}"))
                        .collect::<Vec<_>>()
                        .join(" && ")
                };
                out.push_str(&format!(
                    "impl {struct_name} with PartialEq {{\n\
                     \tfun eq(self, other: {struct_name}): bool {{\n\
                     \t\t{comparison}\n\
                     \t}}\n\
                     }}\n"
                ));
            }
            "Default" => {
                // `{ a = i32::default(), b = Point::default() }`.
                let initializers = fields
                    .iter()
                    .map(|(field, type_)| format!("{field} = {type_}::default()"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "impl {struct_name} with Default {{\n\
                     \tfun default(): {struct_name} {{\n\
                     \t\t{struct_name} {{ {initializers} }}\n\
                     \t}}\n\
                     }}\n"
                ));
            }
            "Debug" => {
                // `"T { " + "a = " + self.a.debug() + ", " + … + " }"`; a
                // field-less struct is just its name.
                let body = if fields.is_empty() {
                    format!("\"{struct_name}\"")
                } else {
                    let parts = fields
                        .iter()
                        .map(|(field, _)| format!("\"{field} = \" + self.{field}.debug()"))
                        .collect::<Vec<_>>()
                        .join(" + \", \" + ");
                    format!("\"{struct_name} {{ \" + {parts} + \" }}\"")
                };
                out.push_str(&format!(
                    "impl {struct_name} with Debug {{\n\
                     \tfun debug(self): str {{\n\
                     \t\t{body}\n\
                     \t}}\n\
                     }}\n"
                ));
            }
            "Json" => {
                // `"{" + "\"a\":" + self.a.to_json() + "," + "\"b\":" +
                // self.b.to_json() + "}"` — a JSON object with the real field
                // names; each value serializes via its own `to_json`.
                let mut body = String::from("\"{\"");
                for (index, (field, _)) in fields.iter().enumerate() {
                    if index > 0 {
                        body.push_str(" + \",\"");
                    }
                    body.push_str(" + \"\\\"");
                    body.push_str(field);
                    body.push_str("\\\":\" + self.");
                    body.push_str(field);
                    body.push_str(".to_json()");
                }
                body.push_str(" + \"}\"");
                out.push_str(&format!(
                    "impl {struct_name} with Json {{\n\
                     \tfun to_json(self): str {{\n\
                     \t\t{body}\n\
                     \t}}\n\
                     }}\n"
                ));
                // The reverse direction: parse a document into the struct, reading
                // each field by name from the host value and coercing it back via
                // the field type's own `from_json_value` (nested structs recurse).
                let initializers = fields
                    .iter()
                    .map(|(field, type_)| {
                        format!("{field} = {type_}::from_json_value(value.field(\"{field}\"))")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "impl {struct_name} with FromJson {{\n\
                     \tfun from_json(text: str): {struct_name} {{\n\
                     \t\t{struct_name}::from_json_value(parse_json_value(text))\n\
                     \t}}\n\
                     \tfun from_json_value(value: JsonValue): {struct_name} {{\n\
                     \t\t{struct_name} {{ {initializers} }}\n\
                     \t}}\n\
                     }}\n"
                ));
            }
            _ => {}
        }
    }
    out
}

/// Synthesizes and parses the trait impls for every `@derive(..)` item at the top
/// level of `nodes`, returning the appended node list (leaked so it lives for the
/// whole compilation, like a loaded module), or `None` when there are no derives.
fn expand_derives(nodes: &NodeList<'_>) -> Option<&'static NodeList<'static>> {
    use chumsky::prelude::*;
    let mut source = String::new();
    let mut traits: HashSet<&str> = HashSet::new();
    // `@derive(Json)` synthesizes the reverse `FromJson` impl too, which
    // references `FromJson`/`JsonValue`/`parse_json_value`; the enum form also
    // calls `panic` on an unknown tag.
    let mut enum_derives_json = false;
    for (node, _span) in nodes {
        if let Node::Derive(derives, item) = node {
            traits.extend(derives.iter().copied());
            if derives.contains(&"Json") && matches!(item.0, Node::Enum(..)) {
                enum_derives_json = true;
            }
            source.push_str(&derive_impl_source(derives, item));
        }
    }
    if source.trim().is_empty() {
        return None;
    }
    // Each derived trait lives in a std module; the synthesized impls are walked
    // in the entry scope, so prepend the imports they reference.
    let mut prelude = String::new();
    if traits.contains("PartialEq") {
        prelude.push_str("import std::compare::PartialEq;\n");
    }
    if traits.contains("Default") {
        prelude.push_str("import std::default::Default;\n");
    }
    if traits.contains("Json") {
        prelude.push_str("import std::json::{ Json, FromJson, JsonValue, parse_json_value };\n");
    }
    if enum_derives_json {
        prelude.push_str("import std::io::panic;\n");
    }
    if traits.contains("Debug") {
        prelude.push_str("import std::debug::Debug;\n");
    }
    let source: &'static str = Box::leak(format!("{prelude}{source}").into_boxed_str());
    let tokens = crate::lexer::lexer().parse(source).into_output()?;
    let end = source.len();
    let (root, _span) = crate::parser::parser()
        .map_with(|ast, e| (ast, e.span()))
        .parse(
            tokens
                .as_slice()
                .map((end..end).into(), |(token, span)| (token, span)),
        )
        .into_output()?;
    Some(Box::leak(Box::new(root.0)))
}

/// Builds the path to a module file under the `std` package's source root.
fn std_module_path(std_root: &Path, file: &str) -> String {
    std_root.join(file).to_string_lossy().into_owned()
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
            for (path, leaf, _span) in entries {
                if path.first().map(|(name, _)| *name) != Some(root) {
                    continue;
                }
                // `import std::option::..` -> the module is the segment after the
                // root; a bare `import std::random` -> the module is the leaf.
                if path.len() >= 2 {
                    modules.push(path[1].0);
                } else {
                    modules.push(leaf);
                }
            }
        }
    }
    modules
}

pub fn analyze<'src>(
    nodes: &'src Spanned<NodeList<'src>>,
    std_root: &Path,
    entry_path: &Path,
    target: Target,
) -> Program<'src> {
    // `sources[0]` is the entry file; std modules are appended as they load.
    // `source_ranges` records the entity-id span each file's walk produced.
    let mut sources: Vec<PathBuf> = vec![entry_path.to_path_buf()];
    let mut source_ranges: Vec<SourceRange> = Vec::new();
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
    let lib_path = std_module_path(std_root, "lib.vl");
    let lib_ast = load_package_module(&lib_path);
    sources.push(PathBuf::from(&lib_path));
    let lib_source_id = SourceId((sources.len() - 1) as u32);
    let mut module_scopes: HashMap<&str, Id> = HashMap::new();
    let mut loaded: Vec<(&str, &Spanned<NodeList>, Id, SourceId)> = Vec::new();
    // A module's package: `Std` modules resolve under `std_root` and are
    // addressable as both `std::name` and `pkg::name`; `Pkg` modules — the entry
    // program's own multi-file siblings — resolve under `pkg_root` (the entry's
    // directory) and are addressable only as `pkg::name`.
    #[derive(Clone, Copy, PartialEq)]
    enum Origin {
        Std,
        Pkg,
    }
    // The entry program's package root: the directory its `import pkg::..` siblings
    // live in. When it equals `std_root` we're compiling std itself (or a std file
    // opened in an editor), so every module is `Std` and the original single-root
    // behavior is preserved exactly.
    let pkg_root = entry_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let compiling_std = match (
        std::fs::canonicalize(pkg_root),
        std::fs::canonicalize(std_root),
    ) {
        (Ok(pkg_canonical), Ok(std_canonical)) => pkg_canonical == std_canonical,
        _ => pkg_root == std_root,
    };
    let entry_pkg_origin = if compiling_std {
        Origin::Std
    } else {
        Origin::Pkg
    };

    // Synthesize `@derive(..)` impls up front: they reference std modules (e.g.
    // `PartialEq` in `std::compare`) that must be pulled into the reachable set
    // alongside the user's own imports, and they're walked into the entry scope
    // later. Computed once and reused.
    let derived = expand_derives(&nodes.0);

    let mut to_load: Vec<(Origin, &str)> = lib_ast
        .map(|ast| collect_module_refs(&ast.0, "pkg"))
        .unwrap_or_default()
        .into_iter()
        .map(|name| (Origin::Std, name))
        .collect();
    if let Some(derived) = derived {
        to_load.extend(
            collect_module_refs(derived, "std")
                .into_iter()
                .map(|name| (Origin::Std, name)),
        );
    }
    // The entry program addresses std submodules by path (`std::option::..`), so
    // its imports also seed the reachable set. Names that aren't modules (e.g. the
    // `print` in `std::print`) simply find no file and are skipped.
    to_load.extend(
        collect_module_refs(&nodes.0, "std")
            .into_iter()
            .map(|name| (Origin::Std, name)),
    );
    // The entry's `pkg::sibling` references pull in its own package's modules from
    // `pkg_root`. When the entry is itself a std file (`compiling_std`) these are
    // std siblings, so the origin stays `Std` and the load is unchanged.
    to_load.extend(
        collect_module_refs(&nodes.0, "pkg")
            .into_iter()
            .map(|name| (entry_pkg_origin, name)),
    );
    // `bool`, `List`, and `null` are core primitives, so their (dependency-free)
    // modules are always loaded even when not imported.
    for core in ["boolean", "list", "null", "promise"] {
        to_load.push((Origin::Std, core));
    }
    // Set when the entry file is itself a std module (a std file opened directly
    // in an editor): it is then analyzed *as* that module from the editor's
    // buffer rather than loaded a second time from disk, which would create
    // duplicate, non-unifiable types. The separate entry walk is skipped below.
    let mut entry_is_module = false;
    while let Some((origin, name)) = to_load.pop() {
        if module_scopes.contains_key(name) {
            continue;
        }
        let root = match origin {
            Origin::Std => std_root,
            Origin::Pkg => pkg_root,
        };
        // Gate the platform std layer by target: a browser build can't load the
        // Node layer (`std::http`, `std::fs`, `std::process`), and vice versa. A
        // clear diagnostic beats the downstream "cannot find" once the symbols go
        // missing. (Pkg modules are the user's own — never platform-gated.)
        if origin == Origin::Std {
            let platform = Platform::of_std_module(name);
            if !platform.is_available_for(target) {
                analyzer.diagnostics.push(Error {
                    span: EMPTY_SPAN,
                    msg: format!(
                        "`std::{name}` is part of the {} platform layer and is not available \
                         when building for `{}`",
                        platform.name(),
                        target.name()
                    ),
                });
                continue;
            }
        }
        let module_path = std_module_path(root, &format!("{name}.vl"));
        let is_entry_module = match (
            std::fs::canonicalize(&module_path),
            std::fs::canonicalize(entry_path),
        ) {
            (Ok(module_canonical), Ok(entry_canonical)) => module_canonical == entry_canonical,
            _ => false,
        };
        // Use the entry's (buffer) AST for its own module; load the rest from
        // disk. The entry module keeps SourceId 0 so editor features resolve to
        // the open document.
        let (ast, module_source_id): (&Spanned<NodeList>, SourceId) = if is_entry_module {
            entry_is_module = true;
            (nodes, SourceId(0))
        } else {
            let Some(loaded_ast) = load_package_module(&module_path) else {
                continue;
            };
            sources.push(PathBuf::from(&module_path));
            (loaded_ast, SourceId((sources.len() - 1) as u32))
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
        // Give the module entity a location (its file, at the top) so a path
        // segment naming it can go-to-definition. A one-id range maps it to its
        // source; the span is the file start.
        analyzer.span_map.insert(module_id, &EMPTY_SPAN);
        source_ranges.push(SourceRange {
            start: module_id.0,
            end: module_id.0 + 1,
            source: module_source_id,
        });
        analyzer
            .expr_id_to_expr_map
            .insert(module_id, Expr::Module(module_id));
        analyzer
            .mut_scope_for_scope_id(pkg_scope_id)
            .name_to_id_map
            .insert(name, module_id);
        // A std module is also exposed under the `std` root so it's addressable by
        // path from outside (`std::<module>::item`), mirroring its internal
        // `pkg::<module>` reference. A user (`Pkg`) module lives only in `pkg::`.
        if origin == Origin::Std {
            analyzer
                .mut_scope_for_scope_id(std_scope_id)
                .name_to_id_map
                .insert(name, module_id);
        }
        module_scopes.insert(name, module_scope_id);
        // A module's `pkg::` siblings inherit its package; a user module's `std::`
        // imports are std. (Std modules reference each other via `pkg::` only, so
        // collecting their `std::` imports would be a no-op — skip it to keep the
        // std load byte-for-byte unchanged.)
        to_load.extend(
            collect_module_refs(&ast.0, "pkg")
                .into_iter()
                .map(|sibling| (origin, sibling)),
        );
        if origin == Origin::Pkg {
            to_load.extend(
                collect_module_refs(&ast.0, "std")
                    .into_iter()
                    .map(|module| (Origin::Std, module)),
            );
        }
        loaded.push((name, ast, module_scope_id, module_source_id));
    }
    for (_name, ast, module_scope_id, source_id) in &loaded {
        analyzer.current_source_id = *source_id;
        let start = analyzer.entity_id;
        analyzer.walk_expr_nodes(&ast.0, *module_scope_id);
        source_ranges.push(SourceRange {
            start,
            end: analyzer.entity_id,
            source: *source_id,
        });
    }
    if let Some(lib_ast) = lib_ast {
        analyzer.current_source_id = lib_source_id;
        let start = analyzer.entity_id;
        analyzer.walk_expr_nodes(&lib_ast.0, std_scope_id);
        source_ranges.push(SourceRange {
            start,
            end: analyzer.entity_id,
            source: lib_source_id,
        });
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

    // The `std::set` `Set` struct, if `set.vl` loaded. Its `new`/`insert`/... method
    // ids are captured below after `build()`. `Set` is imported explicitly (not an
    // always-loaded core module), so it isn't bound into the global scope.
    let set_struct_id = module_scopes
        .get("set")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("Set").copied());
    if let Some(set_struct_id) = set_struct_id {
        analyzer.primitive_struct_ids.insert("Set", set_struct_id);
    }

    // The `std::map` `Map` struct, if `map.vl` loaded — same treatment as `Set`.
    let map_struct_id = module_scopes
        .get("map")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("Map").copied());
    if let Some(map_struct_id) = map_struct_id {
        analyzer.primitive_struct_ids.insert("Map", map_struct_id);
    }

    // The `std::shared` `Shared` struct, if `shared.vl` loaded — same treatment.
    let shared_struct_id = module_scopes
        .get("shared")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("Shared").copied());
    if let Some(shared_struct_id) = shared_struct_id {
        analyzer
            .primitive_struct_ids
            .insert("Shared", shared_struct_id);
    }

    // The `std::json` `JsonValue` struct, if `json.vl` loaded — same treatment.
    // Its `field` method id is captured after `build()` to lower to `self[name]`.
    let json_value_struct_id = module_scopes
        .get("json")
        .and_then(|scope_id| analyzer.scopes.get(scope_id))
        .and_then(|scope| scope.name_to_id_map.get("JsonValue").copied());
    if let Some(json_value_struct_id) = json_value_struct_id {
        analyzer
            .primitive_struct_ids
            .insert("JsonValue", json_value_struct_id);
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

    // A normal entry is walked here in the global scope. When the entry is a std
    // module it was already walked (as its module) in the loop above, so skip
    // this to avoid analyzing it twice.
    if !entry_is_module {
        analyzer.current_source_id = SourceId(0);
        let entry_walk_start = analyzer.entity_id;
        analyzer.walk_expr_nodes(&nodes.0, global_scope_id);
        // Synthesized `@derive(..)` impls are walked into the same (entry) scope,
        // right after the user's items, so they see the derived types.
        if let Some(derived) = derived {
            analyzer.walk_expr_nodes(derived, global_scope_id);
        }
        source_ranges.push(SourceRange {
            start: entry_walk_start,
            end: analyzer.entity_id,
            source: SourceId(0),
        });
    }
    analyzer.build();
    // Infer the `borrows` effect before any check reads it (readonly-mutation
    // and the scalar-view lowering both consult `Function.borrows`).
    analyzer.infer_borrows();
    // Record `Some(let v)` captures over wrapped-scalar-view calls before the
    // checks + view classification consult them.
    analyzer.wrapped_view_captures = analyzer.compute_wrapped_view_captures();
    analyzer.check_readonly_mutation();
    analyzer.check_mutable_arguments();
    analyzer.check_mutable_references();
    analyzer.check_view_escape();
    analyzer.check_invalidation();
    analyzer.check_reseat_escape();

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
    // `random::range_{i32,u32,f64}` (the `Random` trait impls forward to these).
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
                    ("to_uppercase", Intrinsic::StrToUppercase),
                    ("len", Intrinsic::StrLen),
                    ("contains", Intrinsic::StrContains),
                    ("starts_with", Intrinsic::StrStartsWith),
                    ("ends_with", Intrinsic::StrEndsWith),
                    ("replace", Intrinsic::StrReplace),
                    ("repeat", Intrinsic::StrRepeat),
                    ("split", Intrinsic::StrSplit),
                    ("substring", Intrinsic::StrSubstring),
                    ("parse_i32", Intrinsic::ParseI32),
                    ("parse_f64", Intrinsic::ParseF64),
                ] {
                    if let Some(id) = implementation.declarations.get(name).copied() {
                        intrinsics.insert(id, intrinsic);
                    }
                }
            }
        }
    }
    if let Some(list_struct_id) = analyzer.primitive_struct_ids.get("List").copied() {
        for implementation in &analyzer.implementations {
            let subject_is_list = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == list_struct_id
            );
            if subject_is_list {
                for (name, intrinsic) in [
                    ("len", Intrinsic::ListLen),
                    ("get", Intrinsic::ListGet),
                    ("pop", Intrinsic::ListPop),
                ] {
                    if let Some(id) = implementation.declarations.get(name).copied() {
                        intrinsics.insert(id, intrinsic);
                    }
                }
            }
        }
    }
    if let Some(set_struct_id) = analyzer.primitive_struct_ids.get("Set").copied() {
        for implementation in &analyzer.implementations {
            let subject_is_set = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == set_struct_id
            );
            if subject_is_set {
                for (name, intrinsic) in [
                    ("new", Intrinsic::SetNew),
                    ("insert", Intrinsic::SetInsert),
                    ("contains", Intrinsic::SetContains),
                    ("remove", Intrinsic::SetRemove),
                    ("len", Intrinsic::SetLen),
                ] {
                    if let Some(id) = implementation.declarations.get(name).copied() {
                        intrinsics.insert(id, intrinsic);
                    }
                }
            }
        }
    }
    if let Some(map_struct_id) = analyzer.primitive_struct_ids.get("Map").copied() {
        for implementation in &analyzer.implementations {
            let subject_is_map = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == map_struct_id
            );
            if subject_is_map {
                for (name, intrinsic) in [
                    ("new", Intrinsic::MapNew),
                    ("insert", Intrinsic::MapInsert),
                    ("get", Intrinsic::MapGet),
                    ("contains_key", Intrinsic::MapContainsKey),
                    ("remove", Intrinsic::MapRemove),
                    ("len", Intrinsic::MapLen),
                    ("keys", Intrinsic::MapKeys),
                    ("values", Intrinsic::MapValues),
                ] {
                    if let Some(id) = implementation.declarations.get(name).copied() {
                        intrinsics.insert(id, intrinsic);
                    }
                }
            }
        }
    }
    if let Some(json_value_struct_id) = analyzer.primitive_struct_ids.get("JsonValue").copied() {
        for implementation in &analyzer.implementations {
            let subject_is_json_value = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == json_value_struct_id
            );
            if subject_is_json_value {
                for (name, intrinsic) in [
                    ("field", Intrinsic::JsonField),
                    ("tag", Intrinsic::JsonTag),
                    ("elements", Intrinsic::JsonElements),
                    ("is_null", Intrinsic::JsonIsNull),
                ] {
                    if let Some(id) = implementation.declarations.get(name).copied() {
                        intrinsics.insert(id, intrinsic);
                    }
                }
            }
        }
    }
    if let Some(shared_struct_id) = analyzer.primitive_struct_ids.get("Shared").copied() {
        for implementation in &analyzer.implementations {
            let subject_is_shared = matches!(
                analyzer.type_id_to_type_map.get(&implementation.subject),
                Some(Type::Struct(id, _)) if *id == shared_struct_id
            );
            if subject_is_shared {
                for (name, intrinsic) in [
                    ("new", Intrinsic::SharedNew),
                    ("clone", Intrinsic::SharedClone),
                    ("read", Intrinsic::SharedValue),
                    ("write", Intrinsic::SharedValue),
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
    if let Some(scan_id) = module_member("process", "scan") {
        intrinsics.insert(scan_id, Intrinsic::Scan);
    }
    for (name, intrinsic) in [
        ("range_i32", Intrinsic::RandomInt),
        ("range_u32", Intrinsic::RandomInt),
        ("range_f64", Intrinsic::RandomFloat),
    ] {
        if let Some(id) = module_member("random", name) {
            intrinsics.insert(id, intrinsic);
        }
    }
    if let Some(args_id) = module_member("process", "args") {
        intrinsics.insert(args_id, Intrinsic::Args);
    }
    if let Some(env_id) = module_member("process", "env") {
        intrinsics.insert(env_id, Intrinsic::Env);
    }
    if let Some(id) = module_member("dom", "query_selector_all") {
        intrinsics.insert(id, Intrinsic::QuerySelectorAll);
    }

    // Transparent references (R5): rewrite bare assignments to a view into the
    // write-through deref form before codegen reads the targets.
    analyzer.rewrite_view_assignment_targets();
    let clone_sites = analyzer.compute_clone_sites();
    let boxed_locals = analyzer.compute_boxed_locals();
    let primitive_views = analyzer.compute_primitive_views();
    let scalar_view_refs = analyzer.compute_scalar_view_refs();
    let scalar_view_calls = analyzer.compute_scalar_view_calls();

    // Pre-render a type label for every typed expression (for hover). Done here
    // while the analyzer still holds the type tables; `expr_id_to_type_id_map`
    // is applied last so it wins over `resolved_types`, matching `type_of_expr`.
    let empty_substitution = SubstitutionContext::new();
    let mut expr_types: HashMap<Id, String> = HashMap::new();
    // The same merge, kept as raw type ids for the transformer (tuple layout).
    let mut expr_type_ids: HashMap<Id, TypeId> = HashMap::new();
    for (expr_id, type_id) in analyzer
        .resolved_types
        .iter()
        .chain(analyzer.expr_id_to_type_id_map.iter())
    {
        let type_ = type_id.get_type(&analyzer);
        expr_types.insert(
            *expr_id,
            analyzer.pretty_print_type(&type_, &empty_substitution),
        );
        expr_type_ids.insert(*expr_id, *type_id);
    }
    // Also label variable and parameter bindings by their own id: a *use* of one
    // (an `Expr::Local`/`Expr::Parameter`) carries no type on its own expr id, so
    // hover resolves through the binding.
    for (binding_id, variable) in &analyzer.variables {
        let type_ = variable.type_id.get_type(&analyzer);
        expr_types.insert(
            *binding_id,
            analyzer.pretty_print_type(&type_, &empty_substitution),
        );
    }
    for (binding_id, parameter) in &analyzer.parameters {
        let type_ = parameter.type_id.get_type(&analyzer);
        expr_types.insert(
            *binding_id,
            analyzer.pretty_print_type(&type_, &empty_substitution),
        );
    }
    // Label declarations themselves, so hover works on a function/type at its
    // definition (and on a bare reference to one).
    for function_id in analyzer
        .functions
        .keys()
        .chain(analyzer.external_functions.keys())
        .copied()
        .collect::<Vec<_>>()
    {
        let label = analyzer.pretty_print_type(&Type::Function(function_id), &empty_substitution);
        expr_types.insert(function_id, label);
    }
    for struct_id in analyzer.structs.keys().copied().collect::<Vec<_>>() {
        let label =
            analyzer.pretty_print_type(&Type::Struct(struct_id, Vec::new()), &empty_substitution);
        expr_types.insert(struct_id, label);
    }
    for enum_id in analyzer.enums.keys().copied().collect::<Vec<_>>() {
        let label =
            analyzer.pretty_print_type(&Type::Enum(enum_id, Vec::new()), &empty_substitution);
        expr_types.insert(enum_id, label);
    }
    for trait_id in analyzer.traits.keys().copied().collect::<Vec<_>>() {
        let label =
            analyzer.pretty_print_type(&Type::Trait(trait_id, Vec::new()), &empty_substitution);
        expr_types.insert(trait_id, label);
    }

    // Render each type reference's hover label now that all types are resolved.
    let type_references = analyzer
        .type_references
        .iter()
        .map(|(source, span, definition, type_id)| {
            let type_ = type_id.get_type(&analyzer);
            let label = analyzer.pretty_print_type(&type_, &empty_substitution);
            (*source, *span, *definition, label)
        })
        .collect::<Vec<_>>();

    Program {
        target,
        closures: analyzer.closures,
        diagnostics: analyzer.diagnostics,
        enums: analyzer.enums,
        entity_map: analyzer.expr_id_to_expr_map,
        entity_scope_map: analyzer.expr_id_to_scope_id_map,
        function_calls: analyzer.function_calls,
        functions: analyzer.functions,
        external_functions: analyzer.external_functions,
        traits: analyzer.traits,
        generic_dispatch: analyzer.generic_dispatch,
        for_each_next: analyzer.for_each_next,
        for_each_views: analyzer.for_each_views,
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
        parameters: analyzer.parameters,
        sources,
        source_ranges,
        member_name_spans: analyzer.member_name_spans,
        struct_initializer_to_def: analyzer.struct_initializer_to_def,
        type_references,
        expr_types,
        expr_type_ids,
        next_entity_id: analyzer.entity_id,
        async_functions: HashSet::new(),
        clone_sites,
        boxed_locals,
        primitive_views,
        scalar_view_refs,
        scalar_view_calls,
    }
}
