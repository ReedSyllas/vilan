use crate::node::{BinaryOp, Func, If, IfElseBranch, ImportBranch, Node, NodeList};
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

    let struct_initializer_field = identifier
        .then(
            just(Token::Op("="))
                .ignore_then(expression.clone())
                .or_not(),
        )
        .map_with(|(name, value), e| ((name, value), e.span()));

    let struct_initializer = identifier
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
                .map_with(|x, e| (x.0.unwrap_or_else(|| Vec::new()), x.1)),
        )
        .map_with(|(name, fields), e| (Node::StructInitializer(name, fields), e.span()))
        .labelled("struct initializer")
        .boxed();

    let local = identifier.map_with(|x, e| (Node::Local(x), e.span()));

    let list = expression_list
        .clone()
        .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')))
        .map_with(|x, e| (Node::List(x), e.span()));

    let tuple = expression
        .clone()
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
        .map_with(|x, e| (Node::Tuple(x), e.span()));

    // 'Atoms' are expressions that contain no ambiguity
    let atom = choice((
        struct_initializer,
        literal,
        local,
        list,
        tuple,
        if_.clone(),
        block
            .clone()
            .map(|(x, span)| (Node::Block((x, span)), span)),
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

    // Function calls have very high precedence so we prioritize them
    let call = atom.foldl_with(
        expression_list
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
            .map_with(|args, e| (args, e.span()))
            .repeated(),
        |f, args, e| (Node::Call(Box::new(f), args), e.span()),
    );

    let property = call.foldl_with(
        just(Token::Ctrl('.')).ignore_then(identifier).repeated(),
        |subject, property, e| (Node::Member(Box::new(subject), property), e.span()),
    );

    // Product ops (multiply and divide) have equal precedence
    let op = just(Token::Op("*"))
        .to(BinaryOp::Mul)
        .or(just(Token::Op("/")).to(BinaryOp::Div));
    let product = property
        .clone()
        .foldl_with(op.then(property).repeated(), |a, (op, b), e| {
            (Node::Binary(op, Box::new(a), Box::new(b)), e.span())
        });

    // Sum ops (add and subtract) have equal precedence
    let op = just(Token::Op("+"))
        .to(BinaryOp::Add)
        .or(just(Token::Op("-")).to(BinaryOp::Sub));
    let sum = product
        .clone()
        .foldl_with(op.then(product).repeated(), |a, (op, b), e| {
            (Node::Binary(op, Box::new(a), Box::new(b)), e.span())
        });

    // Comparison ops (equal, not-equal) have equal precedence
    let op = just(Token::Op("=="))
        .to(BinaryOp::Eq)
        .or(just(Token::Op("!=")).to(BinaryOp::NotEq));
    let compare = sum
        .clone()
        .foldl_with(op.then(sum).repeated(), |a, (op, b), e| {
            (Node::Binary(op, Box::new(a), Box::new(b)), e.span())
        });

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
                .ignore_then(expression.clone())
                .then(block.clone())
                .then(
                    just(Token::Else)
                        .ignore_then(
                            block
                                .clone()
                                .map_with(|x, e| (IfElseBranch::Else(x), e.span()))
                                .or(if_),
                        )
                        .or_not(),
                )
                .map_with(|((cond, a), b), e| {
                    (
                        IfElseBranch::If(Box::new(If {
                            condition: Box::new(cond),
                            then: a,
                            else_: b,
                        })),
                        e.span(),
                    )
                })
        })
        .map(|x| match x {
            (IfElseBranch::If(x), span) => (Node::If(*x), span),
            _ => unreachable!(),
        })
        .labelled("if block"),
    );

    let let_ = just(Token::Let)
        .ignore_then(identifier)
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
        .map_with(|((name, type_), val), e| {
            (
                Node::Let(name, type_.map(|x| Box::new(x)), val.map(|x| Box::new(x))),
                e.span(),
            )
        })
        .labelled("let binding")
        .boxed();

    let function = just(Token::Fun)
        .ignore_then(
            identifier
                .map_with(|name, e| (name, e.span()))
                .labelled("function name"),
        )
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
        .then(block.clone())
        .map_with(|(((name, parameters), return_type), body), e| {
            (
                Node::Func(Func {
                    name,
                    return_type,
                    parameters,
                    body,
                }),
                e.span(),
            )
        })
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
                .map_with(|x, e| (x.0.unwrap_or_else(|| Vec::new()), x.1)),
        )
        .map_with(|(name, body), e| (Node::Struct(name, body), e.span()))
        .labelled("struct")
        .boxed();

    expression.define(choice((
        compare.labelled("expression").as_context(),
        let_,
        return_,
    )));

    statement.define(choice((
        expression.clone().then_ignore(just(Token::Ctrl(';'))),
        if_,
        function,
        struct_,
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

    type_.define(choice((local, tuple_type)));

    statement
        .clone()
        .repeated()
        .collect::<Vec<_>>()
        .map_with(|children, e| (children, e.span()))
}
