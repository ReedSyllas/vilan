//! Pins for the cheap-first parse path (`parse_clean`): the fast, mute
//! instantiation must accept exactly the clean inputs and produce exactly the
//! tree the rich instantiation produces — diagnostics-bearing callers fall
//! back to the rich pipeline on ANY failure, including recovered ones.

use chumsky::prelude::*;
use vilan_core::{lexer, parse_clean, parser};

/// The rich pipeline's parse of `source`, as the diagnostics path runs it.
fn rich_parse(
    source: &str,
) -> (
    Option<vilan_core::Spanned<vilan_core::node::NodeList<'_>>>,
    usize,
) {
    let (tokens, lex_errors) = lexer().parse(source).into_output_errors();
    let Some(tokens) = tokens else {
        return (None, lex_errors.len());
    };
    let end = source.len();
    let (root, parse_errors) = parser()
        .parse(
            tokens
                .as_slice()
                .map((end..end).into(), |(token, span)| (token, span)),
        )
        .into_output_errors();
    (root, lex_errors.len() + parse_errors.len())
}

#[test]
fn clean_source_parses_and_matches_the_rich_tree() {
    let source = r#"
        import std::print;

        struct Point { x: i32, y: i32 }

        fun main() {
            let point = Point { x = 1, y = 2 };
            let sum = point.x + point.y;
            print(i"sum: {sum}");
        }
    "#;
    let fast = parse_clean(source).expect("clean source must take the fast path");
    let (rich, errors) = rich_parse(source);
    assert_eq!(errors, 0);
    let rich = rich.expect("clean source must parse rich too");
    assert_eq!(
        format!("{fast:?}"),
        format!("{rich:?}"),
        "fast and rich instantiations must build the same tree"
    );
}

#[test]
fn lex_error_rejects_the_fast_path() {
    assert!(parse_clean("fun main() { let s = \"unterminated; }").is_none());
}

#[test]
fn parse_error_rejects_the_fast_path() {
    assert!(parse_clean("fun main() { let x = ; }").is_none());
}

/// Recovery makes a broken file parse into a PARTIAL tree with errors — the
/// fast path must reject that too (returning the partial tree with no
/// diagnostics would silently degrade the rich fallback's error reporting).
#[test]
fn recovered_parse_rejects_the_fast_path() {
    let source = "fun broken() { let = 3; }\nfun fine() { }";
    let (root, errors) = rich_parse(source);
    assert!(
        root.is_some() && errors > 0,
        "precondition: this input must parse WITH recovery (got root={}, {errors} errors)",
        root.is_some(),
    );
    assert!(parse_clean(source).is_none());
}

#[test]
fn trivia_only_source_is_clean() {
    let root = parse_clean("// just a comment\n\n").expect("trivia-only files are clean");
    assert!(root.0.is_empty());
}
