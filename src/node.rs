use crate::span::Spanned;

pub type GenericParameters<'src> = Spanned<Vec<GenericParameter<'src>>>;

#[derive(Debug)]
pub struct GenericParameter<'src> {
    pub name: &'src str,
    // Declared with the `type` keyword (a binder, e.g. `impl Foo<type T>`).
    pub is_type: bool,
    // Trait bounds: `T: A + B` collects `[A, B]`.
    pub bounds: Vec<Spanned<Node<'src>>>,
    // A default, e.g. the `Self` in `<B = Self>`.
    pub default: Option<Box<Spanned<Node<'src>>>>,
}

pub type GenericArguments<'src> = Spanned<Vec<Spanned<Node<'src>>>>;

#[derive(Debug)]
pub struct Func<'src> {
    pub name: Spanned<&'src str>,
    // Declared with the `external` keyword: an intrinsic with no Vilan body,
    // implemented by the runtime/compiler (e.g. `external fun print(..);`).
    pub external: bool,
    pub generic_parameters: Option<GenericParameters<'src>>,
    pub parameters: Spanned<Vec<(&'src str, Option<Box<Spanned<Node<'src>>>>)>>,
    pub return_type: Option<Box<Spanned<Node<'src>>>>,
    // `None` for a function signature without a body: a required trait method
    // declaration (`fun default(): Self;`) or an `external` intrinsic.
    pub body: Option<Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>>,
}

#[derive(Debug)]
pub struct Closure<'src> {
    pub parameters: Spanned<Vec<(&'src str, Option<Box<Spanned<Node<'src>>>>)>>,
    pub return_type: Option<Box<Spanned<Node<'src>>>>,
    pub return_value: Box<Spanned<Node<'src>>>,
}

#[derive(Debug)]
pub struct If<'src> {
    pub condition: Box<Spanned<Node<'src>>>,
    pub then: Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>,
    pub else_: Option<Spanned<NodeIfBranch<'src>>>,
}

#[derive(Debug)]
pub enum NodeIfBranch<'src> {
    If(Box<If<'src>>),
    Else(Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>),
}

#[derive(Debug)]
pub enum ImportBranch<'src> {
    Path(&'src str, Option<Box<Self>>),
    Set(Vec<Self>),
}

pub type NodeList<'src> = Vec<Spanned<Node<'src>>>;

#[derive(Debug)]
pub enum Node<'src> {
    Accessor(&'src str),
    AccessorWithGenerics(&'src str, GenericArguments<'src>),
    // A `type X` generic binder appearing inside a type — the impl subject
    // pattern (`impl Option<(type T, type U)>`), including a bare blanket
    // (`impl type T`). The optional bounds are `T: A + B`.
    TypeBinder(&'src str, Vec<Spanned<Self>>),
    // `x = v` or a compound assignment like `x += v` (the operator is the
    // binary op the assignment applies, e.g. `Add` for `+=`). The target is an
    // lvalue: a local (`Accessor`) or a field place (`MemberAccessor`, e.g.
    // `self.n = v`).
    Assign(Box<Spanned<Self>>, Option<BinaryOp>, Box<Spanned<Self>>),
    Binary(BinaryOp, Box<Spanned<Self>>, Box<Spanned<Self>>),
    Block(Spanned<(NodeList<'src>, Box<Spanned<Self>>)>),
    Bool(bool),
    Call(
        Box<Spanned<Self>>,
        Option<GenericArguments<'src>>,
        Spanned<NodeList<'src>>,
    ),
    Closure(Closure<'src>),
    ClosureType(
        Spanned<Vec<(Option<&'src str>, Box<Spanned<Node<'src>>>)>>,
        Option<Box<Spanned<Node<'src>>>>,
    ),
    // An enum declaration: name, generics, and the variants — each a name,
    // the types of its optional data, and an optional explicit discriminant
    // (`Less = -1`).
    Enum(
        &'src str,
        Option<GenericParameters<'src>>,
        Spanned<Vec<Spanned<EnumVariant<'src>>>>,
    ),
    Error,
    // A loop: `for { .. }` (infinite, condition `None`) or `for cond { .. }`
    // (while).
    For(
        Option<Box<Spanned<Self>>>,
        Spanned<(NodeList<'src>, Box<Spanned<Self>>)>,
    ),
    // `for item in iterable { .. }` — the binding name, the iterable, the body.
    ForIn(
        &'src str,
        Box<Spanned<Self>>,
        Spanned<(NodeList<'src>, Box<Spanned<Self>>)>,
    ),
    Func(Func<'src>),
    FuncReturn(Box<Spanned<Self>>),
    If(NodeIfBranch<'src>),
    // `subject is pattern` — a pattern test that yields a `bool` and binds the
    // pattern's captures into the surrounding scope.
    Is(Box<Spanned<Self>>, Box<Spanned<Pattern<'src>>>),
    // `jump break` / `jump continue` — the target keyword that follows `jump`.
    Jump(&'src str),
    Impl(
        // The subject type pattern. May contain `type X` binders anywhere
        // (`impl Option<(type T, type U)>`) or be a bare binder (`impl type T`);
        // those binders are the impl's generic parameters.
        Box<Spanned<Self>>,
        // The traits being implemented: the `A`, `B` in `impl Subject with A + B`.
        Vec<Spanned<Self>>,
        Spanned<NodeList<'src>>,
    ),
    Import(ImportBranch<'src>),
    // `export <item>` — re-export an import or expose a local declaration.
    Export(Box<Spanned<Self>>),
    // `let`/`mut` binding: name, type annotation, value, mutability.
    Let(
        &'src str,
        Option<Box<Spanned<Self>>>,
        Option<Box<Spanned<Self>>>,
        bool,
    ),
    List(NodeList<'src>),
    // A match expression: subject and legs of `patterns (if guard)? => body`.
    Match(Box<Spanned<Self>>, Spanned<Vec<MatchLeg<'src>>>),
    MemberAccessor(Box<Spanned<Self>>, Box<Spanned<Self>>),
    Module(&'src str, Spanned<NodeList<'src>>),
    Null,
    // The whole part, an optional fractional part, and an optional type suffix.
    Number(&'src str, Option<&'src str>, Option<&'src str>),
    StaticAccessor(Box<Spanned<Self>>, &'src str),
    String(&'src str),
    // A struct declaration. The `bool` marks an `external` (intrinsic) struct.
    // The body is `Some(fields)` for `{ .. }` and `None` for a bodyless `;`
    // declaration (only valid when `external`).
    Struct(
        &'src str,
        Option<GenericParameters<'src>>,
        bool,
        Option<Spanned<Vec<Spanned<(&'src str, Option<Spanned<Self>>)>>>>,
    ),
    StructInitializer(
        &'src str,
        Option<GenericArguments<'src>>,
        Spanned<Vec<Spanned<(&'src str, Option<Spanned<Self>>)>>>,
    ),
    Trait(
        &'src str,
        Option<GenericParameters<'src>>,
        // Supertraits: the `A`, `B` in `trait T with A + B`.
        Vec<Spanned<Self>>,
        Spanned<NodeList<'src>>,
    ),
    Tuple(NodeList<'src>),
    // A prefix operator: `!x` or `-x`.
    Unary(char, Box<Spanned<Self>>),
    // `use Namespace::{ a, b };` — destructures items out of a namespace
    // (a module or an enum) into the current scope.
    Use(ImportBranch<'src>),
    Void,
}

// One enum variant: name, the types of its optional data, and an optional
// explicit integer discriminant (`Less = -1`).
pub type EnumVariant<'src> = (&'src str, Vec<Spanned<Node<'src>>>, Option<i64>);

// A match-leg pattern.
#[derive(Debug)]
pub enum Pattern<'src> {
    // `_` — matches anything without binding it.
    Wildcard,
    // `let x` / `mut x` — matches anything, capturing the value.
    Binding(&'src str, bool),
    // A path to an enum variant with optional payload patterns: a bare `Name`
    // (`["Name"]`) or a qualified `Enum::Variant` (`["Enum", "Variant"]`).
    Variant(Vec<&'src str>, Option<Vec<Spanned<Pattern<'src>>>>),
    // `(a, b, ...)` — a tuple pattern.
    Tuple(Vec<Spanned<Pattern<'src>>>),
    // A literal value pattern (`"quit"`, `42`, `true`): matches by equality,
    // binding nothing. Holds the literal as its node.
    Literal(Box<Spanned<Node<'src>>>),
}

// One match leg: the patterns it matches (more than one is an or-pattern,
// `"y", "" => ..`), an optional `if` guard, and the body.
pub type MatchLeg<'src> = (
    Vec<Spanned<Pattern<'src>>>,
    Option<Box<Spanned<Node<'src>>>>,
    Spanned<Node<'src>>,
);

#[derive(Clone, Copy, Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    // Logical AND (`&&`), also produced by the compiler for nested
    // match-pattern tests.
    And,
}
