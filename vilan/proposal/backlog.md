# Vilan Backlog — everything outstanding

A running capture of work that is *known but not done*, so nothing is lost to conversation. This
is the tactical companion to [`roadmap.md`](roadmap.md) (the ranked strategic view); items that
`roadmap.md` already tracks are cross-referenced by number rather than duplicated in full.

Per the project's engineering principles (see `CLAUDE.md`): each non-trivial item below should get
a **formal definition + unit tests + regression tests** before it is implemented, and should be
built to subsume special cases rather than patch them. Items carry a rough size (S/M/L) and known
dependencies. Unordered within a section.

Item numbers are **stable identifiers** (other documents cite them — `backlog F3`, `I2`):
completed items are removed and their numbers retired, so numbering within a section may
have gaps.

---

## A. Reactive core & UI (`std::reactive`, `std::ui`)

3. **`bind_each` keyed reconciliation** (M) — currently clear-and-rebuild on every change (correct
   but not keyed). The `key` argument is reserved for this. Reorder rows with their items, dispose a
   removed key's row, re-render only a changed row.

4. **`flatten` reactive combinator** (M) — the monadic join: a `Signal<Signal<U>>` followed to its
   current inner signal (a dynamic dependency). Listed in the README API table; not built.

5. **Ambient owner / `comp` ergonomic layer** (M; deferred from `reactive-ownership.md`) — sugar
   over the explicit `Owner`/`Disposable` primitives, once an API is proven against
   async/callbacks. The magic desugars to the shipped primitives; nothing blocks on it.

6. **Ambient microtask flush + async turns/actions** (M–L; the future sections of
   `reactive-batching.md`) — auto-`flush` on the next microtask (committed, deferred), and the
   optimistic-write → `await` → reconcile lifecycle for handlers that span ticks. The async-turn
   half interacts with C3 (no-view-across-`await`).

---

## B. Type system & the type solver

3. **Variadic-generics deferred tail** (M–L; `variadic-generics.md` §Deferred) — shipped:
   flat-tuple lowering, mapped tuple types `(U in T: F<U>)`, tuple comprehensions, `combine`.
   **Not done:** `keyof`; spread parameters (`...items: T`); elision of the flat-tuple
   construction copy; **enforcement** of arity bounds `T: (2..)` and tuple element bounds
   `(..: Display)` (parsed, not checked); trait-typed-value dispatch (B4).

4. **Trait objects / dynamic dispatch** (L; own proposal when demanded) — a value typed as a bare
   trait (`let x: Display = …`) is a clean compile error today (the silent-miscompile half was
   fixed). Making it *work* by value needs a runtime representation (a `(value, vtable)` pair /
   `Box<dyn>`-style) — a real language feature; nothing uses it today.

6. **Closure-return element inference gap** (M) — a method whose **result element** comes from a
   field-access closure return (`xs.map(|p| p.x)`) types as `List<unknown>` instead of `List<i32>`.
   Root: `map` binds its result generic `U` from `infer_type(closure return)` while the closure's
   `p.field` accessor is still in-flight, so `U` commits wrong. A general fix (in-flight reports
   `Unresolved`, dependents defer and wake) fixed the literal case but deadlocked the slot case
   (`List::new()`+`push`+`map().sum()`), so it was reverted — the clean fix needs the slot-fill and
   closure-return resolutions both observable to the wake (its own slice). Common uses (`sum`,
   `for`, arithmetic over the mapped element) work today.

8. **Trait-argument binders** (M; pin ledger) — `impl X with Trait<type S: Bound>` is an
   unsupported *feature* with a clean error, pinned `#[ignore]`d. Also notable as the alternative
   route to trait-shaped visitors (p6-followups #2/#4 record the context).

9. **Impl-binder declaration order** (S; pin ledger) — the second `#[ignore]` pin; declaration
   order affects binder resolution. Trivial workaround (reorder declarations); fix for hygiene.

10. **`ret` return-type checking is missing entirely** (S–M) — `ret "nope"` in an `i32` function
    compiles clean, and a bare `ret` in a value-returning function compiles to a `return;` — the
    solver never constrains a `FunctionReturn` against the enclosing signature. Two `#[ignore]`
    pins in `inference.rs` (`ret_value_is_checked_…`, `bare_ret_in_a_value_returning_…`).

11. **`?`/try** (M–L; transport-rpc Q10) — terse error propagation. Unblocks `arg → Result`
    handler ergonomics and guarded parsing (I3); wants its own proposal (interaction with `Result`
    combinators, `async`, and the reply envelope).

---

## C. Memory model — Phase 6+ tail (deferred; see `memory-management-impl-plan.md`)

1. **`Weak<T>`** (M) — non-owning handle for breaking `Shared` cycles.

2. **Dynamic rule-4** (M) — the write-while-view-live trap (a write through one path while a live
   view of the same place exists).

3. **No-view-across-`await`** (M) — reject a second-class view held across a suspension point.
   Open sub-question: whether `Shared`'s view is exempt (a ref-counted cell, so the usual escape
   restriction may be false). Interacts with A6's async turns.

4. **Deterministic destruction** (L) — scope-end destructors / `Drop`-equivalent.

5. **Transparent-references remainder** (M; `transparent-references.md` shipped the model) —
   two sub-items:
   - **Inline `Option<&mut T>` transient:** `match Some(&mut a) { Some(let x) => … }` —
     constructing and matching a wrapped view *inline* is only recognized when the subject is a
     view-returning *call*; extend `compute_wrapped_view_captures` (and the escape analysis) to
     admit an immediately-matched inline constructor and a bare `&[mut]`-parameter forward.
   - **`&mut bool`:** broken for both concrete and generic — `bool` is a numeric enum, excluded
     from `is_scalar_primitive`, so it takes the aggregate view path. Fixing it means a scalar
     `(base, key)` view representation for `bool` across the view machinery (its own slice).

---

## D. Language specification & documentation

1. **Write a language specification** (L) — a single source-of-truth document for the grammar and
   semantics, so grammar changes/issues can be checked against a definition rather than inferred
   from the parser. Should cover: lexical grammar, the full expression/statement/item grammar
   (reconciled with the chumsky parser and the formatter), the type system and the memory model
   (value semantics, second-class views, `borrows`, conventions), and the evaluation/lowering
   model. Becomes the reference solver and parser work is checked against.

---

## E. LSP & tooling

1. **Diagnostics: source attribution + cross-file invalidation + a real test suite** (M–L) —
   fixes two observed bugs and the gap that makes them invisible.
   - *Root cause:* `Error { span, msg }` carries a bare span with **no `SourceId`**, even though
     the analyzer has the `source_map`/`SourceId` machinery. `analyze_and_publish` analyzes one
     document and maps **all** `program.diagnostics` through the *open doc's* line index,
     publishing to that one URI.
   - *Bug — missing diagnostics:* an error originating in an imported file is mapped through the
     wrong line index (out of range → vanishes) and never surfaced on the file that has it.
   - *Bug — stale diagnostics on import change:* editing file A re-analyzes only A; an open file B
     that imports A is never re-analyzed, so B's diagnostics never clear/update.
   - *Fix:* put `SourceId` on `Error`; group diagnostics per file; on any change re-analyze every
     open doc (or the dependents) and publish an explicit (possibly empty) list **per file** so
     stale ones clear. Also publish `Program.warnings` (`[must_use]` is collected but never
     surfaced in the editor).
   - *Testing (the durable part):* extract the diagnostic pipeline out of the async `Backend` into
     a sync, testable `Workspace` + a fake `Client` that records `publish_diagnostics`. Test the
     **lifecycle**: error→clean clears; close clears; editing an imported file republishes
     dependents; an imported-file diagnostic is attributed to *its* URI at the right span. The
     invariant that kills "stale forever": *published diagnostics == a fresh from-scratch analysis*
     for every file.

2. **LSP semantic highlighting** (M; roadmap #10) — semantic tokens, precision over TextMate.

3. **Fix per-analysis `Box::leak` + incremental analysis** (L; roadmap #12, caching Tier 2/3) —
   the leak grows each keystroke/compile; true incremental is blocked by global
   `entity_id`/`type_id` counters. Measure first; debounce currently masks it.

4. **LSP sub-file incremental parsing** (L; roadmap #13) — tree-sitter-style reuse; chumsky is a
   batch parser, so this is the largest, lowest-priority LSP item.

5. **Migrate the codegen-snapshot corpus into `vilan test`** (S) — `vilan/test/` is a dev-time
   `.js` snapshot check, separate from the behavior runner; unify.

---

## F. Backend & platform

2. **Numeric types `u8`…`i64`/`f32`** (S; roadmap #15) — low value on a JS target (collapse to
   `f64`/`BigInt`); do via the macro engine (G1) or defer to a non-JS backend (F3/F4, where the
   distinct integer widths are real). Prune superseded `vilan/outdated/` sketches.

3. **WASM backend** (L; far future) — the second emitter on the platform model's `Backend` axis
   (`Js` is the only variant today; `platform-model.md` §7.1 reserves `Wasm`). Three parts, only
   one of which is "codegen":
   - **Emitter** — Vilan's lowered IR → WebAssembly (via a `wasm-encoder`-style crate, or emit
     WAT). Most language constructs (functions, structs, control flow) lower straightforwardly;
     closures and generics (already monomorphized) are the work.
   - **Host-import seam** (`platform-model.md` §5) — a WASM module imports host functions
     differently than JS, so an `[extern]` binding may gate on **backend**: `http_sys.wasm.vl`, or
     a layer with `backend = ["wasm"]`. The *shared interface* is unchanged — only the `_sys` impl
     differs. Needs **backend-gating on layers** (`LayerDecl` carries only `platform` today;
     `Layer.backend: Option<Backend>` per §7.1) — the one piece of platform-model scaffolding
     deferred from the stabilizing slice.
   - **Memory-model lowering** — the model is GC-free by design
     (`memory-management-rev-1.md`, goal #1): values are scope-owned copies, views are
     second-class (never outlive a frame), and `Arena` owns its slots outright with
     generational handles — none of these need collection. What a non-JS backend needs is a
     linear-memory allocator, **scope-end destruction (C4 — the linchpin**, deferred today
     precisely because the JS GC makes deferral free), and an **ARC lowering for `Shared`**
     (+ `Weak`, C1, for cycles). This is the heavy part and is **shared with F4**; do it
     once. Targets both `browser` and `@process` (WASM runs in each).

4. **Native backend — server performance** (XL; far future) — a third `Backend` emitting native
   machine code, motivated by server throughput (no V8/JS overhead). For comparison, **Rust**
   lowers `source → HIR → MIR → LLVM IR → machine code`, with **LLVM** the default backend and
   **Cranelift**/**GCC** as alternates. A Vilan native path wants the same shape — a typed
   mid-level IR to lower from — and faces two choices:
   - **Backend infra** (cheapest → fastest peak): **emit C** (portable, leans on the C compiler;
     simplest to maintain — Nim/V do this) ▸ **Cranelift** (Rust-native, fast compiles, solid
     codegen; the natural fit for a Rust project) ▸ **LLVM** (peak performance, heavy dependency,
     slow builds).
   - **Memory model** — the central challenge (bigger than codegen), but smaller than
     "build a GC": the model is deterministic by design, so the lowering is allocator +
     scope-end drops (C4) + ARC for `Shared` (+ `Weak`, C1). A bundled tracing GC would
     *contradict* rev-1's goal #1 (deterministic, GC-free memory) and is not on the table.
     Shares the F3 lowering work.
   - **Standing cost:** maintaining ≥3 backends is a real tax (each language feature must lower to
     each). Gate this behind a **stable backend abstraction + a shared lowered IR**, and prove the
     seam with a *single* non-JS backend (F3) before committing to a third. Far future — flagged
     here so the IR/abstraction work that unblocks it is designed with this in mind.

---

## G. Macros

1. **General macro engine** (L; roadmap #9) — built-in derives and `[service]` generation shipped
   as special-cased subsets; the prize is user-written macros / compiler bindings (numeric-type
   generation, custom derives, struct reflection) via a real macro-expansion phase.

---

## H. Parser gaps

1. **Struct literal as an operator operand** (S) — `Point { .. } == x` fails (bind to a variable
   first); needs a `no-struct-literal` expression mode for conditions (à la Rust). Currently
   degrades to a clean parse error, documented at the parser site.

---

## I. Collections

1. **Struct keys for `Map`/`Set`** (M) — value `==` exists, but JS Map/Set key objects by
   *reference*, so by-value aggregate keys need key-serialization or a custom table.

2. **`[T; n]` — a general fixed-length array type** (M) — the codec slice shipped this item's
   immediate wants (hex literals, bitwise/shift operators, `std::bytes` over `Uint8Array` —
   `bits-and-bytes.md`); what remains is the general fixed-length / contiguous array type,
   cheaper than the heap-boxed, length-mutable `List<T>` stand-in.

3. **Validating per-type `from_json`** (M; interacts with B11 `?`/try) — the codec seam validates
   end to end (sticky deserializer errors, `RpcError::Decode`, and malformed JSON is a decode
   error rather than a thrown `JSON.parse`); the per-type `to_json`/`from_json` convenience
   surface is what remains trusting: a missing/mistyped field decodes to `undefined` and flows
   onward as garbage — the *silent* failure mode. Wanted: decode reports an error (a `Result`, or
   at minimum a `panic` naming the field) when a field is absent or the wrong shape.

---

## J. Concurrency

1. **Async/await remaining phases** (L; see the `context-async-plan` memory) — `context` (scoped
   value) landed and threads as a hidden parameter; the shared call-graph (Phase 0) is in
   `call_graph.rs`. The async/await execution-model phases remain.

---

## K. Std runtime

1. **`Server` streaming responses** (M) — `serve_connected` builds on `std::http`'s raw
   `node:http` bindings because an SSE stream needs partial writes and `Server`'s
   request→`Response` model is fully buffered (the seam is documented in
   `std/src/process/rpc_server.vl`'s header). Give `Server` streaming-response support and move
   `serve_connected` onto its public surface.
