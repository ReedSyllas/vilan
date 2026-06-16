# Memory Management — Implementation Plan

Companion to `memory-mangement-rev-1.md` (the design). This is the build
sequence plus the resolved Phase 0 decisions.

## Decision: full switch to value semantics

vilan today has **JS reference semantics** — there is no deep copy anywhere in
the transformer; structs and `List`s lower to JS arrays, and `mut b = a` emits
`const b = a`, which aliases. Adopting rules 1–4 is a deliberate redefinition,
not an addition: existing programs are reinterpreted. The "byte-identical `.js`"
invariant used to validate `context`/`async` is **retired** for this work;
validation moves to behavior snapshots (see below).

## Phases

Each phase is independently testable. Phases 1–3 produce a complete, runnable
language on JS; 4–5 earn the safety guarantees; 6+ is opt-in surface.

- **Phase 0 — Decide & baseline (no code).** *This document.*
- **Phase 1 — Rule 1, always-copy. — DONE.** Analyzer `compute_clone_sites()`
  marks value expressions that bind/assign an aggregate *place* (a `Local` or
  `Field` of a non-scalar `Struct`/`List`/`Tuple` type — scalars `str`/`i32`/…
  and `bool` excluded); exposed as `Program.clone_sites`. Transformer
  `maybe_clone()` wraps those in `structuredClone(...)` at the `Variable` and
  `Assignment` emission sites. `let` immutability was *already* enforced
  ("cannot assign to immutable variable"). Validated: zero clone sites in the
  existing corpus (every committed `.js` byte-identical — the audit's P1=0
  prediction held), and a new `test/value-semantics.vl` proves divergence
  (`1 99 3 102`, vs `99 99 102 102` under the old reference semantics).
  *Known limitation:* `structuredClone` throws on aggregates containing closures
  (e.g. `List<|T|void>`); none occur in the corpus, and a type-aware copy
  routine can replace it later.
- **Phase 2 — Rule 2, elision. — DONE.** `is_elidable_copy` downgrades a copy to
  a move when the source is a *local* read exactly once (`reference_count == 1`,
  which also rules out closure captures — a capture is a second read) and that
  read is not inside a loop or closure (`collect_repeatable_interiors` /
  `mark_repeatable`, a depth-tracking walk mirroring the call graph). Parameters
  are never elided (they alias the caller). Elision is implemented as *not*
  marking the site a clone site, so the transformer is unchanged. Validated:
  `test/copy-elision.vl` elides (no `structuredClone`, `99`), `test/copy-in-loop.vl`
  keeps the copy in a loop (`3`, not the `6` aliasing would give),
  `value-semantics.vl` keeps its live-source clones, zero corpus regression.
  *Scope note:* this is a deliberately sound-not-complete elision built on the
  existing `reference_count`; the fuller *positional* liveness pass ("last use
  *through views*") is deferred to Phase 3, where views define its real
  requirements — the `mark_repeatable` walk is the reusable seed.
- **Phase 3 — Rule 3 core (views) + conventions. — IN PROGRESS.** Built in
  sound, testable slices:
  - **Slice 1 — expression-level `&` / `&mut` / `*`. — DONE.** Lexer already
    lexes `&`; added `Node::Reference(mutable, _)` / `Node::Dereference(_)` (AST),
    `Expr::Reference(Id, bool)` / `Expr::Dereference(Id)` (IR), the `unary`-parser
    prefixes (`*` unambiguous in operand position), identity typing and
    pass-through codegen (a view of an aggregate *is* the value's JS reference).
    Views are not clone candidates, so they alias rather than copy. Validated by
    `test/view-basic.vl` (`&mut c` aliases → `99`; a plain binding copies → `10`);
    zero corpus regression. Primitive-local views (which need a boxed cell) and
    `*v = wholeValue` writes are deferred.
  - **Slice 2 — param/`self` conventions. — DONE.** A `Convention`
    (`Bare`/`Own`/`Ref`/`RefMut`) is threaded onto the analyzer's `Parameter`,
    parsed from: a `&`/`&mut` prefix (`&self`, `&mut self`, `&mut x`), the `own`
    keyword (new token), or a type-position `&T`/`&mut T` (`x: &mut T`, a new
    `reference_type` arm in the type parser; `walk_type_node` carries it as the
    inner type by identity). Codegen is unchanged (aggregates are already JS
    references), so it is additive — `test/view-params.vl` (`&mut self` +
    `&mut c: Counter`) runs `21` and `test/view-conventions.vl` (type-position
    `&mut T` + `own`) runs `11 11`; no corpus regression.
  - **Slice 3a — default-flip + assignment mutability check. — DONE.** Bare
    `self`/params are readonly (the `Bare`/`Ref` conventions). A post-`build()`
    pass `check_readonly_mutation` (with `readonly_root_parameter`, which walks a
    field/deref place chain to its root) rejects an *assignment* rooted in a
    readonly parameter: "cannot mutate through readonly parameter 'x'; declare it
    `&mut x`". Runs after `build()` so field accessors have resolved to
    `Expr::Field`. Migrated `field-assignment.vl` (`increment`/`bump` →
    `&mut self`) — and its `.js` is byte-identical (pass-through codegen confirms
    additivity). Enforcement proven by a negative case; no corpus regression.
  - **Slice 3b — method-call mutability check. — DONE.** A post-`build()` pass
    `check_mutable_arguments` resolves each call's callee via the direct
    `subject -> Local(callee)` path (functions *and* external functions carry
    parameter ids/conventions); for any parameter with `RefMut` convention, the
    matching argument may not be readonly-rooted — so `recv.method()` where the
    method is `&mut self` and `recv` roots in a readonly parameter is rejected
    (dispatched/generic callees are conservatively skipped). Migrated std
    `List::push` → `&mut self` (blast radius across the whole corpus was exactly
    `program-1.vl`) and `program-1.vl` (`add_car`/`purchase` → `&mut self` +
    `own`; two mutated `let` locals → `mut`). program-1 runs unchanged; the only
    `.js` delta is `const`→`let` for the now-`mut` bindings (the `&mut self`/`own`
    changes are pure pass-through). Method-call negative case errors; no
    functional regression. *Known gap:* a `&mut self` call on an immutable `let`
    *local* (e.g. `let xs = List::new(); xs.push(1)`) is not yet flagged — the
    check covers readonly parameters, not let-locals.
  - **Slice 4 — primitive-local views.** Box a viewed primitive local into a
    `(base, key)` cell; support `*v = wholeValue`.
- **Phase 4 — Rule 4 checker.** Lexical view-range analysis flagging
  invalidation + the two interaction rules; enforced on JS → "JS build ⇒
  native-safe".
- **Phase 5 — Projections + provenance.** `borrows` as an inferred signature
  effect (reuses the async/context effect-inference machinery); provenance
  origin-sets enforced via Phase 2 liveness + Phase 4 rule 4; subscript/field
  projection. Unlocks `Arena::get` and mutable iteration over user containers.
- **Phase 6+ (trigger-gated, deferred).** std `Arena<T>`/`Handle<T>`;
  `Shared<T>` + deterministic destruction (the tripwire); coroutine/computed
  projections; `Store<T>` trait; native/WASM backend.

Complexity concentrates in `src/analyzer.rs` (copy classification, view typing,
the three analyses) and `src/transformer.rs` (copy/elision/view lowering).

## Phase 0 findings

### Aliasing audit

Corpus: 21 `test/` programs + ~15 root examples + ~19 std modules. A program's
result can change when aggregates stop aliasing in three ways:

- **P1** — binding alias: an aggregate bound to a second name (`b = a`), then
  divergent mutation.
- **P2** — mutate-through-`self`/param, relied upon by the caller.
- **P3** — field/element alias: an aggregate stored, then mutated via two paths.

Results:

- **P1 — zero.** Every `let/mut x = y` in the corpus has a *scalar* RHS
  (`mut i = 0`, `let flag = true`, `mut count = 0`). No aggregate is ever bound
  to a second name. The divergence case does not occur.
- **P2 — the entire migration.** Mutating methods take **bare `self`** today and
  rely on JS-reference passing:
  - `test/field-assignment.vl` — `increment`, `bump` (`self.value = …`)
  - `program-1.vl` — `add_car`, `purchase` (`self.cars.push`, `self.purchases.push`)
  - `signals.vl` — `set`, `map`, `sub` (`self.value = …`, `self.observers.push`)
- **P3 — none in practice.** Aggregates flow through lists (`find_car_by_make`
  returns a `Car` that is pushed into an `Order`), but nothing mutates a value
  then observes it through another alias; cars are read-only after construction.

**Sequencing consequence (refines the plan):**

- **Phase 1 is behaviorally a near-no-op** on the corpus — it inserts copies only
  at aggregate `let/mut x = <aggregate>` sites, of which there are none.
- **Phase 3 carries the break.** When bare `self`/params become readonly views,
  the P2 methods become **compile errors** ("can't mutate through a readonly
  view") — a loud, compiler-guided migration.

**Migration list (apply in Phase 3 unless noted):**

- `test/field-assignment.vl`: `increment`, `bump` → `&mut self`.
- `program-1.vl`: `add_car`, `purchase` → `&mut self`; stored params (`car`,
  `order`) → `own`.
- `signals.vl`: genuine **shared mutable state** — the `sub` method returns an
  unsubscribe closure that mutates the signal after escaping. Needs
  `Shared<Signal>` or an arena, *not* a Phase-3 `&mut self` fix. **Defer to
  Phase 6** as a `Shared<T>` example.

**Excluded as non-compiling design sketches** (not regressions): `lifetime.vl`
(uses `---` section separators, `&i32`, `Slice`, tuple-index fields — an older
lifetime sketch now **superseded by this proposal; candidate for deletion**),
and the aspirational parts of `signals.vl` / `ui-framework.vl`.

> Phase 1's first task is to establish the actual golden set by building every
> program and recording which currently compile and run — the static audit above
> predicts *behavior* change, but the compiling set must be confirmed empirically.

### Parser surface

Operators lex generically as `Op(&str)` and `mut` is already a keyword, so the
additions are small:

- **New keywords:** `own`, `borrows`.
- **Type grammar:** `&T`, `&mut T` (view types) — a new prefix in type position.
- **Expression prefix** (in the existing `unary` combinator, alongside
  `!`/`async`): `&e`, `&mut e`, `*e`.
- **Parameters:** bare `x: T` = readonly view (default); `x: &mut T` = writable
  view; `own x: T` = consume. Receivers: `self` (readonly view), `&mut self`,
  `own self`.
- **Patterns:** `&x`, `&mut x` binding patterns in match arms and `for`
  (`Some(&mut x)`, `for &mut item in xs`).
- **Return:** `: &T borrows X` / `: &mut T borrows X` — the `borrows` clause is a
  list of parameter names, inferred where omitted.

**`*` deref vs. `*` multiply:** prefix-deref `*e` is recognized only in unary
(operand-start) position; binary `*` (multiply) lives in `product` between two
operands. Since `unary` is `product`'s operand, a leading `*` is unambiguously
prefix — `a * *b` parses as `multiply(a, deref(b))`. Same scheme as Rust; no
lexer change (`Op("*")`/`Op("&")` already produced, and `&mut` lexes as
`Op("&")` then the `mut` keyword).

### Interaction rules

- **Closure captures a view ⇒ the closure is second-class.** It inherits the
  view's escape restrictions (no return, no field, no collection): an escaping
  closure capturing a view would outlive the view's target. A closure that must
  escape may capture only owned values or handles. This is what routes the
  signals "return an unsubscribe closure that mutates the signal" pattern to the
  escape hatch (`Shared`/arena) — correct, because it is genuinely shared
  mutable state.
- **A view may not be live across `await`.** A suspension point is an escape; a
  view spanning it would dangle on native (harmless on JS). Enforced by the
  Phase 4 rule-4 pass.
- **`context` is unaffected.** Context closures capture the threaded context
  *value* (a copy/snapshot, per rule 1), never a view, so the closure-capture
  rule never fires for them; capture-at-creation semantics is preserved.

### Validation strategy

Byte-identical-to-committed is retired (Phase 1 changes output by design).
Per phase:

1. Run each compiling program, capture stdout, assert it matches the
   expected-output comments in the source (`// prints …`).
2. Snapshot the generated `.js` once per phase; guard against unintended drift
   *within* a phase.
3. Track the migration list explicitly: a P2 program is *expected* to error
   until migrated in Phase 3, then expected to pass.

**Stale baselines (regenerate when convenient):** the per-phase checks so far
only regenerated `test/*.vl`, so the committed `.js` for the *root examples* is
stale w.r.t. Phase 1 value semantics — `http-server.js` legitimately gains a
`structuredClone` on `let server = <struct param>` (a copy; `server` is
read-only, so behavior is unchanged). Do a one-shot regenerate-all pass to
refresh the root-example golden files before relying on them as baselines.
