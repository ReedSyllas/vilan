use crate::node::{
    BinaryOp, Closure, Func, GenericArguments, If, ImportBranch, Node, NodeIfBranch, NodeList,
};
use crate::span::{Span, Spanned};
use crate::token::Token;
use chumsky::{input::ValueInput, prelude::*};

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

    let identifier = select! { Token::Ident(text) => text }.labelled("identifier");

    let literal = select! {
        Token::Null => Node::Null,
        Token::Bool(x) => Node::Bool(x),
        Token::Number(whole, fraction) => Node::Number(whole, fraction),
        Token::String(s) => Node::String(s),
    }
    .labelled("value")
    .map_with(|x, e| (x, e.span()));

    let generic_parameters = identifier
        .labelled("generic parameter name")
        .then(
            just(Token::Op(":"))
                .ignore_then(type_.clone())
                .labelled("generic parameter type")
                .or_not(),
        )
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

    let closure = identifier
        .labelled("parameter name")
        .then(
            just(Token::Op(":"))
                .ignore_then(type_.clone().map(|x| Box::new(x)))
                .labelled("parameter type")
                .or_not(),
        )
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

    let import = just(Token::Import)
        .ignore_then(recursive(|branch| {
            let path = identifier
                .then(just(Token::Op("::")).ignore_then(branch).or_not())
                .map(|(a, b)| ImportBranch::Path(a, b.map(|b| Box::new(b))));

            path.clone().or(path
                .separated_by(just(Token::Ctrl(',')))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
                .map(|x| ImportBranch::Set(x)))
        }))
        .map_with(|import_path, e| (Node::Import(import_path), e.span()))
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

    let let_ = choice((just(Token::Let).to(false), just(Token::Mut).to(true)))
        .then(identifier)
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
        .map_with(|(((mutable, name), type_), val), e| {
            (
                Node::Let(
                    name,
                    type_.map(|x| Box::new(x)),
                    val.map(|x| Box::new(x)),
                    mutable,
                ),
                e.span(),
            )
        })
        .labelled("let binding")
        .boxed();

    let assignment = identifier
        .then(choice((
            just(Token::Op("=")).to(None),
            just(Token::Op("+=")).to(Some(BinaryOp::Add)),
            just(Token::Op("-=")).to(Some(BinaryOp::Sub)),
            just(Token::Op("*=")).to(Some(BinaryOp::Mul)),
            just(Token::Op("/=")).to(Some(BinaryOp::Div)),
        )))
        .then(expression.clone())
        .map_with(|((name, op), value), e| (Node::Assign(name, op, Box::new(value)), e.span()))
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
    let for_ = just(Token::For)
        .ignore_then(choice((
            block.clone().map(|body| (None, body)),
            secondary_expression
                .clone()
                .labelled("loop condition")
                .then(block.clone())
                .map(|(condition, body)| (Some(Box::new(condition)), body)),
        )))
        .map_with(|(condition, body), e| (Node::For(condition, body), e.span()))
        .labelled("for loop")
        .boxed();

    let function = just(Token::Fun)
        .ignore_then(
            identifier
                .map_with(|name, e| (name, e.span()))
                .labelled("function name"),
        )
        .then(generic_parameters.clone().or_not())
        .then(
            identifier
                .labelled("parameter name")
                .then(
                    just(Token::Op(":"))
                        .ignore_then(type_.clone().map(|x| Box::new(x)))
                        .labelled("parameter type")
                        .or_not(),
                )
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
            // A function either has a block body or, for a signature-only
            // declaration (e.g. a required trait method), ends with `;`.
            block
                .clone()
                .map(Some)
                .or(just(Token::Ctrl(';')).map(|_| None))
                .labelled("function body"),
        )
        .map_with(
            |((((name, generic_parameters), parameters), return_type), body), e| {
                (
                    Node::Func(Func {
                        name,
                        generic_parameters,
                        parameters,
                        return_type,
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

    let struct_ = just(Token::Struct)
        .ignore_then(identifier.labelled("struct name"))
        .then(generic_parameters.clone().or_not())
        .then(
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
                .map(|x| (x.0.unwrap_or_else(|| Vec::new()), x.1)),
        )
        .map_with(|((name, generic_parameters), body), e| {
            (Node::Struct(name, generic_parameters, body), e.span())
        })
        .labelled("struct")
        .boxed();

    let impl_ = just(Token::Impl)
        // The subject is a plain type name; any `<...>` after it declares the
        // impl's generic parameters (`impl List<T: str>`), so it must not be
        // parsed as a generic type usage here.
        .ignore_then(
            identifier
                .map_with(|name, e| (Node::Accessor(name), e.span()))
                .labelled("implementation subject"),
        )
        .then(generic_parameters.clone().or_not())
        .then(
            just(Token::With)
                .ignore_then(type_.clone().labelled("implemented trait"))
                .or_not(),
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
        .map_with(|(((subject, generic_parameters), trait_), body), e| {
            (
                Node::Impl(
                    Box::new(subject),
                    generic_parameters,
                    trait_.map(|x| Box::new(x)),
                    body,
                ),
                e.span(),
            )
        })
        .boxed();

    let trait_ = just(Token::Trait)
        .ignore_then(identifier.labelled("trait name"))
        .then(generic_parameters.clone().or_not())
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
        .map_with(|((name, generic_parameters), body), e| {
            (Node::Trait(name, generic_parameters, body), e.span())
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

    secondary_expression.define(choice((
        closure,
        block
            .clone()
            .map(|(x, span)| (Node::Block((x, span)), span)),
        if_.clone(),
        for_.clone(),
        jump,
        let_,
        return_,
        assignment,
        chain_expr_parser(identifier, generic_arguments, expression_list, atom),
    )));

    expression.define(
        choice((struct_initializer.clone(), secondary_expression))
            .labelled("expression")
            .as_context(),
    );

    statement.define(choice((
        expression.clone().then_ignore(just(Token::Ctrl(';'))),
        if_,
        for_,
        function,
        struct_,
        impl_,
        trait_,
        module,
        import.then_ignore(just(Token::Ctrl(';'))),
        block.map(|(x, span)| (Node::Block((x, span)), span)),
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

    // `local_type` (e.g. `FromFn<T>`) must come before the plain identifier so
    // generic arguments are consumed as part of the type.
    type_.define(choice((closure_type, local_type, local, tuple_type)));

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
    atom: impl Parser<'tokens, I, Spanned<Node<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>>
    + Clone
    + 'tokens,
) -> impl Parser<'tokens, I, Spanned<Node<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let static_accessor = atom
        .clone()
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

    let member_accessor = call
        .clone()
        .foldl_with(
            just(Token::Ctrl('.')).ignore_then(call).repeated(),
            |subject, member, e| {
                (
                    Node::MemberAccessor(Box::new(subject), Box::new(member)),
                    e.span(),
                )
            },
        )
        .boxed();

    // Product ops (multiply and divide) have equal precedence
    let op = just(Token::Op("*"))
        .to(BinaryOp::Mul)
        .or(just(Token::Op("/")).to(BinaryOp::Div));
    let product = member_accessor
        .clone()
        .foldl_with(op.then(member_accessor).repeated(), |a, (op, b), e| {
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
