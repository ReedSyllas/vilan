# Vilan Backlog — everything outstanding

A running capture of work that is *known but not done*, so nothing is lost to conversation. This
is the tactical companion to [`roadmap.md`](roadmap.md) (the ranked strategic view); items that
`roadmap.md` already tracks are cross-referenced by number rather than duplicated in full.

Per the project's engineering principles (see `CLAUDE.md`): each non-trivial item below should get
a **formal definition + unit tests + regression tests** before it is implemented, and should be
built to subsume special cases rather than patch them. Items carry a rough size (S/M/L) and known
dependencies. Unordered within a section.

---

## A. Reactive core & UI (`std::reactive`, `std::ui`)

1. **`Shared.write` as a real view — `&mut T borrows self`** (**done 2026-06-21**). `write` now
   returns a mutable view (honest signature; it participates in the view rules — e.g. R1 flags
   `let x: T = c.write()`), while `read(self): T` stays a value copy-out. How it shipped — simpler
   than the originally-feared `(base, key)` generalization + auto-deref:
   - `external` functions now record `borrows`, and `call_returns_view` consults
     `external_functions`, so `c.write()` is a recognized view: `c.write() = v` write-throughs via
     transparent references (R5).
   - Split the `SharedValue` intrinsic into `SharedValue` (read) / `SharedWrite` (write); both lower
     to `cell.v`. Because `write()` lowers to the `cell.v` **lvalue** directly, member access
     (`c.write().push(z)`, `c.write().count = x`) just works — **no auto-deref needed** (this is why
     B2 wasn't required).
   - The single-slot rebind: a write *through* a `SharedWrite` view (`*c.write() = v`, the R5 form)
     lowers to `cell.v = v` (rebind the slot), not `Object.assign` (which would *merge* — wrong for
     `Shared<List<…>>`). A `is_shared_write` check in the assignment lowering picks the rebind.
     Verified: a rebind propagates through clones (`shared_write_*` inference tests).
   - Codegen is **byte-identical** to the old `write(): T` at every call site (shared / reactive /
     generic-methods goldens unchanged).
   - **Deferred edge:** binding the view first (`let w = c.write(); w = v`) is not a write-through —
     it's treated as a value copy, as before, because `view_binding_mutability` doesn't yet consult
     external `borrows`. Unused in practice (Shared writes are immediate). Revisit with the
     bound-view rebind story.
   - **Open (memory C):** whether `Shared`'s view is exempt from a future no-view-across-`await` rule
     (it's a ref-counted cell, so the usual escape restriction may be false) — decided when that
     rule lands.

2. **Ownership & disposal — explicit owners** (**done 2026-06-22**;
   [`reactive-ownership.md`](reactive-ownership.md)) — shipped the `Disposable` trait + `Owner`
   (`new`/`take`/`dispose`), `View` self-collecting its bindings' subs, `bind_each`'s per-render
   child owner (the leak fix, proven by a bounded-count test), and `[must_use]` `sub`. Also delivered
   B7 (`[must_use]` + a `Warning` severity) and H2 (the `[name(..)]` attribute syntax). Deferred: the
   ambient-owner / `comp` ergonomic layer (future sugar over these primitives). *Original notes:*
   `sub()`
   returns a `Subscription` every caller drops, so observers fire forever; `bind_each` makes it a real
   bug (re-render `sub()`s fresh rows and `clear()`s the DOM, but old rows' subs stay live, mutating
   detached nodes — unbounded growth). **Decision (revised):** *explicit* ownership, no ambient magic
   for now — a `Context`/owner-stack mechanism each carried a tax (literal-body, sync-only,
   context-pass work), so defer it until we have an ergonomic API proven against async/callbacks; the
   magic later just desugars to these primitives. `Owner` (`new` / `take<T: Disposable>` / `dispose`)
   + a `Disposable` trait (`Subscription`/`View`/`Owner` implement it; verified working). `View`s
   collect their bindings' subs (rolled up via `.child`), so `owner.take(view)` owns a subtree;
   `bind_each` keeps an internal child owner cleared/refilled per render; `mount(id, view)` and
   `sub() → Subscription` stay unchanged. Loudness comes from **`[must_use]` on `sub`** (item B7),
   not ambient tracking. `show` stays hide-only (destroy-on-hide `mount_when` is separate, ties to
   A3). Deferred ergonomic layer (ambient owner / `comp` macro) is future sugar.

3. **`bind_each` keyed reconciliation** (M) — currently clear-and-rebuild on every change (correct
   but not keyed). The `key` argument is reserved for this. Reorder rows with their items, dispose a
   removed key's row (ties into A2), re-render only a changed row. (roadmap #8 reactive path.)

4. **`flatten` reactive combinator** (M) — the monadic join: a `Signal<Signal<U>>` followed to its
   current inner signal (a dynamic dependency). Listed in the README API table; not built.

---

## B. Type system & the type solver

1. **Type-solver investigation — formally prove or disprove its capability** (L). Stand back from
   the accumulated constraint machinery and characterize what the solver *can and cannot* decide.
   Write down the formal model (the constraint kinds, their priorities, the fixpoint/queue
   discipline — see `constraint-queue-plan.md`), find the cases it gets wrong or defers forever, and
   **merge the special cases into general code that handles all cases**, simplifying the solver. The
   goal is a solver we can reason about, not a pile of targeted fixes. This is the umbrella under
   which the dispatch/inference gaps below should be resolved rather than individually patched.

2. **Auto-deref through view-returning calls** (**done** — was stale) — `obj.slot().field` and
   `obj.slot().method()` deref the returned view and reach the inner struct's member; verified at
   runtime (`auto_deref_through_view_returning_call` in `inference.rs`). Only the note here was
   stale.

3. **Variadic-generics deferred tail** (M–L; see `variadic-generics.md` §Deferred). Shipped:
   flat-tuple lowering, mapped tuple types `(U in T: F<U>)`, tuple comprehensions `(x in xs => e)`,
   `combine`. **Not done:** `keyof`; spread parameters (`...items: T`); elision of the flat-tuple
   construction copy; **enforcement** of arity bounds `T: (2..)` and tuple element bounds
   `(..: Display)` (parsed, not checked); trait-typed-value dispatch (below).

4. **Trait-typed-value dispatch** — the silent-miscompile half is **resolved** (fix 7db4705): a
   method call on a value typed as a bare trait (`let x: Display = 5; x.to_string()`) used to lower
   to the empty abstract method (`undefined`); it is now a clean compile error pointing at the
   generic-parameter / concrete-type fix. The `self`/`Self` receiver inside a trait *default* (the
   one legitimate `Type::Trait` receiver) is exempted via the call's scope chain. **Deferred as a
   distinct feature: trait objects / dynamic dispatch** — making `x: Display` actually *work* by
   value needs a runtime representation (a `(value, vtable)` pair or `Box<dyn>`-style), a real
   language feature with its own proposal and demand; nothing uses it today. (`combine` sidestepped
   it with concrete `Signal<U>`.)

5. **Analyzer Bug C** (M; see `analyzer-refactor.md`) — transformer monomorphization through a nested
   generic call. Bugs A & B fixed; C open. Fold into B1.

6. **Inferred-element closure-param inference** — the documented case is **resolved** (fix 651630f).
   A `List` whose element is inferred (`List::new()` + `push`, or a chained `filter().map()`) was
   typed too late for the closure-param inference, so a field access in the closure failed; the
   workaround was `mut xs: List<T>`. Now a method on such a receiver **defers while a `push`/`run`
   to fill the slot is pending**, so `xs.filter(|p| p.x > 0)` / `xs.map(|p| p.x)` type the parameter
   concretely — parity with a literal list, no annotation. (The `filter().map()` form already
   worked via the B1 re-queue.) **Adjacent gaps:**
   - (a) **inline `match` on an inferred-list method result — ✅ fixed** (3e5e1b1), and the root was a
     *general* fixpoint-termination bug, not a special case: the run-all backstop ignored its
     `wake_ready` result and could end the loop with a just-woken constraint left unrun. `match
     xs.get(0) { Some(let p) => p.x }` (and `pop`) now resolve inline; the loop continues while a
     wake is pending.
   - (b) a method whose **result element** comes from a field-access closure return —
     `xs.map(|p| p.x)` typing as `List<unknown>` instead of `List<i32>` — **still open** (pre-existing;
     affects literal lists too, so a distinct issue from B6). Root: `map` binds its result generic
     `U` from `infer_type(closure return)` while the closure's `p.field` accessor is still in-flight
     (the accessor is priority 2 but needs `p`, which `map` types at priority 6), so `U` commits
     wrong. A general fix was attempted — infer an in-flight field access as `Unresolved` + defer a
     method while a closure argument's body is unresolved — and it *does* fix the literal case, but
     it **regresses the slot case** (`List::new()`+`push`+`map(...).sum()`, which worked) into a
     defer deadlock, so it was reverted. The clean general fix needs the slot/closure-return
     interaction untangled (likely the same "in-flight type reports `Unresolved`, dependents defer
     and wake" mechanism, but with the slot-fill and closure-return resolutions both made observable
     to the wake — its own slice). Common uses (`sum`, `for`, arithmetic over the mapped element)
     work for slot lists today.
7. **`[must_use]`** (**done 2026-06-22**) — a general attribute marking a function's result
   must-be-consumed. A *dropped* result (a call that is a bare statement in a function body or block —
   not bound / `let _` / an argument) is a **warning** (a new, non-fatal `Program.warnings` list
   rendered with `ReportKind::Warning`; `check_must_use` scans body/block statement lists for a
   dropped call to a `must_use` callee). First user: `std::reactive::sub` (A2). Future: surface
   warnings in the LSP (E1) — they're collected but not yet published.

---

## C. Memory model — Phase 6+ tail (deferred; see `memory-mangement-impl-plan.md`)

1. **`Weak<T>`** (M) — non-owning handle for breaking `Shared` cycles.
2. **Dynamic rule-4** (M) — the write-while-view-live trap (a write through one path while a live
   view of the same place exists).
3. **No-view-across-`await`** (M) — reject a second-class view held across a suspension point.
   Interacts with A1's `borrows`-exemption question for `Shared`.
4. **Deterministic destruction** (L) — scope-end destructors / `Drop`-equivalent.
5. **Transparent references — implicit place, explicit value** (**mostly done 2026-06-21**;
   [`transparent-references.md`](transparent-references.md)) — a view is implicitly a *place* (assign
   and `.`-project through it with no `*`), while its value is explicit (`*v` is an rvalue copy, never
   an assignment target). **Landed:** R5 (assign through a view, plain + compound), R6 (`*` rejected
   as an assignment target), R1 (annotation view-ness must match the initializer), R7 (no `mut` view
   binding); the view corpus migrated to bare form (byte-identical JS), conformance test +
   `transparent_references_*` inference tests. **Remaining sub-items:**
   - **R8 — no implicit borrow at call sites** (**done**): a `&`/`&mut` parameter must be passed a
     view (`&[mut] place` or an existing view), not a bare value place; the `self` receiver is
     exempt. Pinned by `r8_*` + `reject_bare_value_to_shared_reference_param` (covers shared `&`).
   - **Generic `&mut T` / `&T` views** (**done** — fix c3cfa44): the scalar-vs-aggregate view
     representation is now resolved at monomorphization, so a generic view behaves like a concrete
     `&mut <T>` (i32/u32/f64/str read/write through; aggregate stays the in-place copy; `&mut` of a
     generic *local* boxes + builds the `(base, key)` pair). Pinned by `generic_mut_view_*`.
     **Known limitation:** `&mut bool` is broken for *both* concrete and generic — `bool` is a
     numeric enum, excluded from `is_scalar_primitive`, so it takes the aggregate path. Fixing it
     means giving `bool` a scalar `(base, key)` view representation across the view machinery (its
     own slice).
   - **Inline `Option<&mut T>` transient** (M): `match Some(&mut a) { Some(let x) => … }` —
     constructing and matching a wrapped view *inline* (not via a function that returns one). Today
     wrapped-view captures are only recognized when the match subject is a view-returning *call*;
     extend `compute_wrapped_view_captures` (and the escape analysis) to admit an immediately-matched
     inline constructor and a bare `&[mut]`-parameter forward (`Some(x)`).

---

## D. Language specification & documentation

1. **Write a language specification** (L) — *new.* A single source-of-truth document for the grammar
   and semantics, so grammar changes/issues can be checked against a definition rather than inferred
   from the parser. Should cover: lexical grammar, the full expression/statement/item grammar
   (reconciled with the chumsky parser and the formatter), the type system and the memory model
   (value semantics, second-class views, `borrows`, conventions), and the evaluation/lowering model.
   Becomes the reference the type-solver investigation (B1) and parser work (H) are checked against.

---

## E. LSP & tooling

1. **Diagnostics: source attribution + cross-file invalidation + a real test suite** (M–L) — fixes
   two observed bugs and the gap that makes them invisible.
   - *Root cause:* `Error { span, msg }` carries a bare span with **no `SourceId`**, even though the
     analyzer has the `source_map`/`SourceId` machinery. `analyze_and_publish` analyzes one document
     and maps **all** `program.diagnostics` through the *open doc's* line index, publishing to that
     one URI.
   - *Bug — missing diagnostics:* an error originating in an imported file is mapped through the
     wrong line index (out of range → vanishes) and never surfaced on the file that has it.
   - *Bug — stale diagnostics on import change:* editing file A re-analyzes only A; an open file B
     that imports A is never re-analyzed, so B's diagnostics never clear/update.
   - *Fix:* put `SourceId` on `Error`; group diagnostics per file; on any change re-analyze every
     open doc (or the dependents) and publish an explicit (possibly empty) list **per file** so stale
     ones clear.
   - *Testing (the durable part):* extract the diagnostic pipeline out of the async `Backend` into a
     sync, testable `Workspace` + a fake `Client` that records `publish_diagnostics`. Test the
     **lifecycle**: error→clean clears; close clears; editing an imported file republishes dependents;
     an imported-file diagnostic is attributed to *its* URI at the right span. Add multi-file
     fixtures driven through edit sequences. The invariant that kills "stale forever":
     *published diagnostics == a fresh from-scratch analysis* for every file (property test over
     random edit sequences). Note: today's LSP tests are 6, all completion-focused.

2. **LSP semantic highlighting** (M; roadmap #10) — semantic tokens, precision over TextMate.
3. **Fix per-analysis `Box::leak` + incremental analysis** (L; roadmap #12, caching Tier 2/3) — the
   leak grows each keystroke/compile; true incremental is blocked by global `entity_id`/`type_id`
   counters. Measure first; debounce currently masks it.
4. **LSP sub-file incremental parsing** (L; roadmap #13) — tree-sitter-style reuse; chumsky is a
   batch parser, so this is the largest, lowest-priority LSP item.
5. **Migrate the codegen-snapshot corpus into `vilan test`** (S; roadmap #7 note) — `vilan/test/` is
   a dev-time `.js` snapshot check, separate from the behavior runner; unify.
6. **Re-evaluate the project structure + how the LSP identifies a file's target** (**planned:**
   roadmap *Next up* P1–P3) — the browser backend introduces `--target node|browser` and
   platform-gated std (core/node/dom), so a file's *target* decides which std layer and host globals
   are in scope; the LSP has no per-file notion of target today, so diagnostics/hover/completion for
   a `dom`- or `node`-only file can be wrong or missing. **Resolved by the Next-up direction:** a
   file's target is its *package*'s target (P1/P2 — a multi-package workspace where each package's
   `vilan.toml` declares `target`), and cross-target imports diagnose without breaking typings (P3).
   The chosen rule belongs in the language/project spec (D1).

---

## F. Backend & platform

1. **Browser backend — full-stack slice 5** (L; roadmap #8, see `browser-backend` memory) — slices
   1–4 done (`--target` flag, platform-gated std core/node/dom, `std::dom`). The full-stack project
   model is **redesigned in roadmap *Next up* P2** — a multi-package workspace with per-package
   targets (`packages/client` + `packages/server` + `packages/common`), **replacing** the
   `[server]`/`[client]` sketch. `vilan build` still emits a server + client bundle, server serves
   the client; the transport between them is the *Next up* P6 RPC library.
   - **Update (2026-06-23):** the platform model has **shipped** (`Target` → `Backend` + `Platform`;
     library layers; `deno`/`bun` runtimes; the §4.2 contract check) — see `platform-model.md` and the
     [[roadmap]] memory. So the `--target`/`[server]`/`[client]` vocabulary above is historical; the
     remaining open piece here is the **P6 RPC transport** between client and server.
2. **Numeric types `u8`…`i64`/`f32`** (S; roadmap #15) — low value on a JS target (collapse to
   `f64`/`BigInt`); do via the macro engine (G1) or defer to a non-JS backend (F3/F4, where the
   distinct integer widths are real). Prune superseded `vilan/outdated/` sketches.
3. **WASM backend** (L; far future) — the second emitter on the platform model's `Backend` axis
   (`Js` is the only variant today; `platform-model.md` §7.1 reserves `Wasm`). Three parts, only one
   of which is "codegen":
   - **Emitter** — Vilan's lowered IR → WebAssembly (via a `wasm-encoder`-style crate, or emit WAT).
     Most language constructs (functions, structs, control flow) lower straightforwardly; closures
     and generics (already monomorphized) are the work.
   - **Host-import seam** (`platform-model.md` §5) — a WASM module imports host functions differently
     than JS, so an `[extern]` binding may gate on **backend**: `http_sys.wasm.vl`, or a layer with
     `backend = ["wasm"]`. The *shared interface* is unchanged — only the `_sys` impl differs. This
     needs **adding backend-gating to layers** (`LayerDecl` carries only `platform` today; the
     resolver/`PackageSpec` would gain a backend constraint — `Layer.backend: Option<Backend>` per
     §7.1) — the one piece of platform-model scaffolding deferred from the stabilizing slice.
   - **Memory-model lowering** — no JS GC, so value semantics / `Shared` / `Arena` / `__clone` need a
     linear-memory allocator plus a GC or refcount scheme. This is the heavy part and is **shared with
     F4**; do it once. Targets both `browser` and `@process` (WASM runs in each).
4. **Native backend — server performance** (XL; far future) — a third `Backend` emitting native
   machine code, motivated by server throughput (no V8/JS overhead). For comparison, **Rust** lowers
   `source → HIR → MIR` (its own mid-level IR, CFG form) `→ LLVM IR` (the architecture-portable
   intermediate) `→` per-target machine code; **LLVM is the default backend**, with **Cranelift**
   (`rustc_codegen_cranelift`, faster debug builds) and **GCC** (`rustc_codegen_gcc`) as alternates.
   A Vilan native path wants the same shape — a typed mid-level IR to lower from — and faces two
   choices:
   - **Backend infra** (cheapest → fastest peak): **emit C** (portable, leans on the C compiler;
     simplest to maintain — Nim/V do this) ▸ **Cranelift** (Rust-native, fast compiles, solid codegen;
     the natural fit for a Rust project) ▸ **LLVM** (peak performance, heavy dependency, slow builds).
   - **Memory model** — the central challenge (bigger than codegen): JS's GC currently backs the whole
     value-semantics/`Shared`/`Arena` model, so native needs either a **bundled GC runtime** or the
     ownership/view model **lowered to manual/ARC**. Shares the F3 lowering work.
   - **Standing cost:** maintaining ≥3 backends is a real tax (each language feature must lower to
     each). Gate this behind a **stable backend abstraction + a shared lowered IR**, and prove the
     seam with a *single* non-JS backend (WASM, F3) before committing to a third. Far future — flagged
     here so the IR/abstraction work that unblocks it is designed with this in mind.

---

## G. Macros

1. **General macro engine** (L; roadmap #9, see `compiler-bindings.md`) — built-in derives shipped as
   a special-cased subset; the prize is user-written macros / compiler bindings (numeric-type
   generation, custom derives, struct reflection) via a real macro-expansion phase.

---

## H. Parser gaps

1. **Struct literal as an operator operand** (S; roadmap #2) — `Point { .. } == x` fails (bind to a
   variable first); needs a `no-struct-literal` expression mode for conditions (à la Rust). Currently
   degrades to a clean parse error, documented at the parser site.
2. **Attribute syntax: `@name(..)` → `[name(..)]`** (**done 2026-06-22**) — decorator-style attributes
   are now bracket-style (Rust-like, without the `#`): `[extern("console.log")]`, `[derive(Debug)]`,
   `[must_use]`. Lexer (dropped `@`) + parser (no collision with `[..]` arrays — extern/derive require
   a keyword + `(args)`) + formatter; corpus-wide migration (84 sites) with goldens unchanged.

---

## I. Collections

1. **Struct keys for `Map`/`Set`** (M; roadmap #3) — M2 gives value `==`, but JS Map/Set key objects
   by *reference*, so by-value aggregate keys need key-serialization or a custom table.
2. **A proper array type (`[u8]` / `Bytes`, and `[T; n]` generally)** (M; RPC binary codec;
   **NEXT UP — codec prerequisite, agreed 2026-07-02**) — a binary `Codec` produces *bytes*,
   and Vilan has no byte/array type today: `List<u8>` is the stand-in (heap-boxed,
   length-mutable — workable but not ideal). A fixed-length / contiguous array type — `[u8]`
   (and a `Bytes` view over one) as the immediate want, `[T; n]` as the general feature — is
   the right substrate: cheaper, and the natural target for the `Serializer`/binary-codec
   path (`transport-rpc.md` §6, §10). The same item covers the binary-framing *language*
   floor: **hex literals (`0xFF`) and bitwise/shift operators (`&`/`|`/`^`/`<<`/`>>`)** —
   also what the WebSocket frame codec is gated on. The codec slice takes `Bytes` (a view
   over the host `Uint8Array`) + hex/bitwise first; `[T; n]` generally stays here.
3. **Validating `from_json` — decode errors instead of `undefined`** (M; codec hardening,
   `transport-rpc.md` Q6; **folded into the codec slice's prerequisites, agreed 2026-07-02**:
   the `Deserializer` returns `Result`, finally constructing the never-yet-used
   `RpcError::Decode` — transport-rpc.md §6 status) — `from_json` doesn't validate: a
   missing/mistyped field decodes to `undefined` and flows onward as garbage — the *silent*
   failure mode, worse than a crash. It surfaces under RPC version skew (a changed Wire
   shape), but hurts on **any** malformed input. Wanted: decode reports an error (a `Result`,
   or at minimum a `panic` naming the field) when a field is absent or the wrong shape.
   Interacts with the `?`/try story (transport-rpc.md Q10) for how handlers/stubs propagate
   it tersely.

---

## J. Concurrency

1. **Async/await remaining phases** (L; see `context-async-plan` memory) — `context` (scoped value)
   landed and threads as a hidden parameter; the shared call-graph (Phase 0) is in `call_graph.rs`.
   The async/await execution model phases remain.

---

## Open decisions (block the items above)

- **A1 (`Shared.write`):** *done 2026-06-21* — `write(): &mut T borrows self` shipped (item A1):
  view-tracked, codegen byte-identical, rebind-through-handles verified.
- **C5 (transparent references):** *resolved + landed 2026-06-21* — went with uniform `*` on
  value-reads (no type-direction). R5/R6/R1/R7 shipped; R8 and inline-`Option<&mut>` transient
  remain as the C5 sub-items above.
