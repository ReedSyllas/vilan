use crate::span::{Span, Spanned};
use crate::token::Token;
use chumsky::prelude::*;

pub fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
    // A parser for numbers
    let number = text::int(10)
        .to_slice()
        .then(just('.').ignore_then(text::digits(10).to_slice()).or_not())
        .map(|(whole, fraction)| Token::Number(whole, fraction));

    // A parser for strings
    let string = just('"')
        .ignore_then(none_of('"').repeated().to_slice())
        .then_ignore(just('"'))
        .map(Token::String);

    // A parser for operators
    let op = one_of("-:!*/+=")
        .repeated()
        .at_least(1)
        .to_slice()
        .map(Token::Op);

    // A parser for control characters (delimiters, semicolons, etc.)
    let ctrl = one_of("()[]{};,.").map(Token::Ctrl);

    // A parser for identifiers and keywords
    let identifier = text::ascii::ident().map(|ident: &str| match ident {
        "else" => Token::Else,
        "false" => Token::Bool(false),
        "fun" => Token::Fun,
        "if" => Token::If,
        "impl" => Token::Impl,
        "import" => Token::Import,
        "let" => Token::Let,
        "null" => Token::Null,
        "ret" => Token::Ret,
        "struct" => Token::Struct,
        "true" => Token::Bool(true),
        _ => Token::Ident(ident),
    });

    // A single token can be one of the above
    let token = choice((number, string, op, ctrl, identifier));

    let comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .padded();

    token
        .map_with(|x, e| (x, e.span()))
        .padded_by(comment.repeated())
        .padded()
        // If we encounter an error, skip and attempt to lex the next character as a token instead
        .recover_with(skip_then_retry_until(any().ignored(), end()))
        .repeated()
        .collect()
}
