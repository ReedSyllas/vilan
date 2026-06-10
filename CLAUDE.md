# vilan

Rust project: a compiler/interpreter for the "vilan" language.

## Conventions

- Use 4 spaces for indentation.
- Use full variable names like `parameter` over abbreviations like `p`.
- Run `cargo fmt` after every change. It may reformat pre-existing code that was not part of the change — that is expected and desired.
- Clean up surrounding code when it's reasonable to do so (rename unclear identifiers, remove dead code, simplify logic, etc.).

## Testing

- Regression test files live in `src/vilan-source/test/`. Run them against the compiler to verify nothing is broken by a change.
