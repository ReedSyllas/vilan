//! The handwritten frontend's corpus-scale regression sweep + fmt tripwire
//! (`proposal/frontend.md` §3 S5).
//!
//! Through the H6 arc this was a differential against the chumsky ORACLE, proving
//! the handwritten frontend byte-identical over every real source. At the S5
//! cutover chumsky is deleted, so the oracle arm retires: this target becomes the
//! new parser's own regression corpus — every `*.vl` in the repo, every std layer,
//! every example, every compilable docs fence, and the corpus-absent construct set
//! must parse CLEAN (a tree, zero diagnostics) through `parsing::parse`, with no
//! panic. The trees themselves were proven byte-identical to chumsky's at S3 and
//! are re-checked end-to-end by the corpus byte-gate (`vilan-cli --test corpus`);
//! this sweep guards the *front* of the pipeline — that the whole clean corpus
//! still parses without error or panic.
//!
//! The fmt tripwire converts the formatter's silent-no-op failure mode (§0: the
//! re-lex-and-compare safety net turns `fmt` into a no-op when the token stream
//! drifts, indistinguishable from an already-canonical file) into loud, external
//! checks: `formatter_output_token_matches_input_over_the_corpus` guards against
//! token-drifting output, and `formatter_never_silently_bails_over_the_corpus`
//! (the E13 closing gate, live since 2026-07-22) asserts `fmt` never silently
//! no-ops on a corpus file.

use std::path::{Path, PathBuf};
use vilan_core::token::Token;
use vilan_core::{formatter, lexing, parsing};

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
/// file-derived sweep never reaches them), each a clean program the parser must
/// accept. Only PARSED here (types need not resolve), so bare type names are fine.
/// This closes the corpus's coverage gaps — notably `[trait_only]` / `[doc(hidden)]`
/// (zero corpus uses) and the tuple-bound endpoint variants — alongside the
/// (durable) in-module pins in `parsing.rs`.
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

/// The full sweep corpus: `(label, source)` over `vilan/test`, every
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
// The regression sweep
// ---------------------------------------------------------------------------

#[test]
fn the_handwritten_frontend_parses_every_clean_source() {
    let sources = all_sources();
    assert!(
        sources.len() > 150,
        "suspiciously few sources enumerated: {}",
        sources.len()
    );

    // Every enumerated source is a complete, valid program (it compiles, or is a
    // parse-only adversarial construct), so it must parse CLEAN: a tree comes back
    // (always — the frontend never discards), with an EMPTY diagnostic list. A
    // non-empty list means the parser rejects a source it must accept — a real
    // regression, localized by label. `parsing::parse` never panics on any input
    // (the recovery contract), so reaching the end at all is part of the sweep.
    let mut rejected: Vec<String> = Vec::new();
    let mut clean = 0usize;
    for (label, source) in &sources {
        let (tree, errors) = parsing::parse(source);
        if tree.is_none() {
            rejected.push(format!("{label}: no tree returned"));
        } else if !errors.is_empty() {
            rejected.push(format!(
                "{label}: {} diagnostic(s) on a clean source: {}",
                errors.len(),
                parsing::render(&errors[0])
            ));
        } else {
            clean += 1;
        }
    }

    eprintln!(
        "parse sweep (handwritten frontend): N={} sources, {} parsed clean",
        sources.len(),
        clean
    );
    assert!(
        rejected.is_empty(),
        "the handwritten frontend rejected {} source(s) it must accept:\n{}",
        rejected.len(),
        rejected.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The fmt tripwire
// ---------------------------------------------------------------------------

/// The formatter's own notion of "the same code": the lexer's token stream with
/// spans stripped. Re-implemented here against the PUBLIC `lexing::tokenize` so the
/// check is external to `formatter.rs` (the point of a tripwire). Mirrors
/// `formatter::code_tokens` + `formatter::normalize`.
fn normalized_tokens(source: &str) -> Option<Vec<Token<'_>>> {
    let (spanned, lex_errors) = lexing::tokenize(source);
    if !lex_errors.is_empty() {
        return None;
    }
    let tokens: Vec<Token<'_>> = spanned.into_iter().map(|(token, _span)| token).collect();
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
    // successful reprint matches by the formatter's contract). This catches any
    // token-drifting output that slips the formatter's internal safety net.
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

/// The corpus files `vilan fmt` still silently no-ops on. E13 closed nine of the
/// ten H6 S0 bailers by adding the missing printer arms (destructuring, fixed
/// arrays, macro forms, unary minus, and the lift-chain postfix subject); this is
/// what remains.
///
/// `numeric-types.vl` is a DESIGN GAP, not a missing arm: it writes redundant
/// parentheses around a number literal in method-subject position —
/// `(300).as_u8()`. The parser dissolves parentheses around an atom (they add no
/// structure), so the tree is `MemberAccessor(Number(300), …)` — identical to
/// `300.as_u8()`. The printer canonicalizes it to `300.as_u8()` (correct — both
/// spellings compile to the same JS), but the safety net compares the OUTPUT's
/// tokens to the SOURCE's, sees the dropped parens, and refuses the reprint. The
/// net cannot allow paren-normalization (parens are usually semantic) and the AST
/// E13 closed 2026-07-22: every corpus file formats. The residual DESIGN gap
/// stays recorded (backlog E13 closure note): a redundant paren around a BARE
/// ATOM — `(300).as_u8()` — is dissolved by the parser and unrecorded in the
/// AST, so the printer can neither preserve nor safely drop it; such a file
/// bails safely (`fmt` no-ops). The corpus's four such sites were canonicalized
/// (emission byte-identical, probe-proven); a future fix is an AST-aware net or
/// parser-recorded parens.
#[test]
fn formatter_never_silently_bails_over_the_corpus() {
    let bails = current_bail_set();
    assert!(
        bails.is_empty(),
        "formatter SILENTLY BAILED on {} corpus file(s): {:?}",
        bails.len(),
        bails
    );
}
