# C4 implementation plan — deterministic destruction, to completion

> **Status: ACTIVE 2026-07-18.** The build sequence for `destruction.md` (**ACCEPTED**
> same day — every §14 call and `claims-and-epochs.md` §8 ratified per recommendation;
> the record is §0 below). Five slices, strictly ordered, each landing green on the full
> suite before the next begins. Companion docs: `destruction.md` (the design),
> `claims-and-epochs.md` (the frame + closure rule), `memory-management-rev-1.md` (the
> data world this leaves untouched).

## 0. Ratified decisions (2026-07-18) — the record

From `destruction.md` §14:

- **(a)** Spelling: **`resource`**, prefix modifier (`resource struct`,
  `resource external struct`).
- **(b)** Naming: trait **`Drop { fun drop(&mut self) }`** + std **`drop<T>(own value)`**;
  `Disposable` stays the data-world protocol.
- **(c)** **R7 strict in v1** — conditional moves rejected; no runtime drop flags. Drop
  flags are the recorded relaxation if real code demands them (proposal-level event).
- **(d)** Owned-nursery children keep **free-task failure reporting** after the owner
  drops; cancellation echoes stay silent.
- **(e)** *(carried no recommendation)* — the draft's working name **`OwnedNursery`**
  stands. The rename window closes when S4 ships it.
- **(f)** **R10 = `Option`-only containment** in v1; move-in/view-out `List<R>` is the
  recorded v1.5.
- **(g)** **Tier 2 wholly deferred to the native arc**; C1's blocker refines from "C4"
  to "counting".
- **(h)** The two Tier-2 clarifications ratified: **`Weak.get(&self): Option<&T> borrows
  self`** and **the trap law matches static rule 4** (spec-only until the native arc).

From `claims-and-epochs.md` §8:

- **(a)** The **closure rule** stands: C4 is the last major change; post-C4 memory work
  must name its cell and classify as refinement; **async drop is out, not deferred**.
- **(b)** **C7 wire-blessed handles: blessed**, under §6's hard conditions (per-session
  arenas as the blessed default; per-arena random `brand` for anything cross-tenant;
  the derive tolerates `Handle`'s phantom parameter). Build stays backlog C7 — not on
  C4's critical path.
- **(c)** The frame **graduates into the spec**: the memory chapter opens with the
  claims/epochs law at S5.

## 1. Ground rules (every slice)

- **Suite gate on exit code**: `cargo test` (workspace) must exit 0; commit gates are
  one-command (`… && grep -q "suite exit: 0" … && git -C ~/code/vilan commit …`); never
  `cd` in a compound with a mutating git command; mutating git always `git -C`.
- **Rebuild the debug binary before regenerating any golden or corpus check** (a stale
  binary silently writes wrong goldens).
- **Docs in the same commit** as the surface they describe: S1 → spec grammar appendix +
  errors appendix entries; S3 → the `Option` std page; S4 → the db + task std pages (and
  any guide page showing `Database`); S5 → the new chapter + full sweep. The docs gate
  (`cargo test --test docs`) is part of each slice's suite.
- **Per-case pins, not per-example**: every §13 matrix entry is its own test in
  `crates/vilan-core/tests/inference.rs` (`assert_compiles` / `assert_compiles_and_runs`
  / `assert_fails`); a found-but-deferred gap is an `#[ignore]`d pin. §13 distributes
  across the slices — S1 the static rules, S2 drop/order, S3 `take`/`replace` + the
  runtime match-move, S4 the e2e pair — and the plan adds pins beyond §13 where
  interactions demand (the derive rejects, the turn-wave drop, detached-mode
  reporting).
- **Every new codegen helper gets a macro-interpreter arm** in the same commit
  (`interpreter.rs`) — the recorded equivalence-gate gotcha.
- **Corpus expectations**: S1 is validation-only → byte-identical. S2+ add new corpus
  programs (`vilan/test/resource*.vl`); any diff to an existing golden is a stop-and-
  investigate event, not a regen.

## 2. S1 — classification + the affine checker (L; pure validation)

**Surface** (`token.rs`, `lexer.rs`, `parser.rs`, `node.rs`, `formatter.rs`):

- `Token::Resource` keyword; parser accepts the modifier in `external`'s position on
  `struct` / `external struct` / `enum` declarations; an `is_resource` flag on the
  declaration node; formatter prints it back (idempotence test). Grammar + errors
  appendices updated in the same commit.

**Classification** (`analyzer.rs`):

- `type_is_resource(TypeId)`: declared `resource` OR any field / enum payload / tuple
  member is a resource — recursive, memoized, cycle-guarded (the `Wire`/`Hashable`
  all-fields machinery with the polarity flipped: any resource member marks the whole).
- **Per-instantiation** for generics (`Option<Database>` yes, `Option<i32>` no): the
  instance-worklist precedent (async adaptation, platform coloring). Memoize per
  substituted instantiation.
- `Drop` impl on a non-resource type → error steering to add `resource`.
- The `Wire` / `Hashable` / `PartialEq` derive all-fields checks reject resource fields
  (inert until std grows a resource — S4 exercises them).

**The affine checker** (new post-`build()` pass; machinery reuse noted per rule):

- **R1/R3/R4** moves + use-after-move: the rule-2 last-use machinery
  (`reference_count`, elision) already computes "is this the last use" — resources
  *require* what elision *prefers*; the new part is erroring (note channel: "moved
  here") instead of copying. `own` argument not at last use → error.
- **R5** struct literals move in; field access is loan-only; moving out of a live
  aggregate rejected (no partial moves).
- **R6** match by value consumes the subject (captures are the move — today's
  match-capture alias emission is exactly move-correct; see §7 risk); `match &place`
  inspects.
- **R7** every-path-or-none conditional-move rejection: block-structured scan (the
  rule-4 `scan_invalidation` shape).
- **R8** loop-interior moves: `collect_repeatable_interiors` reuse.
- **R9** closure/spawn captures rejected (`closure_captures_view_param` precedent,
  extended from views to resource-typed captures, spawn closures included); injected
  `context`-clause bodies receive resource *parameters* as loans — exempt.
- **R10** resource type arguments rejected for `List`/`Map`/`Set` and every external
  generic (`Shared`, `Task`, `Promise`, `Context`); `Option` sanctioned.
- **R11** per-instantiation move-clean re-check of generic bodies (T := resource),
  spanned at the instantiation site. Recorded fallback if the general check drags:
  bless `Option`'s surface first, ship the general rule as the immediate follow-up —
  but the general rule is the design. Shipped residues (recorded with the chunk):
  the move scan descends into *direct* lexical closures only — a nested closure's
  internal T-double-move is unseen (`#[ignore]`d pin
  `r11_nested_closure_internal_double_move_is_rejected`; captures are caught
  transitively); dispatched callees skip discovery (the standing convention); the
  *primary* span uses the chunk-3 `SourceId(0)` convention, so cross-file
  instantiations mis-anchor the primary (the body note carries the correct source)
  — diagnostics polish, not semantics.
- **R12** no coercion to `any` (arguments, bindings, returns; `print(db)` included).
  Recorded conservatism (shipped with the chunk): argument coverage resolves callees
  via `subject -> Local(callee)` — free functions *and* concrete-receiver methods are
  both seen (pinned); the residue is *dispatched* callees (trait-typed receivers),
  skipped like the existing convention checks skip them — R11's per-instantiation
  re-check is the recorded net under that residue.

**Diagnostics**: the §11 vocabulary verbatim, each with its steer, per the diagnostics
standard.

**Pins (S1 subset of the §13 matrix)**: let-move, own-param-move, own-not-last-use,
loans via `self`/`&`/`&mut`, return-move, struct-literal-move, field-loan-only,
match-consume vs match-loan (the static half — the runtime alias-as-move pin is S3's),
conditional-move reject, loop-interior reject,
closure-capture reject, spawn-capture reject, injected-body loan accept,
container-element reject, `Context<R>` reject, generic move-clean accept
(`Option::unwrap`, `map`-shape) + dirty reject, `any` reject,
derive-rejects-resource-field. (The `Drop`-on-data reject pin rides S2 — the trait only
exists from S2, so keying the check on a bare trait name earlier would be fragile.)

**Acceptance**: suite green, corpus byte-identical, no std fallout (std declares no
resources yet).

## 3. S2 — `Drop`, insertion, lowering (M–L)

- **The trait** in std (`Drop { fun drop(&mut self) }`; home: a small `drop.vl` beside
  `option.vl`, or `lib.vl` — decide at impl), plus its restriction: `Drop` implemented
  on a data type errors, steering to add `resource` (the pin moved here from S1 — the
  check keys on the real std trait, which exists only from this slice).
- **Analyzer drop planning**: per scope, the still-owned resource locals at each exit
  (static by R7) in reverse declaration order; R2 overwrite-drop points; per-type field
  order (body before fields, fields reversed); enum payloads with the value.
- **Transformer**: `try`/`finally` only on resource-owning scopes; per-type drop helper
  (direct call to the impl's `drop`, then field drops; helper naming decided at impl);
  **interpreter arms for every helper**; `ret`/`jump break`/`jump continue`/panic all
  flow through `finally` natively on JS; module-level resources never drop; a drop
  panicking during unwind replaces the in-flight error (JS `finally` semantics —
  document).
- **Corpus**: new `test/resource.vl` (+ golden, debug binary rebuilt first).
- **S4a residue (recorded 2026-07-19):** the §5 loan-only check covers every
  function-body consuming use of a module-level resource; a module-*initializer*
  global→global move (`let b: Res = a` at module scope) is not scanned — benign
  (module globals never drop, so no double-close), a diagnostic-completeness
  residue, not soundness.
- **S2b implementation findings (settled 2026-07-19, §8 amended):** a context-requiring
  `drop` body is REJECTED in v1 (the helper cannot thread `$ctx`; the "joins the wave"
  sentence was unimplementable as written); platform coloring of drops needs a
  **synthetic ownership edge** (owner scope → drop impl) because the inserted call is
  transformer-side and invisible to reachability. Both ship with S2b, pinned.
- **Pins**: drop order (locals reverse; fields reverse; body-before-fields), early
  `ret`/`jump` drops, panic-unwind drops, overwrite drops (R2), enum payloads drop with
  the value, containment-only type
  (no `Drop` impl) still drops fields, module-level-never-drops, across-`await`
  ownership legal (E3 untouched — loans still fenced), a signal-writing drop joins the
  ambient turn's wave (§8 Turns sentence, pinned).

## 4. S3 — `Option.take`/`replace` + `drop<T>(own)` + match-move (S–M)

- **Settled before building (2026-07-19, forced by S2b's findings):** S2b left `own`
  resource parameters un-dropped (a recorded safe leak). S3 closes it — with a fork
  forced by generic erasure: a generic body is EMITTED ONCE, so a `T`-typed `own`
  parameter cannot get instantiation-conditional drop insertion (and drop flags are
  ratified out, R7/(c)). Therefore: **concrete-typed `own` resource parameters drop at
  scope end like locals; under a resource instantiation an `own T` parameter must be
  moved on EVERY path** — R11 tightens from at-most-once to exactly-once for `own T`
  parameters (zero-move = the leak the shared body cannot drop), spanned at the
  instantiation. `drop<T>(own value)` itself satisfies exactly-once… by moving into
  its own scope — the compiler treats the std `drop` sink's parameter as consumed
  (it IS the drop site; special-known like the `Shared` intrinsics). Pins: concrete
  own-param drops (runtime order), generic zero-move rejected at a resource
  instantiation + accepted at data, `drop(db)` early teardown order.
- **Ruling (2026-07-19, S3 finding):** the sink lowers by CALL-SITE rewrite to the
  concrete `__drop` helper; inside a generic body `drop(x)` on a still-`T`-typed value
  has no concrete destructor (erased emission), so R11's per-instantiation re-check
  treats a drop-sink call with a generic-typed argument as dirt **under a resource
  instantiation** ("the erased body cannot destroy a `T` — move it out to the caller")
  — data instantiations keep the legitimate generic consume-idiom (no-op is correct
  for data). Sink-call arguments also seed the §8 synthetic coloring edges (a
  `@process` drop reached only via `drop(db)` must still color/reject).

- **Intrinsics** `take(&mut self): Option<T>` / `replace(&mut self, value: T):
  Option<T>` — compiler-known (the `Shared` intrinsics pattern): analyzer registration,
  transformer lowering (read slot, write, return old), interpreter arms. Land as
  ordinary std surface on `option.vl` — useful for data, *required* for resources (R5's
  sanctioned partial move; R7's conditional-teardown idiom).
- **std `fun drop<T>(own value: T) {}`** — a data no-op sink; for resources, early
  teardown at its immediate scope end. No public `close()` anywhere.
- **Match-move emission audit**: pin that a resource match-consume aliases the payload
  as the move (runtime pin), and that `match opt.take() { Some(let c) => drop(c), .. }`
  tears down conditionally.
- **Docs**: the `Option` std page gains `take`/`replace` in the same commit.

## 5. S4 — std adoption: `Database` + `OwnedNursery` (M)

- **`Database`** (`std/src/process/db.vl:15`): `resource external struct Database` +
  `impl Database with Drop` closing the `node:sqlite` handle (needs a private close
  hook — an external fn or intrinsic used only by `drop`; none exists today). No public
  `close()`; `drop(db)` is the early form. The kolt/server module-level idiom is
  untouched by design (module-level never drops).
- **`OwnedNursery`** (`std/src/task.vl`, beside `Nursery`/`ambient_nursery`): the §9
  API verbatim — `new()` detached via `__nursery_new`, `enter<T>(&self, body: (|| T)
  context ambient_nursery): T` = Part B's registration machinery minus the join,
  `cancel(&self)` idempotent, `Drop` cancels. **`Nursery` itself stays data** (it is
  the ambient handle; `Context<R>` is R10-rejected — ownership lives only in the
  wrapper). **Detached mode is new machinery** (decision (d)): under shipped semantics a
  nursery-owned child never default-reports, so `enter`'s children need the free-task
  reporting path re-opened — a non-cancellation failure reports (console, spawn
  origin), no sibling cancellation, cancellation echoes silent. Pin all three.
- **Free-spawn migration**: move std's remaining free spawns (the SSE pump,
  `Draft.commit`'s spawned commit) onto owned nurseries per the async-polymorphism
  opens; then the **J4 free-spawn lint** if the rule states cleanly (*a spawn happens
  inside a `nursery` extent or an `OwnedNursery.enter` — anything else is a lint*); if
  it doesn't state cleanly, record why on J4 and defer.
- **e2e**: dropping an owner cancels an in-flight sleeping task (the cancellation.rs
  shape); drop-runs-under-cancellation (bridged rejection → unwind → drops).
- **Docs**: db + task std pages, any guide page showing `Database`, same commit.

## 6. S5 — the docs slice (M)

- **Spec**: new §6.x "Resources and destruction" (affine rules, drop timing/order, the
  §8 interaction sentences, honesty limits); **re-word §6.4's implementation note and
  §6.7's "exclusive" parenthetical** to the reconciled trap law (trap on invalidation,
  never on overlap); **open the memory chapter with the claims/epochs law** (§8(c)
  ratified): owners, epochs, events, claims, the two regimes, the table — mechanisms
  derived from it, escape-ladder teaching preserved.
- **Tour**: the resources chapter (declare, move, loan, `drop(x)`, `OwnedNursery`,
  `Option.take`).
- **Errors appendix**: the full §11 sweep. Glossary entries if D2 is in flight.
- Every fenced example compiles (`cargo test --test docs`); book regenerated.
- **Bookkeeping**: backlog C4 → shipped; C1's blocker reads "counting"; memories
  updated; `destruction.md` status → SHIPPED with commit ids.

## 7. Risks and recorded fallbacks

- **R11 general check drags** → the recorded fallback ((f)/(R11)): bless `Option`'s
  surface first; the general rule ships as the immediate follow-up.
- **R7 strictness meets real code** → drop flags are the recorded relaxation; any such
  hit is a proposal-level event, never a checker special-case.
- **Match-capture alias dependency**: resource match-consume *relies* on the recorded
  data-world alias gap being move-correct. If that gap is ever fixed for data, the
  resource path must keep the alias-as-move emission — the S3 runtime pin is the guard.
- **Helper/interpreter equivalence**: every S2/S3 helper needs its interpreter arm or
  the equivalence gate breaks (bitten twice before).
- **Classification blowup**: memoize per instantiation; the worklist dedupes — watch
  compile times on std (E3-scale budget: none measurable expected; classification is
  per-type, not per-expression).
- **Hidden std copies of `Database`**: S4 flips it to affine; any std/kolt code that
  copies a `Database` binding becomes an error — that fallout is the *point* (each hit
  is a real double-close hazard); fix call sites with loans, never weaken the rule.
- **Turn interaction**: drops are ordinary statements at scope exits (§8) — no special
  turn machinery; the S2 pin (signal-writing drop joins the current wave) documents it.

## 8. Explicitly after C4 (the closure rule applies)

Post-C4, every memory-model item must name its `claims-and-epochs.md` §3 cell and
classify as refinement. The standing queue: **Tier 2** with the native arc (counting,
`Weak` + `get`, counted closure environments, the JS counted debug mode), **C2**
runtime generations (F4, debug-mode), **C6** `bumps` geometry-effect inference, **C7**
wire-blessed handles (decision made: bless), **C8** `Arena.get` view-form migration
(std-only, independent of C4 — delegatable anytime), `Store<T>` extraction (trigger =
Tier 2), `List<R>` move-in/view-out (v1.5), computed projections (coroutine lowering). Async
drop is **out** — awaited teardown is met with surface idioms, never a suspending
`drop`.
