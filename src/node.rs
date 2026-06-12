use crate::span::Spanned;

pub type GenericParameters<'src> = Spanned<Vec<(&'src str, Option<Spanned<Node<'src>>>)>>;

pub type GenericArguments<'src> = Spanned<Vec<Spanned<Node<'src>>>>;

#[derive(Debug)]
pub struct Func<'src> {
    pub name: Spanned<&'src str>,
    pub generic_parameters: Option<GenericParameters<'src>>,
    pub parameters: Spanned<Vec<(&'src str, Option<Box<Spanned<Node<'src>>>>)>>,
    pub return_type: Option<Box<Spanned<Node<'src>>>>,
    // `None` for a function signature without a body, e.g. a required trait
    // method declaration like `fun default(): Self;`.
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
    // `x = v` or a compound assignment like `x += v` (the operator is the
    // binary op the assignment applies, e.g. `Add` for `+=`).
    Assign(&'src str, Option<BinaryOp>, Box<Spanned<Self>>),
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
    // An enum declaration: name, generics, and the variants — each a name
    // with the types of its optional data.
    Enum(
        &'src str,
        Option<GenericParameters<'src>>,
        Spanned<Vec<Spanned<(&'src str, Vec<Spanned<Node<'src>>>)>>>,
    ),
    Error,
    // A loop: `for { .. }` (infinite, condition `None`) or `for cond { .. }`
    // (while). The `for .. in ..` iterator form maps here too once added.
    For(
        Option<Box<Spanned<Self>>>,
        Spanned<(NodeList<'src>, Box<Spanned<Self>>)>,
    ),
    Func(Func<'src>),
    FuncReturn(Box<Spanned<Self>>),
    If(NodeIfBranch<'src>),
    // `jump break` / `jump continue` — the target keyword that follows `jump`.
    Jump(&'src str),
    Impl(
        Box<Spanned<Self>>,
        Option<GenericParameters<'src>>,
        // The trait being implemented, i.e. the `T` in `impl Subject with T`.
        Option<Box<Spanned<Self>>>,
        Spanned<NodeList<'src>>,
    ),
    Import(ImportBranch<'src>),
    // `let`/`mut` binding: name, type annotation, value, mutability.
    Let(
        &'src str,
        Option<Box<Spanned<Self>>>,
        Option<Box<Spanned<Self>>>,
        bool,
    ),
    List(NodeList<'src>),
    // A match expression: subject and legs of `pattern => expression`.
    Match(
        Box<Spanned<Self>>,
        Spanned<Vec<(Spanned<Pattern<'src>>, Spanned<Node<'src>>)>>,
    ),
    MemberAccessor(Box<Spanned<Self>>, Box<Spanned<Self>>),
    Module(&'src str, Spanned<NodeList<'src>>),
    Null,
    Number(&'src str, Option<&'src str>),
    StaticAccessor(Box<Spanned<Self>>, &'src str),
    String(&'src str),
    Struct(
        &'src str,
        Option<GenericParameters<'src>>,
        Spanned<Vec<Spanned<(&'src str, Option<Spanned<Self>>)>>>,
    ),
    StructInitializer(
        &'src str,
        Option<GenericArguments<'src>>,
        Spanned<Vec<Spanned<(&'src str, Option<Spanned<Self>>)>>>,
    ),
    Trait(
        &'src str,
        Option<GenericParameters<'src>>,
        Spanned<NodeList<'src>>,
    ),
    Tuple(NodeList<'src>),
    // `use Namespace::{ a, b };` — destructures items out of a namespace
    // (a module or an enum) into the current scope.
    Use(ImportBranch<'src>),
    Void,
}

// A match-leg pattern.
#[derive(Debug)]
pub enum Pattern<'src> {
    // `_` — matches anything without binding it.
    Wildcard,
    // `let x` / `mut x` — matches anything, capturing the value.
    Binding(&'src str, bool),
    // `Name` or `Name(patterns...)` — an enum variant with payload patterns.
    Variant(&'src str, Option<Vec<Spanned<Pattern<'src>>>>),
}

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
    // Logical AND. Currently only produced by the compiler itself (e.g. for
    // nested match-pattern tests); there is no surface syntax yet.
    And,
}
