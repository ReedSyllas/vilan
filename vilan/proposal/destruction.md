# Deterministic destruction — the owned-resource class (backlog C4)

> **Status: ACCEPTED 2026-07-18 — reviewed same day; every §14 call and the companion's
> §8 ratified per recommendation. Build sequence: `destruction-impl-plan.md`. Nothing
> implemented yet.** The keystone of
> backlog §C: `memory-management-rev-1.md` deferred destruction behind a tripwire ("revisit
> before the first type with a non-memory drop obligation targets JS") — std has since grown
> several (`Database` has no `close`, sockets and timers lean on process exit, task teardown
> is manual). This proposal answers the tripwire. It also *specifies* C1 (`Weak<T>`) against
> the counted tier (§10) — C1 ships with counting, not with this v1 — and leaves C2 folded
> into F4's native arc, per `backlog-2026-07-18.md`. **Companion: `claims-and-epochs.md`**
> (2026-07-18) states the one law behind the whole model and records the closure decision
> — C4 is the **last major change** to the memory model; its two Tier-2 clarifications
> (`Weak.get`, the trap law) are folded into §10 below.

## 1. Why now

- **The resource-owner story is the named blocker** for Part B's free-spawn lint
  (`async-polymorphism.md` opens): every remaining free spawn in std is object-lifetime work
  that a function-scoped `nursery` cannot own. Objects need destructors before they can own
  tasks.
- **F3/F4 call C4 the linchpin** of the non-JS memory lowering (allocator + scope-end drops
  + ARC for `Shared`). The semantics must exist — and be exercised on JS — before an
  emitter needs them.
- **It is the last breaking-flavored change on the board.** The affine rules below change
  how resource values bind and pass. Every month adds std surface, and F5/F7 will add
  users; the break is cheapest now (the agreed order: C4 → A13 → F5/F7 → A7).

## 2. The tension, and the shape of the answer

Rule 1 of the memory model says values copy. A droppable value cannot mean anything under
copying: a copied file handle double-closes, a copied refcount miscounts. So destruction
cannot be bolted onto the data world — the world must be partitioned:

- **Data** — everything vilan has today. Copies on binding, elides at last use, reclaimed
  by GC on JS / the stack+arena story on native. **Entirely unchanged by this proposal.**
- **Resources** — a small, explicitly-rooted class with *affine* discipline: a resource
  value has exactly one owner at a time; it **moves** on binding and `own`-passing, is
  **loaned** through the existing view conventions, can never be copied, and its owner's
  scope end runs its destructor.

Rejected shapes:

- **ARC everywhere (Swift)** — retain/release instrumentation on every copy site, on a JS
  backend that needs none of it for data. Pays a global cost for a corner problem.
- **Affine everything (Rust)** — rejected by rev-1 from the start; the move checker's
  complexity lands on every user instead of the advanced corner.
- **Protocol-only (status quo)** — `Disposable`/`Owner` works where a framework drives it
  (UI boundaries), but nothing enforces it, nothing composes it (a struct holding a
  `Database` has no story), and native cannot be built on convention.

The class is **two tiers**. Tier 1 — this proposal, ships on JS — is *unique* resources
(one owner, move-only). Tier 2 — specified in §10, built with the native arc — is
*counted* resources (`Shared` ARC, `Weak`, counted closure environments). The split
exists because counted handles must be closure-capturable (that is `Shared`'s whole job),
and capture-with-release requires counted closure environments — native-arc machinery.
Nothing in Tier 1 forecloses Tier 2.

## 3. Classification — what is a resource

- **`resource` is a declaration modifier** (position like `external`):

  ```vilan
  resource external struct Database;

  resource struct Session {
      db: Database,
      tasks: OwnedNursery,
  }
  ```

- **Containment infers.** An aggregate (struct, enum, tuple, fixed array) with a
  resource field, payload, or element type *is* a resource — recursively, the `Wire`/`Hashable`
  all-fields machinery with the polarity flipped (any resource member marks the whole).
  `Session` above needs no modifier; writing it is legal and checked (declaring
  `resource` on a type is always allowed — intent: "will gain teardown / must not be
  copied" — but omitting it never hides resource-ness).
- **The modifier is required at leaves**: an `external struct` is opaque, so host-object
  resources (`Database`) must say so themselves.
- **`Drop` may be implemented only for resource types** — an impl on a data type errors,
  steering to add `resource` (destruction without move discipline is exactly the
  double-close bug).
- **Per-instantiation for generics**: `Option<Database>` is a resource *instantiation*;
  `Option<i32>` stays data. Resource-ness of a generic type is decided at each
  instantiation, like platform coloring and asyncness bits already are.

## 4. The affine rules

Terminology: *move* = ownership transfer, source binding dead after; *loan* = the existing
second-class view (`self`/`&`/`&mut` conventions), no ownership change, rule-4 policed.

- **R1 — binding moves.** `let b = a;` transfers; any later use of `a` is a compile error
  naming the move site (note-channel: "moved here"). No clone sites ever fire for
  resources.
- **R2 — overwrite drops.** Assigning onto a binding that still owns a resource drops the
  old value first, then moves the new one in (deterministic; Rust's rule).
- **R3 — parameters.** `self` / `&x` / `&mut x` conventions are loans, unchanged. `own x`
  is a move — and for resources it is *only* a move: where a data `own` argument silently
  copies when not at last use, a resource argument that is not the binding's last use is
  an error.
- **R4 — returns move out.** Including through `if`/`match` tails (a diverging leg is
  exempt as ever).
- **R5 — fields.** A struct literal moves resources in. A resource field is accessed by
  loan only (`self.db.exec(..)`, `&mut self.db`); copying it out is impossible and
  *moving* it out of a live aggregate is rejected (no partial moves in v1). The sanctioned
  partial move is `Option`: `self.slot.take()` (§6).
- **R6 — match consumes.** Matching a resource *by value* consumes the subject; pattern
  captures move the payloads into the arm. (Today's match-capture emission aliases the
  payload — a recorded data-world gap that is exactly move-correct here: the subject is
  dead, the alias is the move.) Matching a loan (`match &self.state`) inspects without
  consuming.
- **R7 — no conditional moves.** A binding must be moved on every path through a scope or
  on none: `let f = open(); if c { consume(f); }` errors ("moved on one path —
  restructure with `Option` + `take`, or move on every path"). This keeps end-of-scope
  ownership static — no runtime drop flags in v1; drop flags are the recorded relaxation
  if real code demands them.
- **R8 — no moves in repeatable interiors.** Moving a binding declared outside a loop from
  inside its body errors (`collect_repeatable_interiors`, the machinery rule 2's elision
  already uses).
- **R9 — closures and spawns cannot capture resources.** The P4c precedent
  (`closure_captures_view_param`) extended from views to resources, spawn closures
  included. The idioms instead: pass a loan down the call graph; make the closure's owner
  a struct that owns the resource; own tasks through an `OwnedNursery` (§9). Injected
  bodies (`context`-clause closures) receive resource *parameters* as loans — parameters
  are per-call, not captures — so `nursery(|n| ..)`-shaped APIs are unaffected.
- **R10 — no resource elements in the native containers.** `List`/`Map`/`Set` (and every
  external generic: `Shared`, `Task`, `Promise`, `Context`) reject resource type
  arguments in v1 — their internals are host code the move checker cannot see. `Option`
  is the sanctioned container (it is a vilan enum, checkable under R11). A move-in/
  view-out `List<R>` API is the recorded v1.5 (connection registries want it eventually).
- **R11 — generics must be move-clean per instantiation.** Instantiating a type parameter
  with a resource type re-checks the instantiated body under the affine rules (T := the
  resource): every T-typed value used at most once as a move, no captures, no copies.
  `Option::unwrap(self): T` passes (self consumed once, payload moved once); a body that
  reads its parameter twice fails at the instantiation site, not inside std. Mechanism:
  the instance-worklist precedent (async adaptation, platform coloring) — checks keyed by
  (function, resource bindings). Fallback if the general check drags in v1: bless
  `Option`'s surface first and ship the general rule as the follow-up — but the general
  rule is the design.
- **R12 — no coercion to `any`.** A resource passed where `any` is expected errors
  (`print(db)` included) — `any` is a data sink; the discipline must not launder away.
  Debug-print fields instead.

## 5. Destruction

- **The trait:**

  ```vilan
  trait Drop {
      fun drop(&mut self);
  }
  ```

  `&mut self`, exactly Rust's shape: the body cleans up through a loan, and the compiler
  destroys the fields *afterward* (reverse field order). This makes resurrection
  impossible — an `own self` destructor could move `self` somewhere that keeps it alive,
  and would need to suppress its own re-drop. Rejected alternative: evolving `Disposable`
  — that is a *cooperative protocol* for data-world teardown (subscriptions, owners; its
  `dispose(self)` is a bare loan, and `Owner.take` stores `|| item.dispose()` closures —
  captures, which R9 forbids for resources). The two coexist: `Disposable` for
  framework-driven data teardown, `Drop` for the language hook. A resource without a
  `Drop` impl is legal — containment alone still enforces moves and drops its fields.

- **Timing and order.** At the owner's scope end, still-owned resource locals drop in
  reverse declaration order; a value's own `drop` body runs before its fields (reverse
  field order); enum payloads drop with the value. Every exit runs drops: fall-through,
  `ret`, `jump break`/`jump continue` (out of the scopes they leave), and panic
  unwinding.
- **Early teardown is a move, not a method:** std gains

  ```vilan
  fun drop<T>(own value: T) {}
  ```

  — moving into `drop(db)` destroys at its (immediate) scope end. No public `close()`
  surfaces to keep in sync with destructors, no double-close states.
- **Module-level resources never drop** (process lifetime; Rust-statics precedent;
  documented — a serve-forever app's `Database` is exactly this).
- **Panic during unwind:** a `drop` that panics while unwinding replaces the in-flight
  error (JS `finally` semantics — documented; a native backend would abort, also
  documented).
- **Across `await`:** owning a resource across a suspension is legal — frames own their
  locals; E3's no-view-across-`await` is about *loans* and is untouched. Under
  cancellation, bridged operations reject (`AbortError`) → the frame unwinds → drops run.
  Honesty limit, same one Part B recorded: an *unbridged*, never-settling await leaks the
  frame and its drops.
- **`drop` is synchronous in v1.** An `async`/awaiting drop body is rejected ("teardown
  must be synchronous — cancel owned tasks via `OwnedNursery`; awaited teardown is a
  future design"). Async-drop is unsolved in Rust for good reasons; not v1's fight.

## 6. `Option.take` — the sanctioned partial move

Moving out of a place must leave a valid value behind. One new intrinsic pair on
`Option<T>` (compiler-known, like the `Shared` intrinsics):

```vilan
impl Option<type T> {
    fun take(&mut self): Option<T>;              // Some(v) -> (None left behind, Some(v) out)
    fun replace(&mut self, value: T): Option<T>; // new in, old out
}
```

Useful for data too (they land as ordinary std surface), but *required* for resources:
`self.conn.take()` is how a field's resource leaves a live aggregate (R5), and
`match opt.take() { Some(let c) => drop(c), None => {} }` is the conditional-teardown
idiom R7 pushes toward.

## 7. JS lowering

- **`try`/`finally` per resource-owning scope.** Only scopes that own resources pay. The
  `finally` drops still-owned locals in reverse order; R7 makes "still-owned" static, so
  there are no runtime flags. `ret`/`jump`/panic all flow through `finally` natively.
- **Drop dispatch** is a direct call to the impl's `drop`, then field drops — emitted as a
  per-type helper (naming/shape decided at implementation). **Every helper needs its
  macro-interpreter arm** (the recorded equivalence-gate gotcha).
- **Moves compile to nothing** (the JS reference passes as it always did); the affine
  rules are purely static. This is the same "checked on JS, meaningful on native"
  single-conformance stance as rule 4.
- **`take`/`replace`** lower like the existing intrinsics (read slot, write slot, return
  old) — the one genuinely new runtime behavior besides `finally`.

## 8. Interactions (each gets a spec sentence)

- **Views / rule 4:** loans are views; E1/E2/E3 apply unchanged. Scope-end drop coincides
  with lexical view death, so no new event kind is needed; a `borrows` projection of a
  resource cannot outlive it (second-class already).
- **Turns / contexts** *(amended 2026-07-19 — S2b implementation finding)*: drops are
  synchronous statements at scope exits. The draft's "a drop that writes signals joins
  the current wave — documented, not special" was wrong about its own lowering: a
  context-requiring `drop` compiles to `drop(self, $ctx)`, and scope-exit insertion
  points neither thread contexts nor statically guarantee one. **v1 rejects a `drop`
  body that requires an ambient context** ("teardown must be context-free — hand
  turn-joining work to an owner inside the turn"); context-threading into teardown is
  recorded future work if a real driver appears. Neither std driver (`Database`,
  `OwnedNursery`) needs it.
- **Platform coloring** *(amended 2026-07-19 — same finding's sibling)*: a drop body
  colors like any function, but the inserted call is transformer-side — reachability
  cannot see it. The mechanism is a **synthetic ownership edge**: a scope owning a
  droppable resource of type `T` reaches `T`'s drop impl in the call graph, so a
  `@process`-needing drop colors its owning scopes.
- **Wire / Hashable / PartialEq derives:** the all-fields checks reject resource fields
  (a resource is not plain data; it cannot be sent, hashed by value, or compared by copy).
- **`const`:** resources are not plain data — const evaluation already rejects them.
- **Contexts:** `Context<R>` is rejected by R10 (context values thread as data). This is
  why `Nursery` — the ambient *handle* — stays data, and ownership lives in a wrapper
  (§9).
- **Macros/worlds:** `resource` is a parse-level modifier; worlds and expansion are
  indifferent.

## 9. Std in v1 — two drivers, and what deliberately does not change

- **`Database`** becomes `resource external struct` with `impl Database with Drop`
  (closing the underlying `node:sqlite` handle). No public `close()` — `drop(db)` is the
  early form. The kolt/server idiom (module-level, process-lifetime) is untouched by
  design (§5).
- **`OwnedNursery`** (name open, (e) in §14) — *the* resource-owner story:

  ```vilan
  resource struct OwnedNursery {
      nursery: Nursery,
  }

  impl OwnedNursery {
      fun new(): OwnedNursery;                                  // __nursery_new, detached
      fun enter<T>(&self, body: (|| T) context ambient_nursery): T;  // spawns inside register here
      fun cancel(&self);                                        // early, idempotent
  }

  impl OwnedNursery with Drop {
      fun drop(&mut self) { self.nursery.cancel(); }
  }
  ```

  `enter` runs its body under `ambient_nursery.run(self.nursery, ..)` — Part B's existing
  registration machinery, minus the join. Drop cancels: in-flight bridged IO aborts.
  Reporting needs **new machinery, not registration as-is**: under shipped semantics a
  nursery-owned child never default-reports (absorption exists for the join to re-raise
  — `task.vl`'s "no `await`, no enclosing nursery" rule), so a never-joining nursery
  would be exactly the silent error sink decision (d) forbids. `enter`'s nursery
  therefore runs in **detached mode**: a child failure that is not a cancellation echo
  takes the free-task reporting path (console, spawn origin) instead of being stored for
  a join, and does not cancel its siblings — children are independent; ownership is
  lifetime, not fate-sharing. Cancellation echoes stay silent. This is what the SSE
  pump and `Draft.commit` become owned by, and what lets J4's **free-spawn lint** finally
  state its rule: *a spawn happens inside a `nursery` extent or an `OwnedNursery.enter` —
  anything else is a lint.*
- **Deliberately unchanged in v1:** `Shared` (stays a copyable data handle on JS — §10
  owns its counted future), `Owner`/`Disposable`/subscriptions (cooperative data-world
  teardown, framework-driven, capture-based — R9 is exactly why they must not be
  resources), transports (hold `Shared` cells; reconnect semantics want sharing), and
  `ResponseStream` (host-lifecycle via `on_close`).

## 10. Tier 2 — the counted class (specified now, built with the native arc)

- **`Shared<T>` becomes a counted resource**: `clone()` = retain; a handle's death =
  release; zero → the cell's value drops. Handle death is deterministic *because handles
  ride the Tier-1 machinery* (scope-end, moves) — the counting itself is what JS never
  needed and native requires (F3's "ARC for `Shared`"). An optional JS *counted mode*
  (debug builds) is recorded as a verification tool, not a semantic.
- **The dynamic trap law matches static rule 4** (from `claims-and-epochs.md` §5b —
  rev-1's "a `write()` while any other view is live traps" is *stricter* than the static
  rule it claims to mirror, and the asymmetry must not fossilize into the native tier):
  statics deliberately permit aliased views and content writes (two `&mut` to one
  scalar; sibling-field writes under a field view) and forbid only *invalidation*. The
  dynamic check enforces the same event set: a cell-value reassignment,
  geometry-bumping operation, or death under another live view into the cell traps;
  overlapping content writes never do. C2's runtime generations key off the same
  events, and C6's inferred geometry effects (`bumps`, the twin of `borrows`) are what
  classify a method call through `write()` — one law, one event classifier, two
  checkers.
- **`Weak<T>` (C1)**: `Shared::downgrade(&self): Weak<T>`; `Weak.upgrade(): Option<Shared<T>>`
  — `Some` (retaining) while strong > 0, `None` after, *deterministically*. Ships with
  counting; the 2026-07-07 rejection of GC-timing `WeakRef` stands. **Also
  `Weak.get(&self): Option<&T> borrows self`** (from `claims-and-epochs.md` §5a): the
  scoped, non-retaining twin of `upgrade`, mirroring `Arena.get`'s specified form
  (backlog C8 migrates std's interim copy-returning `get` to it) — every dynamic alias
  then answers the same verb with the same `Option<&T>` shape. The view is second-class and
  rule-4-policed; on native it pins the cell for its lexical extent (a scoped
  retain/release pair — a last-strong release inside that extent defers the cell's drop
  to the view's block end: deterministic, merely later), on JS it is a plain read.
  `upgrade` is for keeping the cell alive; `get` is for touching it. This also delivers
  the second store rev-1's deferred `Store<T>` trait was waiting for (`Shared`/`Weak`
  is a one-slot counted arena: `clone` = retain, `Weak` = the handle) — extract the
  trait when Tier 2 builds, not before.
- **Counted closure environments**: a closure capturing a counted handle holds a retain,
  released when the environment dies — which requires environments themselves to be
  counted objects (Swift's model). This is the single reason `Shared` cannot join Tier 1:
  capture is its job (subscriber lists, turn queues), and R9 would gut it. Nothing in
  Tier 1 assumes closure environments are free, so the door stays open.
- **C2's dynamic rule-4** (cross-handle generation checks) rides the same native lowering,
  per `backlog-2026-07-18.md`.

## 11. Diagnostics vocabulary (the standard applies)

- Use-after-move: primary at the use, note at the move ("`db` was moved here — a resource
  has one owner; loan it with `&db` / `&mut db`, or restructure with `Option` + `take`").
- Capture: "a closure cannot capture the resource `db` — pass a loan into the call, or
  give ownership to the struct that owns this closure's lifetime".
- Conditional move (R7), loop move (R8), container element (R10), unclean generic (R11,
  spanned at the instantiation), `any` coercion (R12), `Drop` on data, non-last-use `own`
  argument — each with a steer.

## 12. Implementation plan (slices, each suite-gated, docs in the same commit)

1. **S1 — classification + the affine checker** (no destructors yet): `resource` modifier
   through lexer/parser/formatter/analyzer; containment inference; R1–R12 checks; the
   full pin matrix (below). Pure validation — corpus byte-identical.
2. **S2 — `Drop` + insertion + lowering**: the trait, scope-end `finally` emission,
   overwrite-drop, ordering; macro-interpreter arms; corpus `resource.vl`.
3. **S3 — `Option.take`/`replace` intrinsics + match-move rules + std `drop<T>(own)`**.
4. **S4 — std adoption**: `Database` + `OwnedNursery` (+ e2e: dropping an owner cancels
   an in-flight sleeping task — the cancellation.rs shape); the J4 free-spawn lint if the
   rule states cleanly.
5. **S5 — spec §6.x "Resources and destruction" + tour chapter + errors appendix.** Also
   re-words spec §6.4's implementation note and §6.7's "exclusive" parenthetical to the
   reconciled trap law (§10 — trap on invalidation, not on overlap), and — per the
   ratified §8(c) of `claims-and-epochs.md` — opens the memory chapter with the
   claims/epochs law.

## 13. Pin matrix (S1/S2 acceptance)

{let-move, mut-overwrite-drops, own-param-move, own-not-last-use-error, loans via
`self`/`&`/`&mut`, return-move, struct-literal-move, field-loan-only, enum-payload,
match-consume, match-loan-inspects, `take`/`replace`, conditional-move reject,
loop-interior reject, closure-capture reject, spawn-capture reject, injected-body loan
accept, container-element reject, `Context<R>` reject, generic move-clean accept
(`Option::unwrap`, `map`-shape), generic dirty reject, `any` reject, `Drop`-on-data
reject, drop order (locals reverse; fields reverse; body-before-fields), early `ret` /
`jump` drops, panic-unwind drops, across-`await` ownership, cancellation-runs-drops
(e2e), module-level-never-drops, resource-without-Drop (containment-only) drops fields}
— each its own pin, per the per-case testing rule.

## 14. Open questions — user calls wanted before S1

> **All calls made 2026-07-18** — recommendations ratified as written. (e), which
> carried no recommendation: the draft's working name **`OwnedNursery`** stands; the
> rename window closes when S4 ships it. Items kept below for the record.

- **(a) Spelling**: `resource` as a prefix modifier (`resource struct`, `resource external
  struct`) — or another word (`owned`?). Recommendation: `resource`.
- **(b) Naming**: trait `Drop { fun drop(&mut self) }` + std `drop<T>(own value)`.
  Recommendation: as written (short, unambiguous, precedented; `Disposable` stays for the
  data-world protocol).
- **(c) R7 strictness**: reject conditional moves in v1 (recommendation) vs runtime drop
  flags from day one.
- **(d) Owned-nursery children reporting**: keep free-task failure reports (recommendation)
  vs silent absorption after the owner drops.
- **(e) `OwnedNursery` naming** — `OwnedNursery` / `TaskOwner` / `Tasks` / other.
- **(f) R10 for v1**: `Option`-only containment (recommendation), `List<R>` API recorded
  as v1.5.
- **(g) Tier 2 wholly deferred to the native arc** (recommendation) — including `Weak`,
  whose C1 blocker refines from "C4" to "counting".
- **(h) The two Tier-2 clarifications from `claims-and-epochs.md`** — `Weak.get` and the
  trap-law reconciliation (§10). Recommendation: ratify with this proposal; both are
  spec-only until the native arc builds them. (`claims-and-epochs.md` §8 carries its own
  three decisions — the closure rule itself, C7 wire handles, and where the frame lives.)
