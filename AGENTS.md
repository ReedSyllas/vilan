# AGENTS.md — briefing for coding agents

Read this fully before touching code. `CLAUDE.md` states the contract (correctness over
speed of delivery, refactor-first, proven-before-implemented, root causes over
symptoms); this file is the map and the accumulated scar tissue. Where the two seem to
conflict, `CLAUDE.md` wins.

## The lay of the land

Rust workspace, four crates, plus the language's own tree:

- `crates/vilan-core` — the whole compiler as a library. Pipeline order: `lexer.rs` /
  `token.rs` → `parser.rs` (chumsky; cheap-first with rich fallback) → AST in `node.rs`
  → macro expansion in `macros.rs` (with `interpreter.rs`, the native evaluator that
  must stay behaviorally equivalent to emitted JS) → `lift.rs` (the expression-lifting
  rewrite — hooked at every parse entry: `lib.rs`, the CLI, the module loader, macro
  expansion; the formatter deliberately receives raw, un-lifted trees) → `analyzer.rs`
  (type solving + the inferred effects: `async_infer.rs`, `platform_color.rs`,
  `context.rs`, `call_graph.rs`, `const_eval.rs`) → `transformer.rs` (JS emission).
  Shared type machinery in `type_.rs`; diagnostics in `error.rs` — there is a house
  diagnostics standard, so match the shape of existing messages; `formatter.rs` is
  `vilan fmt`.
- `crates/vilan-cli` — the `vilan` binary and the end-to-end suites
  (`tests/corpus.rs`, `cancellation.rs`, `rpc_http.rs`, `streaming.rs`,
  `transport_robustness.rs`, …).
- `crates/vilan-lsp` — the language server.
- `crates/vilan-embedded-std` — embeds the std source into the binary.
- `vilan/std/src/*.vl` — the standard library, written in vilan. std loads as its own
  package with root-scoped module resolution.
- `vilan/test/` — the corpus: `.vl` programs with **byte-identical** `.js` goldens.
- `vilan/docs/` — the user-facing book + spec; every fenced example compiles.
- `vilan/proposal/` — design documents. Semantics are settled here **before** code;
  the proposal named in your work order is the spec for your change.

## Definition of done (the gates)

1. **Full suite green by exit code.** `cargo test` from the workspace root must exit
   0. Never judge success by grepping output: a piped grep masks the status, and a
   test target that fails to *compile* prints no `test result:` line at all. Capture
   the exit code explicitly (run the suite, then `echo "suite exit: $?"`) and report
   that line verbatim.
2. **Corpus byte-identical** (`cargo test -p vilan-cli --test corpus`) unless the work
   order says otherwise. If an *existing* golden changes: stop and report — never
   regenerate. New goldens require rebuilding the debug binary first (`cargo build`);
   a stale binary silently writes wrong goldens.
3. **Docs compile** (`cargo test --test docs`), and any change to std, a framework, or
   the language updates the affected `vilan/docs/` page in the same change-set.
4. **Per-case pins.** Every behavior added or changed gets its own tests in
   `crates/vilan-core/tests/inference.rs` (`assert_compiles`,
   `assert_compiles_and_runs`, `assert_fails`) — one pin per case, including the edge
   cases (multi-parameter, nested, mixed, ordering-sensitive), not one representative
   example. A known-but-unfixed gap is pinned `#[ignore]` with a comment saying why.
5. **`cargo fmt` after every Rust change.** It may reformat neighboring code —
   expected and desired. 4-space indent in Rust; full variable names (`parameter`,
   never `p`).

## Invariants and scar tissue (each of these has bitten before)

- **A new codegen helper in `transformer.rs` needs a matching `interpreter.rs` arm in
  the same change**, or the macro/native equivalence gate breaks.
- **Scalar-view classification goes only through `is_scalar_view_pointee` /
  `SCALAR_PRIMITIVE_NAMES`** (`type_.rs`). `bool` is an enum special-case that must
  appear in *every* view-pointee predicate, analyzer and transformer both — three
  drift sites have shipped real miscompiles.
- **Adding a variant to a core enum (`Type`, `Expr`, node kinds…) requires auditing
  every `_ =>` catch-all** that now silently mistreats it. Prefer exhaustive matches.
- **Never special-case a checker to quiet a pattern it rejects.** If legitimate std,
  corpus, or docs code trips a new check, that is a semantics-level event: stop and
  report it.
- **`.vl` probe files outside a package resolve no std** (no `Some`/`None`, no
  imports). Put probes in `vilan/test/` or a scratch directory carrying a
  `vilan.toml`.
- **Writing vilan:** match arms need a trailing comma even after `{}` block bodies;
  pattern bindings use `let` (`Some(let x)`); `.vl` indentation is tabs — the
  formatter (`target/debug/vilan fmt`) is authoritative.
- **std `.vl` files must not dispatch macros at world-load.** Derives carry their own
  imports and can leak them into the deriving module — never depend on a leaked
  import.
- **Generic-inference traps:** a trait bound can fail to propagate through the second
  parameter of a two-parameter generic call (restructure toward single-parameter
  shapes); struct-literal fields do not direct generic-call inference — annotate via a
  `let` binding.
- **Numerics:** the JS-backed integers are `i53`/`u53` (a ±2^53 contract); unknown
  numeric suffixes are hard errors.
- **A post-`analyze()` pass must be wired into BOTH pipelines** — `lib.rs`'s
  `analyze_source` (tests + LSP) *and* the CLI's duplicated sequence in
  `crates/vilan-cli/src/main.rs` — and verified with a CLI probe, not only an
  inference pin. A pass added to one place ships a check the other silently skips.
- **Mutating git is not yours.** Do not commit, push, tag, or regenerate published
  artifacts. (House rule for everyone: mutating git always names the repo via
  `git -C`, and never shares a compound command with `cd`.)

## How to work

- **Probe before you assert.** A five-line `.vl` program run through the freshly built
  binary (`cargo build`, then `target/debug/vilan run …`) beats speculation about
  semantics. Rebuild before trusting output.
- **Root causes.** Fix the general path; a special case that handles one input is a
  smell. If the general fix implies a refactor, say so in your report rather than
  building around the debt.
- **Read the named proposal sections first.** The current arc:
  `vilan/proposal/destruction-impl-plan.md` (the build sequence),
  `vilan/proposal/destruction.md` (the C4 design),
  `vilan/proposal/claims-and-epochs.md` (the memory-model frame and closure rule).
- **Report honestly and compactly:** what changed (files + why), what you ran with
  exact exit codes, what you did *not* verify, open questions. A true "unverified" is
  worth more than a false "works".

## Stop conditions (report instead of proceeding)

- An existing corpus golden or shipped test would need to change.
- A new check rejects existing std / corpus / docs code.
- The proposal underdetermines a semantic choice you would otherwise be making alone.
- The change wants a new dependency, new public CLI surface, or release machinery.
- Anything that would weaken a gate in order to pass it.
