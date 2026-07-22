//! Per-site parser/lexer recovery pins (H6 S0, `proposal/frontend.md` §3).
//!
//! The handwritten frontend that replaces chumsky (H6) must reproduce the
//! CURRENT parser's recovery behavior byte-for-byte. §0 of the proposal notes
//! that only the trailing-`.` member case (`member_completion_on_incomplete_receiver`
//! in vilan-lsp) is pinned as an observable today; the ten `nested_delimiters`
//! sites, the `?.` sibling, the misplaced-`resource` steer, and the lexer's
//! skip-then-retry are exercised only indirectly. These pins make each one an
//! explicit contract, asserting — against the handwritten `parsing::parse` (which
//! S4 gave recovery; through the arc these also ran against the chumsky oracle,
//! now deleted — see `frontends`) — that a garbled input at the site:
//!   (a) does NOT hard-fail (a partial tree comes back),
//!   (b) recovers to the documented placeholder (empty vec / `None` / `Node::Error`
//!       / empty block), and
//!   (c) reports a diagnostic (the error is not swallowed) — for the member cases
//!       the diagnostic surfaces during analysis, so those are pinned at both the
//!       parse level (tree shape) and the analyze level (see the analyze module).
//!
//! Diagnostic *counts and wording* are deliberately NOT over-pinned: proposal §6(a)
//! allows parse errors to improve at cutover. The pins assert "at least one error",
//! never an exact count. Recovered-tree *spans* ARE part of the contract (the S0
//! differential harness is span-inclusive) so the shape substrings carry them.
//!
//! Every recovered shape below was captured from the current binary, not asserted
//! from reading (H6 S0 is probe-first).

use vilan_core::parsing;

/// The handwritten frontend (H6 S4). Through the arc these pins ran against BOTH
/// this and the chumsky oracle (proven byte-identical); at the S5 cutover the
/// oracle is deleted and the pins hold the recovered SHAPES on the handwritten
/// frontend alone. Returns the recovered tree's `Debug` (if any) and the diagnostic
/// count.
fn handwritten_recovered(source: &str) -> (Option<String>, usize) {
    let (tree, errors) = parsing::parse(source);
    (tree.map(|tree| format!("{tree:?}")), errors.len())
}

/// The frontends every parse-level pin runs against. Post-cutover this is the
/// handwritten frontend alone (the `for_each_frontend` loop is retained so the
/// pins read unchanged).
type Frontend = fn(&str) -> (Option<String>, usize);
fn frontends() -> [(&'static str, Frontend); 1] {
    [("handwritten", handwritten_recovered)]
}

/// Run `check` against every frontend's recovery of `source`, asserting first that
/// recovery did NOT hard-fail (a partial tree came back — contract (a)). `check`
/// receives the frontend's name (for the failure message), the tree's `Debug`, and
/// the diagnostic count.
#[track_caller]
fn for_each_frontend(source: &str, check: impl Fn(&str, &str, usize)) {
    for (name, frontend) in frontends() {
        let (tree, errors) = frontend(source);
        let tree = tree.unwrap_or_else(|| {
            panic!("[{name}] recovery must yield a partial tree, not hard-fail (a), for {source:?}")
        });
        check(name, &tree, errors);
    }
}

// --- The ten `nested_delimiters` sites (parser.rs, verified 2026-07-21) --------

#[test]
fn recovers_garbled_generic_parameters() {
    // parser.rs ~248: a garbled `<...>` generic-PARAMETER list (on a declaration)
    // recovers via `nested_delimiters(<, >, .., |span| (Vec::new(), span))` to an
    // EMPTY parameter vec.
    for_each_frontend("fun f<1 2 3>() {}\n", |name, tree, errors| {
        assert!(
            errors > 0,
            "[{name}] garbled generic parameters must report (c): {tree}"
        );
        assert!(
            tree.contains("generic_parameters: Some(([]"),
            "[{name}] recovered to an empty generic-parameter vec (b); got: {tree}"
        );
    });
}

#[test]
fn recovers_garbled_generic_arguments() {
    // parser.rs ~269: a garbled `<...>` generic-ARGUMENT list (in a type position)
    // recovers to an EMPTY argument vec — here on the type `List<..>`.
    for_each_frontend("fun f(x: List<1 2 3>) {}\n", |name, tree, errors| {
        assert!(
            errors > 0,
            "[{name}] garbled generic arguments must report (c): {tree}"
        );
        assert!(
            tree.contains("AccessorWithGenerics(\"List\", ([]"),
            "[{name}] recovered to an empty generic-argument vec (b); got: {tree}"
        );
    });
}

#[test]
fn recovers_garbled_struct_initializer_fields() {
    // parser.rs ~299: a garbled `Name { .. }` struct-initializer field list
    // recovers via `|span| (None, span)` (then mapped to an empty vec) to EMPTY
    // fields.
    for_each_frontend(
        "fun main() { let p = Point { 1 2 3 }; }\n",
        |name, tree, errors| {
            assert!(
                errors > 0,
                "[{name}] garbled struct-init fields must report (c): {tree}"
            );
            assert!(
                tree.contains("StructInitializer(\"Point\", None, ([]"),
                "[{name}] recovered to empty struct-initializer fields (b); got: {tree}"
            );
        },
    );
}

#[test]
fn recovers_garbled_parenthesized_expression() {
    // parser.rs ~432: a garbled `( .. )` expression group recovers via
    // `|span| (Node::Error, span)` to a `Node::Error` in expression position.
    // (The shape is shared with the list site below; the `(` delimiter is what
    // routes recovery here — only the paren-recovery can fire on a paren group.)
    for_each_frontend("fun main() { let x = (1 +); }\n", |name, tree, errors| {
        assert!(
            errors > 0,
            "[{name}] garbled paren group must report (c): {tree}"
        );
        assert!(
            tree.contains("Let((\"x\", 17..18), None, Some((Error,"),
            "[{name}] garbled paren recovered to a Node::Error expression (b); got: {tree}"
        );
    });
}

#[test]
fn recovers_garbled_list_literal() {
    // parser.rs ~442: a garbled `[ .. ]` list literal recovers via
    // `|span| (Node::Error, span)` to a `Node::Error` in expression position.
    // The `[` delimiter routes recovery to the list site (not the paren site).
    for_each_frontend("fun main() { let x = [1 +]; }\n", |name, tree, errors| {
        assert!(
            errors > 0,
            "[{name}] garbled list literal must report (c): {tree}"
        );
        assert!(
            tree.contains("Let((\"x\", 17..18), None, Some((Error,"),
            "[{name}] garbled list recovered to a Node::Error expression (b); got: {tree}"
        );
    });
}

#[test]
fn recovers_garbled_block() {
    // parser.rs ~539: a garbled `{ .. }` block recovers via `|span| (None, span)`
    // to an EMPTY block (no statements, a `Void` tail). The non-empty source with
    // `errors > 0` proves this is recovery, not a legitimately-empty `fun main() {}`.
    for_each_frontend("fun main() { let x = 1 + ; }\n", |name, tree, errors| {
        assert!(errors > 0, "[{name}] garbled block must report (c): {tree}");
        assert!(
            tree.contains("body: Some((([], (Void,"),
            "[{name}] garbled block recovered to an empty block (b); got: {tree}"
        );
    });
}

#[test]
fn recovers_garbled_struct_body() {
    // parser.rs ~1160: a garbled `struct N { .. }` body recovers via
    // `|span| (None, span)` (mapped to an empty vec) to an EMPTY braced body.
    for_each_frontend("struct S { 1 2 3 }\n", |name, tree, errors| {
        assert!(
            errors > 0,
            "[{name}] garbled struct body must report (c): {tree}"
        );
        assert!(
            tree.contains("Struct((\"S\", 7..8), None, false, false, Some(([]"),
            "[{name}] garbled struct body recovered to empty fields (b); got: {tree}"
        );
    });
}

#[test]
fn recovers_garbled_impl_body_and_continues() {
    // parser.rs ~1210: a garbled `impl X { .. }` body recovers via
    // `|span| (Vec::new(), span)` to an EMPTY body, AND the following item still
    // parses (recovery synchronizes at the item boundary).
    for_each_frontend(
        "impl Foo { 1 2 3 }\nfun after() {}\n",
        |name, tree, errors| {
            assert!(
                errors > 0,
                "[{name}] garbled impl body must report (c): {tree}"
            );
            assert!(
                tree.contains("Impl((Accessor(\"Foo\"), 5..8), [], ([]"),
                "[{name}] garbled impl body recovered to an empty body (b); got: {tree}"
            );
            assert!(
                tree.contains("(\"after\""),
                "[{name}] the item after a recovered impl body must still parse; got: {tree}"
            );
        },
    );
}

#[test]
fn recovers_garbled_trait_body_and_continues() {
    // parser.rs ~1252: a garbled `trait X { .. }` body recovers via
    // `|span| (Vec::new(), span)` to an EMPTY body, and the following item parses.
    for_each_frontend(
        "trait Foo { 1 2 3 }\nfun after() {}\n",
        |name, tree, errors| {
            assert!(
                errors > 0,
                "[{name}] garbled trait body must report (c): {tree}"
            );
            assert!(
                tree.contains("Trait((\"Foo\", 6..9), None, [], ([]"),
                "[{name}] garbled trait body recovered to an empty body (b); got: {tree}"
            );
            assert!(
                tree.contains("(\"after\""),
                "[{name}] the item after a recovered trait body must still parse; got: {tree}"
            );
        },
    );
}

#[test]
fn recovers_garbled_module_body_and_continues() {
    // parser.rs ~1280: a garbled `mod X { .. }` body recovers via
    // `|span| (Vec::new(), span)` to an EMPTY body, and the following item parses.
    for_each_frontend(
        "mod foo { 1 2 3 }\nfun after() {}\n",
        |name, tree, errors| {
            assert!(
                errors > 0,
                "[{name}] garbled module body must report (c): {tree}"
            );
            assert!(
                tree.contains("Module(\"foo\", ([]"),
                "[{name}] garbled module body recovered to an empty body (b); got: {tree}"
            );
            assert!(
                tree.contains("(\"after\""),
                "[{name}] the item after a recovered module body must still parse; got: {tree}"
            );
        },
    );
}

// --- The member-recovery siblings (parse-level tree shape) ---------------------

#[test]
fn recovers_trailing_dot_member_keeping_receiver() {
    // parser.rs ~1933: a trailing `.` with no member (`p.`, mid-edit) recovers to
    // `Postfix::Member((Node::Error, dot_span))` while KEEPING the receiver — the
    // property the LSP's member completion relies on (its analyze-level diagnostic
    // is pinned in the `analyze` module below). This recovery is deliberately
    // SILENT at parse (0 parse errors), so no `errors` assertion here.
    for_each_frontend(
        "fun main() { let p = Point { x = 1 }; p. }\n",
        |name, tree, _errors| {
            assert!(
                tree.contains("MemberAccessor((Accessor(\"p\")"),
                "[{name}] the receiver `p` must survive the trailing `.` (b); got: {tree}"
            );
            assert!(
                tree.contains(", (Error,"),
                "[{name}] the missing member is a `Node::Error` placeholder (b); got: {tree}"
            );
        },
    );
}

#[test]
fn recovers_trailing_question_dot_member_keeping_receiver() {
    // parser.rs ~1958: the `?.` sibling of the trailing-`.` recovery — `p?.`
    // mid-edit recovers to `Postfix::LiftMember((Node::Error, dot_span))`, keeping
    // the receiver `p`. Also silent at parse (0 errors); the lift diagnostic is
    // pinned at the analyze level.
    for_each_frontend(
        "fun main() { let p = Point { x = 1 }; p?. }\n",
        |name, tree, _errors| {
            assert!(
                tree.contains("Lift((Accessor(\"p\")"),
                "[{name}] the receiver `p` must survive the trailing `?.` (b); got: {tree}"
            );
            assert!(
                tree.contains(", (Error,"),
                "[{name}] the missing `?.` member is a `Node::Error` placeholder (b); got: {tree}"
            );
        },
    );
}

// --- The misplaced-`resource` steer (recovery half) ----------------------------

#[test]
fn recovers_misplaced_resource_and_continues() {
    // parser.rs ~1501: `resource` before anything but `struct`/`enum` steers — it
    // emits a diagnostic and a `Node::Error` placeholder, leaving the offending
    // token unconsumed so `fun`/`impl`/`let`/`trait` parse as themselves. The
    // MESSAGE is already pinned in inference.rs (`resource_on_a_*_is_rejected`);
    // this pins the RECOVERY half — the steer placeholder plus the fact that the
    // steered item AND every subsequent item still parse.
    for_each_frontend(
        "resource fun foo() {}\nfun after() {}\n",
        |name, tree, errors| {
            assert!(
                errors > 0,
                "[{name}] the misplaced `resource` must report (c): {tree}"
            );
            assert!(
                tree.contains("(Error, 0..8)"),
                "[{name}] `resource` steered to a Node::Error placeholder (b); got: {tree}"
            );
            assert!(
                tree.contains("(\"foo\"") && tree.contains("(\"after\""),
                "[{name}] the steered `fun foo` and the following `fun after` must both parse; got: {tree}"
            );
        },
    );
}

// --- The lexer's skip-then-retry (lexer.rs ~257) -------------------------------

#[test]
fn lexer_skips_an_illegal_character_and_lexes_the_rest() {
    // lexer.rs ~257: `.recover_with(skip_then_retry_until(any().ignored(), end()))`
    // — an illegal character (here U+0007 BEL, which matches no token) is reported
    // and skipped, and the rest of the file lexes and parses normally. (The
    // handwritten lexer, S1, records the same skip; the char is mid-file, so
    // chumsky does NOT discard the stream — both frontends agree here.)
    for_each_frontend(
        "fun main() { let x = 1; \u{0007} let y = 2; }\n",
        |name, tree, errors| {
            assert!(
                errors > 0,
                "[{name}] the illegal character must report (c): {tree}"
            );
            assert!(
                tree.contains("(\"x\"") && tree.contains("(\"y\""),
                "[{name}] both statements around the illegal character must parse (b); got: {tree}"
            );
        },
    );
}

// --- Analyze-level pins for the member/resource contracts ----------------------
//
// The member recoveries are silent at parse; the diagnostic — and the proof the
// receiver still TYPES — surfaces during analysis. These pins mirror the LSP's
// `member_completion_on_incomplete_receiver` contract at the core level (per the
// H6 S0 work order), without touching vilan-lsp.
mod analyze {
    use std::path::{Path, PathBuf};
    use vilan_core::{Workspace, analyze_source};

    fn std_spec() -> vilan_core::PackageSpec {
        vilan_core::manifest::resolve_std(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"),
        )
    }

    /// Analyze `source` and return `(program_came_back, diagnostic_messages)`.
    #[track_caller]
    fn analyze(source: &str) -> (bool, Vec<String>) {
        let leaked: &'static str = Box::leak(source.to_string().into_boxed_str());
        let (program, errors) = analyze_source(
            leaked,
            &std_spec(),
            Path::new("."),
            Path::new("test.vl"),
            None,
            &Workspace::default(),
        );
        (
            program.is_some(),
            errors.into_iter().map(|error| error.msg).collect(),
        )
    }

    #[test]
    fn trailing_dot_member_analyzes_with_a_diagnostic() {
        // The receiver `p: Point` is typed despite the trailing `.`; the analyzer
        // reports the missing member rather than discarding the statement.
        let (program, messages) = analyze(
            "struct Point { x: i32, y: i32 }\n\
             fun main() { let p = Point { x = 1, y = 2 }; p. }\n",
        );
        assert!(program, "a recovered `p.` must still produce a Program");
        assert!(
            messages
                .iter()
                .any(|m| m.contains("field or method name after")),
            "the missing member must be diagnosed; got: {messages:#?}"
        );
    }

    #[test]
    fn trailing_question_dot_member_types_the_receiver() {
        // The decisive proof the receiver still TYPES: the `?.` lift diagnostic
        // names the receiver's resolved type ("this is Point"), which the analyzer
        // could only know by typing `p` — exactly what completion after `p?.`
        // needs. Mirrors the LSP `member_completion_on_incomplete_receiver` pin.
        let (program, messages) = analyze(
            "struct Point { x: i32, y: i32 }\n\
             fun main() { let p = Point { x = 1, y = 2 }; p?. }\n",
        );
        assert!(program, "a recovered `p?.` must still produce a Program");
        assert!(
            messages.iter().any(|m| m.contains("this is Point")),
            "the receiver must type to Point despite the trailing `?.`; got: {messages:#?}"
        );
    }

    #[test]
    fn misplaced_resource_analyzes_the_rest_of_the_file() {
        // The recovery half at the analyze level: after the steered `resource fun`,
        // the following struct + function still analyze (the sole diagnostic is the
        // steer message, and `Point` is usable downstream).
        let (program, messages) = analyze(
            "resource fun foo() {}\n\
             struct Point { x: i32 }\n\
             fun after() { let q = Point { x = 5 }; }\n",
        );
        assert!(program, "a steered `resource` must still produce a Program");
        assert!(
            messages
                .iter()
                .any(|m| m.contains("type-declaration modifier")),
            "the steer diagnostic must be present; got: {messages:#?}"
        );
        assert!(
            messages
                .iter()
                .all(|m| !m.to_lowercase().contains("cannot find")
                    && !m.to_lowercase().contains("unknown")),
            "no downstream item should be lost to the steer; got: {messages:#?}"
        );
    }
}
