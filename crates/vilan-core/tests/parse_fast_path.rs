//! Pins for the cheap-first parse path (`parse_clean`): the fast, mute
//! instantiation must accept exactly the clean inputs and produce exactly the
//! tree the rich instantiation produces — diagnostics-bearing callers fall
//! back to the rich pipeline on ANY failure, including recovered ones.

use chumsky::prelude::*;
use vilan_core::{lexer, parse_clean, parse_clean_cached, parser};

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

// --- The shared content-addressed parse cache (backlog E12) -----------------

/// The cache reuses a clean parse across calls with identical content: the
/// second call returns the IDENTICAL leaked pointer the first stored, proving it
/// did not re-parse. Pointer identity is a timing-free reuse assertion, robust
/// under the parallel test runner (each distinct content keys independently), so
/// it needs no wall-clock or shared counter. A unique marker keeps this content
/// off any other test's key.
#[test]
fn cached_clean_parse_is_reused_across_calls() {
    let source = "fun e12_reuse_probe() { let a = 1; }\n";
    let (first, first_text) = parse_clean_cached(source).expect("clean source is cached");
    let (second, second_text) = parse_clean_cached(source).expect("a second call hits the cache");
    assert!(
        std::ptr::eq(first, second),
        "identical content must return the same cached tree (no re-parse)"
    );
    assert!(
        std::ptr::eq(first_text, second_text),
        "identical content must return the same cached source text"
    );
}

/// Different content is a different key — a real edit is parsed afresh (never
/// served the stale tree), which is why the cache is keyed on content, never on
/// a name or mtime.
#[test]
fn cached_parse_is_keyed_on_content_not_identity() {
    let (before, _) =
        parse_clean_cached("fun e12_edit_probe() { let a = 1; }\n").expect("clean before-edit");
    let (after, _) =
        parse_clean_cached("fun e12_edit_probe() { let a = 2; }\n").expect("clean after-edit");
    assert!(
        !std::ptr::eq(before, after),
        "edited content must parse to a distinct tree, not reuse the old one"
    );
}

/// A non-clean file is not cached — the cache returns `None` so the caller can
/// fall back to the rich-diagnostic pipeline, exactly as [`parse_clean`] would.
#[test]
fn cached_parse_declines_a_broken_file() {
    assert!(parse_clean_cached("fun main() { let x = ; }").is_none());
}

/// The cached tree matches the rich pipeline's tree byte-for-byte (via `Debug`),
/// so routing a compile through the cache changes no emitted output.
#[test]
fn cached_tree_matches_the_rich_tree() {
    let source = "import std::print;\n\nfun main() {\n\tprint(\"e12\");\n}\n";
    let (cached, _) = parse_clean_cached(source).expect("clean source is cached");
    let (rich, errors) = rich_parse(source);
    assert_eq!(errors, 0);
    let mut rich = rich.expect("clean source parses rich too");
    // The cache lift-rewrites before storing; match that so the trees compare.
    vilan_core::lift::rewrite_items(&mut rich.0);
    assert_eq!(format!("{cached:?}"), format!("{rich:?}"));
}
