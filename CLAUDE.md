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

- Regression test files live in `vilan/test/`. Run them against the compiler to verify nothing is broken by a change.
