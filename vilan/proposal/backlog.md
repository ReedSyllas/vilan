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

11. **`!` / `?.` deferred tail** (M; `try-and-lift.md`) — the operators shipped 2026-07-04
    (both slices + the stabilization arc: bang-directed return-position generics, closure-`ret`
    participation, user-`Lift` lowering). Remaining here are the recorded deferrals only:
    closure `!` (the RPC-handler follow-up; needs the `arg → Result` linkage design), error
    conversion at the `!` boundary, expression lifting (`a? + 10`), applicatives, and
    `Signal`/`Promise` `Lift` opt-ins.

12. **Missing-impl bound dispatch emits the abstract method** (M; found via `format(7u32)`
    before u32 had a `Display` impl) — a generic bound's dispatch at a type LACKING the
    impl silently monomorphizes to the trait's abstract method and returns `undefined` —
    the silent-miscompile class. The conformance side exists (an `impl .. with` missing
    members errors); the MONOMORPHIZATION side doesn't: instantiating a bound at a type
    with no impl should be a spanned compile error at the call site. (The u32/BigInt
    `Display` holes are fixed; the general check remains.)

13. **A direct call on a closure-typed local doesn't type its unannotated parameter** (M;
    pinned `#[ignore]`d; surfaced writing macro `unroll` callbacks 2026-07-06) — `let f = |i|
    accumulate(i); f(3)` never feeds `i` from the call site (zero-param and annotated forms
    work; closures passed to methods work via reconciliation). The C′-family stabilization
    covered deferred call SUBJECTS; the binding-then-direct-call shape needs the same
    channel. Workaround: annotate (`|i: i32| ..`).

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

2. **LSP semantic highlighting** (M; roadmap #10) — semantic tokens, precision over TextMate.

3. **Fix per-analysis `Box::leak` + incremental analysis** (L; roadmap #12, caching Tier 2/3) —
   the leak grows each keystroke/compile; true incremental is blocked by global
   `entity_id`/`type_id` counters. Measure first; debounce currently masks it.

4. **LSP sub-file incremental parsing** (L; roadmap #13) — tree-sitter-style reuse; chumsky is a
   batch parser, so this is the largest, lowest-priority LSP item.

5. **Migrate the codegen-snapshot corpus into `vilan test`** (S) — `vilan/test/` is a dev-time
   `.js` snapshot check, separate from the behavior runner; unify.

6. **Diagnostics remainder** (M; what E1 left open when it shipped 2026-07-04) —
   - **Buffer overlay for unsaved dependencies:** module loading is disk-backed
     (`load_package_module`), so a dependent's re-analysis sees an edited-but-unsaved import's
     *disk* content until save (`did_save` closes the loop today). A buffer overlay needs a core
     seam for the loader to consult open-document contents.
   - **Async lifecycle harness:** the publish bookkeeping (explicit empties, `published_extra`
     diffing, close-clears-extras) is exercised only structurally; the fake-`Client` +
     edit-sequence property test (*published == fresh analysis, always*) remains to build.
   - **Shared-dependency last-writer-wins:** two open docs importing the same broken module each
     publish their view of it; the merged per-URI union is not computed (harmless while both
     views agree, which re-analyze-all keeps true).

7. **Diagnostic span precision — the long-tail audit** (S–M per batch; the first pass shipped
   2026-07-04) — the harness and the top user-visible classes landed: `assert_fails_spanning`
   (exact-range span pins in the inference harness), and re-anchors for match-leg mismatches
   (→ the offending leg's body), struct-initializer field mismatches (→ that field's value)
   and unknown-struct (→ the initializer incl. its name), import root/segment errors (→ the
   segment), and `use` root/segment errors (→ the segment) — six span pins. Remaining: the
   long tail of the ~150 `diagnostics.push` sites hasn't been audited — when a coarse span
   shows up in use, re-anchor it and pin with `assert_fails_spanning`. The standard: point at
   the narrowest expression that identifies the problem (call-argument mismatches are the
   model).

---

8. **LSP + editor support for the macro engine** (M) — **core shipped 2026-07-07**: the
   TextMate grammar knows the `macro` keyword, `macro fun` definitions, `macro name(..)`
   invocations, and generic line-anchored `[name(args)]` attributes; hover on `[name]` /
   `[derive(Name)]` / `macro name(..)` shows the macro's `macro fun` signature; go-to-definition
   jumps to the defining `macro fun`, cross-file into `std` for prelude derives (derive names
   now carry per-name spans; macro names live in a separate scope namespace so trait/macro
   name sharing resolves both ways). Remaining: completion offering registered macro names at
   attribute sites, and semantic tokens classifying macro names distinctly (see #2 above).

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

1. **General macro engine** (L; roadmap #9; **proposal: `macro-engine.md`; Phases 0–1
   SHIPPED 2026-07-06**) — Phase 0: the interpreter over the transformer's `js::Node` AST
   (`transform_to_ast`), the 70/70 equivalence gate, `macro_std`. Phase 1: `macro fun`
   items, per-file hermetic worlds (blanked-file compile against a macro_std-only
   workspace), `[name(args)]`/`[derive(Name)]` dispatch through `run_entry`, output
   splicing with depth-16 fixpoint, world + expansion caches; library-defined macros work
   (the exit criterion). Phase 2 (also 2026-07-06):
   `macro name(..)` invocations — item + expression position, shape-checked dispatch from
   the signature, `fresh()` gensyms stamped per splice site (capture pinned as a clean
   error), output previews in errors. Phase 3 UNDERWAY (2026-07-06): the
   builtin-derive channel (`std/derives.vl`, names reserved, Rust fallback for
   unmigrated/fixture stds) with `PartialEq`/`Default`/`Debug` migrated byte-identically.
   Derives COMPLETE (2026-07-06):
   all five migrated (`Json`+`Wire` together — one Rust contract — via str-returning
   helper macro funs); `Arguments` typed accessors shipped (construction API step 1).
   `[service]` migrated same day (the
   stress test passed: `Item::Service`/`ServiceItem` reflection with compiler-gathered
   rpc surface, cache keyed on struct+methods text, in-macro djb2 via new `str.code_at`;
   byte-gated on todo/rpc bundles). Scoped names + dissolution SHIPPED
   (2026-07-06): macro names are module-scoped (leaf imports; std prelude ambient; markers
   in the analyzer; lazy per-file worlds), `derives.vl` dissolved into
   compare/default/debug/json/rpc, outputs self-carry imports. **Remaining:** the derive-name registration mechanism (builtins
   settle as fn-name = trait name; decoupling deferred to the first user derive needing
   it); ~~fuel knob~~ (shipped 2026-07-06 as `[macro]` — singular, the user's naming call:
   `fuel`/`depth`, entry-manifest-governed, CLI-pinned); ~~module-scoped macro names~~ (shipped, above); ~~attribute
   use inside dependency files~~ (shipped with the unified epilogue); ambient meta vocabulary in macro scope; the
   **construction API** (macro-engine §3 recorded direction, user request 2026-07-06):
   ~~`Arguments` typed accessors~~ (step 1, shipped 2026-07-06), ~~macro_std output
   builders~~ (step 2, **shipped 2026-07-07** as `macro_std::build` — `quote`/`join`/
   `indent` + `impl_of`/`fun_of`/`match_of`/`struct_of`/`init_of`; all five derives and
   `[service]` rewritten against them byte-identically; exact-bytes e2e pin), tree
   interchange (step 3) only if measured; `macro { .. }` blocks (Phase 4).

---

## H. Parser & grammar

1. **Struct literal as an operator operand** (S) — `Point { .. } == x` fails (bind to a variable
   first); needs a `no-struct-literal` expression mode for conditions (à la Rust). Currently
   degrades to a clean parse error, documented at the parser site.

2. ~~Block-scoped imports~~ — **shipped 2026-07-05** (kept as the design record; macro-engine
   §3 consumes it for `macro_std` resolution). `import`/`use` are statements, legal in any
   block (function/closure/if/match-arm bodies, bare blocks, impl bodies — an impl-scope
   import serves its methods); a binding is visible throughout its enclosing block and a later
   same-name binding shadows by overwrite — both **exactly `let`'s semantics** (vilan scopes
   are flat per block; use-before-`let` already compiled, and imports have no TDZ hazard since
   they compile to nothing). Not re-exportable: `export` in a body is a spanned error. The
   compiler previously PANICKED on a body import (no `Expr` for the statement id → transformer
   `unwrap`; now `Expr::Void`), and the loader only scanned top-level nodes — `Node::for_each_child`
   (the new exhaustive structural visitor, no catch-all) drives `collect_module_refs` at every
   depth, which also carries the P3 cross-target gates, the L1 lib-surface check, the §4.2
   contract check, and the LSP platform sniffer for free. Pins: 12 in `inference.rs`, corpus
   `scoped-import.vl`, workspace body-import + §4.2-at-depth CLI tests.

---

4. **Triple-quoted strings `\"\"\"text\"\"\"`** (S–M; user request 2026-07-06; H3 is retired —
   a retracted finding) — a multi-line string form that auto-trims each line's indentation up
   to the column of the opening `\"\"\"` (Java-text-block / Swift-multiline style), so embedded
   code blocks nest naturally under their vilan surroundings. Design points to settle at
   implementation: whether the newline after the opening `\"\"\"` is dropped; the interpolated
   variant `i\"\"\"..\"\"\"` (the macro-authoring payoff — `source(i\"\"\"impl {name} { .. }\"\"\")`
   with NO brace escapes, since plain braces could be literal in the triple form and only
   `{expr}` holes interpolate... which needs its own escape story); how little escaping the
   body needs (the raw-ish appeal is pasting code verbatim). Complements, not replaces, the
   `\{`/`\}` escapes in ordinary i-strings.

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
