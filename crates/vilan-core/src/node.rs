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
    // `[extern(new, "TextDecoder")]` — `new symbol(args)`: construct a host class
    // instance (host constructors reject a plain call). With a module
    // (`[extern(new, "node:sqlite", "DatabaseSync")]`), the class is imported
    // first, like `Function`'s module form.
    New {
        module: Option<&'src str>,
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
    // Declared `[platform("…", …)]` — a platform FENCE: the function's
    // inferred requirement is checked against these patterns on every
    // compile (platform-coloring.md §3.7). Empty = no fence.
    pub platform_fence: Vec<Spanned<&'src str>>,
    // Declared `[rpc]`: callable over the wire as part of a service's surface.
    // Its parameters and return must be Wire types — checked by the analyzer
    // (`proposal/transport-rpc.md` §4.2).
    pub rpc: bool,
    // Declared `[trait_only]` (on a trait's method declaration): reachable only
    // through a trait bound, never on a concrete type's own surface
    // (`proposal/transport-rpc.md` §3.2).
    pub trait_only: bool,
    // Declared `[doc(hidden)]`: fully callable, but omitted from editor
    // completion (a tooling marker — no resolution change).
    pub doc_hidden: bool,
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
    // `async || T` — a closure type whose calls suspend: calls through a
    // value of this type are implicitly awaited, like direct calls to an
    // async function (backlog J2). Wraps the closure type it marks.
    AsyncType(Box<Spanned<Node<'src>>>),
    // `(|| void) context owner_scope` / `context (a, b)` — a closure type
    // carrying a context requirement (proposal/ambient-owner.md §5): the
    // closure defers those contexts' bindings to its CALL sites instead of
    // capturing at creation. The names (with spans) name context VALUES;
    // written order is the hidden-argument order.
    TypeWithContexts(Box<Spanned<Self>>, Vec<(&'src str, Span)>),
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
    // `ret <expr>` / bare `ret` (an early return of void).
    FuncReturn(Option<Box<Spanned<Self>>>),
    // `expr!` — assert-or-return (proposal/try-and-lift.md): the good half of a
    // `Try` value, or an early return of the bad half from the nearest
    // enclosing function.
    TryAssert(Box<Spanned<Self>>),
    // `a?.b.c` — a lifted member chain (proposal/try-and-lift.md §3): the
    // subject, and the continuation built over `LiftBinder` (the segment from
    // this `?` to the next `?`/`!`/chain end). Maps, or flattens when the
    // continuation yields the subject's own container.
    Lift(Box<Spanned<Self>>, Box<Spanned<Self>>),
    // The continuation's hole: the lifted element inside a `Lift` chain.
    LiftBinder,
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
    // `macro fun name(..) { .. }` — a macro definition (macro-engine.md §3).
    // Its body is HERMETIC: never walked in the program world, compiled in the
    // per-file macro world instead (its imports resolve against `macro_std`
    // only), and executed by the expansion interpreter.
    MacroFun(Func<'src>),
    // `[name(args)] <item>` — a user macro attribute on a struct/enum/function:
    // the macro's name (with its span), the argument SPANS (their source text
    // is what `Arguments` carries — arguments are syntax), and the annotated
    // item. Expanded before analysis; the item itself is walked normally.
    MacroAttribute(&'src str, Span, Vec<Span>, Box<Spanned<Self>>),
    // `macro name(args)` — a macro invocation (macro-engine.md §2). At a
    // module's top level it is an ITEM invocation (the returned Source parses
    // as items, appended to the module); anywhere else it is an EXPRESSION
    // invocation (the returned Source parses as an expression and splices in
    // place). The name (with its span) and the argument SPANS.
    MacroInvocation(&'src str, Span, Vec<Span>),
    // `macro { .. }` — an anonymous, immediately-expanded macro (macro-engine.md
    // Phase 4): the body runs at expansion time (hermetic, like a `macro fun`
    // body) and its returned `Source` splices at this position — as items at a
    // module's top level, as one expression anywhere else. The body shape is a
    // function body; the world compiles it as a synthetic zero-argument
    // `fun __macro_block_<n>(): Source`.
    MacroBlock(Spanned<(NodeList<'src>, Box<Spanned<Self>>)>),
    // `[derive(A, B)] <struct|enum>` — the derive trait names and the item they
    // annotate. Transparent to analysis (the inner item is walked normally); a
    // pre-analysis pass generates the trait impls from the item's fields.
    Derive(Vec<(&'src str, Span)>, Box<Spanned<Self>>),
    // `[service(Client)] struct …` — a per-connection service struct
    // (`proposal/transport-rpc.md` §4.2). Transparent to analysis; a
    // pre-analysis pass generates its dispatcher, its client sibling (named by
    // the argument, defaulting to `<Struct>Client`), and the contract hash from
    // the struct's `[rpc]` impl methods and `[expose]`d fields.
    Service(Option<&'src str>, Box<Spanned<Self>>),
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
    // A triple-quoted string's raw inner text; trimmed to its content by
    // `util::trim_multiline_string` (validated in the analyzer, trimmed in the
    // transformer).
    MultilineString(&'src str),
    // `const expr` — evaluate at compile time (proposal/const-eval.md). The
    // analyzer marks the inner expression and FORWARDS to it (no wrapper
    // entity), so downstream passes see a plain subtree.
    Const(Box<Spanned<Node<'src>>>),
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

impl<'src> Node<'src> {
    /// Visits every direct child node. Whole-tree scans that must see nodes at
    /// any nesting depth (`collect_module_refs` finding a block-scoped `import`
    /// inside a closure, the platform sniffer) recurse with this. The match is
    /// deliberately exhaustive with no catch-all: adding a `Node` variant must
    /// extend it or compilation fails here — a container variant silently
    /// missing from the scan is exactly the bug this prevents.
    pub fn for_each_child<'a>(&'a self, visit: &mut dyn FnMut(&'a Spanned<Node<'src>>)) {
        fn visit_generic_parameters<'a, 'src>(
            parameters: &'a Option<GenericParameters<'src>>,
            visit: &mut dyn FnMut(&'a Spanned<Node<'src>>),
        ) {
            for parameter in parameters.iter().flat_map(|parameters| &parameters.0) {
                for bound in &parameter.bounds {
                    visit(bound);
                }
                if let Some(element) = parameter
                    .tuple_bound
                    .as_ref()
                    .and_then(|bound| bound.element.as_deref())
                {
                    visit(element);
                }
                if let Some(default) = parameter.default.as_deref() {
                    visit(default);
                }
            }
        }
        fn visit_pattern<'a, 'src>(
            pattern: &'a Pattern<'src>,
            visit: &mut dyn FnMut(&'a Spanned<Node<'src>>),
        ) {
            match pattern {
                Pattern::Wildcard | Pattern::Binding(..) | Pattern::Variant(_, None) => {}
                Pattern::Variant(_, Some(payload)) => {
                    for (sub, _) in payload {
                        visit_pattern(sub, visit);
                    }
                }
                Pattern::Tuple(elements) => {
                    for (sub, _) in elements {
                        visit_pattern(sub, visit);
                    }
                }
                Pattern::Literal(literal) => visit(literal),
            }
        }
        fn visit_parameters<'a, 'src>(
            parameters: &'a Spanned<Vec<Parameter<'src>>>,
            visit: &mut dyn FnMut(&'a Spanned<Node<'src>>),
        ) {
            for (pattern, type_, _, _) in &parameters.0 {
                visit_pattern(pattern, visit);
                if let Some(type_) = type_.as_deref() {
                    visit(type_);
                }
            }
        }
        // A `(statements, tail)` body — blocks, loop bodies, function bodies.
        fn visit_body<'a, 'src>(
            body: &'a (NodeList<'src>, Box<Spanned<Node<'src>>>),
            visit: &mut dyn FnMut(&'a Spanned<Node<'src>>),
        ) {
            for statement in &body.0 {
                visit(statement);
            }
            visit(&body.1);
        }
        fn visit_if_branch<'a, 'src>(
            branch: &'a NodeIfBranch<'src>,
            visit: &mut dyn FnMut(&'a Spanned<Node<'src>>),
        ) {
            match branch {
                NodeIfBranch::If(if_) => {
                    visit(&if_.condition);
                    visit_body(&if_.then.0, visit);
                    if let Some(else_) = &if_.else_ {
                        visit_if_branch(&else_.0, visit);
                    }
                }
                NodeIfBranch::Else(body) => visit_body(&body.0, visit),
            }
        }

        match self {
            // Leaves.
            Node::Accessor(_)
            | Node::Bool(_)
            | Node::Error
            | Node::Import(_)
            | Node::Jump(_)
            | Node::LiftBinder
            | Node::MacroInvocation(..)
            | Node::Null
            | Node::Number(..)
            | Node::String(_)
            | Node::MultilineString(_)
            | Node::Use(_)
            | Node::Void => {}
            Node::AccessorWithGenerics(_, arguments) => {
                for argument in &arguments.0 {
                    visit(argument);
                }
            }
            Node::Async(inner)
            | Node::Await(inner)
            | Node::Dereference(inner)
            | Node::Derive(_, inner)
            | Node::Export(inner)
            | Node::Reference(_, inner)
            | Node::Service(_, inner)
            | Node::StaticAccessor(inner, _)
            | Node::TryAssert(inner)
            | Node::Unary(_, inner) => visit(inner),
            Node::TypeBinder(_, bounds) => {
                for bound in bounds {
                    visit(bound);
                }
            }
            Node::Assign(target, _, value) => {
                visit(target);
                visit(value);
            }
            Node::Binary(_, left, right) => {
                visit(left);
                visit(right);
            }
            Node::Block(body) | Node::MacroBlock(body) => visit_body(&body.0, visit),
            Node::TypeWithContexts(inner, _) => visit(inner),
            Node::Call(subject, generic_arguments, arguments) => {
                visit(subject);
                for argument in generic_arguments.iter().flat_map(|arguments| &arguments.0) {
                    visit(argument);
                }
                for argument in &arguments.0 {
                    visit(argument);
                }
            }
            Node::Closure(closure) => {
                visit_parameters(&closure.parameters, visit);
                if let Some(return_type) = closure.return_type.as_deref() {
                    visit(return_type);
                }
                visit(&closure.return_value);
            }
            Node::ClosureType(parameters, return_type) => {
                for (_, type_) in &parameters.0 {
                    visit(type_);
                }
                if let Some(return_type) = return_type.as_deref() {
                    visit(return_type);
                }
            }
            Node::AsyncType(inner) => visit(inner),
            Node::Const(inner) => visit(inner),
            Node::MappedType {
                source, template, ..
            } => {
                visit(source);
                visit(template);
            }
            Node::TupleComprehension { source, body, .. } => {
                visit(source);
                visit(body);
            }
            Node::Enum(_, generic_parameters, variants) => {
                visit_generic_parameters(generic_parameters, visit);
                for (_, data, _) in variants.0.iter().map(|variant| &variant.0) {
                    for type_ in data {
                        visit(type_);
                    }
                }
            }
            Node::For(condition, body) => {
                if let Some(condition) = condition.as_deref() {
                    visit(condition);
                }
                visit_body(&body.0, visit);
            }
            Node::ForIn(_, iterable, body) => {
                visit(iterable);
                visit_body(&body.0, visit);
            }
            Node::Func(function) | Node::MacroFun(function) => {
                visit_generic_parameters(&function.generic_parameters, visit);
                visit_parameters(&function.parameters, visit);
                if let Some(return_type) = function.return_type.as_deref() {
                    visit(return_type);
                }
                if let Some(body) = &function.body {
                    visit_body(&body.0, visit);
                }
            }
            Node::MacroAttribute(_, _, _, item) => visit(item),
            Node::FuncReturn(value) => {
                if let Some(value) = value.as_deref() {
                    visit(value);
                }
            }
            Node::Lift(subject, continuation) => {
                visit(subject);
                visit(continuation);
            }
            Node::If(branch) => visit_if_branch(branch, visit),
            Node::Is(subject, pattern) => {
                visit(subject);
                visit_pattern(&pattern.0, visit);
            }
            Node::Impl(subject, traits, body) => {
                visit(subject);
                for trait_ in traits {
                    visit(trait_);
                }
                for member in &body.0 {
                    visit(member);
                }
            }
            Node::Let(_, type_, value, _) => {
                if let Some(type_) = type_.as_deref() {
                    visit(type_);
                }
                if let Some(value) = value.as_deref() {
                    visit(value);
                }
            }
            Node::LetDestructure(pattern, type_, value, _) => {
                visit_pattern(&pattern.0, visit);
                if let Some(type_) = type_.as_deref() {
                    visit(type_);
                }
                if let Some(value) = value.as_deref() {
                    visit(value);
                }
            }
            Node::List(items) | Node::Tuple(items) => {
                for item in items {
                    visit(item);
                }
            }
            Node::Match(subject, legs) => {
                visit(subject);
                for (patterns, guard, body) in &legs.0 {
                    for (pattern, _) in patterns {
                        visit_pattern(pattern, visit);
                    }
                    if let Some(guard) = guard.as_deref() {
                        visit(guard);
                    }
                    visit(body);
                }
            }
            Node::MemberAccessor(subject, member) | Node::Index(subject, member) => {
                visit(subject);
                visit(member);
            }
            Node::Module(_, body) => {
                for statement in &body.0 {
                    visit(statement);
                }
            }
            Node::Struct(_, generic_parameters, _, fields) => {
                visit_generic_parameters(generic_parameters, visit);
                for (_, type_, _) in fields
                    .iter()
                    .flat_map(|fields| &fields.0)
                    .map(|field| &field.0)
                {
                    if let Some(type_) = type_ {
                        visit(type_);
                    }
                }
            }
            Node::StructInitializer(_, generic_arguments, fields) => {
                for argument in generic_arguments.iter().flat_map(|arguments| &arguments.0) {
                    visit(argument);
                }
                for (_, value) in fields.0.iter().map(|field| &field.0) {
                    if let Some(value) = value {
                        visit(value);
                    }
                }
            }
            Node::Trait(_, generic_parameters, supertraits, body) => {
                visit_generic_parameters(generic_parameters, visit);
                for supertrait in supertraits {
                    visit(supertrait);
                }
                for member in &body.0 {
                    visit(member);
                }
            }
        }
    }
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
    // Truncated remainder (the dividend's sign) — Rust's and JS's shared
    // semantics. Exact for every integer type, so unlike `Div` it needs no
    // trunc wrap in emission.
    Rem,
    // Bitwise/shift operators (proposal/bits-and-bytes.md §2) — integer-typed,
    // overloadable via `std::operators` like the arithmetic four. Vilan
    // precedence (Rust's order, not C's): `<< >>` over `&` over `^` over `|`,
    // all over comparisons.
    Shl,
    Shr,
    // JS-only: the logical right shift `>>>`. The parser never produces it —
    // the transformer rewrites `Shr` to it when the operand type is `u32`
    // (JS `>>` is arithmetic, which is `i32`'s semantics, not `u32`'s).
    UShr,
    BitAnd,
    BitXor,
    BitOr,
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
