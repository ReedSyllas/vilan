use crate::span::{Span, Spanned};
use crate::token::Token;
use chumsky::prelude::*;

pub fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
    // A parser for numbers. A trailing type suffix (`0u32`, `1f`, `2n`, ...)
    // is captured; otherwise the literal's type is inferred from the fractional
    // part or the surrounding context.
    let number = text::int(10)
        .to_slice()
        .then(just('.').ignore_then(text::digits(10).to_slice()).or_not())
        .then(text::ascii::ident().or_not())
        .map(|((whole, fraction), suffix)| Token::Number(whole, fraction, suffix));

    // A parser for strings
    let string = just('"')
        .ignore_then(none_of('"').repeated().to_slice())
        .then_ignore(just('"'))
        .map(Token::String);

    // The match-leg arrow is its own token: `>` is a control character, so the
    // operator charset alone would split `=>` into `=` and `>`.
    let arrow = just("=>").to(Token::Op("=>"));

    // A parser for operators
    let op = one_of("-:!*/+=|&")
        .repeated()
        .at_least(1)
        .to_slice()
        .map(Token::Op);

    // A parser for control characters (delimiters, semicolons, etc.)
    let ctrl = one_of("()[]{}<>;,.").map(Token::Ctrl);

    // A parser for identifiers and keywords
    let identifier = text::ascii::ident().map(|ident: &str| match ident {
        "else" => Token::Else,
        "enum" => Token::Enum,
        "export" => Token::Export,
        "external" => Token::External,
        "false" => Token::Bool(false),
        "for" => Token::For,
        "fun" => Token::Fun,
        "if" => Token::If,
        "impl" => Token::Impl,
        "import" => Token::Import,
        "in" => Token::In,
        "is" => Token::Is,
        "jump" => Token::Jump,
        "let" => Token::Let,
        "match" => Token::Match,
        "mod" => Token::Mod,
        "mut" => Token::Mut,
        "null" => Token::Null,
        "ret" => Token::Ret,
        "struct" => Token::Struct,
        "trait" => Token::Trait,
        "type" => Token::Type,
        "true" => Token::Bool(true),
        "use" => Token::Use,
        "with" => Token::With,
        _ => Token::Ident(ident),
    });

    // A single, self-contained token.
    let single = choice((
        number.clone(),
        string.clone(),
        arrow,
        op.clone(),
        ctrl,
        identifier.clone(),
    ))
    .map_with(|token, e| (token, e.span()));

    // --- Interpolated strings: `i"text {expr} more"` ---
    // An interpolated string is lexed straight into the token stream for an
    // equivalent parenthesised string concatenation, so the embedded
    // expressions are parsed by the normal grammar and the whole thing type
    // checks as `str`:
    //   i"Hello, {name}!"  =>  ("" + "Hello, " + (name) + "!")
    // `\{` and `\}` produce literal braces.
    let interpolated = {
        // Inside an interpolation hole every normal token is allowed except
        // braces, which delimit the hole. (Strings are still lexed whole, so a
        // `}` inside a hole's string literal is not mistaken for the close.)
        let hole_ctrl = one_of("()[]<>;,.").map(Token::Ctrl);
        let hole_token = choice((number, string, op, hole_ctrl, identifier))
            .map_with(|token, e| (token, e.span()))
            .padded();
        // The hole's tokens, wrapped in parentheses so the expression is parsed
        // as a single atom whatever its contents.
        let hole = hole_token
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just('{'), just('}'))
            .map_with(|tokens, e| {
                let span = e.span();
                let mut wrapped = vec![(Token::Ctrl('('), span)];
                wrapped.extend(tokens);
                wrapped.push((Token::Ctrl(')'), span));
                wrapped
            });

        // Literal fragments are captured as source slices so they stay
        // `&'src str`; an escaped brace is just the brace character's slice.
        let escaped_brace = just('\\').ignore_then(one_of("{}").to_slice());
        let backslash = just('\\').to_slice();
        let text = none_of("{}\"\\").repeated().at_least(1).to_slice();

        enum Part<'src> {
            Text(&'src str),
            Hole(Vec<Spanned<Token<'src>>>),
        }

        let part = choice((
            hole.map(Part::Hole),
            escaped_brace.map(Part::Text),
            backslash.map(Part::Text),
            text.map(Part::Text),
        ));

        just('i')
            .ignore_then(just('"'))
            .ignore_then(part.repeated().collect::<Vec<_>>())
            .then_ignore(just('"'))
            .map_with(|parts, e| {
                let span = e.span();
                let mut tokens = vec![(Token::Ctrl('('), span), (Token::String(""), span)];
                for part in parts {
                    tokens.push((Token::Op("+"), span));
                    match part {
                        Part::Text(text) => tokens.push((Token::String(text), span)),
                        Part::Hole(hole_tokens) => tokens.extend(hole_tokens),
                    }
                }
                tokens.push((Token::Ctrl(')'), span));
                tokens
            })
            .boxed()
    };

    let comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .padded();

    // Each lexeme produces one or more tokens — interpolated strings expand to
    // several — which are flattened into a single token stream.
    choice((interpolated, single.map(|token| vec![token])))
        .padded_by(comment.repeated())
        .padded()
        // If we encounter an error, skip and attempt to lex the next character as a token instead
        .recover_with(skip_then_retry_until(any().ignored(), end()))
        .repeated()
        .collect::<Vec<_>>()
        .map(|chunks: Vec<Vec<_>>| chunks.into_iter().flatten().collect())
}
