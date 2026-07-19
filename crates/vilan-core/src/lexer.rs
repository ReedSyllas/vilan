use crate::span::{Span, Spanned};
use crate::token::Token;
use chumsky::extra::ParserExtra;
use chumsky::label::LabelError;
use chumsky::prelude::*;
use chumsky::text::TextExpected;
use chumsky::util::MaybeRef;

/// The lexer with full `Rich` diagnostics — the error-path instantiation.
/// A clean compile never needs these: lex/parse with [`crate::parse_clean`]
/// first and fall back here only when it fails.
pub fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
    lexer_with()
}

/// The lexer, generic over its error type — see [`crate::parser::parser_with`]
/// for why. The `TextExpected` bound is what `text::int`/`text::ident` demand.
pub fn lexer_with<'src, E>() -> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, E>
where
    E: ParserExtra<'src, &'src str> + 'src,
    E::Error: LabelError<'src, &'src str, TextExpected<()>>
        + LabelError<'src, &'src str, MaybeRef<'src, char>>,
{
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

    // A triple-quoted string: RAW (a backslash is just a backslash — the appeal
    // is pasting code verbatim) and multi-line; the whitespace before the
    // closing delimiter is stripped from every line by
    // `util::trim_multiline_string` (validated in the analyzer, trimmed in the
    // transformer). Content runs to the FIRST `\"\"\"`. Must be tried before
    // `string`, or `\"\"\"` lexes as an empty string plus a stray quote.
    let multiline_string = just("\"\"\"")
        .ignore_then(any().and_is(just("\"\"\"").not()).repeated().to_slice())
        .then_ignore(just("\"\"\""))
        .map(Token::MultilineString);

    // The match-leg arrow is its own token: `>` is a control character, so the
    // operator charset alone would split `=>` into `=` and `>`.
    let arrow = just("=>").to(Token::Op("=>"));

    // A parser for operators. `^` is bitwise-xor; `<`/`>` are control tokens
    // (generics), so the shifts have no token here — the parser reads two
    // adjacent `<`/`>` controls in expression position.
    //
    // The known operators are matched longest-first, so a two-character operator
    // (`+=`, `==`, `::`, `&&`, …) wins over its single-character prefix. A blind
    // maximal munch over the operator charset instead FUSED unrelated adjacent
    // prefixes into one bogus token — `!*v` (negate a deref), `!!b` (double
    // negation), `-*v` — which then failed to parse; recognizing the real set
    // keeps those as separate tokens. (`=>` is the `arrow` token above; `?.` and
    // `..` split on the `.` control character, so neither is listed here.)
    let op = choice((
        just("!=").to(Token::Op("!=")),
        just("%=").to(Token::Op("%=")),
        just("&&").to(Token::Op("&&")),
        just("*=").to(Token::Op("*=")),
        just("+=").to(Token::Op("+=")),
        just("-=").to(Token::Op("-=")),
        just("/=").to(Token::Op("/=")),
        just("::").to(Token::Op("::")),
        just("==").to(Token::Op("==")),
        just("||").to(Token::Op("||")),
        one_of("-:!*/+=|&^?%").to_slice().map(Token::Op),
    ));

    // A parser for control characters (delimiters, semicolons, etc.). Attributes
    // use bracket syntax (`[extern(..)]`, `[derive(..)]`), so they need no special
    // character.
    let ctrl = one_of("()[]{}<>;,.").map(Token::Ctrl);

    // A parser for identifiers and keywords
    let identifier = text::ascii::ident().map(|ident: &str| match ident {
        "async" => Token::Async,
        "await" => Token::Await,
        "const" => Token::Const,
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
        "macro" => Token::Macro,
        "match" => Token::Match,
        "mod" => Token::Mod,
        "mut" => Token::Mut,
        "null" => Token::Null,
        "own" => Token::Own,
        "borrows" => Token::Borrows,
        "ret" => Token::Ret,
        "resource" => Token::Resource,
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
        multiline_string.clone(),
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
        let hole_token = choice((
            hex,
            number,
            multiline_string,
            string,
            op,
            hole_ctrl,
            identifier,
        ))
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
    //
    // ORDER AND GREED MATTER: whitespace is consumed as a RUN, and before the
    // comment alternative. The previous shape (comment first, then ONE
    // whitespace char per iteration) was quadratic in a whitespace run's
    // length — each iteration's comment attempt `.padded()`-scanned the whole
    // remaining run before failing on `//` — which made lexing a macro
    // WORLD's blanked file (mostly spaces by construction) take seconds.
    let trivia = any()
        .filter(|c: &char| c.is_whitespace())
        .repeated()
        .at_least(1)
        .ignored()
        .or(comment.clone().ignored())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> Vec<Token<'_>> {
        let (tokens, errors) = lexer().parse(source).into_output_errors();
        assert!(errors.is_empty(), "lex errors: {errors:?}");
        tokens
            .expect("lexing produced no output")
            .into_iter()
            .map(|(token, _span)| token)
            .collect()
    }

    #[test]
    fn trivia_only_files_lex_to_empty_streams() {
        assert!(lex("").is_empty());
        assert!(lex("   \n\t \n").is_empty());
        assert!(lex("// just a comment").is_empty());
        assert!(lex("  // padded comment\n   ").is_empty());
        assert!(lex("// one\n// two\n").is_empty());
    }

    #[test]
    fn leading_trivia_interleavings_reach_the_first_token() {
        let expected = vec![Token::Fun, Token::Ident("main")];
        assert_eq!(lex("fun main"), expected);
        assert_eq!(lex("   \n\n  fun main"), expected);
        assert_eq!(lex("// header\nfun main"), expected);
        assert_eq!(lex("  \n// a\n  // b\n\n  fun main"), expected);
    }

    #[test]
    fn trivia_between_and_after_tokens() {
        assert_eq!(
            lex("fun // trailing comment\n   main   // eof comment"),
            vec![Token::Fun, Token::Ident("main")]
        );
    }

    #[test]
    fn comment_at_end_of_input_without_newline() {
        assert_eq!(lex("main // no newline"), vec![Token::Ident("main")]);
    }

    // Macro-world files are BLANKED source: every byte outside a kept span
    // becomes a space, so a world's text is dominated by huge whitespace runs
    // (often tens of kilobytes before the first token). Lexing must stay
    // linear in such runs — a comment-first trivia loop that consumed one
    // whitespace char per iteration was quadratic here and took seconds per
    // world. A quadratic lexer takes hours on this input; the generous bound
    // only guards against that class.
    #[test]
    fn huge_whitespace_runs_lex_in_linear_time() {
        let mut source = String::new();
        for _ in 0..20_000 {
            source.push_str("                                                \n");
        }
        source.push_str("fun main");
        let start = std::time::Instant::now();
        assert_eq!(lex(&source), vec![Token::Fun, Token::Ident("main")]);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "lexing a ~1MB whitespace prefix took {:?} — the trivia loop has gone quadratic again",
            start.elapsed()
        );
    }

    #[test]
    fn adjacent_prefix_operators_do_not_fuse() {
        // The operator lexer recognizes the real operator set rather than
        // maximal-munching the charset, so a `!`/`-` prefix chained onto `*`/`!`
        // stays two tokens — `!*v` (negate a deref), `!!b` (double negation),
        // `-*v`. The blind munch fused these into bogus `!*` / `!!` / `-*` tokens
        // that then failed to parse (`found '!*' expected expression`).
        assert_eq!(
            lex("!*v"),
            vec![Token::Op("!"), Token::Op("*"), Token::Ident("v")]
        );
        assert_eq!(
            lex("!!b"),
            vec![Token::Op("!"), Token::Op("!"), Token::Ident("b")]
        );
        assert_eq!(
            lex("-*v"),
            vec![Token::Op("-"), Token::Op("*"), Token::Ident("v")]
        );
    }

    #[test]
    fn multi_character_operators_win_over_their_prefixes() {
        // Longest-match: each two-character operator lexes whole, not as its
        // single-character prefix. (A regression to per-char lexing would split
        // every one of these.)
        for op in ["!=", "%=", "&&", "*=", "+=", "-=", "/=", "::", "==", "||"] {
            assert_eq!(lex(op), vec![Token::Op(op)], "lexing {op:?}");
        }
        // The boundary holds mid-stream: `x-=-y` is `x`, `-=`, `-`, `y` — the
        // compound assignment wins, then a separate prefix `-`.
        assert_eq!(
            lex("x-=-y"),
            vec![
                Token::Ident("x"),
                Token::Op("-="),
                Token::Op("-"),
                Token::Ident("y"),
            ]
        );
    }
}
