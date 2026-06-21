use crate::node::{
    BinaryOp, Closure, Convention, ExternBinding, Func, GenericArguments, GenericParameter, If,
    ImportBranch, Node, NodeIfBranch, NodeList, Pattern,
};
use crate::span::{Span, Spanned};
use crate::token::Token;
use chumsky::{input::ValueInput, prelude::*};

// One argument inside a `@extern(..)` attribute — a bare word (`method`/`get`/
// `set`) or a quoted string (a module path or host symbol).
enum ExternArg<'src> {
    Word(&'src str),
    Text(&'src str),
}

/// Stamps a binder pattern's bindings as mutable (or not) — the `let`/`mut` that
/// introduces a binder applies to all of its names, but the binder grammar can't
/// see which keyword preceded it, so it parses them immutable and we fix up here.
fn apply_binding_mutability<'src>(pattern: Pattern<'src>, mutable: bool) -> Pattern<'src> {
    match pattern {
        Pattern::Binding(name, _) => Pattern::Binding(name, mutable),
        Pattern::Tuple(patterns) => Pattern::Tuple(
            patterns
                .into_iter()
                .map(|(pattern, span)| (apply_binding_mutability(pattern, mutable), span))
                .collect(),
        ),
        other => other,
    }
}

/// Interprets a `@extern(..)` attribute's arguments into a host binding.
fn extern_binding_from_args<'src>(args: &[ExternArg<'src>]) -> ExternBinding<'src> {
    use ExternArg::{Text, Word};
    match args {
        [Text(symbol)] => ExternBinding::Function {
            module: None,
            symbol,
        },
        [Text(module), Text(symbol)] => ExternBinding::Function {
            module: Some(module),
            symbol,
        },
        [Word("method")] => ExternBinding::Method { symbol: None },
        [Word("method"), Text(symbol)] => ExternBinding::Method {
            symbol: Some(symbol),
        },
        [Word("get"), Text(symbol)] => ExternBinding::Get { symbol },
        [Word("set"), Text(symbol)] => ExternBinding::Set { symbol },
        // A malformed attribute (author error) lowers to an empty global symbol.
        _ => ExternBinding::Function {
            module: None,
            symbol: "",
        },
    }
}

pub fn parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<NodeList<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let mut statement = Recursive::declare();

    let mut expression = Recursive::declare();

    let mut secondary_expression = Recursive::declare();

    // A comma-delimited list of expressions.
    let expression_list = expression
        .clone()
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>();

    let mut type_ = Recursive::declare();

    let mut if_ = Recursive::declare();

    let mut block = Recursive::declare();

    // The operand of a `*` assignment target (`*<operand> = …`). Declared here and
    // defined as `member_accessor` once that exists, so a view-returning call can
    // sit under the `*` (`*node.slot() = 10`) with the same postfix shape a method
    // call parses to — while a bare name / `.field` chain lowers to the identical
    // place nodes the non-deref target uses.
    let mut place_operand = Recursive::declare();

    let identifier = select! { Token::Ident(text) => text }.labelled("identifier");

    // A name in a declaration or path position. Accepts the boolean literals so
    // the bootstrap `bool` enum (`enum bool { false, true }`) and re-exports of
    // its variants (`bool::{ self, true, false }`) can be written.
    let name = select! {
        Token::Ident(text) => text,
        Token::Bool(true) => "true",
        Token::Bool(false) => "false",
    }
    .labelled("name");

    let literal = select! {
        Token::Null => Node::Null,
        Token::Bool(x) => Node::Bool(x),
        Token::Number(whole, fraction, suffix) => Node::Number(whole, fraction, suffix),
        Token::String(s) => Node::String(s),
    }
    .labelled("value")
    .map_with(|x, e| (x, e.span()));

    // A generic parameter: an optional `type` binder marker, a name, optional
    // `: A + B` bounds, and an optional `= Default`.
    let generic_parameter = just(Token::Type)
        .or_not()
        .then(
            identifier
                .labelled("generic parameter name")
                .map_with(|name, e| (name, e.span())),
        )
        .then(
            just(Token::Op(":"))
                .ignore_then(
                    type_
                        .clone()
                        .separated_by(just(Token::Op("+")))
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .labelled("generic parameter bounds")
                .or_not(),
        )
        .then(
            just(Token::Op("="))
                .ignore_then(type_.clone().map(Box::new))
                .labelled("generic parameter default")
                .or_not(),
        )
        .map(
            |(((type_keyword, (name, name_span)), bounds), default)| GenericParameter {
                name,
                name_span,
                is_type: type_keyword.is_some(),
                bounds: bounds.unwrap_or_default(),
                default,
            },
        );

    let generic_parameters = generic_parameter
        .labelled("generic parameter")
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Ctrl('<')), just(Token::Ctrl('>')))
        .map_with(|generic_parameters, e| (generic_parameters, e.span()))
        .recover_with(via_parser(nested_delimiters(
            Token::Ctrl('<'),
            Token::Ctrl('>'),
            [
                (Token::Ctrl('('), Token::Ctrl(')')),
                (Token::Ctrl('['), Token::Ctrl(']')),
                (Token::Ctrl('{'), Token::Ctrl('}')),
            ],
            |span| (Vec::new(), span),
        )))
        .labelled("generic parameters")
        .boxed();

    let generic_arguments = type_
        .clone()
        .labelled("generic argument")
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Ctrl('<')), just(Token::Ctrl('>')))
        .map_with(|generic_arguments, e| (generic_arguments, e.span()))
        .recover_with(via_parser(nested_delimiters(
            Token::Ctrl('<'),
            Token::Ctrl('>'),
            [
                (Token::Ctrl('('), Token::Ctrl(')')),
                (Token::Ctrl('['), Token::Ctrl(']')),
                (Token::Ctrl('{'), Token::Ctrl('}')),
            ],
            |span| (Vec::new(), span),
        )))
        .labelled("generic arguments")
        .boxed();

    let struct_initializer_field = identifier
        .then(
            just(Token::Op("="))
                .ignore_then(expression.clone())
                .or_not(),
        )
        .map_with(|(name, value), e| ((name, value), e.span()));

    let struct_initializer = identifier
        .then(generic_arguments.clone().or_not())
        .then(
            struct_initializer_field
                .separated_by(just(Token::Ctrl(',')))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map_with(|fields, e| (Some(fields), e.span()))
                .recover_with(via_parser(nested_delimiters(
                    Token::Ctrl('{'),
                    Token::Ctrl('}'),
                    [
                        (Token::Ctrl('('), Token::Ctrl(')')),
                        (Token::Ctrl('['), Token::Ctrl(']')),
                    ],
                    |span| (None, span),
                )))
                .map(|x| (x.0.unwrap_or_else(|| Vec::new()), x.1)),
        )
        .map_with(|((name, generic_arguments), fields), e| {
            (
                Node::StructInitializer(name, generic_arguments, fields),
                e.span(),
            )
        })
        .labelled("struct initializer")
        .boxed();

    let local = identifier.map_with(|x, e| (Node::Accessor(x), e.span()));

    let local_type =
        identifier
            .then(generic_arguments.clone())
            .map_with(|(name, generic_arguments), e| {
                (
                    Node::AccessorWithGenerics(name, generic_arguments),
                    e.span(),
                )
            });

    let list = expression_list
        .clone()
        .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')))
        .map_with(|x, e| (Node::List(x), e.span()));

    let tuple = expression
        .clone()
        .separated_by(just(Token::Ctrl(',')))
        .at_least(2)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
        .map_with(|x, e| (Node::Tuple(x), e.span()));

    // 'Atoms' are expressions that contain no ambiguity
    let atom = choice((
        literal,
        local,
        local_type.clone(),
        list,
        tuple,
        expression
            .clone()
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))),
    ))
    // Attempt to recover anything that looks like a parenthesised expression but contains errors
    .recover_with(via_parser(nested_delimiters(
        Token::Ctrl('('),
        Token::Ctrl(')'),
        [
            (Token::Ctrl('['), Token::Ctrl(']')),
            (Token::Ctrl('{'), Token::Ctrl('}')),
        ],
        |span| (Node::Error, span),
    )))
    // Attempt to recover anything that looks like a list but contains errors
    .recover_with(via_parser(nested_delimiters(
        Token::Ctrl('['),
        Token::Ctrl(']'),
        [
            (Token::Ctrl('('), Token::Ctrl(')')),
            (Token::Ctrl('{'), Token::Ctrl('}')),
        ],
        |span| (Node::Error, span),
    )))
    .boxed();

    // A binder in `let`/parameter position: a plain name, or a tuple of binders
    // (irrefutable destructuring — `let (a, b) = pair`, `|(a, b)|`). Distinct from
    // the `match` pattern grammar (no variants/literals/guards). Nests recursively.
    let binder = recursive(|binder| {
        let tuple = binder
            .separated_by(just(Token::Ctrl(',')))
            .at_least(2)
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
            .map_with(|patterns, e| (Pattern::Tuple(patterns), e.span()));
        let name = identifier.map_with(|name, e| (Pattern::Binding(name, false), e.span()));
        choice((tuple, name))
    });

    let closure = binder
        .clone()
        .labelled("parameter")
        .then(
            just(Token::Op(":"))
                .ignore_then(type_.clone().map(|x| Box::new(x)))
                .labelled("parameter type")
                .or_not(),
        )
        // Closure parameters carry no view convention yet.
        .map(|((pattern, pattern_span), parameter_type)| {
            (pattern, parameter_type, Convention::Bare, pattern_span)
        })
        .labelled("parameter")
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Op("|")), just(Token::Op("|")))
        .or(just(Token::Op("||")).map(|_| Vec::new()))
        .map_with(|parameters, e| (parameters, e.span()))
        .labelled("closure parameters")
        .then(
            just(Token::Op(":"))
                .ignore_then(type_.clone().map(|x| Box::new(x)))
                .labelled("return type")
                .or_not(),
        )
        .then(expression.clone())
        .map_with(|((parameters, return_type), return_value), e| {
            (
                Node::Closure(Closure {
                    parameters,
                    return_type,
                    return_value: Box::new(return_value),
                }),
                e.span(),
            )
        })
        .labelled("closure")
        .boxed();

    // Blocks are a sequence of statements delimited with braces.
    block.define(
        statement
            .clone()
            .repeated()
            .collect::<Vec<_>>()
            .then(expression.clone().map(|x| Box::new(x)).or_not())
            .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
            .map_with(|(statements, expression), e| {
                let span: Span = e.span();
                (
                    Some((
                        statements,
                        expression.unwrap_or_else(|| Box::new((Node::Void, span.to_end()))),
                    )),
                    span,
                )
            })
            .recover_with(via_parser(nested_delimiters(
                Token::Ctrl('{'),
                Token::Ctrl('}'),
                [
                    (Token::Ctrl('('), Token::Ctrl(')')),
                    (Token::Ctrl('['), Token::Ctrl(']')),
                ],
                |span| (None, span),
            )))
            .map_with(|x, e| {
                let span: Span = e.span();
                (
                    x.0.unwrap_or_else(|| (Vec::new(), Box::new((Node::Void, span.to_end())))),
                    x.1,
                )
            }),
    );

    // A `::`-separated namespace path ending in a name or a `{ a, b }` set,
    // shared by `import` and `use`.
    let namespace_path = recursive(|branch| {
        let path = name
            .map_with(|name, e| (name, e.span()))
            .then(just(Token::Op("::")).ignore_then(branch).or_not())
            .map(|((name, span), b)| ImportBranch::Path(name, span, b.map(|b| Box::new(b))));

        path.clone().or(path
            .separated_by(just(Token::Ctrl(',')))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
            .map(|x| ImportBranch::Set(x)))
    })
    .boxed();

    let import = just(Token::Import)
        .ignore_then(namespace_path.clone())
        .map_with(|import_path, e| (Node::Import(import_path), e.span()))
        .boxed();

    let use_ = just(Token::Use)
        .ignore_then(namespace_path)
        .map_with(|use_path, e| (Node::Use(use_path), e.span()))
        .labelled("use")
        .boxed();

    if_.define(
        recursive(|if_| {
            just(Token::If)
                .ignore_then(secondary_expression.clone().labelled("condition"))
                .then(block.clone())
                .then(
                    just(Token::Else)
                        .ignore_then(
                            block
                                .clone()
                                .map_with(|x, e| (NodeIfBranch::Else(x), e.span()))
                                .or(if_),
                        )
                        .or_not(),
                )
                .map_with(|((cond, a), b), e| {
                    (
                        NodeIfBranch::If(Box::new(If {
                            condition: Box::new(cond),
                            then: a,
                            else_: b,
                        })),
                        e.span(),
                    )
                })
        })
        .map(|x| match x {
            (NodeIfBranch::If(x), span) => (Node::If(NodeIfBranch::If(Box::new(*x))), span),
            _ => unreachable!(),
        })
        .labelled("if block"),
    );

    // A binder in `let`/parameter position: a plain name, or a tuple of binders
    // (irrefutable destructuring — `let (a, b) = pair`). Distinct from the `match`
    // pattern grammar (no variants, literals, or guards). Nests via recursion.
    let let_ = choice((just(Token::Let).to(false), just(Token::Mut).to(true)))
        .then(binder.clone())
        .then(
            just(Token::Op(":"))
                .ignore_then(type_.clone())
                .labelled("type")
                .or_not(),
        )
        .then(
            just(Token::Op("="))
                .ignore_then(expression.clone())
                .labelled("value")
                .or_not(),
        )
        .map_with(|(((mutable, (pattern, pattern_span)), type_), val), e| {
            let type_ = type_.map(Box::new);
            let val = val.map(Box::new);
            // A bare name keeps the simple `Let`; a tuple destructures.
            let node = match pattern {
                Pattern::Binding(name, _) => Node::Let((name, pattern_span), type_, val, mutable),
                pattern => Node::LetDestructure((pattern, pattern_span), type_, val, mutable),
            };
            (node, e.span())
        })
        .labelled("let binding")
        .boxed();

    // An assignment target is an lvalue: a local (`x`), a field place (`self.n`,
    // `a.b.c`), or a deref through a view (`*v`). A `.field` chain folds into
    // `MemberAccessor`s, the same shape a field read parses to.
    // A bare assignment-target suffix: `.field` or `[index]`.
    // An assignment target is `(*)? <place>` where the place is the postfix /
    // precedence expression (`place_operand` = `chained`): a local (`x`), a
    // `.field` chain (`a.b.c`), a subscript (`list[i]`), or a place reached through
    // a call (`a.write().count`, `*node.slot()`). A bare name / field chain lowers
    // to the same `Accessor`/`MemberAccessor`/`Index` nodes as before (`local` is
    // `Accessor`), so existing targets are byte-identical; `*` derefs the result.
    let assignment_target = just(Token::Op("*"))
        .or_not()
        .then(place_operand.clone())
        .map_with(|(deref, place), e| match deref {
            Some(_) => (Node::Dereference(Box::new(place)), e.span()),
            None => place,
        });
    let assignment = assignment_target
        .then(choice((
            just(Token::Op("=")).to(None),
            just(Token::Op("+=")).to(Some(BinaryOp::Add)),
            just(Token::Op("-=")).to(Some(BinaryOp::Sub)),
            just(Token::Op("*=")).to(Some(BinaryOp::Mul)),
            just(Token::Op("/=")).to(Some(BinaryOp::Div)),
        )))
        .then(expression.clone())
        .map_with(|((target, op), value), e| {
            (
                Node::Assign(Box::new(target), op, Box::new(value)),
                e.span(),
            )
        })
        .labelled("assignment")
        .boxed();

    // `jump` is a namespace for the loop-control keywords; the target keyword
    // (`break`, `continue`, ...) follows it.
    let jump = just(Token::Jump)
        .ignore_then(identifier.labelled("jump target"))
        .map_with(|target, e| (Node::Jump(target), e.span()))
        .labelled("jump")
        .boxed();

    // `for` covers every loop. With no condition it is an infinite loop
    // (`for { .. }`); with one it is a while loop (`for cond { .. }`). The body
    // is always the final block, so an infinite loop is tried first to avoid
    // mistaking its block for a condition.
    // `for item in iterable { .. }` — iterate over an `Iterable`. Tried before
    // the condition/infinite forms since `item in ..` starts with an identifier.
    let for_in = just(Token::For)
        .ignore_then(identifier.labelled("loop variable"))
        .then_ignore(just(Token::In))
        .then(
            secondary_expression
                .clone()
                .labelled("iterable")
                .map(Box::new),
        )
        .then(block.clone())
        .map_with(|((variable, iterable), body), e| {
            (Node::ForIn(variable, iterable, body), e.span())
        });

    let for_loop = just(Token::For)
        .ignore_then(choice((
            block.clone().map(|body| (None, body)),
            secondary_expression
                .clone()
                .labelled("loop condition")
                .then(block.clone())
                .map(|(condition, body)| (Some(Box::new(condition)), body)),
        )))
        .map_with(|(condition, body), e| (Node::For(condition, body), e.span()));

    let for_ = choice((for_in, for_loop)).labelled("for loop").boxed();

    // An explicit integer discriminant: `= 0` or `= -1`.
    let discriminant = just(Token::Op("="))
        .ignore_then(just(Token::Op("-")).or_not().map(|sign| sign.is_some()))
        .then(select! { Token::Number(whole, _fraction, _suffix) => whole })
        .map(|(negative, whole)| {
            let magnitude = whole.parse::<i64>().unwrap_or(0);
            if negative { -magnitude } else { magnitude }
        });

    let enum_variant = name
        .labelled("variant name")
        .then(
            type_
                .clone()
                .separated_by(just(Token::Ctrl(',')))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
                .or_not(),
        )
        .then(discriminant.or_not())
        .map_with(|((name, data), discriminant), e| {
            ((name, data.unwrap_or_default(), discriminant), e.span())
        });

    let enum_ = just(Token::Enum)
        .ignore_then(
            identifier
                .labelled("enum name")
                .map_with(|name, e| (name, e.span())),
        )
        .then(generic_parameters.clone().or_not())
        .then(
            enum_variant
                .separated_by(just(Token::Ctrl(',')))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map_with(|variants, e| (variants, e.span())),
        )
        .map_with(|((name, generic_parameters), variants), e| {
            (Node::Enum(name, generic_parameters, variants), e.span())
        })
        .labelled("enum")
        .boxed();

    // A match-leg pattern: `_`, `let x` / `mut x`, a literal (`"quit"`, `42`), or
    // a variant (`Some(let x)`, qualified `Signal::Quit`).
    let pattern = recursive(|pattern| {
        // `let x` / `mut x`, or a destructuring binder `let (a, b)` — the binder
        // grammar (a name or a tuple of binders) is shared with `let`/parameters.
        let binding = choice((just(Token::Let).to(false), just(Token::Mut).to(true)))
            .then(binder.clone())
            .map(|(mutable, (pattern, _))| apply_binding_mutability(pattern, mutable));
        // `(a, b, ...)` — a tuple pattern (at least two elements, to keep a
        // single parenthesised pattern unambiguous as grouping).
        let tuple = pattern
            .clone()
            .separated_by(just(Token::Ctrl(',')))
            .at_least(2)
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
            .map(Pattern::Tuple);
        // A string/number literal pattern — matched by equality. (`bool`/`null`
        // stay variant/keyword patterns, resolved against their enum.)
        let literal_pattern = select! {
            Token::String(s) => Node::String(s),
            Token::Number(whole, fraction, suffix) => Node::Number(whole, fraction, suffix),
        }
        .map_with(|node, e| Pattern::Literal(Box::new((node, e.span()))));
        // A variant path (`Name`, `Enum::Variant`) with optional payload patterns.
        let variant = name
            .then(
                just(Token::Op("::"))
                    .ignore_then(identifier)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then(
                pattern
                    .separated_by(just(Token::Ctrl(',')))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
                    .or_not(),
            )
            .map(|((head, rest), payload)| {
                if rest.is_empty() && head == "_" && payload.is_none() {
                    Pattern::Wildcard
                } else {
                    let mut path = vec![head];
                    path.extend(rest);
                    Pattern::Variant(path, payload)
                }
            });
        choice((binding, tuple, literal_pattern, variant)).map_with(|x, e| (x, e.span()))
    })
    .labelled("pattern")
    .boxed();

    // A leg: one or more comma-separated patterns (an or-pattern), an optional
    // `if` guard, then `=> body`.
    let match_leg = pattern
        .clone()
        .separated_by(just(Token::Ctrl(',')))
        .at_least(1)
        .collect::<Vec<_>>()
        .then(
            just(Token::If)
                .ignore_then(expression.clone().labelled("match guard"))
                .or_not(),
        )
        .then_ignore(just(Token::Op("=>")))
        .then(expression.clone().labelled("match leg body"))
        .map(|((patterns, guard), body)| (patterns, guard.map(Box::new), body));

    let match_ = just(Token::Match)
        .ignore_then(secondary_expression.clone().labelled("match subject"))
        .then(
            // Each arm may be followed by a comma; it's optional, so the idiomatic
            // no-comma-after-a-`{ }`-block style (`Some(x) => { .. } None => 0`)
            // parses as well as the comma-separated one. An ambiguous omission —
            // an expression-bodied arm whose value would absorb the next arm's
            // pattern as a postfix (`=> f(a) (b) => ..`) — still errors, so a
            // missing comma is never a silent mis-parse.
            match_leg
                .then_ignore(just(Token::Ctrl(',')).or_not())
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map_with(|legs, e| (legs, e.span())),
        )
        .map_with(|(subject, legs), e| (Node::Match(Box::new(subject), legs), e.span()))
        .labelled("match expression")
        .boxed();

    // `@extern("node:http", "createServer")` / `@extern(method)` / `@extern(get,
    // "statusCode")` — the host binding for the `external` function that follows.
    let extern_attribute = just(Token::Ctrl('@'))
        .ignore_then(select! { Token::Ident("extern") => () }.labelled("`extern`"))
        .ignore_then(
            choice((
                select! { Token::Ident(word) => ExternArg::Word(word) },
                select! { Token::String(text) => ExternArg::Text(text) },
            ))
            .separated_by(just(Token::Ctrl(',')))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))),
        )
        .map(|args| extern_binding_from_args(&args))
        .labelled("extern attribute")
        .boxed();

    let function = extern_attribute
        .or_not()
        .then(just(Token::Async).or_not().map(|async_| async_.is_some()))
        .then(
            just(Token::External)
                .or_not()
                .map(|external| external.is_some()),
        )
        .then_ignore(just(Token::Fun))
        .then(
            identifier
                .map_with(|name, e| (name, e.span()))
                .labelled("function name"),
        )
        .then(generic_parameters.clone().or_not())
        .then(
            // A `own` / `&` / `&mut` prefix gives the parameter's convention
            // (`own x`, `&self`, `&mut self`); a `&T` / `&mut T` type does too.
            // Bare is the default.
            choice((
                just(Token::Own).to(Convention::Own),
                just(Token::Op("&"))
                    .ignore_then(just(Token::Mut).or_not())
                    .map(|mutable| {
                        if mutable.is_some() {
                            Convention::RefMut
                        } else {
                            Convention::Ref
                        }
                    }),
            ))
            .or_not()
            .then(binder.clone().labelled("parameter name"))
            .then(
                just(Token::Op(":"))
                    .ignore_then(type_.clone().map(|x| Box::new(x)))
                    .labelled("parameter type")
                    .or_not(),
            )
            .map(|((prefix, (pattern, pattern_span)), parameter_type)| {
                // A prefix wins; otherwise a `&T` / `&mut T` type gives the
                // convention; otherwise bare.
                let convention = prefix.unwrap_or_else(|| {
                    match parameter_type.as_deref().map(|spanned| &spanned.0) {
                        Some(Node::Reference(true, _)) => Convention::RefMut,
                        Some(Node::Reference(false, _)) => Convention::Ref,
                        _ => Convention::Bare,
                    }
                });
                (pattern, parameter_type, convention, pattern_span)
            })
            .labelled("parameter")
            .separated_by(just(Token::Ctrl(',')))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
            .map_with(|parameters, e| (parameters, e.span()))
            .labelled("function parameters"),
        )
        .then(
            just(Token::Op(":"))
                .ignore_then(type_.clone().map(|x| Box::new(x)))
                .labelled("return type")
                .or_not(),
        )
        .then(
            // `borrows <param>` — the returned view is a projection of that
            // parameter, so it may escape (rule 3's one sanctioned case).
            just(Token::Borrows)
                .ignore_then(identifier)
                .labelled("borrows clause")
                .or_not(),
        )
        .then(
            // A function either has a block body or, for a signature-only
            // declaration (e.g. a required trait method), ends with `;`.
            block
                .clone()
                .map(Some)
                .or(just(Token::Ctrl(';')).map(|_| None))
                .labelled("function body"),
        )
        .map_with(
            |(
                (
                    (
                        (
                            ((((extern_binding, is_async), external), name), generic_parameters),
                            parameters,
                        ),
                        return_type,
                    ),
                    borrows,
                ),
                body,
            ),
             e| {
                (
                    Node::Func(Func {
                        name,
                        is_async,
                        external,
                        extern_binding,
                        generic_parameters,
                        parameters,
                        return_type,
                        borrows,
                        body,
                    }),
                    e.span(),
                )
            },
        )
        .labelled("function")
        .boxed();

    let return_ = just(Token::Ret)
        .ignore_then(expression.clone())
        .map_with(|x, e| (Node::FuncReturn(Box::new(x)), e.span()));

    let struct_field = identifier
        .then(just(Token::Op(":")).ignore_then(type_.clone()).or_not())
        .map_with(|(name, type_), e| ((name, type_), e.span()));

    let struct_ = just(Token::External)
        .or_not()
        .map(|external| external.is_some())
        .then_ignore(just(Token::Struct))
        // `null` is a keyword but also the name of the built-in `external struct
        // null`, so the struct name accepts it alongside ordinary identifiers.
        .then(
            choice((identifier, just(Token::Null).to("null")))
                .labelled("struct name")
                .map_with(|name, e| (name, e.span())),
        )
        .then(generic_parameters.clone().or_not())
        .then(choice((
            // `struct Name { field: T, ... }`
            struct_field
                .separated_by(just(Token::Ctrl(',')))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map_with(|fields, e| (Some(fields), e.span()))
                .recover_with(via_parser(nested_delimiters(
                    Token::Ctrl('{'),
                    Token::Ctrl('}'),
                    [
                        (Token::Ctrl('('), Token::Ctrl(')')),
                        (Token::Ctrl('['), Token::Ctrl(']')),
                    ],
                    |span| (None, span),
                )))
                .map(|x| Some((x.0.unwrap_or_else(Vec::new), x.1))),
            // `struct Name;` — a bodyless declaration (only valid for an
            // `external` struct, e.g. a primitive like `external struct str;`).
            just(Token::Ctrl(';')).map(|_| None),
        )))
        .map_with(|(((external, name), generic_parameters), body), e| {
            (
                Node::Struct(name, generic_parameters, external, body),
                e.span(),
            )
        })
        .labelled("struct")
        .boxed();

    let impl_ = just(Token::Impl)
        // The subject is a type pattern. `type X` binders in it (anywhere —
        // `impl List<type T>`, `impl Option<(type T, type U)>`, or a bare
        // blanket `impl type T`) declare the impl's generic parameters.
        .ignore_then(type_.clone().labelled("implementation subject"))
        .then(
            just(Token::With)
                .ignore_then(
                    type_
                        .clone()
                        .labelled("implemented trait")
                        .separated_by(just(Token::Op("+")))
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .or_not()
                .map(|traits| traits.unwrap_or_default()),
        )
        .then(
            statement
                .clone()
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map_with(|statements, e| (statements, e.span()))
                .recover_with(via_parser(nested_delimiters(
                    Token::Ctrl('{'),
                    Token::Ctrl('}'),
                    [
                        (Token::Ctrl('('), Token::Ctrl(')')),
                        (Token::Ctrl('['), Token::Ctrl(']')),
                    ],
                    |span| (Vec::new(), span),
                ))),
        )
        .map_with(|((subject, traits), body), e| {
            (Node::Impl(Box::new(subject), traits, body), e.span())
        })
        .boxed();

    let trait_ = just(Token::Trait)
        .ignore_then(
            identifier
                .labelled("trait name")
                .map_with(|name, e| (name, e.span())),
        )
        .then(generic_parameters.clone().or_not())
        .then(
            just(Token::With)
                .ignore_then(
                    type_
                        .clone()
                        .labelled("supertrait")
                        .separated_by(just(Token::Op("+")))
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .or_not()
                .map(|supertraits| supertraits.unwrap_or_default()),
        )
        .then(
            function
                .clone()
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map_with(|items, e| (items, e.span()))
                .recover_with(via_parser(nested_delimiters(
                    Token::Ctrl('{'),
                    Token::Ctrl('}'),
                    [
                        (Token::Ctrl('('), Token::Ctrl(')')),
                        (Token::Ctrl('['), Token::Ctrl(']')),
                    ],
                    |span| (Vec::new(), span),
                ))),
        )
        .map_with(|(((name, generic_parameters), supertraits), body), e| {
            (
                Node::Trait(name, generic_parameters, supertraits, body),
                e.span(),
            )
        })
        .labelled("trait")
        .boxed();

    let module = just(Token::Mod)
        .ignore_then(identifier.labelled("module name"))
        .then(
            statement
                .clone()
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map_with(|statements, e| (statements, e.span()))
                .recover_with(via_parser(nested_delimiters(
                    Token::Ctrl('{'),
                    Token::Ctrl('}'),
                    [
                        (Token::Ctrl('('), Token::Ctrl(')')),
                        (Token::Ctrl('['), Token::Ctrl(']')),
                    ],
                    |span| (Vec::new(), span),
                ))),
        )
        .map_with(|(name, body), e| (Node::Module(name, body), e.span()))
        .boxed();

    // The operator-precedence expression (calls, member/static access,
    // arithmetic, comparison), then the `is` pattern test, then `&&`.
    let chained = chain_expr_parser(
        identifier,
        generic_arguments.clone(),
        expression_list.clone(),
        expression.clone(),
        atom.clone(),
        block
            .clone()
            .map(|(x, span)| (Node::Block((x, span)), span)),
    );
    // The `*` assignment-target operand is the postfix/precedence expression, so a
    // view-returning call (`*node.slot() = 10`) is recognized; a bare name or
    // `.field` chain still produces the same `Accessor`/`MemberAccessor` nodes as
    // the non-deref place (`local` is `Accessor`), keeping codegen byte-identical.
    place_operand.define(chained.clone());
    let is_expression = chained
        .then(just(Token::Is).ignore_then(pattern.clone()).or_not())
        .map_with(|(subject, matched), e| match matched {
            Some(matched) => (Node::Is(Box::new(subject), Box::new(matched)), e.span()),
            None => subject,
        })
        .boxed();
    let logical_and = is_expression
        .clone()
        .foldl_with(
            just(Token::Op("&&")).ignore_then(is_expression).repeated(),
            |a, b, e| {
                (
                    Node::Binary(BinaryOp::And, Box::new(a), Box::new(b)),
                    e.span(),
                )
            },
        )
        .boxed();

    secondary_expression.define(choice((
        closure,
        block
            .clone()
            .map(|(x, span)| (Node::Block((x, span)), span)),
        if_.clone(),
        for_.clone(),
        match_.clone(),
        jump,
        let_,
        return_,
        assignment,
        logical_and,
    )));

    // A struct literal may be the subject of a `.field` access or `.method()`
    // call (`Point { x = 1, y = 2 }.length()`). Struct literals are parsed only
    // at the top expression level (conditions use `secondary_expression`, so
    // `if Foo { .. }` stays unambiguous); this folds any trailing member chain
    // onto the literal there. Each member is a field name or a call, mirroring
    // the postfix shape `MemberAccessor` resolves.
    //
    // Consequence: a struct literal can't be an operator operand
    // (`Point { .. } == x`) — bind it to a variable first. Lifting that needs a
    // `no-struct-literal` expression mode for condition positions (as in Rust),
    // i.e. a second binary/operator chain that excludes struct literals — a larger
    // change, deferred.
    let struct_member = identifier
        .map_with(|name, e| (Node::Accessor(name), e.span()))
        .then(
            expression_list
                .clone()
                .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
                .map_with(|args, e| (args, e.span()))
                .or_not(),
        )
        .map_with(|(accessor, args), e| match args {
            Some(args) => (Node::Call(Box::new(accessor), None, args), e.span()),
            None => accessor,
        });
    let struct_initializer_expression = struct_initializer.clone().foldl_with(
        just(Token::Ctrl('.')).ignore_then(struct_member).repeated(),
        |subject, member, e| {
            (
                Node::MemberAccessor(Box::new(subject), Box::new(member)),
                e.span(),
            )
        },
    );

    expression.define(
        choice((struct_initializer_expression, secondary_expression))
            .labelled("expression")
            .as_context(),
    );

    // `export <statement>` — re-export an import or expose a declaration.
    let export_ = just(Token::Export)
        .ignore_then(statement.clone())
        .map_with(|inner, e| (Node::Export(Box::new(inner)), e.span()))
        .labelled("export");

    // A block-like expression (`if`/`for`/`match`/`{ .. }`) may be used as a
    // statement, but only when it isn't the last thing in its block — i.e. a
    // non-`}` token follows. When it *is* last, it falls through to the block's
    // trailing expression and so becomes the block's value (e.g. a function
    // whose body is a single `match`).
    let not_block_end = just(Token::Ctrl('}')).not();

    // `@derive(A, B) struct/enum …` — a derive attribute wrapping a struct or
    // enum, recorded as `Node::Derive`; a pre-analysis pass synthesizes the named
    // trait impls from the item's fields.
    let derive_attribute = just(Token::Ctrl('@'))
        .ignore_then(select! { Token::Ident("derive") => () }.labelled("`derive`"))
        .ignore_then(
            identifier
                .separated_by(just(Token::Ctrl(',')))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))),
        )
        .labelled("derive attribute");
    let derived_item = derive_attribute
        .then(choice((struct_.clone(), enum_.clone())))
        .map_with(|(derives, item), e| (Node::Derive(derives, Box::new(item)), e.span()))
        .boxed();

    statement.define(choice((
        derived_item,
        export_,
        expression.clone().then_ignore(just(Token::Ctrl(';'))),
        if_.then_ignore(not_block_end.clone()),
        for_.then_ignore(not_block_end.clone()),
        match_.then_ignore(not_block_end.clone()),
        function,
        struct_,
        enum_,
        impl_,
        trait_,
        module,
        import.then_ignore(just(Token::Ctrl(';'))),
        use_.then_ignore(just(Token::Ctrl(';'))),
        block
            .clone()
            .map(|(x, span)| (Node::Block((x, span)), span))
            .then_ignore(not_block_end),
    )));

    let tuple_type = type_
        .clone()
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
        .map_with(|x, e| (Node::Tuple(x), e.span()));

    let closure_type = identifier
        .labelled("closure type parameter name")
        .then_ignore(just(Token::Op(":")))
        .or_not()
        .then(
            type_
                .clone()
                .map(|x| Box::new(x))
                .labelled("closure type parameter type"),
        )
        .labelled("closure type parameter")
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Op("|")), just(Token::Op("|")))
        .or(just(Token::Op("||")).map(|_| Vec::new()))
        .map_with(|parameters, e| (parameters, e.span()))
        .labelled("closure type parameters")
        .then(
            type_
                .clone()
                .map(|x| Box::new(x))
                .labelled("closure type return type")
                .or_not(),
        )
        .map_with(|(parameters, return_type), e| {
            (Node::ClosureType(parameters, return_type), e.span())
        })
        .labelled("closure type")
        .boxed();

    // A `type X` generic binder in type position, with optional `: A + B`
    // bounds. Only meaningful in an impl subject pattern (`impl Option<type T>`,
    // `impl Option<(type T, type U)>`), where it declares the impl's generics.
    let type_binder = just(Token::Type)
        .ignore_then(identifier.labelled("type binder name"))
        .then(
            just(Token::Op(":"))
                .ignore_then(
                    type_
                        .clone()
                        .separated_by(just(Token::Op("+")))
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .or_not()
                .map(|bounds| bounds.unwrap_or_default()),
        )
        .map_with(|(name, bounds), e| (Node::TypeBinder(name, bounds), e.span()));

    // `&T` / `&mut T` — a view type. Lowers to the inner type for now (identity);
    // a parameter captures the `&`/`&mut` as its convention.
    let reference_type = just(Token::Op("&"))
        .ignore_then(just(Token::Mut).or_not())
        .then(type_.clone())
        .map_with(|(mutable, inner), e| {
            (
                Node::Reference(mutable.is_some(), Box::new(inner)),
                e.span(),
            )
        });

    // `local_type` (e.g. `FromFn<T>`) must come before the plain identifier so
    // generic arguments are consumed as part of the type. `type_binder` is first
    // so the `type` keyword isn't mistaken for an identifier.
    type_.define(choice((
        reference_type,
        type_binder,
        closure_type,
        local_type,
        local,
        tuple_type,
    )));

    statement
        .clone()
        .repeated()
        .collect::<Vec<_>>()
        .map_with(|children, e| (children, e.span()))
}

fn chain_expr_parser<'tokens, 'src: 'tokens, I>(
    identifier: impl Parser<'tokens, I, &'src str, extra::Err<Rich<'tokens, Token<'src>, Span>>>
    + Copy
    + 'tokens,
    generic_arguments: impl Parser<
        'tokens,
        I,
        GenericArguments<'src>,
        extra::Err<Rich<'tokens, Token<'src>, Span>>,
    > + Clone
    + 'tokens,
    expression_list: impl Parser<
        'tokens,
        I,
        NodeList<'src>,
        extra::Err<Rich<'tokens, Token<'src>, Span>>,
    > + Clone
    + 'tokens,
    // A single expression, for a subscript index (`list[i]`).
    expression: impl Parser<
        'tokens,
        I,
        Spanned<Node<'src>>,
        extra::Err<Rich<'tokens, Token<'src>, Span>>,
    > + Clone
    + 'tokens,
    atom: impl Parser<'tokens, I, Spanned<Node<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>>
    + Clone
    + 'tokens,
    // A `{ .. }` block already mapped to a `Node::Block`, for `async { .. }`.
    block_expr: impl Parser<
        'tokens,
        I,
        Spanned<Node<'src>>,
        extra::Err<Rich<'tokens, Token<'src>, Span>>,
    > + Clone
    + 'tokens,
) -> impl Parser<'tokens, I, Spanned<Node<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    // `Name<Args>` is the head of a `::` path only when a `::` actually
    // follows (e.g. `List<str>::new()`); the trailing `::` is matched with a
    // lookahead so a generic *call* like `default<Id>()` is left untouched.
    let generic_static_head = identifier
        .then(generic_arguments.clone())
        .then_ignore(just(Token::Op("::")).rewind())
        .map_with(|(name, generic_arguments), e| {
            (
                Node::AccessorWithGenerics(name, generic_arguments),
                e.span(),
            )
        });

    let static_accessor = generic_static_head
        .or(atom.clone())
        .foldl_with(
            just(Token::Op("::")).ignore_then(identifier).repeated(),
            |subject, member_name, e| {
                (
                    Node::StaticAccessor(Box::new(subject), member_name),
                    e.span(),
                )
            },
        )
        .boxed();

    let call = static_accessor
        .foldl_with(
            generic_arguments
                .or_not()
                .then(
                    expression_list
                        .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
                        .map_with(|args, e| (args, e.span())),
                )
                .repeated(),
            |f, (generic_arguments, args), e| {
                (Node::Call(Box::new(f), generic_arguments, args), e.span())
            },
        )
        .boxed();

    // A postfix suffix: `.member` or `[index]`, folded left onto the subject.
    enum Postfix<'src> {
        Member(Spanned<Node<'src>>),
        Index(Spanned<Node<'src>>),
    }
    let member_accessor = call
        .clone()
        .foldl_with(
            choice((
                // A trailing `.` with no member yet (`p.`, mid-edit) recovers to an
                // `Error` member rather than failing the whole statement — so the
                // receiver still analyzes, which the language server's member
                // completion relies on. A complete `a.b` always takes the `Some`
                // path, so valid programs parse identically.
                just(Token::Ctrl('.'))
                    .map_with(|_, e| e.span())
                    .then(call.or_not())
                    .map(|(dot_span, member)| {
                        Postfix::Member(member.unwrap_or((Node::Error, dot_span)))
                    }),
                // `[index]` — a `List` subscript.
                expression
                    .clone()
                    .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')))
                    .map(Postfix::Index),
            ))
            .repeated(),
            |subject, postfix, e| match postfix {
                Postfix::Member(member) => (
                    Node::MemberAccessor(Box::new(subject), Box::new(member)),
                    e.span(),
                ),
                Postfix::Index(index) => {
                    (Node::Index(Box::new(subject), Box::new(index)), e.span())
                }
            },
        )
        .boxed();

    // Unary prefix operators, binding tighter than the binary ops: `!` (logical
    // not), `await` (resolve a promise), and `async` (spawn a promise). `async`
    // takes a block (`async { .. }`) or any unary expression (`async fetch(x)`).
    let unary = recursive(|unary| {
        choice((
            just(Token::Op("!"))
                .ignore_then(unary.clone())
                .map_with(|expr, e| (Node::Unary('!', Box::new(expr)), e.span())),
            // Prefix `-` (negation). Binary subtract lives in `sum` below, applied
            // after this, so `a - b` is still a subtraction and `-1` / `f(-x)` parse.
            just(Token::Op("-"))
                .ignore_then(unary.clone())
                .map_with(|expr, e| (Node::Unary('-', Box::new(expr)), e.span())),
            just(Token::Await)
                .ignore_then(unary.clone())
                .map_with(|expr, e| (Node::Await(Box::new(expr)), e.span())),
            just(Token::Async)
                .ignore_then(choice((block_expr.clone(), unary.clone())))
                .map_with(|expr, e| (Node::Async(Box::new(expr)), e.span())),
            // `&x` / `&mut x` — take a view of a place. `*x` — deref a view.
            // Prefix `*` is unambiguous against binary `*` (multiply), which only
            // appears between two operands in `product`.
            just(Token::Op("&"))
                .ignore_then(just(Token::Mut).or_not())
                .then(unary.clone())
                .map_with(|(mutable, expr), e| {
                    (Node::Reference(mutable.is_some(), Box::new(expr)), e.span())
                }),
            just(Token::Op("*"))
                .ignore_then(unary.clone())
                .map_with(|expr, e| (Node::Dereference(Box::new(expr)), e.span())),
            member_accessor.clone(),
        ))
        .boxed()
    });

    // Product ops (multiply and divide) have equal precedence
    let op = just(Token::Op("*"))
        .to(BinaryOp::Mul)
        .or(just(Token::Op("/")).to(BinaryOp::Div));
    let product = unary
        .clone()
        .foldl_with(op.then(unary).repeated(), |a, (op, b), e| {
            (Node::Binary(op, Box::new(a), Box::new(b)), e.span())
        })
        .boxed();

    // Sum ops (add and subtract) have equal precedence
    let op = just(Token::Op("+"))
        .to(BinaryOp::Add)
        .or(just(Token::Op("-")).to(BinaryOp::Sub));
    let sum = product
        .clone()
        .foldl_with(op.then(product).repeated(), |a, (op, b), e| {
            (Node::Binary(op, Box::new(a), Box::new(b)), e.span())
        })
        .boxed();

    // Comparison ops have equal precedence. `<` and `>` are control tokens
    // (also used for generics), and `<=`/`>=` lex as `<`/`>` followed by `=`.
    let op = choice((
        just(Token::Op("==")).to(BinaryOp::Eq),
        just(Token::Op("!=")).to(BinaryOp::NotEq),
        just(Token::Ctrl('<'))
            .ignore_then(just(Token::Op("=")).or_not())
            .map(|eq| {
                if eq.is_some() {
                    BinaryOp::LtEq
                } else {
                    BinaryOp::Lt
                }
            }),
        just(Token::Ctrl('>'))
            .ignore_then(just(Token::Op("=")).or_not())
            .map(|eq| {
                if eq.is_some() {
                    BinaryOp::GtEq
                } else {
                    BinaryOp::Gt
                }
            }),
    ));
    let compare = sum
        .clone()
        .foldl_with(op.then(sum).repeated(), |a, (op, b), e| {
            (Node::Binary(op, Box::new(a), Box::new(b)), e.span())
        })
        .boxed();

    compare
}
