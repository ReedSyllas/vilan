# Vilan Roadmap

A ranked backlog, most-important first, to be tackled roughly in order. Ranked by:
*unblocks real programs* > *cheap correctness* > *daily DX* > *strategic reach* >
*perf / advanced / cleanup*. Effort is S/M/L. Dependencies are noted inline.

Status note: value semantics + second-class views are substantially landed (the
memory-management impl plan's Phases 1–4 — `compute_clone_sites`, `is_elidable_copy`,
and the `view-*` / `value-semantics` tests). The active memory-model frontier is
Phase 5 (projections / `borrows`) and the deferred Phase 6 (`Shared<T>` / arenas).

---

## Tier 1 — Make it usable & keep it correct

1. **Stdlib essentials** (S–M, high ROI; thin JS wrappers over existing features)
   - `Result` combinators (`map`/`map_err`/`and_then`/`ok`/`unwrap_or`/…). Today only
     `is_ok`/`is_err`, while `Option` is rich — cheapest win, mirror `Option`. **(starting here)**
   - `String`: `len`/`split`/`contains`/`replace`/`starts_with`/`substring`/`to_uppercase`/`is_empty`.
   - `List`: `len`/`get`/`pop`/`map`/`filter`/`fold`/`contains`/`sort`.
   - `Iterator` combinators (`map`/`filter`/`fold`/`take`/`enumerate`) + a concrete
     `ListIterator` — needs an array-length intrinsic.
   - `PartialEq`/`Eq` impls for primitives (traits exist; no impls).

2. **Compiler-core robustness** (S, high trust — esp. for the LSP)
   - Convert internal `panic!`s to diagnostics (analyzer.rs:2292/2295/3075/4063); they're
     the reason a `catch_unwind` net exists.
   - Remove dead `prepped_struct_initializers`; decide the fate of the skeletal
     `interpreter.rs`; address `TODO: interning` (analyzer.rs:960).
   - **Parser gaps** (found while testing): (a) unary minus on a literal/expression
     (`-1`, `-x`) doesn't parse in value/argument position — only binary `a - b` works;
     (b) a struct literal can't be a binary operand — `Point { .. } == x` fails to parse,
     bind it to a variable first.
   - **Type-inference gap** (found while testing, M effort — arguably promote): field access
     on a value whose type is a generic parameter bound to a concrete struct fails —
     `list.map(|p| p.x)` and `list.get(i).unwrap().field` on a `List<Struct>` both error
     "cannot access field on generic T". The concrete element type isn't propagated to the
     closure-param / unwrapped site. **Gates collections (and `Option`) of structs with
     higher-order methods** — the Tier-1 `map`/`filter`/`get` work over primitive element
     types but not struct fields. This undercuts the stdlib for a common case (a list of
     records), so consider doing it before the rest of Tier 1. **Same root** affects
     operators inside a generic body: `==` on a generic-typed value doesn't dispatch (uses
     native `===`), so `Option<Struct>`/`List<Struct>` equality compares inner structs by
     reference. Concrete `==`/`!=` (structs, `Option<primitive>`, `Result<primitive,..>`)
     works (see equality.vl); the generic-element case needs this fix.

3. **Collections: `Map`/`Set` + `Hash`** (M; needs a small compiler `Hash`/equality story)

## Tier 2 — Toolchain & daily DX

4. **CLI subcommands + project model** (M) — `build`/`run`/`check`/`test`/`fmt`, a
   `vilan.toml` manifest, and multi-file project resolution (today: single entry file +
   special-cased std). Foundational: `fmt`/`test` hang off it.
5. **LSP autocomplete** (M–L) — highest-value editor feature; tiered plan already drafted
   (member `x.`, path `::`, scope/keyword).
6. **Code formatter** (`vilan fmt` + LSP formatting) (M–L) — needs comment-preserving
   formatting (the parser drops comments as trivia), not naive AST print-back.
7. **Test runner** (`vilan test`) (S–M) — replaces the shell-driven corpus check.

## Tier 3 — Strategic reach

8. **Browser backend** (L) — codegen is hardwired to Node (`console.log`, `process.exit`,
   `node:`). Needs a `Backend` abstraction + DOM intrinsics. Basic DOM codegen is
   independent; reactive UI also needs memory Phase 6 (`Shared<T>`).
9. **Compiler bindings / macros** (L) — see proposal/compiler-bindings.md. Unlocks numeric-type
   generation and derives (`PartialEq`/`Debug`/constructors). Needs a macro-expansion phase
   + struct reflection.
10. **LSP semantic highlighting** (M) — semantic tokens, precision over the TextMate grammar.
11. **More stdlib** (M, incremental) — `Display`/`Debug` + `format`, JSON, time/date,
    env/process, `fs`/`http` expansion. Some derives want #9.

## Tier 4 — Perf, advanced, cleanup

12. **Fix per-analysis `Box::leak` + incremental analysis** (L) — leak grows each
    keystroke/compile; true incremental is blocked by the global `entity_id`/`type_id`
    counters. Debounce masks the latency — measure first. (caching plan Tier 2/3)
13. **LSP sub-file incremental parsing** (L) — tree-sitter-style reuse; chumsky is a batch
    parser, so this is the largest, lowest-priority LSP item.
14. **Memory management Phase 5 → 6** (L) — projections/`borrows` (returned views, custom
    containers), then `Shared<T>`/arenas (signals, cyclic graphs, reactive UI). Prerequisite
    for #8's reactive path.
15. **Numeric types `u8`…`i64`/`f32`** (S) — low value for a JS target (collapse to
    `f64`/`BigInt`); do via #9's macro or defer to a WASM/native backend. Plus prune
    superseded `vilan/outdated/` sketches.

## Key dependencies
- array-length intrinsic → concrete iterators (#1)
- `Hash`/equality → collections (#3)
- manifest / multi-file → `fmt` / `test` (#4)
- macros → numeric types & derives (#9, #15)
- memory Phase 6 → reactive browser UI (#8, #14)
