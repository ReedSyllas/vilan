//! The expression-level differential (H6 S2, `proposal/frontend.md` §3 step 3).
//!
//! The handwritten parser (`vilan_core::parsing::parse`) must produce the same
//! `Spanned<Node>` tree — spans included — that the chumsky grammar produces, over
//! every expression and type shape in S2's implemented subset. S2 does not yet
//! parse whole files (items and macro forms are the S3 seam), so this harness does
//! not repoint `parse_differential.rs`'s whole-file candidate seam (that is S3,
//! where the gate is total). Instead it drives the EXPRESSION grammar through both
//! frontends and compares:
//!
//!   1. An adversarial fixture set (built from the grammar inventory — every
//!      operator at every adjacent-precedence boundary, the postfix/`?.` grouping
//!      cases, split shifts vs comparisons, unary stacking, `is` patterns, the
//!      struct-literal-vs-condition ambiguity, i-string holes, and every type
//!      form). Each fixture is wrapped identically for both frontends and the
//!      relevant subtree is extracted and `Debug`-compared (span-inclusive).
//!   2. Every corpus / std / example / docs source whose TOP-LEVEL items parse
//!      entirely within S2's subset — a count, reported, of how much of the 260 is
//!      already whole-file byte-identical before S3 wires the full grammar.
//!
//! THE ORACLE-ENTRY TECHNIQUE. The chumsky parser exposes no bare-expression
//! entry, so a fixture `EXPR` is embedded in a program the parser DOES accept and
//! the expression subtree is pulled back out:
//!   - expression position: `let __probe = EXPR;`  → the `Let`'s initializer.
//!   - condition position:  `if EXPR { }`           → the `If`'s condition.
//! Both frontends parse the SAME wrapped text, so spans align and a `Debug` match
//! is a byte-identical (span-identical) subtree. The wrapper is thin (`let`/`if`
//! are themselves S2 forms), so a wrapper bug would surface as a wholesale failure,
//! not a silent pass. Sound because `let`'s value and `if`'s condition are exactly
//! the `expression` / `condition_expression` productions the fixtures target.
//!
//! A SIBLING of `parse_differential.rs` (the S0 whole-file harness), not a change
//! to it: that target's fn-pointer seam still compares chumsky-against-chumsky and
//! S3 repoints it. This one retires at S5 with the rest of the differential.

use std::path::{Path, PathBuf};
use vilan_core::node::{Node, NodeIfBranch, NodeList};
use vilan_core::parse_clean;
use vilan_core::span::Spanned;

// ---------------------------------------------------------------------------
// The two frontends and the subtree extraction
// ---------------------------------------------------------------------------

/// ORACLE — the chumsky clean parse (held constant for the whole H6 arc). `None`
/// when the source is not perfectly clean (a lex/parse error or any recovery),
/// exactly as `parse_differential.rs`'s candidate uses it.
fn oracle(source: &str) -> Option<Spanned<NodeList<'_>>> {
    parse_clean(source)
}

/// CANDIDATE — the handwritten parser. `None` on any error (the clean-or-decline
/// contract), so it compares like-for-like with the oracle.
fn candidate(source: &str) -> Option<Spanned<NodeList<'_>>> {
    let (tree, errors) = vilan_core::parsing::parse(source);
    if errors.is_empty() { tree } else { None }
}

/// The `Debug` of the `let __probe = EXPR;` initializer, or `None` if the wrapper
/// did not parse to a single `Let` with a value.
fn let_initializer_debug(root: &Spanned<NodeList<'_>>) -> Option<String> {
    let (first, _) = root.0.first()?;
    if let Node::Let(_, _, Some(value), _) = first {
        Some(format!("{value:?}"))
    } else {
        None
    }
}

/// The `Debug` of the `if EXPR { }` condition, or `None` if the wrapper did not
/// parse to a single `If`.
fn if_condition_debug(root: &Spanned<NodeList<'_>>) -> Option<String> {
    let (first, _) = root.0.first()?;
    if let Node::If(NodeIfBranch::If(if_)) = first {
        Some(format!("{:?}", if_.condition))
    } else {
        None
    }
}

/// Compare one fixture in expression position through both frontends.
fn check_expression(fixture: &str) -> Result<(), String> {
    let wrapped = format!("let __probe = {fixture};");
    compare(fixture, &wrapped, let_initializer_debug)
}

/// Compare one fixture in condition position (struct-literal-free) through both
/// frontends.
fn check_condition(fixture: &str) -> Result<(), String> {
    let wrapped = format!("if {fixture} {{ }}");
    compare(fixture, &wrapped, if_condition_debug)
}

fn compare(
    fixture: &str,
    wrapped: &str,
    extract: impl Fn(&Spanned<NodeList<'_>>) -> Option<String>,
) -> Result<(), String> {
    let oracle_tree = oracle(wrapped);
    let candidate_tree = candidate(wrapped);
    let oracle_debug = oracle_tree.as_ref().and_then(&extract);
    let candidate_debug = candidate_tree.as_ref().and_then(&extract);
    match (oracle_debug, candidate_debug) {
        (Some(oracle_debug), Some(candidate_debug)) if oracle_debug == candidate_debug => Ok(()),
        (Some(oracle_debug), Some(candidate_debug)) => Err(format!(
            "MISMATCH on `{fixture}`\n    wrapped:   {wrapped}\n    oracle:    {oracle_debug}\n    candidate: {candidate_debug}"
        )),
        (Some(oracle_debug), None) => Err(format!(
            "candidate DECLINED `{fixture}` the oracle parses\n    wrapped:   {wrapped}\n    oracle:    {oracle_debug}"
        )),
        (None, Some(candidate_debug)) => Err(format!(
            "candidate parsed `{fixture}` the oracle DECLINES\n    wrapped:   {wrapped}\n    candidate: {candidate_debug}"
        )),
        (None, None) => Err(format!(
            "neither frontend produced a subtree for `{fixture}` (bad fixture?)\n    wrapped: {wrapped}"
        )),
    }
}

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

include!("parse_expr_fixtures.rs");

#[test]
fn expression_fixtures_agree_with_the_oracle() {
    let mut failures = Vec::new();
    for fixture in EXPRESSION_FIXTURES {
        if let Err(report) = check_expression(fixture) {
            failures.push(report);
        }
    }
    eprintln!(
        "expression fixtures: {} checked, {} failing",
        EXPRESSION_FIXTURES.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} expression fixture(s) diverged:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn condition_fixtures_agree_with_the_oracle() {
    let mut failures = Vec::new();
    for fixture in CONDITION_FIXTURES {
        if let Err(report) = check_condition(fixture) {
            failures.push(report);
        }
    }
    eprintln!(
        "condition fixtures: {} checked, {} failing",
        CONDITION_FIXTURES.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} condition fixture(s) diverged:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn whole_file_fixtures_agree_with_the_oracle() {
    // Whole-program (`parse_program`) coverage the corpus can't give — every corpus
    // file has items — over item-free statement sequences. Compares the entire
    // `NodeList`, span-inclusive.
    let mut failures = Vec::new();
    for fixture in WHOLE_FILE_FIXTURES {
        let oracle_debug = oracle(fixture).map(|root| format!("{root:?}"));
        let candidate_debug = candidate(fixture).map(|root| format!("{root:?}"));
        match (oracle_debug, candidate_debug) {
            (Some(oracle_debug), Some(candidate_debug)) if oracle_debug == candidate_debug => {}
            (oracle_debug, candidate_debug) => failures.push(format!(
                "MISMATCH on whole-file `{}`\n    oracle:    {:?}\n    candidate: {:?}",
                fixture.replace('\n', "\\n"),
                oracle_debug,
                candidate_debug
            )),
        }
    }
    eprintln!(
        "whole-file fixtures: {} checked, {} failing",
        WHOLE_FILE_FIXTURES.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} whole-file fixture(s) diverged:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn candidate_declines_exactly_what_the_oracle_declines() {
    // The handwritten parser must be no MORE permissive than the oracle: every
    // shape the grammar rejects (within the S2 subset) must decline through both.
    let mut failures = Vec::new();
    for fixture in DECLINER_FIXTURES {
        let wrapped = format!("let __probe = {fixture};");
        let oracle_accepts = oracle(&wrapped).and_then(|root| let_initializer_debug(&root));
        let candidate_accepts = candidate(&wrapped).and_then(|root| let_initializer_debug(&root));
        match (oracle_accepts, candidate_accepts) {
            (None, None) => {}
            (Some(_), _) => failures.push(format!(
                "`{fixture}` is not actually an oracle decliner (bad negative fixture)"
            )),
            (None, Some(candidate_debug)) => failures.push(format!(
                "candidate is MORE PERMISSIVE than the oracle on `{fixture}`\n    candidate: {candidate_debug}"
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} decliner fixture(s) diverged:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn fixture_count_makes_s3_boring() {
    // The proposal asks for a fixture count that de-risks S3 (≥200 across the two
    // modes). A tripwire so a future edit that guts the set is noticed.
    let total = EXPRESSION_FIXTURES.len() + CONDITION_FIXTURES.len();
    assert!(
        total >= 200,
        "expected >=200 adversarial fixtures, found {total}"
    );
}

// ---------------------------------------------------------------------------
// The corpus subset: whole files the S2 subset already parses byte-identically
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
                continue;
            }
            collect_markdown(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            into.push(path);
        }
    }
}

/// Mirrors `parse_differential.rs::collect_doc_examples` (test targets cannot
/// import one another).
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

#[test]
fn corpus_subset_within_the_s2_grammar_is_byte_identical() {
    let sources = all_sources();
    assert!(
        sources.len() > 150,
        "suspiciously few sources enumerated: {}",
        sources.len()
    );

    let mut whole_file_matches = 0usize; // candidate + oracle both clean AND equal
    let mut declined = 0usize; // candidate declined (an S3 item / macro form)
    let mut disagreements: Vec<String> = Vec::new();

    for (label, source) in &sources {
        let candidate_tree = candidate(source);
        let Some(candidate_tree) = candidate_tree else {
            declined += 1;
            continue;
        };
        // The candidate parsed the WHOLE file within the S2 subset; the oracle must
        // agree exactly. (An oracle decline here would mean the candidate accepts
        // something the oracle rejects — a real divergence.)
        match oracle(source) {
            Some(oracle_tree) if format!("{oracle_tree:?}") == format!("{candidate_tree:?}") => {
                whole_file_matches += 1;
            }
            Some(oracle_tree) => {
                disagreements.push(format!(
                    "{label}: candidate/oracle whole-file trees differ\n  candidate: {candidate_tree:?}\n  oracle:    {oracle_tree:?}"
                ));
            }
            None => {
                disagreements.push(format!(
                    "{label}: candidate parsed the whole file but the oracle DECLINES it"
                ));
            }
        }
    }

    eprintln!(
        "corpus subset: {} sources, {} whole-file byte-identical within the S2 subset, {} declined (S3 items/macros)",
        sources.len(),
        whole_file_matches,
        declined
    );
    assert!(
        disagreements.is_empty(),
        "{} source(s) the candidate parses diverged from the oracle:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}
