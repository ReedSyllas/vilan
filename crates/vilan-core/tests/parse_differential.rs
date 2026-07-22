//! The corpus-scale differential + fmt tripwire (H6 S0, `proposal/frontend.md` §3).
//!
//! `proposal/frontend.md` replaces the chumsky lexer+parser with a handwritten
//! frontend, holding chumsky in-tree as the ORACLE for the whole arc and requiring
//! the new frontend to produce byte-identical (span-inclusive) trees. This target
//! scales `parse_fast_path.rs`'s `Debug`-string differential to every real source
//! in the repo, so a divergence — now, or when S1 swaps the candidate frontend —
//! fails loudly with the offending file.
//!
//! The differential SEAM (below) is a fn-pointer pair: `ORACLE` is the rich chumsky
//! parse, held constant; `CANDIDATE` is the frontend under test. At S0 the candidate
//! was the fast chumsky path (`parse_clean`); **S3 repoints `CANDIDATE` at the
//! handwritten frontend** (`parsing::parse`), so this is now the TOTAL differential
//! — the whole file, items included — that the proposal's §3 S3 gate requires. The
//! original fast-vs-rich chumsky self-check is NOT lost: `chumsky_candidate` is kept
//! and driven by its own test (`fast_and_rich_chumsky_agree_over_the_corpus`), so
//! the oracle's own invariant (fast path ≡ rich path) still runs.
//!
//! The fmt tripwire converts the formatter's silent-no-op failure mode (§0: the
//! re-lex-and-compare safety net turns `fmt` into a no-op when the token stream
//! drifts, indistinguishable from an already-canonical file) into loud, external
//! checks: `formatter_output_token_matches_input_over_the_corpus` guards against
//! token-drifting output, and `formatter_bail_set_is_the_known_ledger` pins the
//! exact set of files `fmt` currently no-ops on (an S0 FINDING — see
//! `KNOWN_FORMATTER_BAILS`), with `formatter_never_silently_bails_over_the_corpus`
//! the `#[ignore]`d zero-bail goal.

use chumsky::prelude::*;
use std::path::{Path, PathBuf};
use vilan_core::token::Token;
use vilan_core::{formatter, lexer, parse_clean, parser, parsing};

// ---------------------------------------------------------------------------
// The differential seam
// ---------------------------------------------------------------------------

/// A frontend's judgement of one source: the `Debug` of the recovered tree (if a
/// tree came back at all) and the diagnostic count.
type Judgement = (Option<String>, usize);

/// ORACLE — the rich (diagnostics-bearing) chumsky instantiation, exactly as the
/// diagnostics path runs it. Held constant for the whole H6 arc (proposal §3).
fn chumsky_oracle(source: &str) -> Judgement {
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
    (
        root.map(|tree| format!("{tree:?}")),
        lex_errors.len() + parse_errors.len(),
    )
}

/// The fast chumsky path (`parse_clean`): accepts ONLY perfectly clean sources and
/// declines (returns `None`) on any lex/parse error or recovery. Held as the
/// oracle's own self-check candidate through the whole arc — the invariant "the
/// fast path agrees with the rich path" must not be lost when `CANDIDATE` moves to
/// the handwritten frontend.
///
/// (`parse_clean` does NOT lift-rewrite — only `parse_clean_cached` does — so its
/// tree compares directly against the un-lifted oracle tree, exactly as
/// `parse_fast_path.rs::clean_source_parses_and_matches_the_rich_tree` relies on.)
fn chumsky_candidate(source: &str) -> Option<String> {
    parse_clean(source).map(|tree| format!("{tree:?}"))
}

/// CANDIDATE — the frontend under differential test. **S3: the handwritten frontend**
/// (`parsing::parse`, which internally lexes with `lexing::tokenize`). It returns a
/// tree only when the source is perfectly clean (a non-empty error list yields
/// `None`), so — like `chumsky_candidate` — its clean tree compares directly against
/// the un-lifted oracle tree (neither lift-rewrites). Repointing this single const
/// is the whole S3 seam move on the harness side.
fn handwritten_candidate(source: &str) -> Option<String> {
    let (tree, _errors) = parsing::parse(source);
    tree.map(|tree| format!("{tree:?}"))
}

const ORACLE: fn(&str) -> Judgement = chumsky_oracle;
const CANDIDATE: fn(&str) -> Option<String> = handwritten_candidate;

// ---------------------------------------------------------------------------
// Source enumeration: the corpus, every std layer, examples, and docs examples
// ---------------------------------------------------------------------------

fn repo_vilan() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan")
}

/// Every `*.vl` under `root`, recursively, sorted for a stable summary.
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

/// Every compilable fenced example under `vilan/docs/**` (plus the repo README),
/// as `(label, source)`. Mirrors `docs.rs`'s fence logic — `vilan` / `vilan,norun`
/// / `vilan,browser` are complete programs (compiled by the docs gate, hence clean
/// to parse); `vilan,fragment` and non-vilan fences are skipped. Duplicated rather
/// than imported because `docs.rs` is a separate test target, not a library.
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
        let mut open: Option<(usize, String)> = None; // (opened_line, body)
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

/// Whole-file S3 constructs that the repo corpus happens NOT to exercise (so the
/// file-derived differential never reaches them), each a clean program the oracle
/// and candidate must parse byte-identically. Only PARSED here (types need not
/// resolve), so bare type names are fine. This closes the corpus's coverage gaps —
/// notably `[trait_only]` / `[doc(hidden)]` (zero corpus uses) and the tuple-bound
/// endpoint variants — through the same oracle comparison as every real source,
/// rather than leaving them to the (chumsky-free, durable) in-module pins alone.
fn corpus_absent_constructs() -> Vec<(String, String)> {
    [
        // The two attributes with zero corpus uses.
        ("trait_only", "trait Surface { [trait_only] fun hidden(&self): i32; }"),
        ("doc_hidden", "[doc(hidden)] fun helper(): i32 { 0 }"),
        // Every function attribute at once, in the one legal (fixed) order.
        (
            "all_attributes",
            "[extern(\"m\", \"s\")] [must_use] [rpc] [trait_only] [doc(hidden)] [platform(\"@process\", \"browser\")] external fun everything(): i32;",
        ),
        // Tuple-bound endpoint variants: both, hi-only, and an element bound.
        ("tuple_bound_both", "fun a<T: (2..10)>(): T { default() }"),
        ("tuple_bound_hi", "fun b<T: (..10)>(): T { default() }"),
        ("tuple_bound_element", "fun c<T: (..: Show)>(): T { default() }"),
        // A generic default and a `type` binder default together.
        ("generic_defaults", "struct Cell<T = Self, type U = i32> { value: T }"),
        // Import/use path shapes: a top-level set, a deeply nested set, a use set.
        ("import_top_set", "import { alpha, beta };"),
        ("import_nested_set", "import root::mid::{ leaf, twig::{ a, b } };"),
        ("use_set", "use collection::{ Map, Set };"),
        // Every parameter convention in one signature (own + & + &mut + inferred).
        (
            "conventions",
            "fun mix(own a: A, &b: B, &mut c: C, d: D, e: &E): i32 { 0 }",
        ),
        // The `null`-named bodyless external struct and the full resource modifier.
        ("external_null", "external struct null;"),
        ("resource_external", "resource external struct Handle;"),
        ("resource_enum", "resource enum State { Open, Closed }"),
        // An enum with negative + explicit discriminants alongside a payload.
        (
            "enum_discriminants",
            "enum Ordering { Less = -1, Equal = 0, Greater(i32) }",
        ),
        // A tuple comprehension as a value, and macro forms in both positions.
        ("tuple_comprehension", "fun t(): T { (x in xs => x + 1) }"),
        (
            "macro_forms",
            "macro fun make(): Source { source(\"\") }\nmacro grow(a, b)\nfun use_it() { let v = macro pick(x); macro { ret void } }",
        ),
        // `export` wrapping several item kinds, and a nested module.
        ("export_items", "export struct S { x: i32 }\nexport fun f() { }\nexport use m::n;"),
        (
            "nested_module",
            "mod outer { mod inner { fun deep() { } } struct Local { n: i32 } }",
        ),
    ]
    .into_iter()
    .map(|(label, source)| (format!("adversarial:{label}"), source.to_string()))
    .collect()
}

/// The full differential corpus: `(label, source)` over `vilan/test`, every
/// `vilan/std/src` layer, `vilan/examples`, the docs examples, and the
/// corpus-absent S3 constructs above.
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
    sources.extend(corpus_absent_constructs());
    sources
}

// ---------------------------------------------------------------------------
// The differential
// ---------------------------------------------------------------------------

/// Run one candidate against the oracle over every source, returning
/// `(clean_compared, recovered, disagreements, hard_fails)` — the same tally the
/// S0 harness computed, factored out so both the handwritten candidate (the S3
/// gate) and the fast chumsky candidate (the oracle self-check) drive it.
fn differential_report(
    candidate: fn(&str) -> Option<String>,
    sources: &[(String, String)],
) -> (usize, usize, Vec<String>, Vec<String>) {
    let mut clean_compared = 0usize; // M: candidate clean AND oracle agrees
    let mut recovered = 0usize; // K: candidate declined, oracle recovered a tree
    let mut disagreements: Vec<String> = Vec::new();
    let mut hard_fails: Vec<String> = Vec::new();

    for (label, source) in sources {
        let (oracle_tree, oracle_errors) = ORACLE(source);
        match candidate(source) {
            Some(candidate_tree) => {
                // The candidate accepted the source as clean; the oracle MUST
                // agree — a tree, zero diagnostics, and a byte-identical (=
                // span-identical) `Debug`. Any of these failing is a live
                // fast/rich parser divergence (proposal §3 stop condition).
                match oracle_tree {
                    Some(oracle_tree) if oracle_errors == 0 && oracle_tree == candidate_tree => {
                        clean_compared += 1;
                    }
                    Some(oracle_tree) if oracle_errors == 0 => {
                        disagreements.push(format!(
                            "{label}: candidate/oracle trees differ (both clean)\n  \
                             candidate: {candidate_tree}\n  oracle:    {oracle_tree}"
                        ));
                    }
                    _ => {
                        disagreements.push(format!(
                            "{label}: candidate parsed it clean but the oracle reported \
                             {oracle_errors} diagnostic(s)"
                        ));
                    }
                }
            }
            None => {
                // The candidate declined (a lex/parse error or a recovery). The
                // oracle must still return a tree — that a real repo source needs
                // recovery at all is itself worth recording.
                if oracle_tree.is_some() {
                    recovered += 1;
                    if oracle_errors == 0 {
                        // Oracle sees a clean tree but the candidate declined: the
                        // candidate is wrongly stricter than the oracle.
                        disagreements.push(format!(
                            "{label}: candidate declined a source the oracle parses CLEAN \
                             (candidate is too strict)"
                        ));
                    }
                } else {
                    hard_fails.push(format!(
                        "{label}: neither frontend produced a tree ({oracle_errors} diagnostics)"
                    ));
                }
            }
        }
    }
    (clean_compared, recovered, disagreements, hard_fails)
}

#[test]
fn candidate_and_oracle_agree_over_the_corpus() {
    let sources = all_sources();
    assert!(
        sources.len() > 150,
        "suspiciously few sources enumerated: {}",
        sources.len()
    );

    let (clean_compared, recovered, disagreements, hard_fails) =
        differential_report(CANDIDATE, &sources);

    eprintln!(
        "parse differential (handwritten frontend): N={} files, M={} clean-compared, K={} recovered",
        sources.len(),
        clean_compared,
        recovered
    );

    assert!(
        disagreements.is_empty(),
        "candidate/oracle DISAGREEMENT on {} source(s):\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
    assert!(
        hard_fails.is_empty(),
        "{} source(s) produced NO tree from either frontend:\n{}",
        hard_fails.len(),
        hard_fails.join("\n")
    );
    assert!(
        clean_compared > 150,
        "expected the bulk of the corpus to compare clean, got M={clean_compared}"
    );
}

/// The oracle's own self-check, preserved through the S3 candidate move: the fast
/// chumsky path (`parse_clean`) must agree with the rich chumsky path (`parser()`)
/// over every clean source. This is the invariant the S0 harness enforced before
/// `CANDIDATE` pointed at chumsky's fast path; keeping it as its own test means
/// swapping `CANDIDATE` to the handwritten frontend does not silently drop it.
#[test]
fn fast_and_rich_chumsky_agree_over_the_corpus() {
    let sources = all_sources();
    let (clean_compared, _recovered, disagreements, hard_fails) =
        differential_report(chumsky_candidate, &sources);
    assert!(
        disagreements.is_empty(),
        "chumsky fast/rich DISAGREEMENT on {} source(s):\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
    assert!(
        hard_fails.is_empty(),
        "{} source(s) produced NO tree from either chumsky path:\n{}",
        hard_fails.len(),
        hard_fails.join("\n")
    );
    assert!(
        clean_compared > 150,
        "expected the bulk of the corpus to compare clean, got M={clean_compared}"
    );
}

// ---------------------------------------------------------------------------
// The fmt tripwire
// ---------------------------------------------------------------------------

/// The formatter's own notion of "the same code": the lexer's token stream with
/// spans stripped. Re-implemented here against the PUBLIC lexer so the check is
/// external to `formatter.rs` (the point of a tripwire) and survives the H6
/// cutover. Mirrors `formatter::code_tokens` + `formatter::normalize`.
fn normalized_tokens(source: &str) -> Option<Vec<Token<'_>>> {
    let tokens: Vec<Token<'_>> = lexer()
        .parse(source)
        .into_output()?
        .into_iter()
        .map(|(token, _span)| token)
        .collect();
    // A trailing comma before a closer is insignificant in vilan — the formatter
    // may normalize it in or out, so the safety check ignores it.
    let mut result: Vec<Token<'_>> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if matches!(
            token,
            Token::Ctrl('}') | Token::Ctrl(')') | Token::Ctrl(']')
        ) {
            while let Some(Token::Ctrl(',')) = result.last() {
                result.pop();
            }
        }
        result.push(token);
    }
    Some(result)
}

fn corpus_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_vl(&repo_vilan().join("test"), &mut files);
    files
}

#[test]
fn formatter_output_token_matches_input_over_the_corpus() {
    // The durable tripwire: whatever `format` returns for a corpus file, its token
    // stream must match the input's (unchanged output matches trivially; a
    // successful reprint matches by the formatter's contract). This holds today
    // via the internal safety net; post-cutover it catches any token-drifting
    // output that slips that net.
    let files = corpus_files();
    assert!(
        files.len() > 60,
        "suspiciously few corpus files: {}",
        files.len()
    );
    let mut mismatches: Vec<String> = Vec::new();
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let output = formatter::format(&source);
        if output == source {
            continue; // unchanged: trivially token-equal
        }
        let input_tokens = normalized_tokens(&source);
        let output_tokens = normalized_tokens(&output);
        if input_tokens != output_tokens {
            mismatches.push(format!("{}", path.display()));
        }
    }
    assert!(
        mismatches.is_empty(),
        "formatter output token-DRIFTED from the input on {} file(s): {:?}",
        mismatches.len(),
        mismatches
    );
}

/// The corpus files the formatter currently BAILS on, by base name (sorted).
///
/// Detector: `format` is a total canonicalizer over parseable input, so it must
/// map a source and a token-preserving perturbation of it to the SAME output.
/// Appending blank lines is such a perturbation (trailing newlines are trivia,
/// always normalized away, and change no comment). If `format(source)` and
/// `format(source + "\n\n")` DIFFER, the formatter bailed on this file — it
/// returned each input verbatim (with the extra newlines surviving) instead of
/// canonicalizing. A truly-canonical file is NOT flagged: both map to itself.
/// (Verified: every flagged file returns BOTH inputs verbatim — `format(x)==x` —
/// while controls strip the perturbation, the clean bail-vs-canonical signal.)
fn current_bail_set() -> Vec<String> {
    let mut bails: Vec<String> = corpus_files()
        .into_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(&path).ok()?;
            let base = formatter::format(&source);
            let perturbed = formatter::format(&format!("{source}\n\n"));
            (base != perturbed)
                .then(|| path.file_name()?.to_str().map(str::to_string))
                .flatten()
        })
        .collect();
    bails.sort();
    bails
}

/// The known silent-bailers as of H6 S0 (2026-07-21). This is the §0 "silent
/// no-op" failure mode made LOUD and enumerated: `vilan fmt` is a no-op on each of
/// these files because the printer hits a construct its `_ => self.bailed = true`
/// fallbacks (formatter.rs `print_item` / `print_expr`) do not yet handle — the
/// file names correlate with newer language forms (macros, expression lifting,
/// fixed arrays, sized numerics, unary minus, irrefutable destructuring). This is
/// a REPORTED FINDING, not a fix: completing the formatter is a separate work item
/// (out of H6 S0 scope, which is "pin the ground"). Pinning the exact set makes it
/// an active regression tripwire — a NEW bailer, or one the formatter learns to
/// handle, flips this test and forces the ledger (and the goal test below) to be
/// revisited.
const KNOWN_FORMATTER_BAILS: &[&str] = &[
    "destructuring.vl",
    "fixed-arrays.vl",
    "lift-chain.vl",
    "macro-block.vl",
    "macro-derive.vl",
    "macro-invoke.vl",
    "math.vl",
    "numeric-types.vl",
    "reactive-owner.vl",
    "unary-minus.vl",
];

#[test]
fn formatter_bail_set_is_the_known_ledger() {
    let bails = current_bail_set();
    eprintln!(
        "fmt tripwire: {} corpus files, {} silently bailing: {:?}",
        corpus_files().len(),
        bails.len(),
        bails
    );
    assert_eq!(
        bails, KNOWN_FORMATTER_BAILS,
        "the formatter's silent-bail set changed. If a file now bails, `vilan fmt` \
         silently no-ops on it (a §0 regression) — investigate. If a listed file no \
         longer bails, the formatter learned its construct — drop it from the ledger \
         and from the #[ignore]d goal test."
    );
}

#[test]
#[ignore = "H6 S0 FINDING: 10 corpus files silently bail through `vilan fmt` (see \
            KNOWN_FORMATTER_BAILS). The goal is zero; un-ignore when the formatter \
            handles every corpus construct. Do NOT fix in S0 — report only."]
fn formatter_never_silently_bails_over_the_corpus() {
    let bails = current_bail_set();
    assert!(
        bails.is_empty(),
        "formatter SILENTLY BAILED on {} corpus file(s): {:?}",
        bails.len(),
        bails
    );
}
