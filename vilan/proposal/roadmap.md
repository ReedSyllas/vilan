# Vilan Roadmap

A ranked backlog, most-important first, to be tackled roughly in order. Ranked by:
*unblocks real programs* > *cheap correctness* > *daily DX* > *strategic reach* >
*perf / advanced / cleanup*. Effort is S/M/L. Dependencies are noted inline.

Status note (updated): **Tiers 1 and 2 are complete, and most of Tier 3's stdlib/derives.**
The memory model is done through Phase 5 — value semantics, second-class views, inferred
`borrows`, `(base,key)` views, subscript/index + element-iteration views, view-returning
`Option<&T>` (direct/conditional/aggregate — the `Arena::get` shape), `for e in &mut
container`, plus Phase 6 essentials `Arena`/`Handle` and `Shared<T>`. The **frontier is now
the two big strategic items: the browser backend (#8) and the general macro engine (#9)** —
derives already shipped as a special-cased subset of #9. Remaining Phase 6+ memory tail
(`Weak<T>`, dynamic rule-4, no-view-across-`await`, deterministic destruction) is deferred.

---

## Tier 1 — Make it usable & keep it correct

1. ✅ **Stdlib essentials** (S–M, high ROI; thin JS wrappers over existing features) — done.
   - ✅ `Result` combinators (c9fa96f, mirrors `Option`).
   - ✅ `String`: `len`/`split`/`contains`/`replace`/`starts_with`/`substring`/… (08d4da3).
   - ✅ `List`: `len`/`get`/`pop`/`map`/`filter`/`fold`/`contains`/`sort` (3018d5d, 4b3b79d).
   - ✅ `PartialEq`/`Eq` primitive impls + `==`/`!=` dispatch (7da43bf).
   - `Iterator` combinators **deprioritized** — `List` covers iteration; `for x in <custom>`
     works via `next(): Option<T>`, and `for e in &mut <custom>` via `next_mut(): Option<&mut T>`.

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
6. ✅ **Code formatter** (`vilan fmt` + LSP formatting) — done (5bf7a3a CLI, 44e23d3/4841223
   printer, 41294e4 i-strings, 85159de LSP). A whole-AST pretty-printer in
   `vilan-core/src/formatter.rs` with a token-stream safety net (re-lex output, compare tokens
   modulo trivia/trailing-commas; on any mismatch return the source unchanged, so a printer bug
   can't corrupt a file). Comments reattached from source spans; `i"..."` recovered verbatim by
   span. Handles the whole language; the entire std+test corpus formats with zero bails, all
   programs recompile byte-identical. `textDocument/formatting` wires it into the LSP (whole-doc
   edit; the VS Code client auto-enables Format Document + format-on-save).
7. ✅ **Test runner** (`vilan test`) (commit 7a18546) — runs `*_test.vl` tests (Go-style,
   alongside source so `pkg::` resolves); pass = exit 0, fail = compile error / non-zero
   (a failed `assert` panics). Reports `N passed, M failed`. Added `std::assert`. NOTE: this is a
   *behavior* runner for user code — the compiler's own `.js` codegen-snapshot corpus (vilan/test/)
   is a separate dev-time check, not yet migrated to it.

## Tier 3 — Strategic reach

8. **Browser backend** (L) — **the top remaining strategic item.** Codegen is hardwired to Node
   (`console.log`, `process.exit`, `node:` host imports). The goal: one Vilan project where the
   **server and client are both Vilan**, the Node server serves the compiled client bundle to the
   browser, and shared modules compile under both targets. Design (full-stack, see notes below):
   a `--target node|browser` flag threaded through codegen; the std split into a universal **core**
   layer + platform layers (`node`: fs/http/process; `dom`: document/events/fetch) gated by target;
   target-aware `@extern` host-import emission (Node `import {x} from "node:.."` vs browser globals);
   a project model with `[server]`/`[client]` entries (`vilan build` emits `dist/server.js` +
   `dist/client.js`, server serves the client). Reactive UI uses `Shared<T>` (now landed).
   First slice — a `--target browser` + minimal `std::dom` — is independent.
9. **Compiler bindings / macros** (L) — see proposal/compiler-bindings.md. ✅ **Built-in derives
   done** (0cb21c8 PartialEq, 01918b3 Default, 3b250f2 Json, 691f0b6 Debug, d3409e6 enums): a
   pre-analysis `expand_derives` generates trait impls as Vilan *source text* from an item's
   fields, then leaks+lexes+parses+walks it — `@derive(PartialEq, Default, Debug, Json)` on structs
   and enums (Default skipped for enums). **Remaining (the bigger prize):** the *general* macro
   engine — user-written macros / compiler bindings (numeric-type generation, custom derives, struct
   reflection). Still needs a real macro-expansion phase.
10. **LSP semantic highlighting** (M) — semantic tokens, precision over the TextMate grammar.
11. **More stdlib** (M, incremental) — ✅ essentially done. Landed: math methods on `i32`/`f64`
    (`abs`/`sqrt`/`pow`/`floor`/`ceil`/`round`/`min`/`max`, `@extern("Math.*")` — 6fc6eea); `Range`
    + custom-iterator `next(): Option<T>` for-loops (4b3833d); `Display::to_string` (480dc77);
    `time::now` (480dc77); `process::exit` (480dc77) + `process::args`/`env` (6e6a1e0); `str.parse_f64`;
    `fs`/`http` already complete. **JSON deferred to #9 (derives):** structs compile to field-less
    positional arrays (`Point{x,y}` → `[x,y]`), so faithful object serialization needs derive-based
    field-name reflection, not a host-`JSON` bridge. `Debug` + `format` still want #9 too.

## Tier 4 — Perf, advanced, cleanup

12. **Fix per-analysis `Box::leak` + incremental analysis** (L) — leak grows each
    keystroke/compile; true incremental is blocked by the global `entity_id`/`type_id`
    counters. Debounce masks the latency — measure first. (caching plan Tier 2/3)
13. **LSP sub-file incremental parsing** (L) — tree-sitter-style reuse; chumsky is a batch
    parser, so this is the largest, lowest-priority LSP item.
14. ✅ **Memory management Phase 5** (L) — done. Inferred `borrows` (a43d23c); view-returning
    `Option<&T>` — direct (924d0d7), conditional + aggregate (8b53e53) = the `Arena::get` shape;
    `for e in &mut container` via `next_mut(): Option<&mut T>` (72d8759). With Phase 6 essentials
    (`Arena`/`Handle` 75f9529, `Shared<T>` c2d2a25) already landed, the rev-1 escape ladder and
    view-returning collections are fully expressible. **Phase 6+ tail (deferred):** `Weak<T>`,
    dynamic rule-4 (write-while-view-live trap), no-view-across-`await`, deterministic destruction.
    `Shared<T>` already unblocks #8's reactive path.
15. **Numeric types `u8`…`i64`/`f32`** (S) — low value for a JS target (collapse to
    `f64`/`BigInt`); do via #9's macro or defer to a WASM/native backend. Plus prune
    superseded `vilan/outdated/` sketches.

## Key dependencies
- array-length intrinsic → concrete iterators (#1)
- `Hash`/equality → collections (#3)
- manifest / multi-file → `fmt` / `test` (#4)
- macros → numeric types & derives (#9, #15)
- memory Phase 6 → reactive browser UI (#8, #14)
