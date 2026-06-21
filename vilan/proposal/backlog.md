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

1. **`Shared.write` as a real view — `&mut T borrows self`** (M). Today `write(self): T` lowers to
   the `SharedValue` intrinsic (`cell.v`, a JS lvalue), and `cell.write() = x` works only because
   assignment to a call expression is not place-checked. The type-honest signature is
   `fun write(self): &mut T borrows self` — a mutable view projecting the cell's slot. Doing this
   *correctly* (not a different lie) is feature-sized:
   - Split the intrinsic: `read → SharedRead` (`cell.v`, a value) vs `write → SharedWrite` (a view).
   - Generalize the `(base, key)` view to a **single-slot cell projection** `(cell, "v")` that
     rebinds on whole-write (`*w = x → cell.v = x`) for *any* `T`. The existing scalar-view path is
     scalar-primitive-only; the aggregate-view path uses `Object.assign`, which is **wrong** here
     (it merges instead of rebinding — observably broken for `Shared<List<…>>`, which is used).
   - Teach `function_returns_scalar_view` / `call_returns_scalar_view` about `external` functions
     (they currently consult only `self.functions`, never `external_functions`).
   - Either depend on **auto-deref through view-returning calls** (item B2) so `cell.write().push()`
     and `a.write().count` keep working, or rewrite every call site to `*cell.write() = x` /
     `(*cell.write()).push()` (an ergonomic regression at ~6 sites in `reactive.vl` + `test/shared.vl`).
   - **Formal question:** `borrows self` subjects the result to second-class-view rules (no escape,
     eventually no-cross-`await`). `Shared` is a deliberately first-class, ref-counted escape hatch,
     so some of those restrictions may be *false* for it. Decide whether `Shared`'s view is exempt.
   - **Decision (2026-06-21):** keep `write(): T` for now. The hoped-for shortcut — *forbid binding a
     view to a local (`mut a = &mut x`) so the `*` deref operator can be removed, leaving clean call
     sites* — does not hold: `*` is independently load-bearing for `&mut` **parameters**
     (`fun bump(slot: &mut i32) { *slot = *slot + 1 }`, which cannot be forbidden), for `for e in
     &mut` and `Option<&mut T>` captures, and for `*`-reads. The clean route is **transparent
     references** (item C5, **proposal written:** [`transparent-references.md`](transparent-references.md)),
     a surface change that makes `write(): &mut T borrows self` clean with no call-site edits.
     Implement C5, then A1 is a one-liner.

2. **Ownership & disposal via `context`** (M) — *correctness bug, not just a leak.* `sub()` returns
   a `Subscription` that every caller drops, so observers fire forever. For `bind_each` this is a
   real bug: each re-render `sub()`s fresh rows and `clear()`s the DOM, but the old rows'
   subscriptions stay live, firing on every change and mutating detached nodes (unbounded growth).
   Design (the deferred "owner scope" from the reactive proposal, README §"Lifecycle"):
   - An `Owner` type collecting `dispose` thunks, threaded as a `Context<Owner>`.
   - `sub()` registers its `Subscription` with the current owner via `Context::get()` instead of
     returning it to the void. Context threading already captures into closures, so **no compiler
     work** is needed.
   - `mount` establishes a root owner; **`bind_each` keeps a child owner per render**, disposing the
     previous one before re-rendering (scope-per-render) — this is what actually plugs the leak.
   - Follow Solid 2.0: a `sub` body may return a per-run cleanup thunk (runs before the next apply
     and on dispose).

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

2. **Auto-deref through view-returning calls** (M) — `resolve_field_accessor` requires a
   `Type::Struct` subject, so `obj.slot().field` / `obj.slot().method()` do not deref the returned
   view; only scalar `*obj.slot()` is supported. Auto-inserting the deref for field/method access on
   a view-returning subject is broadly valuable (every `borrows` method benefits) and is the
   ergonomic prerequisite for A1. Needs its own formal definition + tests.

3. **Variadic-generics deferred tail** (M–L; see `variadic-generics.md` §Deferred). Shipped:
   flat-tuple lowering, mapped tuple types `(U in T: F<U>)`, tuple comprehensions `(x in xs => e)`,
   `combine`. **Not done:** `keyof`; spread parameters (`...items: T`); elision of the flat-tuple
   construction copy; **enforcement** of arity bounds `T: (2..)` and tuple element bounds
   `(..: Display)` (parsed, not checked); trait-typed-value dispatch (below).

4. **Trait-typed-value dispatch** (L) — calling a trait method on a value whose type is a bare trait
   (`Type::Trait(Id, args)`). Infrastructure landed (parameterized traits, impl trait-args), but
   dispatch is deferred behind the generic-dispatch cluster and a blanket-`Into` ambiguity. `combine`
   sidestepped it by taking concrete `Signal<U>`. Fold into B1.

5. **Analyzer Bug C** (M; see `analyzer-refactor.md`) — transformer monomorphization through a nested
   generic call. Bugs A & B fixed; C open. Fold into B1.

6. **Inferred-element closure-param inference** (S–M; roadmap #2 remaining) — a `List` whose element
   is *inferred* (`List::new()` + `push`, or a chained `filter().map()`) is typed too late for the
   closure-param inference, so field access in the closure still fails; workaround is an annotation
   (`mut xs: List<T>`). README's documented "known limitation." Fold into B1.

---

## C. Memory model — Phase 6+ tail (deferred; see `memory-mangement-impl-plan.md`)

1. **`Weak<T>`** (M) — non-owning handle for breaking `Shared` cycles.
2. **Dynamic rule-4** (M) — the write-while-view-live trap (a write through one path while a live
   view of the same place exists).
3. **No-view-across-`await`** (M) — reject a second-class view held across a suspension point.
   Interacts with A1's `borrows`-exemption question for `Shared`.
4. **Deterministic destruction** (L) — scope-end destructors / `Drop`-equivalent.
5. **Transparent references — implicit place, explicit value** (M; **proposal written:**
   [`transparent-references.md`](transparent-references.md)) — *the lever for A1.* A view is
   implicitly a *place* (assign and `.`-project through it with no `*`), while its value is explicit
   (`*v` is an rvalue copy, never an assignment target). The delta from today is small: drop `*` from
   assignment left-hand sides (`*x = v` → `x = v`), add `.`-place-projection through views, make `*`
   rvalue-only; value-reads keep their `*`. Surface-only — the second-class/`borrows` safety model is
   unchanged. Resolves A1 with no call-site changes. Next: implement against the proposal's rules +
   conformance test, re-baselining the view corpus (`view-*.vl`, `borrows*.vl`, `for-*.vl`,
   `option-view.vl`, `subscript.vl`).

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

---

## F. Backend & platform

1. **Browser backend — full-stack slice 5** (L; roadmap #8, see `browser-backend` memory) — slices
   1–4 done (`--target` flag, platform-gated std core/node/dom, `std::dom`). Remaining: the
   full-stack project model — `[server]`/`[client]` entries, `vilan build` emits
   `dist/server.js` + `dist/client.js`, server serves the client bundle.
2. **Numeric types `u8`…`i64`/`f32`** (S; roadmap #15) — low value on a JS target (collapse to
   `f64`/`BigInt`); do via the macro engine (G1) or defer to a WASM/native backend. Prune
   superseded `vilan/outdated/` sketches.

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

---

## I. Collections

1. **Struct keys for `Map`/`Set`** (M; roadmap #3) — M2 gives value `==`, but JS Map/Set key objects
   by *reference*, so by-value aggregate keys need key-serialization or a custom table.

---

## J. Concurrency

1. **Async/await remaining phases** (L; see `context-async-plan` memory) — `context` (scoped value)
   landed and threads as a hidden parameter; the shared call-graph (Phase 0) is in `call_graph.rs`.
   The async/await execution model phases remain.

---

## Open decisions (block the items above)

- **A1 (`Shared.write`):** *resolved 2026-06-21* — keep `write(): T` until **transparent references**
  (C5, proposal [`transparent-references.md`](transparent-references.md)) lands; that route gives a
  type-honest `&mut T borrows self` with clean call sites and no special-casing. (The earlier
  alternatives — auto-deref through view calls keeping `*`, or explicit `*` at every call site — are
  superseded by C5.)
- **C5 (transparent references):** the one live sub-decision is uniform `*` on value-reads vs.
  auto-deref in value contexts; the proposal recommends uniform (no type-direction). Confirm before
  implementing.
