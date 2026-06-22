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

2. **Ownership & disposal — explicit owners** (M; **proposal written:**
   [`reactive-ownership.md`](reactive-ownership.md)) — *correctness bug, not just a leak.* `sub()`
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
7. **`[must_use]`** (S–M; drives A2's loudness) — a general attribute marking a function's result as
   must-be-consumed. A call whose result is *dropped* (a non-tail statement expression, not bound /
   argument / assigned) gets a diagnostic ("unused `Subscription`: `take()` into an `Owner`,
   `dispose()`, or `let _ = …`"). Detection scans block statement lists for a dropped call to a
   `must_use` callee (the transformer already separates statement vs. tail). Severity: a **warning**
   is the right fit — needs a `Warning` severity added (diagnostics are all errors today; the LSP
   already filters by severity); fallback is an error + `let _ =` escape. Written `[must_use]` (needs
   H2). First user: `std::reactive::sub` (A2).

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
   - **R8 — no implicit borrow at call sites** (S–M): a `&`/`&mut` parameter must be passed a view
     (`&[mut] place` or an existing view), not a bare value place. Today a bare value to a scalar
     `&mut` param compiles to *broken* JS (the param expects a `(base, key)` pair) — a pre-existing
     bug. The check must **exclude the method `self` receiver** (implicitly borrowed). Ties into
     roadmap #2 (compiler-core robustness).
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
6. **Re-evaluate the project structure + how the LSP identifies a file's target** (M; ties into F1) —
   the browser backend introduces `--target node|browser` and platform-gated std (core/node/dom), so
   a file's *target* decides which std layer and host globals are in scope. The LSP has no per-file
   notion of target today, so diagnostics/hover/completion for a `dom`- or `node`-only file can be
   wrong or missing. Revisit the project model (`vilan.toml`, `[server]`/`[client]` entries, entry
   discovery) and **define how a file's target is determined** — manifest-entry membership, directory
   convention, or an in-file annotation — so each file is analyzed under the right target. The chosen
   rule belongs in the language/project spec (D1).

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
2. **Attribute syntax: `@name(..)` → `[name(..)]`** (M) — change decorator-style attributes to
   bracket-style (Rust-like, without the `#`): `[extern("console.log")]`, `[derive(Debug)]`,
   `[must_use]`. Lexer + parser change, a corpus-wide migration (every `@extern` / `@derive` across
   `std` + `test` + examples), and the formatter. Mechanical but broad; goldens unchanged (attributes
   don't affect codegen). Sequence **before / with `[must_use]` (B7)** so that lands in the new
   syntax. Do as its own commit (the migration) ahead of the feature.

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

- **A1 (`Shared.write`):** *done 2026-06-21* — `write(): &mut T borrows self` shipped (item A1):
  view-tracked, codegen byte-identical, rebind-through-handles verified.
- **C5 (transparent references):** *resolved + landed 2026-06-21* — went with uniform `*` on
  value-reads (no type-direction). R5/R6/R1/R7 shipped; R8 and inline-`Option<&mut>` transient
  remain as the C5 sub-items above.
