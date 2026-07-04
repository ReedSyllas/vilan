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

    // A hex integer literal (`0xFF`, `0x80000000u32`, `0xDEADn`) — an integer in
    // another spelling, kept verbatim as the whole part (valid JS as-is). Must
    // precede `number`, which would otherwise read `0xFF` as `0` with suffix
    // `xFF`. The digit munch is maximal, so `f` in `0xFFf` is a digit — a type
    // suffix must start with a non-hex letter (`u32`, `n`).
    let hex = just("0x")
        .then(one_of("0123456789abcdefABCDEF").repeated().at_least(1))
        .to_slice()
        .then(text::ascii::ident().or_not())
        .map(|(whole, suffix)| Token::Number(whole, None, suffix));

    // A parser for strings. A backslash escapes the next character, so `\"` and
    // `\\` don't terminate the string; the raw (still-escaped) slice is kept and
    // the escapes are interpreted at code generation.
    let string_char = choice((just('\\').then(any()).ignored(), none_of("\"\\").ignored()));
    let string = just('"')
        .ignore_then(string_char.repeated().to_slice())
        .then_ignore(just('"'))
        .map(Token::String);

    // The match-leg arrow is its own token: `>` is a control character, so the
    // operator charset alone would split `=>` into `=` and `>`.
    let arrow = just("=>").to(Token::Op("=>"));

    // A parser for operators. `^` is bitwise-xor; `<`/`>` are control tokens
    // (generics), so the shifts have no token here — the parser reads two
    // adjacent `<`/`>` controls in expression position.
    let op = one_of("-:!*/+=|&^?")
        .repeated()
        .at_least(1)
        .to_slice()
        .map(Token::Op);

    // A parser for control characters (delimiters, semicolons, etc.). Attributes
    // use bracket syntax (`[extern(..)]`, `[derive(..)]`), so they need no special
    // character.
    let ctrl = one_of("()[]{}<>;,.").map(Token::Ctrl);

    // A parser for identifiers and keywords
    let identifier = text::ascii::ident().map(|ident: &str| match ident {
        "async" => Token::Async,
        "await" => Token::Await,
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
        "own" => Token::Own,
        "borrows" => Token::Borrows,
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
        hex.clone(),
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
        let hole_token = choice((hex, number, string, op, hole_ctrl, identifier))
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
        // `&'src str`. `\{`/`\}` collapse to the brace itself; any other escape
        // (`\n`, `\"`, `\\`) is kept raw and interpreted at code generation, like
        // a plain string.
        let escaped_brace = just('\\').ignore_then(one_of("{}").to_slice());
        let escape = just('\\').then(none_of("{}")).to_slice();
        let text = none_of("{}\"\\").repeated().at_least(1).to_slice();

        enum Part<'src> {
            Text(&'src str),
            Hole(Vec<Spanned<Token<'src>>>),
        }

        let part = choice((
            hole.map(Part::Hole),
            escaped_brace.map(Part::Text),
            escape.map(Part::Text),
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

    // Comments and whitespace are trivia, normally consumed as padding around
    // tokens. A file that is *only* trivia (blank lines, a lone comment) has no
    // token for that padding to attach to, so consume a leading run of trivia up
    // front — leaving a clean, possibly empty, token stream rather than leaking
    // the trivia into the parser.
    let trivia = comment
        .clone()
        .ignored()
        .or(any().filter(|c: &char| c.is_whitespace()).ignored())
        .repeated();

    // Each lexeme produces one or more tokens — interpolated strings expand to
    // several — which are flattened into a single token stream.
    trivia.ignore_then(
        choice((interpolated, single.map(|token| vec![token])))
            .padded_by(comment.repeated())
            .padded()
            // If we encounter an error, skip and attempt to lex the next character as a token instead
            .recover_with(skip_then_retry_until(any().ignored(), end()))
            .repeated()
            .collect::<Vec<_>>()
            .map(|chunks: Vec<Vec<_>>| chunks.into_iter().flatten().collect()),
    )
}
