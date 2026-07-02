use crate::span::{Span, Spanned};

pub type GenericParameters<'src> = Spanned<Vec<GenericParameter<'src>>>;

#[derive(Debug)]
pub struct GenericParameter<'src> {
    pub name: &'src str,
    /// The span of the parameter's name (for go-to-definition on a use of it).
    pub name_span: Span,
    // Declared with the `type` keyword (a binder, e.g. `impl Foo<type T>`).
    pub is_type: bool,
    // Trait bounds: `T: A + B` collects `[A, B]`.
    pub bounds: Vec<Spanned<Node<'src>>>,
    // A tuple bound: `T: (2..)` / `(..10)` / `(..: Display)` — the parameter is a
    // tuple of the given arity, optionally with a per-element trait bound. Mutually
    // exclusive with `bounds` (a tuple bound replaces the trait-bound list).
    pub tuple_bound: Option<TupleBound<'src>>,
    // A default, e.g. the `Self` in `<B = Self>`.
    pub default: Option<Box<Spanned<Node<'src>>>>,
}

// A tuple-arity bound on a generic parameter (`T: (lo..hi : Element)`). Either
// endpoint may be omitted (`(..)`, `(2..)`, `(..10)`); `element` is the optional
// per-element trait bound (`(..: Display)`).
#[derive(Debug)]
pub struct TupleBound<'src> {
    pub lo: Option<u32>,
    pub hi: Option<u32>,
    pub element: Option<Box<Spanned<Node<'src>>>>,
    pub span: Span,
}

pub type GenericArguments<'src> = Spanned<Vec<Spanned<Node<'src>>>>;

// How an `external` function is bound to the host (JS): a `[extern(..)]`
// attribute selects the form. The receiver of a method/property is the
// function's first parameter.
#[derive(Clone, Debug)]
pub enum ExternBinding<'src> {
    // `[extern("node:http", "createServer")]` — import `symbol` from `module`
    // (or, with no module, a global/verbatim symbol like `"console.log"`) and
    // call it: `symbol(args)`.
    Function {
        module: Option<&'src str>,
        symbol: &'src str,
    },
    // `[extern(method)]` / `[extern(method, "setHeader")]` — `receiver.symbol(rest)`
    // (the JS name defaults to the function's own name).
    Method {
        symbol: Option<&'src str>,
    },
    // `[extern(get, "statusCode")]` — `receiver.symbol` (a property read).
    Get {
        symbol: &'src str,
    },
    // `[extern(set, "statusCode")]` — `receiver.symbol = value` (a property write).
    Set {
        symbol: &'src str,
    },
}

#[derive(Debug)]
pub struct Func<'src> {
    pub name: Spanned<&'src str>,
    // Declared with the `async` keyword. For an `external` (a leaf with no body)
    // this is the only signal that it is async; for an ordinary function it is
    // usually inferred instead, but `async fun` forces it.
    pub is_async: bool,
    // Declared with the `external` keyword: an intrinsic with no Vilan body,
    // implemented by the runtime/compiler (e.g. `external fun print(..);`).
    pub external: bool,
    // A `[extern(..)]` host binding, lowering this external to a JS import/call,
    // method, or property access. `None` for a plain `external` (compiler
    // intrinsic) or an ordinary function.
    pub extern_binding: Option<ExternBinding<'src>>,
    // Declared `[must_use]`: dropping a call's result (a bare statement that
    // discards it) is a warning.
    pub must_use: bool,
    // Declared `[rpc]`: callable over the wire as part of a service's surface.
    // Its parameters and return must be Wire types — checked by the analyzer
    // (`proposal/transport-rpc.md` §4.2).
    pub rpc: bool,
    pub generic_parameters: Option<GenericParameters<'src>>,
    pub parameters: Spanned<Vec<Parameter<'src>>>,
    pub return_type: Option<Box<Spanned<Node<'src>>>>,
    // The `borrows <param>` clause on a view-returning function
    // (`fun slot(&mut self): &mut i32 borrows self`): the returned view is a
    // projection of that parameter, so it may escape (rule 3's sanctioned case).
    pub borrows: Option<&'src str>,
    // `None` for a function signature without a body: a required trait method
    // declaration (`fun default(): Self;`) or an `external` intrinsic.
    pub body: Option<Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>>,
}

/// A parsed parameter: its name, optional declared type, and how it receives its
/// argument (rule 3 conventions).
/// A parsed parameter: name, optional declared type, view convention, and the
/// span of the name (for go-to-definition / hover in the language server).
pub type Parameter<'src> = (
    // The binder: a plain name (`x`) or a tuple destructure (`(a, b)`).
    Pattern<'src>,
    Option<Box<Spanned<Node<'src>>>>,
    Convention,
    Span,
);

/// How a parameter receives its argument (rule 3). `Bare` is the default (a
/// readonly view, once the default flip lands); `Ref` / `RefMut` are `&` / `&mut`
/// views. `Own` (owned value) is added with its keyword later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Convention {
    Bare,
    Own,
    Ref,
    RefMut,
}

#[derive(Debug)]
pub struct Closure<'src> {
    pub parameters: Spanned<Vec<Parameter<'src>>>,
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
    // A path segment: its name, the span of that name, and an optional `::`
    // continuation. The span drives go-to-definition / hover on imports.
    Path(&'src str, Span, Option<Box<Self>>),
    Set(Vec<Self>),
}

pub type NodeList<'src> = Vec<Spanned<Node<'src>>>;

#[derive(Debug)]
pub enum Node<'src> {
    Accessor(&'src str),
    AccessorWithGenerics(&'src str, GenericArguments<'src>),
    // `async <block-or-expr>` — runs the body as a promise, evaluating to a
    // `Promise<T>` immediately (non-blocking). Lowers to an invoked async arrow.
    Async(Box<Spanned<Self>>),
    // `await <expr>` — suspends until the promise resolves, yielding `T`. Forces
    // its enclosing function to be async.
    Await(Box<Spanned<Self>>),
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
    // A mapped tuple type `(U in T: F<U>)`: bind each element of the source tuple
    // type `T` as `U`, and the corresponding result slot is the template `F<U>`.
    MappedType {
        binder: &'src str,
        binder_span: Span,
        source: Box<Spanned<Node<'src>>>,
        template: Box<Spanned<Node<'src>>>,
    },
    // A tuple comprehension `(x in xs = e)`: build a tuple by evaluating the body
    // `e` for each element of the source tuple `xs`, with the element bound as `x`.
    TupleComprehension {
        binder: &'src str,
        binder_span: Span,
        source: Box<Spanned<Node<'src>>>,
        body: Box<Spanned<Node<'src>>>,
    },
    // An enum declaration: name, generics, and the variants — each a name,
    // the types of its optional data, and an optional explicit discriminant
    // (`Less = -1`).
    Enum(
        Spanned<&'src str>,
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
    // `[derive(A, B)] <struct|enum>` — the derive trait names and the item they
    // annotate. Transparent to analysis (the inner item is walked normally); a
    // pre-analysis pass generates the trait impls from the item's fields.
    Derive(Vec<&'src str>, Box<Spanned<Self>>),
    // `let`/`mut` binding: name, type annotation, value, mutability.
    Let(
        Spanned<&'src str>,
        Option<Box<Spanned<Self>>>,
        Option<Box<Spanned<Self>>>,
        bool,
    ),
    // `let`/`mut` binding with a destructuring pattern: `let (a, b) = pair`. The
    // pattern is irrefutable (a tuple of names/sub-patterns); the rest mirrors
    // `Let` (type annotation, value, mutability).
    LetDestructure(
        Spanned<Pattern<'src>>,
        Option<Box<Spanned<Self>>>,
        Option<Box<Spanned<Self>>>,
        bool,
    ),
    List(NodeList<'src>),
    // A match expression: subject and legs of `patterns (if guard)? => body`.
    Match(Box<Spanned<Self>>, Spanned<Vec<MatchLeg<'src>>>),
    MemberAccessor(Box<Spanned<Self>>, Box<Spanned<Self>>),
    // `subject[index]` — a subscript into a `List` (element access / assignment,
    // and `&mut list[i]` element views). Subject and index expressions.
    Index(Box<Spanned<Self>>, Box<Spanned<Self>>),
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
        Spanned<&'src str>,
        Option<GenericParameters<'src>>,
        bool,
        Option<Spanned<Vec<Spanned<StructField<'src>>>>>,
    ),
    StructInitializer(
        &'src str,
        Option<GenericArguments<'src>>,
        Spanned<Vec<Spanned<(&'src str, Option<Spanned<Self>>)>>>,
    ),
    Trait(
        Spanned<&'src str>,
        Option<GenericParameters<'src>>,
        // Supertraits: the `A`, `B` in `trait T with A + B`.
        Vec<Spanned<Self>>,
        Spanned<NodeList<'src>>,
    ),
    Tuple(NodeList<'src>),
    // A prefix operator: `!x` or `-x`.
    Unary(char, Box<Spanned<Self>>),
    // `&x` / `&mut x` — take a (readonly / writable) view of a place. The bool is
    // whether the view is writable (`&mut`).
    Reference(bool, Box<Spanned<Self>>),
    // `*v` — read or write through a view.
    Dereference(Box<Spanned<Self>>),
    // `use Namespace::{ a, b };` — destructures items out of a namespace
    // (a module or an enum) into the current scope.
    Use(ImportBranch<'src>),
    Void,
}

// One enum variant: name, the types of its optional data, and an optional
// explicit integer discriminant (`Less = -1`).
pub type EnumVariant<'src> = (&'src str, Vec<Spanned<Node<'src>>>, Option<i64>);

// One struct field: its name (with the name's own span), optional type
// annotation, and whether it is `[expose]`d — observable by a service's client
// as a mirrored `Source` (`proposal/transport-rpc.md` §4.2).
pub type StructField<'src> = (Spanned<&'src str>, Option<Spanned<Node<'src>>>, bool);

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
    // Logical OR (`||`). Binds looser than `&&`.
    Or,
}
