# vilan

Rust project: a compiler/interpreter for the "vilan" language.

## Engineering principles

This project prioritizes **correctness and long-term maintainability over speed of delivery**,
and it can afford to — there is no deadline pressure. Hold the bar high:

- **Correctness over quickness.** Never rush a feature. A slower, correct, well-understood
  implementation always beats a fast one that mostly works. The project can afford the time.
- **Refactoring is the preferred path.** Refactoring is a *good thing*. When technical debt or an
  awkward abstraction makes a feature hard to implement cleanly, pay the debt down first rather
  than building around it. The project can afford it. The codebase only grows more feature-dense
  over time, so maintainability is a primary goal, not an afterthought — it is what keeps the
  whole thing tractable.
- **Prove a feature before implementing it.** A new feature should be *proven* first: a formal
  definition of its semantics, then unit tests and regression tests that pin that behavior down.
  Design on paper (a proposal under `vilan/proposal/`), settle the semantics, then build.
- **Fix root causes, not symptoms.** A bug fix must address the underlying cause. Do not paper
  over a failure with a special-case patch that handles one input — find why the *general* path is
  wrong and fix that. Special cases are a smell; prefer general code that subsumes them.

## Conventions

- Use 4 spaces for indentation.
- Use full variable names like `parameter` over abbreviations like `p`.
- Run `cargo fmt` after every change. It may reformat pre-existing code that was not part of the change — that is expected and desired.
- Clean up surrounding code when it's reasonable to do so (rename unclear identifiers, remove dead code, simplify logic, etc.).

## Testing

- Regression test files live in `vilan/test/`. Run them against the compiler to verify nothing is broken by a change. Their `.js` goldens are a **byte-identical** gate: a change that alters them is either a bug or a deliberate, verified improvement — regenerate a golden only after confirming the new output is correct (and rebuild the debug binary first, or a stale binary writes wrong goldens).
- **Add unit tests for critical code.** Any change to a critical subsystem — the type solver / analyzer (`crates/vilan-core/src/analyzer.rs`), the transformer / codegen (`src/transformer.rs`), the language server (`crates/vilan-lsp/`), and the lexer/parser — must come with unit tests that pin its behavior, including the **edge cases** (the multi-parameter, nested, mixed, and ordering-sensitive forms — not just the happy path). The compiler-behavior harness lives in `crates/vilan-core/tests/inference.rs` (`assert_compiles`, `assert_compiles_and_runs`, `assert_fails`); a known-but-unfixed bug is pinned as an `#[ignore]`d test and un-ignored when fixed.
- **Docs are gated and part of done.** Every fenced example in `vilan/docs/` is compiled by `cargo test --test docs`; a change to std, a framework, or the language updates the affected docs page **in the same commit** (see `vilan/proposal/documentation.md`).
- **"Fixed" and "closed" require a pinned test — per case, not per example.** Do not claim a bug fixed, or a class of bugs closed, on the strength of a green suite plus one representative program. Each distinct case needs its own passing (or, if still open, `#[ignore]`d) test. Edge cases without a test are how a "closed" item silently regresses or turns out never to have been covered.
