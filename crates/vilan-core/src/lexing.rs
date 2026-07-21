//! The handwritten lexer (H6 S1, `proposal/frontend.md` §2).
//!
//! A dependency-free, single-pass scan over `&str` producing `Vec<(Token, Span)>`
//! byte-identical — spans included — to the chumsky lexer in `lexer.rs`, which
//! stays in-tree as the oracle for the whole H6 arc (deleted at S5). Nothing in
//! the pipeline calls this yet; it is exercised only by the differential and unit
//! tests. At S5 this module replaces `lexer.rs` and [`tokenize`] takes over from
//! `lexer()`.
//!
//! The behaviour reproduced here (keyword classification, longest-match operators,
//! `<`/`>` always `Ctrl`, `=>` an `Op`, numeric-literal shape, in-lexer i-string
//! desugaring, linear trivia skipping, and the exact span each token carries) is a
//! faithful copy of the chumsky lexer, quirks included — the differential is the
//! referee, not any judgement about what a token or span *should* be. Ugly-but-
//! reproduced behaviours are recorded for the S4/S5 error-quality pass, not fixed.

use crate::span::{Span, Spanned};
use crate::token::Token;

/// A lexing error: the byte offset and the character the lexer could not turn into
/// a token. S1 records these but does not yet reproduce chumsky's error *messages*
/// (that is S4's concern); the differential compares token streams, not errors.
///
/// One error is recorded per un-lexable character. The chumsky lexer coalesces a
/// run of consecutive un-lexable characters into a single diagnostic — a
/// difference in error *count*, not in the token stream, deferred to S4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub position: usize,
    pub character: char,
}

/// Lex `source` into its token stream (with spans) and any lexing errors. The
/// token stream is byte-identical to `lexer().parse(source)` for every source the
/// H6 differential covers.
pub fn tokenize(source: &str) -> (Vec<Spanned<Token<'_>>>, Vec<LexError>) {
    let mut lexer = Lexer::new(source);
    lexer.run();
    (lexer.tokens, lexer.errors)
}

fn span(start: usize, end: usize) -> Span {
    (start..end).into()
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_operator_byte(byte: u8) -> bool {
    // The operator charset, exactly `one_of("-:!*/+=|&^?%")` in `lexer.rs`.
    matches!(
        byte,
        b'-' | b':' | b'!' | b'*' | b'/' | b'+' | b'=' | b'|' | b'&' | b'^' | b'?' | b'%'
    )
}

fn is_control_byte(byte: u8) -> bool {
    // `one_of("()[]{}<>;,.")` — `<`/`>` are control tokens (the parser reassembles
    // span-adjacent pairs into shifts), and `.` splits `?.`/`..` apart.
    matches!(
        byte,
        b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'<' | b'>' | b';' | b',' | b'.'
    )
}

/// The control charset *inside* an interpolation hole: the top-level set minus the
/// braces, which delimit the hole (`one_of("()[]<>;,.")` in `lexer.rs`).
fn is_hole_control_byte(byte: u8) -> bool {
    is_control_byte(byte) && byte != b'{' && byte != b'}'
}

struct Lexer<'src> {
    source: &'src str,
    bytes: &'src [u8],
    position: usize,
    tokens: Vec<Spanned<Token<'src>>>,
    errors: Vec<LexError>,
}

impl<'src> Lexer<'src> {
    fn new(source: &'src str) -> Self {
        Lexer {
            source,
            bytes: source.as_bytes(),
            position: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn run(&mut self) {
        loop {
            self.skip_trivia();
            if self.position >= self.bytes.len() {
                break;
            }
            self.lex_one();
        }
    }

    /// Skip a maximal run of trivia — whitespace and `//` line comments,
    /// interleaved — leaving `position` on the next token (or at end). Linear in
    /// the run's length (the pinned property: a quadratic trivia loop once made a
    /// blanked macro world take seconds).
    fn skip_trivia(&mut self) {
        loop {
            self.skip_whitespace();
            if self.bytes[self.position..].starts_with(b"//") {
                self.position += 2;
                // A comment runs to (not including) the next newline; the newline
                // is left for the whitespace pass. `\n` is ASCII, so scanning bytes
                // never lands inside a multi-byte character.
                while self.position < self.bytes.len() && self.bytes[self.position] != b'\n' {
                    self.position += 1;
                }
                continue;
            }
            break;
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(character) = self.current_char() {
            if character.is_whitespace() {
                self.position += character.len_utf8();
            } else {
                break;
            }
        }
    }

    /// The character at `position` (which is always a UTF-8 boundary), or `None` at
    /// end of input.
    fn current_char(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    /// Lex one lexeme at `position`, which is neither trivia nor end of input.
    fn lex_one(&mut self) {
        let start = self.position;
        let first = self.bytes[start];
        if first == b'i' && self.bytes.get(start + 1) == Some(&b'"') {
            self.lex_interpolated();
        } else if first == b'"' {
            match self.read_string(start) {
                Some((token, end)) => self.push(token, start, end),
                // An unterminated string: skip the opening quote and retry, exactly
                // as chumsky's skip-then-retry recovery does (`"unterminated` lexes
                // the tail as identifiers).
                None => self.skip_illegal(),
            }
        } else if first.is_ascii_digit() {
            let (token, end) = self.read_number(start);
            self.push(token, start, end);
        } else if is_ident_start(first) {
            let (token, end) = self.read_identifier(start);
            self.push(token, start, end);
        } else if is_operator_byte(first) {
            let (token, end) = self.read_operator(start, true);
            self.push(token, start, end);
        } else if is_control_byte(first) {
            self.push(Token::Ctrl(first as char), start, start + 1);
        } else {
            self.skip_illegal();
        }
    }

    fn push(&mut self, token: Token<'src>, start: usize, end: usize) {
        self.tokens.push((token, span(start, end)));
        self.position = end;
    }

    /// Record an un-lexable character and step over it (one whole char), leaving the
    /// rest of the stream to be lexed — chumsky's `skip_then_retry_until` recovery.
    fn skip_illegal(&mut self) {
        let character = self.current_char().expect("skip_illegal called at end");
        self.errors.push(LexError {
            position: self.position,
            character,
        });
        self.position += character.len_utf8();
    }

    // --- Lexeme readers (pure: they compute a token and its end, never mutate) ---

    /// A numeric literal: a hex integer (`0x…`) or a decimal with an optional
    /// fraction, each with an optional identifier-shaped type suffix. `start` is a
    /// digit.
    fn read_number(&self, start: usize) -> (Token<'src>, usize) {
        // Hex (`0x` + at least one hex digit) is tried first, so `0xFF` is not read
        // as `0` with suffix `xFF`. `0X` (capital) and `0x` with no hex digit fall
        // through to the decimal path (`0` with an identifier suffix).
        if self.bytes[start] == b'0'
            && self.bytes.get(start + 1) == Some(&b'x')
            && self
                .bytes
                .get(start + 2)
                .is_some_and(|byte| byte.is_ascii_hexdigit())
        {
            let mut position = start + 2;
            while self
                .bytes
                .get(position)
                .is_some_and(|byte| byte.is_ascii_hexdigit())
            {
                position += 1;
            }
            let whole = &self.source[start..position];
            let (suffix, end) = self.read_optional_suffix(position);
            return (Token::Number(whole, None, suffix), end);
        }

        // Decimal integer part: a lone `0`, or `[1-9]` followed by any digits (the
        // no-leading-zero rule of `text::int`, so `007` is three `0`,`0`,`7`).
        let mut position = if self.bytes[start] == b'0' {
            start + 1
        } else {
            let mut position = start;
            while self.bytes.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
            position
        };
        let whole = &self.source[start..position];

        // A fraction (`.` then at least one digit). A `.` not followed by a digit
        // is left as a control token (`1.` is `1` then `.`).
        let mut fraction = None;
        if self.bytes.get(position) == Some(&b'.')
            && self.bytes.get(position + 1).is_some_and(u8::is_ascii_digit)
        {
            let fraction_start = position + 1;
            position += 1;
            while self.bytes.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
            fraction = Some(&self.source[fraction_start..position]);
        }

        let (suffix, end) = self.read_optional_suffix(position);
        (Token::Number(whole, fraction, suffix), end)
    }

    /// An optional identifier-shaped type suffix (`u32`, `f`, `n`, `_000`, …)
    /// starting at `position`. Returns the suffix slice (or `None`) and the new end.
    /// The suffix is a raw identifier slice — never keyword-classified — so `1if` is
    /// `Number("1", None, Some("if"))`, matching `text::ascii::ident().or_not()`.
    fn read_optional_suffix(&self, position: usize) -> (Option<&'src str>, usize) {
        if self
            .bytes
            .get(position)
            .is_some_and(|&byte| is_ident_start(byte))
        {
            let end = self.identifier_end(position);
            (Some(&self.source[position..end]), end)
        } else {
            (None, position)
        }
    }

    fn identifier_end(&self, start: usize) -> usize {
        let mut position = start + 1;
        while self
            .bytes
            .get(position)
            .is_some_and(|&byte| is_ident_continue(byte))
        {
            position += 1;
        }
        position
    }

    /// An identifier or keyword. `start` is an identifier-start byte.
    fn read_identifier(&self, start: usize) -> (Token<'src>, usize) {
        let end = self.identifier_end(start);
        let text = &self.source[start..end];
        let token = match text {
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
            _ => Token::Ident(text),
        };
        (token, end)
    }

    /// An operator. `start` is an operator-charset byte. `allow_arrow` selects `=>`
    /// as its own token (`Token::Op("=>")`); inside an interpolation hole `=>` is
    /// not a token, so `=` and `>` split (`>` is a hole control character).
    ///
    /// Longest-match: the two-character operators win over their one-character
    /// prefixes, so `!*v` stays `!`,`*`,`v` and `&&&` is `&&`,`&`.
    fn read_operator(&self, start: usize, allow_arrow: bool) -> (Token<'src>, usize) {
        let first = self.bytes[start];
        let second = self.bytes.get(start + 1).copied();
        let two_characters = match (first, second) {
            (b'=', Some(b'>')) => allow_arrow,
            (b'!', Some(b'=')) => true,
            (b'%', Some(b'=')) => true,
            (b'&', Some(b'&')) => true,
            (b'*', Some(b'=')) => true,
            (b'+', Some(b'=')) => true,
            (b'-', Some(b'=')) => true,
            (b'/', Some(b'=')) => true,
            (b':', Some(b':')) => true,
            (b'=', Some(b'=')) => true,
            (b'|', Some(b'|')) => true,
            _ => false,
        };
        let end = if two_characters { start + 2 } else { start + 1 };
        (Token::Op(&self.source[start..end]), end)
    }

    /// A string literal. `start` is `"`. Returns the token and its end, or `None`
    /// if the string is unterminated. A triple-quoted `"""…"""` is tried first (it
    /// is raw and runs to the first `"""`); otherwise a `"…"` string whose body is
    /// kept raw (escapes are interpreted at code generation).
    fn read_string(&self, start: usize) -> Option<(Token<'src>, usize)> {
        if self.bytes[start..].starts_with(b"\"\"\"") {
            let content_start = start + 3;
            let closing = self.source[content_start..].find("\"\"\"")?;
            let content_end = content_start + closing;
            let content = &self.source[content_start..content_end];
            return Some((Token::MultilineString(content), content_end + 3));
        }

        let content_start = start + 1;
        let mut position = content_start;
        loop {
            match self.bytes.get(position) {
                None => return None,
                Some(b'"') => break,
                Some(b'\\') => {
                    // A backslash escapes the next character (so `\"` does not close
                    // the string). The escaped character may be multi-byte.
                    let escaped = self.source[position + 1..].chars().next()?;
                    position += 1 + escaped.len_utf8();
                }
                Some(_) => {
                    let character = self.source[position..]
                        .chars()
                        .next()
                        .expect("byte present implies a character");
                    position += character.len_utf8();
                }
            }
        }
        let content = &self.source[content_start..position];
        Some((Token::String(content), position + 1))
    }

    // --- Interpolated strings ------------------------------------------------

    /// Desugar `i"…{expr}…"` in place into the token sequence for a parenthesised
    /// string concatenation, e.g. `i"a{x}b"` becomes
    /// `( "" + "a" + ( x ) + "b" )`. Every *wrapper* token (the outer parens, the
    /// seed `""`, the `+`s, and the literal fragments) carries the whole i-string's
    /// span; the hole's tokens carry their own spans and its parens carry the
    /// `{…}` span. `position` is the leading `i`.
    fn lex_interpolated(&mut self) {
        let istring_start = self.position;
        self.position += 2; // `i` and the opening `"`

        // Scan the body into parts first: the wrapper tokens need the closing
        // quote's position, which is only known once the body is consumed.
        let mut parts: Vec<IStringPart<'src>> = Vec::new();
        let close = loop {
            match self.bytes.get(self.position) {
                None => break None, // unterminated
                Some(b'"') => break Some(self.position),
                Some(b'{') => parts.push(IStringPart::Hole(self.lex_hole())),
                Some(b'\\') => match self.bytes.get(self.position + 1) {
                    // `\{` / `\}` collapse to the brace itself (the slice is the
                    // brace character only).
                    Some(b'{') | Some(b'}') => {
                        let brace = &self.source[self.position + 1..self.position + 2];
                        parts.push(IStringPart::Text(brace));
                        self.position += 2;
                    }
                    // Any other escape is kept raw as a `\X` fragment (interpreted
                    // at code generation, like a plain string).
                    Some(_) => {
                        let escaped = self.source[self.position + 1..]
                            .chars()
                            .next()
                            .expect("byte present implies a character");
                        let end = self.position + 1 + escaped.len_utf8();
                        parts.push(IStringPart::Text(&self.source[self.position..end]));
                        self.position = end;
                    }
                    None => break None,
                },
                // A bare, unmatched `}` makes the i-string malformed (a clean source
                // never reaches here); record it and stop the body scan.
                Some(b'}') => {
                    self.skip_illegal();
                    break None;
                }
                Some(_) => {
                    let text_start = self.position;
                    while let Some(&byte) = self.bytes.get(self.position) {
                        if matches!(byte, b'{' | b'}' | b'"' | b'\\') {
                            break;
                        }
                        let character = self.source[self.position..]
                            .chars()
                            .next()
                            .expect("byte present implies a character");
                        self.position += character.len_utf8();
                    }
                    parts.push(IStringPart::Text(&self.source[text_start..self.position]));
                }
            }
        };

        let Some(close) = close else {
            // Unterminated: best-effort. A clean source never gets here; chumsky
            // discards its whole output in this case (a recovery pathology recorded
            // for S4). We keep what we scanned and record the error.
            self.errors.push(LexError {
                position: istring_start,
                character: 'i',
            });
            return;
        };

        let whole = span(istring_start, close + 1);
        self.tokens.push((Token::Ctrl('('), whole));
        self.tokens.push((Token::String(""), whole));
        for part in parts {
            self.tokens.push((Token::Op("+"), whole));
            match part {
                IStringPart::Text(text) => self.tokens.push((Token::String(text), whole)),
                IStringPart::Hole(hole_tokens) => self.tokens.extend(hole_tokens),
            }
        }
        self.tokens.push((Token::Ctrl(')'), whole));
        self.position = close + 1;
    }

    /// Lex one interpolation hole `{…}` into its parenthesised token list. The
    /// hole's parens carry the `{…}` span; the inner tokens carry their own. Hole
    /// tokens differ from top-level ones: no `=>` arrow, braces are not control
    /// characters (they delimit the hole), no comments, and no nested i-string
    /// desugaring (a `i"…"` in a hole is an `i` identifier then a string).
    /// `position` is the opening `{`.
    fn lex_hole(&mut self) -> Vec<Spanned<Token<'src>>> {
        let brace_open = self.position;
        self.position += 1; // `{`
        let mut inner = Vec::new();
        let brace_close = loop {
            self.skip_whitespace();
            match self.bytes.get(self.position) {
                None => break self.position, // unterminated; best-effort
                Some(b'}') => break self.position,
                Some(_) => match self.lex_hole_token() {
                    Some(token) => inner.push(token),
                    // A construct no hole token matches (a nested `{`, an illegal
                    // char) makes the hole malformed; stop (clean sources never do).
                    None => break self.position,
                },
            }
        };
        let hole_span = span(brace_open, brace_close + 1);
        self.position = (brace_close + 1).min(self.bytes.len());

        let mut wrapped = Vec::with_capacity(inner.len() + 2);
        wrapped.push((Token::Ctrl('('), hole_span));
        wrapped.extend(inner);
        wrapped.push((Token::Ctrl(')'), hole_span));
        wrapped
    }

    /// Lex one token inside an interpolation hole, or `None` if the current
    /// character starts no hole token. Whitespace is already skipped.
    fn lex_hole_token(&mut self) -> Option<Spanned<Token<'src>>> {
        let start = self.position;
        let first = self.bytes[start];
        let (token, end) = if first == b'"' {
            // An unterminated string inside a hole cannot be recovered locally; the
            // hole is malformed. A clean source never gets here.
            self.read_string(start)?
        } else if first.is_ascii_digit() {
            self.read_number(start)
        } else if is_ident_start(first) {
            self.read_identifier(start)
        } else if is_operator_byte(first) {
            self.read_operator(start, false)
        } else if is_hole_control_byte(first) {
            (Token::Ctrl(first as char), start + 1)
        } else {
            return None;
        };
        self.position = end;
        Some((token, span(start, end)))
    }
}

enum IStringPart<'src> {
    Text(&'src str),
    Hole(Vec<Spanned<Token<'src>>>),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokens only, asserting a clean lex (no errors) — mirrors `lexer.rs`'s helper
    /// so the two lexers' pins read alike.
    fn lex(source: &str) -> Vec<Token<'_>> {
        let (tokens, errors) = tokenize(source);
        assert!(errors.is_empty(), "lex errors: {errors:?}");
        tokens.into_iter().map(|(token, _span)| token).collect()
    }

    /// `(token, start, end)` triples, for span-inclusive pins.
    fn lex_spanned(source: &str) -> Vec<(Token<'_>, usize, usize)> {
        let (tokens, errors) = tokenize(source);
        assert!(errors.is_empty(), "lex errors: {errors:?}");
        tokens
            .into_iter()
            .map(|(token, span)| {
                let range = span.into_range();
                (token, range.start, range.end)
            })
            .collect()
    }

    // --- Trivia (the carried-over `lexer.rs` pins) ---------------------------

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
        assert_eq!(lex("main // no newline"), vec![Token::Ident("main")]);
    }

    // A blanked macro world is dominated by huge whitespace runs; the trivia skip
    // must stay linear (a quadratic loop once took seconds per world).
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
            "lexing a ~1MB whitespace prefix took {:?} — the trivia loop has gone quadratic",
            start.elapsed()
        );
    }

    // --- Operators (longest match, the `<`/`>`-are-control split) ------------

    #[test]
    fn multi_character_operators_win_over_their_prefixes() {
        for operator in ["!=", "%=", "&&", "*=", "+=", "-=", "/=", "::", "==", "||"] {
            assert_eq!(
                lex(operator),
                vec![Token::Op(operator)],
                "lexing {operator:?}"
            );
        }
        assert_eq!(lex("=>"), vec![Token::Op("=>")]);
        // The boundary holds mid-stream: `x-=-y` is `x`,`-=`,`-`,`y`.
        assert_eq!(
            lex("x-=-y"),
            vec![
                Token::Ident("x"),
                Token::Op("-="),
                Token::Op("-"),
                Token::Ident("y"),
            ]
        );
        // A run of an operator character munches longest-first, not maximally.
        assert_eq!(lex("&&&"), vec![Token::Op("&&"), Token::Op("&")]);
        assert_eq!(lex(":::"), vec![Token::Op("::"), Token::Op(":")]);
    }

    #[test]
    fn adjacent_prefix_operators_do_not_fuse() {
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
    fn angle_brackets_are_control_tokens_and_split() {
        // `<`/`>` are always `Ctrl` — the parser reassembles adjacent pairs into
        // shifts. `<=`/`>=` split into a control and an `=` operator; `?.`/`..`
        // split on the `.` control character.
        assert_eq!(lex("<"), vec![Token::Ctrl('<')]);
        assert_eq!(lex("<<"), vec![Token::Ctrl('<'), Token::Ctrl('<')]);
        assert_eq!(lex(">>"), vec![Token::Ctrl('>'), Token::Ctrl('>')]);
        assert_eq!(lex("<="), vec![Token::Ctrl('<'), Token::Op("=")]);
        assert_eq!(lex(">="), vec![Token::Ctrl('>'), Token::Op("=")]);
        assert_eq!(lex("?."), vec![Token::Op("?"), Token::Ctrl('.')]);
        assert_eq!(lex(".."), vec![Token::Ctrl('.'), Token::Ctrl('.')]);
    }

    // --- Keywords and identifiers -------------------------------------------

    #[test]
    fn keywords_classify_and_identifiers_do_not() {
        let keywords = [
            ("async", Token::Async),
            ("await", Token::Await),
            ("const", Token::Const),
            ("else", Token::Else),
            ("enum", Token::Enum),
            ("export", Token::Export),
            ("external", Token::External),
            ("false", Token::Bool(false)),
            ("for", Token::For),
            ("fun", Token::Fun),
            ("if", Token::If),
            ("impl", Token::Impl),
            ("import", Token::Import),
            ("in", Token::In),
            ("is", Token::Is),
            ("jump", Token::Jump),
            ("let", Token::Let),
            ("macro", Token::Macro),
            ("match", Token::Match),
            ("mod", Token::Mod),
            ("mut", Token::Mut),
            ("null", Token::Null),
            ("own", Token::Own),
            ("borrows", Token::Borrows),
            ("ret", Token::Ret),
            ("resource", Token::Resource),
            ("struct", Token::Struct),
            ("trait", Token::Trait),
            ("type", Token::Type),
            ("true", Token::Bool(true)),
            ("use", Token::Use),
            ("with", Token::With),
        ];
        for (text, token) in keywords {
            assert_eq!(lex(text), vec![token], "keyword {text:?}");
        }
        // A keyword with trailing identifier characters is an identifier.
        assert_eq!(lex("asyncx"), vec![Token::Ident("asyncx")]);
        assert_eq!(lex("await123"), vec![Token::Ident("await123")]);
        assert_eq!(lex("_foo"), vec![Token::Ident("_foo")]);
        assert_eq!(lex("_"), vec![Token::Ident("_")]);
    }

    // --- Numbers ------------------------------------------------------------

    #[test]
    fn numeric_literals_split_whole_fraction_and_suffix() {
        assert_eq!(lex("0"), vec![Token::Number("0", None, None)]);
        assert_eq!(lex("123"), vec![Token::Number("123", None, None)]);
        assert_eq!(lex("1.5"), vec![Token::Number("1", Some("5"), None)]);
        assert_eq!(lex("0.000"), vec![Token::Number("0", Some("000"), None)]);
        assert_eq!(lex("0u32"), vec![Token::Number("0", None, Some("u32"))]);
        assert_eq!(
            lex("1.5u32"),
            vec![Token::Number("1", Some("5"), Some("u32"))]
        );
        assert_eq!(lex("1_000"), vec![Token::Number("1", None, Some("_000"))]);
        // No leading zeros (`text::int`): `007` is three `0`,`0`,`7`.
        assert_eq!(
            lex("007"),
            vec![
                Token::Number("0", None, None),
                Token::Number("0", None, None),
                Token::Number("7", None, None),
            ]
        );
        // A `.` not followed by a digit stays a control token.
        assert_eq!(
            lex("1.foo"),
            vec![
                Token::Number("1", None, None),
                Token::Ctrl('.'),
                Token::Ident("foo"),
            ]
        );
    }

    #[test]
    fn hex_literals_keep_the_prefix_and_optional_suffix() {
        assert_eq!(lex("0xFF"), vec![Token::Number("0xFF", None, None)]);
        assert_eq!(lex("0xFFf"), vec![Token::Number("0xFFf", None, None)]);
        assert_eq!(
            lex("0x80000000u32"),
            vec![Token::Number("0x80000000", None, Some("u32"))]
        );
        assert_eq!(
            lex("0xDEADn"),
            vec![Token::Number("0xDEAD", None, Some("n"))]
        );
        // `0X` (capital) and a bare `0x` are not hex: `0` with an identifier suffix.
        assert_eq!(lex("0X10"), vec![Token::Number("0", None, Some("X10"))]);
        assert_eq!(lex("0x"), vec![Token::Number("0", None, Some("x"))]);
        assert_eq!(lex("0xg"), vec![Token::Number("0", None, Some("xg"))]);
    }

    // --- Strings ------------------------------------------------------------

    #[test]
    fn strings_keep_raw_bodies() {
        assert_eq!(lex(r#""hello""#), vec![Token::String("hello")]);
        assert_eq!(lex(r#""""#), vec![Token::String("")]);
        // The body is raw (escapes not yet interpreted): `\"` does not close.
        assert_eq!(
            lex(r#""with \"escaped\" quotes""#),
            vec![Token::String(r#"with \"escaped\" quotes"#)]
        );
        // A `"…"` string may span lines.
        assert_eq!(lex("\"a\nb\""), vec![Token::String("a\nb")]);
        // A triple-quoted string runs to the first `"""` and may hold a lone `"`.
        assert_eq!(
            lex(r#""""with " inner""""#),
            vec![Token::MultilineString(r#"with " inner"#)]
        );
    }

    // --- Interpolated strings (desugaring shape + the span quirk) ------------

    #[test]
    fn interpolated_strings_desugar_to_a_concatenation() {
        assert_eq!(
            lex(r#"i"a{x}b""#),
            vec![
                Token::Ctrl('('),
                Token::String(""),
                Token::Op("+"),
                Token::String("a"),
                Token::Op("+"),
                Token::Ctrl('('),
                Token::Ident("x"),
                Token::Ctrl(')'),
                Token::Op("+"),
                Token::String("b"),
                Token::Ctrl(')'),
            ]
        );
        // Empty i-string, escaped braces, and a keyword inside a hole.
        assert_eq!(
            lex(r#"i"""#),
            vec![Token::Ctrl('('), Token::String(""), Token::Ctrl(')')]
        );
        assert_eq!(
            lex(r#"i"\{x\}""#),
            vec![
                Token::Ctrl('('),
                Token::String(""),
                Token::Op("+"),
                Token::String("{"),
                Token::Op("+"),
                Token::String("x"),
                Token::Op("+"),
                Token::String("}"),
                Token::Ctrl(')'),
            ]
        );
    }

    #[test]
    fn interpolated_string_spans_reproduce_the_chumsky_quirk() {
        // Every wrapper token carries the WHOLE i-string span (`i` through the byte
        // past the closing quote); the hole's tokens carry their own spans and its
        // parens carry the `{…}` span. Reproduced byte-for-byte from chumsky —
        // recorded for the S4/S5 span-quality pass, not to be "corrected" here.
        //   i " H  e  l  l  o  {  n  a  m  e  }  "
        //   0 1 2  3  4  5  6  7  8  9 10 11 12 13
        assert_eq!(
            lex_spanned(r#"i"Hello{name}""#),
            vec![
                (Token::Ctrl('('), 0, 14),
                (Token::String(""), 0, 14),
                (Token::Op("+"), 0, 14),
                (Token::String("Hello"), 0, 14),
                (Token::Op("+"), 0, 14),
                (Token::Ctrl('('), 7, 13),
                (Token::Ident("name"), 8, 12),
                (Token::Ctrl(')'), 7, 13),
                (Token::Ctrl(')'), 0, 14),
            ]
        );
    }

    #[test]
    fn hole_tokens_differ_from_top_level_tokens() {
        // Inside a hole `=>` is not an arrow (`>` is a control character) and a
        // nested `i"…"` is an identifier `i` then a string — no re-desugaring.
        assert_eq!(
            lex(r#"i"{a => b}""#),
            vec![
                Token::Ctrl('('),
                Token::String(""),
                Token::Op("+"),
                Token::Ctrl('('),
                Token::Ident("a"),
                Token::Op("="),
                Token::Ctrl('>'),
                Token::Ident("b"),
                Token::Ctrl(')'),
                Token::Ctrl(')'),
            ]
        );
        assert_eq!(
            lex(r#"i"{i"x"}""#),
            vec![
                Token::Ctrl('('),
                Token::String(""),
                Token::Op("+"),
                Token::Ctrl('('),
                Token::Ident("i"),
                Token::String("x"),
                Token::Ctrl(')'),
                Token::Ctrl(')'),
            ]
        );
    }

    // --- Illegal characters (the error value shape is S1's own choice) -------

    #[test]
    fn illegal_characters_are_skipped_and_recorded() {
        // The token stream skips the illegal character and lexes the rest — the
        // shape the downstream `lexer_skips_an_illegal_character` pin relies on.
        let (tokens, errors) = tokenize("x@y");
        let bare: Vec<Token> = tokens.into_iter().map(|(token, _)| token).collect();
        assert_eq!(bare, vec![Token::Ident("x"), Token::Ident("y")]);
        assert_eq!(
            errors,
            vec![LexError {
                position: 1,
                character: '@',
            }]
        );
    }

    #[test]
    fn illegal_character_position_is_a_byte_offset() {
        // A multi-byte illegal character records its byte offset and the character.
        let (_, errors) = tokenize("x€y");
        assert_eq!(
            errors,
            vec![LexError {
                position: 1,
                character: '€',
            }]
        );
    }

    #[test]
    fn a_run_of_illegal_characters_records_one_error_each() {
        // S1 records one error per un-lexable character (chumsky coalesces a run
        // into a single diagnostic — a count difference deferred to S4). The token
        // stream is identical either way: the run is skipped.
        let (tokens, errors) = tokenize("a@@b");
        let bare: Vec<Token> = tokens.into_iter().map(|(token, _)| token).collect();
        assert_eq!(bare, vec![Token::Ident("a"), Token::Ident("b")]);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].position, 1);
        assert_eq!(errors[1].position, 2);
    }
}
