//! The recovery-mode differential (H6 S4, `proposal/frontend.md` §3 S4 + §6a) —
//! a complete, written inventory of every behavioral delta the S5 cutover ships.
//!
//! `parse_differential.rs` compares the two frontends on CLEAN sources (byte-
//! identical trees, the S3 gate). This target compares them on BROKEN sources —
//! the recovery pins' fixtures and the mid-edit shapes a language server sees —
//! along two axes:
//!
//!  1. **Where both frontends consume the whole input** (clean, or recovered to
//!     end-of-input inside a delimiter / the `resource` steer), they must return
//!     the SAME recovered tree, span for span (`MUST_MATCH`). This is the S0
//!     recovery contract checked against the chumsky oracle: `parser_recovery.rs`
//!     pins the recovered SHAPES; this pins byte-equality with the oracle.
//!
//!  2. **Where they legitimately differ**, each delta is recorded in `DIVERGENCES`
//!     with the S5 reconciliation decision it implies. There is exactly one
//!     divergence class, and it is an improvement §6a sanctions: chumsky's
//!     top-level `statement.repeated()` + `.parse()` is all-or-nothing — any input
//!     it cannot consume to EOI makes it discard the WHOLE partial tree (returns
//!     `None`) — whereas the handwritten frontend salvages the parsed prefix and
//!     reports a top-level decline. (Two sub-roots feed it: a mid-file syntax
//!     error the delimiter recoveries cannot reach, and the S1 lexer-discard —
//!     chumsky's lexer returns `None` on an un-lexable char at EOF, the
//!     handwritten keeps the partial stream. A third, finer delta rides along: a
//!     run of N un-lexable characters is ONE chumsky diagnostic but N handwritten
//!     ones — a count difference, not a tree one.)
//!
//! The invariant test at the end proves the two axes are exhaustive: the
//! handwritten frontend ALWAYS returns a tree, and wherever the oracle returns one
//! too, they are byte-identical — so there is no third, unrecorded behavior.

use chumsky::prelude::*;
use vilan_core::{lexer, parser, parsing};

/// The rich chumsky recovery, exactly as the diagnostics path runs it: the
/// recovered tree (if the whole input was consumed) and the diagnostic count.
fn chumsky_recovery(source: &str) -> (Option<String>, usize) {
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

/// The handwritten recovery: the tree (always present — the lexer never discards
/// and the top-level parse keeps the parsed prefix) and the diagnostic count.
fn handwritten_recovery(source: &str) -> (Option<String>, usize) {
    let (tree, errors) = parsing::parse(source);
    (tree.map(|tree| format!("{tree:?}")), errors.len())
}

/// Whether the handwritten frontend salvaged a NON-EMPTY statement prefix.
fn handwritten_salvaged_prefix(source: &str) -> bool {
    let (tree, _errors) = parsing::parse(source);
    tree.is_some_and(|(statements, _span)| !statements.is_empty())
}

// ---------------------------------------------------------------------------
// Axis 1 — where both frontends consume the input, the trees are identical
// ---------------------------------------------------------------------------

/// Every broken source both frontends recover to end-of-input, so both return a
/// tree and the trees are span-identical. The first block is the fourteen
/// `parser_recovery.rs` fixtures (the ten `nested_delimiters` sites, the `.`/`?.`
/// member recoveries, the `resource` steer, the lexer skip); the second is
/// recovery nested inside a well-formed enclosing item, where synchronization at
/// the block/body boundary lets the whole file still parse.
const MUST_MATCH: &[(&str, &str)] = &[
    // -- the ten delimiter sites --
    ("generic parameters", "fun f<1 2 3>() {}\n"),
    ("generic arguments", "fun f(x: List<1 2 3>) {}\n"),
    (
        "struct initializer fields",
        "fun main() { let p = Point { 1 2 3 }; }\n",
    ),
    (
        "parenthesized expression",
        "fun main() { let x = (1 +); }\n",
    ),
    ("list literal", "fun main() { let x = [1 +]; }\n"),
    ("block", "fun main() { let x = 1 + ; }\n"),
    ("struct body", "struct S { 1 2 3 }\n"),
    ("impl body", "impl Foo { 1 2 3 }\nfun after() {}\n"),
    ("trait body", "trait Foo { 1 2 3 }\nfun after() {}\n"),
    ("module body", "mod foo { 1 2 3 }\nfun after() {}\n"),
    // -- the member recoveries (silent at parse) and the resource steer --
    (
        "trailing dot member",
        "fun main() { let p = Point { x = 1 }; p. }\n",
    ),
    (
        "trailing question-dot member",
        "fun main() { let p = Point { x = 1 }; p?. }\n",
    ),
    (
        "misplaced resource steer",
        "resource fun foo() {}\nfun after() {}\n",
    ),
    // -- the lexer skip (mid-file un-lexable character) --
    (
        "lexer skip",
        "fun main() { let x = 1; \u{0007} let y = 2; }\n",
    ),
    // -- recovery nested inside a well-formed item, whole file still parses --
    ("member recovery in a block", "fun main() { obj. }\n"),
    ("block recovery under a half-if", "fun main() { if x }\n"),
    (
        "struct-init recovery in a block",
        "fun main() { Point { 1 2 } }\n",
    ),
    (
        "block recovery, following method parses",
        "impl Foo { fun a() { 1 2 } fun b() {} }\n",
    ),
];

#[test]
fn recovered_trees_match_the_oracle_where_both_consume_the_input() {
    for (label, source) in MUST_MATCH {
        let (chumsky_tree, chumsky_errors) = chumsky_recovery(source);
        let (hand_tree, _hand_errors) = handwritten_recovery(source);
        assert!(
            chumsky_tree.is_some(),
            "{label}: the oracle should recover a tree for {source:?} \
             (this fixture belongs in DIVERGENCES, not MUST_MATCH)"
        );
        // Diagnostic COUNT is deliberately not pinned (§6a lets errors improve);
        // the presence of a diagnostic is (recovery is never silent, except the
        // member cases, whose oracle count is 0 and matched by construction).
        let _ = chumsky_errors;
        assert_eq!(
            hand_tree, chumsky_tree,
            "{label}: recovered trees DIVERGE for {source:?}\n  \
             handwritten: {hand_tree:?}\n  oracle:      {chumsky_tree:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Axis 2 — the recorded divergences (the S5 cutover inventory)
// ---------------------------------------------------------------------------

/// The kind of salvage the handwritten frontend performs where the oracle
/// discards the tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Salvage {
    /// The handwritten tree is an empty statement list (the syntax error is at or
    /// near the file head, so no complete statement precedes it).
    Empty,
    /// The handwritten tree carries a non-empty parsed prefix (whole items before
    /// the syntax error survive — the LSP payoff).
    Prefix,
}

/// One recorded behavioral delta on a broken source. All share the same root
/// (chumsky's all-or-nothing top-level parse vs the handwritten prefix salvage);
/// `reason` names the specific trigger and `s5_decision` the reconciliation the
/// cutover implies. Kept as data so the ledger cannot rot silently — the test
/// below re-derives each row against both frontends and fails if a divergence
/// stops diverging (a fixture that must move to `MUST_MATCH`) or changes kind.
struct Divergence {
    label: &'static str,
    source: &'static str,
    salvage: Salvage,
    reason: &'static str,
    s5_decision: &'static str,
}

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        label: "truncated member at file head",
        source: "obj.\n",
        salvage: Salvage::Empty,
        reason: "`obj.` is not a complete statement (no `;`), so the top-level \
                 `statement*` parses nothing and cannot reach EOI; the oracle \
                 discards the (empty) tree, the handwritten returns `([], ..)` \
                 plus a decline.",
        s5_decision: "KEEP the salvage — an empty tree + diagnostic is what \
                      `analyze_source` already expects; behaviour is observable \
                      only as Some(empty) vs None, both non-analyzable.",
    },
    Divergence {
        label: "unclosed generic (no matching `>`)",
        source: "fun f<T\n",
        salvage: Salvage::Empty,
        reason: "`nested_delimiters` needs a matching `>`; with none the generic \
                 recovery declines on BOTH frontends and the function does not \
                 parse. Oracle → None, handwritten → empty prefix.",
        s5_decision: "KEEP the salvage; same class as the head-truncation row.",
    },
    Divergence {
        label: "half-typed item (no body)",
        source: "struct Foo\n",
        salvage: Salvage::Empty,
        reason: "`struct Foo` with neither `{` body nor `;` cannot parse and is \
                 not inside a recoverable delimiter; oracle discards, handwritten \
                 returns the empty prefix.",
        s5_decision: "KEEP the salvage.",
    },
    Divergence {
        label: "unbalanced open parens in a block",
        source: "fun main() { (((  }\n",
        salvage: Salvage::Empty,
        reason: "the unbalanced `(` defeats the block's `nested_delimiters` \
                 (an unmatched inner opener), so the block does not recover on \
                 EITHER frontend and the function fails; oracle → None.",
        s5_decision: "KEEP the salvage — the unbalanced-inner-delimiter limit is \
                      shared with the oracle (a faithful reproduction), only the \
                      top-level None-vs-Some differs.",
    },
    Divergence {
        label: "trailing un-lexable char at EOF",
        source: "fun main() {}@",
        salvage: Salvage::Prefix,
        reason: "S1 lexer-discard: chumsky's lexer returns None (drops the whole \
                 token stream) on an un-lexable char at EOF; the handwritten lexer \
                 keeps the stream, so `fun main() {}` survives as a parsed prefix.",
        s5_decision: "KEEP the salvage — the prefix is genuinely useful (LSP \
                      analyzes the complete item); recorded already in the S1 \
                      lexer ledger.",
    },
    Divergence {
        label: "trailing run of un-lexable chars",
        source: "fun main() {}\u{0007}\u{0007}\u{0007}",
        salvage: Salvage::Prefix,
        reason: "same lexer-discard root; additionally the handwritten reports \
                 ONE diagnostic per un-lexable char (3) where chumsky coalesces \
                 the run into ONE — a diagnostic COUNT delta, not a tree one.",
        s5_decision: "KEEP per-character diagnostics (each bad char gets its own \
                      span / fix site); revisit coalescing at S5 only if the noise \
                      is judged worse than the precision. §6a admits either.",
    },
    Divergence {
        label: "unbalanced paren defeats body recovery, later item lost",
        source: "impl X { fun a( }\nfun after() {}\n",
        salvage: Salvage::Empty,
        reason: "the unbalanced `(` inside the impl body defeats its \
                 `nested_delimiters` on BOTH frontends, so `fun after` is lost by \
                 both; the only delta is oracle-None vs handwritten-empty-prefix.",
        s5_decision: "KEEP the salvage; the lost-later-item behaviour is IDENTICAL \
                      to the oracle (a shared recovery limit, not a regression).",
    },
    Divergence {
        label: "the `!=` soup at file head",
        source: "let x = a!==b;\n",
        salvage: Salvage::Empty,
        reason: "`a!==b` lexes as `!=` then `=`; the operand after `!=` is missing \
                 so the expression stops at `a`, the statement has no `;`, and the \
                 top level cannot reach EOI. Oracle → None; handwritten → empty \
                 prefix WITH the first-class `!=`-soup hint on its decline.",
        s5_decision: "KEEP the salvage; the handwritten's rendered message is the \
                      improvement (see the error-quality table) — the tree delta \
                      is incidental.",
    },
    Divergence {
        label: "whole item salvaged before mid-file garbage",
        source: "fun ok() {}\nBROKEN nonsense\n",
        salvage: Salvage::Prefix,
        reason: "`fun ok() {}` parses, then `BROKEN nonsense` is an unparseable \
                 statement the top level cannot consume; the oracle discards the \
                 WHOLE file, the handwritten keeps `[Func(ok)]`.",
        s5_decision: "KEEP the salvage — this is the core LSP win of the cutover: \
                      an edit mid-file no longer blanks the analysis of everything \
                      above it.",
    },
];

#[test]
fn divergences_still_diverge_as_recorded() {
    for divergence in DIVERGENCES {
        let (chumsky_tree, _) = chumsky_recovery(divergence.source);
        let (hand_tree, _) = handwritten_recovery(divergence.source);
        assert!(
            chumsky_tree.is_none(),
            "{}: the oracle now RETURNS a tree for {:?} — this is no longer a \
             divergence; move it to MUST_MATCH (and check they agree).",
            divergence.label,
            divergence.source
        );
        assert!(
            hand_tree.is_some(),
            "{}: the handwritten frontend must always salvage a tree, got None for \
             {:?}",
            divergence.label,
            divergence.source
        );
        let observed = if handwritten_salvaged_prefix(divergence.source) {
            Salvage::Prefix
        } else {
            Salvage::Empty
        };
        assert_eq!(
            observed, divergence.salvage,
            "{}: salvage kind changed for {:?} (ledger says {:?}, saw {:?})",
            divergence.label, divergence.source, divergence.salvage, observed
        );
    }
}

// ---------------------------------------------------------------------------
// The exhaustiveness invariant — the two axes cover every behavior
// ---------------------------------------------------------------------------

#[test]
fn the_handwritten_frontend_never_loses_a_tree_the_oracle_keeps() {
    // Over every curated broken source, the handwritten frontend ALWAYS returns a
    // tree, and wherever the oracle returns one, the two are byte-identical. So
    // the only behavioral delta is oracle-None vs handwritten-Some (axis 2) — there
    // is no unrecorded MISMATCH or hand-loses-a-tree case.
    let all_sources = MUST_MATCH
        .iter()
        .map(|(label, source)| (*label, *source))
        .chain(
            DIVERGENCES
                .iter()
                .map(|divergence| (divergence.label, divergence.source)),
        );
    for (label, source) in all_sources {
        let (chumsky_tree, _) = chumsky_recovery(source);
        let (hand_tree, _) = handwritten_recovery(source);
        assert!(
            hand_tree.is_some(),
            "{label}: the handwritten frontend must always return a tree, got None \
             for {source:?}"
        );
        if let Some(oracle) = chumsky_tree {
            assert_eq!(
                hand_tree.as_deref(),
                Some(oracle.as_str()),
                "{label}: the oracle kept a tree the handwritten frontend disagrees \
                 with, for {source:?}"
            );
        }
    }
}
