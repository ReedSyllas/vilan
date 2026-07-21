//! The lexer differential (H6 S1, `proposal/frontend.md` §3 step 2).
//!
//! The handwritten lexer (`vilan_core::lexing::tokenize`) must produce a token
//! stream byte-identical — spans included — to the chumsky lexer (`lexer()`, the
//! oracle held constant for the whole H6 arc) over every real source in the repo,
//! plus the adversarial fixtures below. A divergence fails loudly with the file and
//! the first differing token.
//!
//! A SIBLING of `parse_differential.rs`, not an extension of it, deliberately:
//! that harness's fn-pointer seam compares whole PARSE TREES (`Debug` of a
//! `NodeList`) and S2+ repoints its `CANDIDATE` at the hand *parser*. This one
//! compares `Vec<(Token, Span)>` — a different layer with a different type — and
//! stays a standalone lexer gate even after S2 moves the parse seam. The source
//! enumeration mirrors `parse_differential.rs` (itself duplicated from `docs.rs`);
//! test targets cannot import one another, so the small collectors are copied.
//!
//! At S5, when chumsky is deleted, this differential retires: its corpus role
//! passes to the parse-level corpus byte-gate, and the durable lexer regression
//! pins live in `lexing.rs`'s own test module.

use chumsky::prelude::*;
use std::path::{Path, PathBuf};
use vilan_core::Span;
use vilan_core::lexer;
use vilan_core::lexing::tokenize;
use vilan_core::token::Token;

// ---------------------------------------------------------------------------
// Rendering and comparison
// ---------------------------------------------------------------------------

/// One token as `Debug@start..end` — span-inclusive, so equal renderings mean
/// byte-identical tokens.
fn render_token(token: &Token, span: &Span) -> String {
    let range = span.into_range();
    format!("{:?}@{}..{}", token, range.start, range.end)
}

/// Compare the two lexers on `source`. `Ok` with the token count when they agree;
/// `Err` with the first divergence otherwise. The chumsky oracle returning no
/// output at all (its recovery pathology on some malformed inputs) is itself a
/// reportable divergence for a source expected to lex.
fn compare(source: &str) -> Result<usize, String> {
    let chumsky = lexer().parse(source).into_output();
    let (hand, _errors) = tokenize(source);
    let Some(chumsky) = chumsky else {
        return Err(format!(
            "chumsky produced NO output; hand produced {} token(s)",
            hand.len()
        ));
    };

    for (index, expected) in chumsky.iter().enumerate() {
        match hand.get(index) {
            Some(actual)
                if render_token(&actual.0, &actual.1) == render_token(&expected.0, &expected.1) => {
            }
            Some(actual) => {
                return Err(format!(
                    "token {index} differs:\n      chumsky: {}\n      hand:    {}",
                    render_token(&expected.0, &expected.1),
                    render_token(&actual.0, &actual.1),
                ));
            }
            None => {
                return Err(format!(
                    "hand stream is short: chumsky has {} tokens, hand has {}; \
                     first missing is {}",
                    chumsky.len(),
                    hand.len(),
                    render_token(&expected.0, &expected.1),
                ));
            }
        }
    }
    if hand.len() > chumsky.len() {
        let extra = &hand[chumsky.len()];
        return Err(format!(
            "hand stream is long: chumsky has {} tokens, hand has {}; \
             first extra is {}",
            chumsky.len(),
            hand.len(),
            render_token(&extra.0, &extra.1),
        ));
    }
    Ok(chumsky.len())
}

// ---------------------------------------------------------------------------
// Source enumeration (mirrors parse_differential.rs)
// ---------------------------------------------------------------------------

fn repo_vilan() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan")
}

fn collect_vl(root: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_vl(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "vl") {
            into.push(path);
        }
    }
}

fn collect_markdown(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "book") {
                continue; // rendered-site output, not content
            }
            collect_markdown(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            into.push(path);
        }
    }
}

/// Compilable fenced examples under `vilan/docs/**` plus the README, as
/// `(label, source)` — the complete `vilan` / `vilan,norun` / `vilan,browser`
/// programs the docs gate compiles (so clean to lex).
fn collect_doc_examples(into: &mut Vec<(String, String)>) {
    let docs_root = repo_vilan().join("docs");
    let mut markdown = Vec::new();
    collect_markdown(&docs_root, &mut markdown);
    let readme = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    if readme.is_file() {
        markdown.push(readme);
    }
    for file in &markdown {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let mut open: Option<(usize, String)> = None;
        for (index, line) in text.lines().enumerate() {
            match &mut open {
                Some((opened_at, body)) => {
                    if line.trim_end() == "```" {
                        let label = format!("docs:{}:{}", file.display(), *opened_at + 1);
                        into.push((label, std::mem::take(body)));
                        open = None;
                    } else {
                        body.push_str(line);
                        body.push('\n');
                    }
                }
                None => {
                    if let Some(info) = line.trim_start().strip_prefix("```") {
                        let compile =
                            matches!(info.trim(), "vilan" | "vilan,norun" | "vilan,browser");
                        if compile {
                            open = Some((index, String::new()));
                        }
                    }
                }
            }
        }
    }
}

fn all_sources() -> Vec<(String, String)> {
    let vilan = repo_vilan();
    let mut files = Vec::new();
    collect_vl(&vilan.join("test"), &mut files);
    collect_vl(&vilan.join("std/src"), &mut files);
    collect_vl(&vilan.join("examples"), &mut files);
    let mut sources: Vec<(String, String)> = files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            Some((path.display().to_string(), text))
        })
        .collect();
    collect_doc_examples(&mut sources);
    sources
}

// ---------------------------------------------------------------------------
// The corpus differential
// ---------------------------------------------------------------------------

#[test]
fn hand_and_chumsky_lex_agree_over_the_corpus() {
    let sources = all_sources();
    assert!(
        sources.len() > 150,
        "suspiciously few sources enumerated: {}",
        sources.len()
    );

    let mut compared = 0usize;
    let mut total_tokens = 0usize;
    let mut total_hand_errors = 0usize;
    let mut disagreements: Vec<String> = Vec::new();

    for (label, source) in &sources {
        match compare(source) {
            Ok(count) => {
                compared += 1;
                total_tokens += count;
            }
            Err(reason) => disagreements.push(format!("{label}: {reason}")),
        }
        total_hand_errors += tokenize(source).1.len();
    }

    eprintln!(
        "lexer differential: N={} sources, {} agreed, {} total tokens, {} hand lex-errors",
        sources.len(),
        compared,
        total_tokens,
        total_hand_errors,
    );

    assert!(
        disagreements.is_empty(),
        "hand/chumsky lexer DISAGREEMENT on {} source(s):\n{}",
        disagreements.len(),
        disagreements.join("\n"),
    );
    // Every enumerated source is a clean, compilable program: it must lex without
    // an error from either lexer. A non-zero count would mean the corpus drifted
    // (or the hand lexer wrongly rejected something) — worth catching here.
    assert_eq!(
        total_hand_errors, 0,
        "the hand lexer reported {total_hand_errors} error(s) over the clean corpus",
    );
    assert_eq!(compared, sources.len(), "not every source compared equal",);
}

// ---------------------------------------------------------------------------
// Targeted adversarial fixtures
// ---------------------------------------------------------------------------

/// Each fixture is lexed by both lexers and asserted byte-identical. Expressing
/// them as a differential (rather than hand-transcribed expected streams) keeps
/// them honest: the referee is chumsky, exactly as for the corpus.
#[test]
fn hand_and_chumsky_agree_on_targeted_fixtures() {
    let fixtures: &[&str] = &[
        // Every operator, and the longest-match pairs against their prefixes.
        "!",
        "!=",
        "?",
        "?.",
        "&",
        "&&",
        "|",
        "||",
        "::",
        "==",
        "=>",
        "=",
        "+",
        "+=",
        "-",
        "-=",
        "*",
        "*=",
        "/",
        "/=",
        "%",
        "%=",
        ":",
        "^",
        "!!b",
        "!*v",
        "-*v",
        "x-=-y",
        "&&&",
        "|||",
        ":::",
        "?..?",
        // `<`/`>` control split (shifts, comparisons, generics).
        "<",
        ">",
        "<<",
        ">>",
        "<=",
        ">=",
        "<>",
        "a<b",
        "a>b",
        "a<=b",
        "a>>b",
        "a<<b",
        "map<Map<K, V>, List<T>>",
        "a >> b << c",
        // Ranges and member access split on `.`.
        ".",
        "..",
        "...",
        "a..b",
        "a?.b",
        "a?b",
        "1..=2",
        "x?.y?.z",
        // Numeric forms: ints, floats, suffixes, hex, no-leading-zero, boundaries.
        "0",
        "123",
        "1.5",
        "0.5",
        "1.",
        "1.foo",
        ".5",
        "007",
        "0u32",
        "1f",
        "2n",
        "3i53",
        "100u53",
        "1.5e3",
        "1_000",
        "12.34.56",
        "0.0",
        "99999999999999",
        "1.5u32",
        "0xFF",
        "0x80000000u32",
        "0xDEADn",
        "0xFFf",
        "0xff",
        "0X10",
        "0x",
        "1x",
        "0xg",
        "1.05",
        "1.0e10",
        "0.",
        "0b101",
        "0o17",
        "1abc",
        "3.14xyz",
        "1e10",
        "0xABCDEF",
        "0xabcdefu32",
        "1.2.3",
        // Strings with escapes, empties, multi-line, triple-quoted, inner quotes.
        r#""hello""#,
        r#""""#,
        r#""with \"escaped\" quotes""#,
        r#""back\\slash""#,
        r#""tab\there""#,
        "\"multi\nline\"",
        "\"\"\"a\nb\nc\"\"\"",
        r#""""with " inner""""#,
        r#""abc\\""#,
        r#""a\nb""#,
        r#""\u{1F600}""#,
        "\"tab\tinside\"",
        r#""\"""#,
        r#""""""""#,
        // i-string edges: empty, adjacent holes, escaped braces, multi-line,
        // nested quotes/i-strings in holes, keywords/operators/indexing in holes.
        r#"i"Hello, {name}!""#,
        r#"i"""#,
        r#"i"{a}{b}""#,
        r#"i"a{x}b{y}c""#,
        r#"i"\{literal\}""#,
        r#"i"pre{f("x")}post""#,
        r#"i"{a + b}""#,
        r#"i"{ a }""#,
        "i\"line1\nline2\"",
        r#"i"nested {g(i"inner")}""#,
        r#"i"escape\n here""#,
        r#"i"{obj.field}""#,
        r#"i"{arr[0]}""#,
        r#"i"just text""#,
        r#"i"{a}{b}{c}""#,
        r#"i"{x => y}""#,
        r#"i"a\tb""#,
        r#"i"\\""#,
        r#"i"100%""#,
        r#"i"{a == b}""#,
        r#"i"{ f(a, b) }""#,
        r#"i"text \n with \{ brace""#,
        r#"i"{if x}""#,
        r#"i"{match y}""#,
        r#"i"{a.b.c}""#,
        r#"i"{a < b}""#,
        r#"i"{x?.y}""#,
        // Comments (line only — vilan has no block comments) and trivia.
        "// line",
        "a // trailing\nb",
        "  \n  x",
        "/* block */ x",
        "a/b",
        "a /* c */ b",
        "x // no newline",
        "",
        "   ",
        "//only",
        "a  \n\n  b",
        "//\n//\n//\nx",
        // Whitespace and line endings the padded pass must treat identically.
        "a\r\nb",
        "a\tb",
        "a\u{000C}b",
        "a\u{000B}b",
        "a\u{00A0}b",
        "a\u{2003}b",
        "a\u{2028}b",
        // Illegal-character recovery where the bad char is not the trailing lexeme
        // (chumsky yields a stream to compare against; a trailing bad char discards
        // chumsky's whole output — a recovery pathology excluded here, covered by
        // the lexing.rs error-value pins).
        "x@y",
        "x @ y",
        "x#y#z",
        "a@@b",
        "x€y",
        "x\\y",
        "1 2 3",
        "foo @bar baz",
        "naïve",
        "x·y",
        "foo'bar",
        "a\u{200b}b",
        "\u{feff}fun",
        // A few whole-snippet shapes.
        "fun main() { let x = 1; }",
        "a < b && c > d",
        "if x == 1 { } else { }",
        "-1 + -2",
        "!done && ready",
        "a[0].b(1, 2)",
    ];

    let mut disagreements: Vec<String> = Vec::new();
    for source in fixtures {
        // Only fixtures the oracle lexes to a stream are asserted equal; a fixture
        // where chumsky discards its output (trailing un-lexable char) is out of
        // scope here (see the note above), so skip that specific pathology.
        if lexer().parse(source).into_output().is_none() {
            continue;
        }
        if let Err(reason) = compare(source) {
            disagreements.push(format!("{source:?}: {reason}"));
        }
    }
    assert!(
        disagreements.is_empty(),
        "hand/chumsky lexer DISAGREEMENT on {} fixture(s):\n{}",
        disagreements.len(),
        disagreements.join("\n"),
    );
}
