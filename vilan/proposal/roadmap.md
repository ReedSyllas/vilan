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
   - ✅ Internal `panic!`s converted to diagnostics (commit bf434eb) — malformed input now
     degrades gracefully instead of "no program"; `catch_unwind` stays as a `.unwrap()` backstop.
   - ✅ Cleanup (commit 8fa1c1e): removed dead `prepped_struct_initializers`; deleted the
     orphaned (uncompiled) `interpreter.rs`; documented the `type_id_for_type` interning decision
     (deferred — in-place type mutation would alias shared ids; needs `Type: Hash + Eq`).
   - **Parser gaps**: (a) ✅ unary minus (`-1`, `-x`, `f(-1)`) now parses (commit ac9e26e);
     (b) a struct literal still can't be an operator operand — `Point { .. } == x` fails (bind it
     to a variable first); needs a `no-struct-literal` expression mode for conditions (à la Rust),
     so deferred — now documented at the parser site. Degrades to a clean parse error.
   - **Generic-dispatch gaps** (M–L; partially fixed):
     - ✅ **Closure params** now type from the concrete receiver (commit 4960aab): a closure
       passed to a generic method types its param to the element type, so `Option<Struct>` and
       explicitly-typed `List<Struct>` higher-order methods (`map`/`filter`/`is_some_and`) work
       (closure-param-inference.vl).
     - Remaining (S–M): an **inferred** List element (`List::new()`+`push`, or a chained
       `filter().map()` whose intermediate element is inferred) is typed too late for the
       closure inference, so field access still fails — workaround: annotate `mut xs: List<T>`.
     - ✅ **M2 (the deep one)**: a method/operator call on a generic-bounded value now
       dispatches to the concrete impl per monomorphization (commit 099d908). `a.eq(b)`,
       `x == y`, `x != y` where `x: T: PartialEq` re-resolve via the receiver's constraint
       (new `generic_method_dispatch` map, mirroring `generic_static_accessors`). `Option<Struct>
       ==` works too — the operator method monomorphizes against the operand's type args so its
       inner element compare dispatches concretely; a native-equality element (primitive / numeric
       enum) stays native. Also fixed a 7da43bf regression where a C-like (numeric) enum's `==`
       errored (`is_native_operator_type` now covers numeric enums). See generic-equality.vl.

3. ✅ **Collections: `Map`/`Set`** (primitive keys) — external wrappers over JS Map/Set, loaded
   on import (commits 586d581 Set, 58afcec Map, be66d0c Map iteration). `Set<T>`:
   new/insert/contains/remove/len/is_empty + `for x in set`. `Map<K, V>`:
   new/insert/get(→`Option<V>`)/contains_key/remove/len/is_empty + keys()/values()(→`List`) for
   iteration. (i) `__clone` now recurses into JS Set/Map (else they alias on copy); (ii) K/V/T bind
   from an explicit annotation (`mut m: Map<str, i32> = Map::new()`) — no List-style element-slot
   inference; (iii) **struct keys deferred**: M2 gives value `==`, but JS Map/Set key objects by
   *reference*, so by-value aggregate keys still need a key-serialization / custom-table strategy.
   Tests: set.vl, map.vl.

## Tier 2 — Toolchain & daily DX

4. ✅ **CLI subcommands + project model** — clap subcommands (commit 9a6dd18): `build` and
   `check` are real; `run`/`fmt`/`test` are placeholders pending their features (#6/#7).
   `vilan.toml` manifest + project discovery (d0b530f): `build`/`check` resolve the entry from
   the nearest `vilan.toml` (or a project dir), `[package] entry` defaulting to `main.vl`. Multi-
   file `import pkg::<module>` resolves to the entry's package siblings (366eee5). `run`
   implemented (47beac6: build + `node`, propagates exit code). Example: `vilan/examples/math`.
5. ✅ **LSP autocomplete** (commit 01ff369) — `textDocument/completion` dispatched by context:
   `receiver.` → fields + methods (receiver resolved by name through scope, with a same-file
   fallback for mid-edit), `Path::` → enum variants / struct methods / module members, open scope
   → visible names + keywords. Self-contained in vilan-lsp over existing `Program` data; unit-
   tested per context. Relied on a core parser fix (72f76cd): an incomplete `p.` now recovers to
   `MemberAccessor(_, Error)` (clearer error + receiver still analyzes), corpus byte-identical.
6. **Code formatter** (`vilan fmt` + LSP formatting) (M–L) — needs comment-preserving
   formatting (the parser drops comments as trivia), not naive AST print-back. The `fmt`
   subcommand is still a placeholder, gated on this.
7. ✅ **Test runner** (`vilan test`) (commit 7a18546) — runs `*_test.vl` tests (Go-style,
   alongside source so `pkg::` resolves); pass = exit 0, fail = compile error / non-zero
   (a failed `assert` panics). Reports `N passed, M failed`. Added `std::assert`. NOTE: this is a
   *behavior* runner for user code — the compiler's own `.js` codegen-snapshot corpus (vilan/test/)
   is a separate dev-time check, not yet migrated to it.

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
