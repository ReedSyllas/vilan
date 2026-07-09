# Views and invalidating events — rule 4 completed, `await` included (C3 + C2's static half)

Status: **PROPOSED 2026-07-09.** Design settled in discussion; implementation
not started. Every §1 behavior claim was verified against the compiler on
2026-07-09 (probe programs `c3-p*.vl` / `c3-probe.vl`, session scratchpad);
pin each as a test when implementation starts.

## 0. The one-sentence model

A view is live from its declaration to the end of its block; while it is
live, its target must not be **invalidated** — and there are exactly three
kinds of invalidating event:

| | Event | Example | Status today |
|---|---|---|---|
| E1 | Reassignment of the viewed root | `a = []` | **Caught** (shipped rule 4, `check_invalidation`) |
| E2 | A mutating call on the viewed root | `a.remove(i)`, `a.push(x)`, `f(&mut a)` | **Silent** — deferred in the shipped check's own comment |
| E3 | A suspension point | `await tick()` | **Silent** — backlog C3 |

E1 is shipped. This proposal adds E2 and E3 to the *same lexical-liveness
scan*, which is the whole implementation story: one scan, three event kinds.
What remains of C2 afterwards is only the genuinely dynamic remainder (§6).

## 1. Current behavior (the probe programs)

Four user-posed cases plus the await case. Each is a standalone program in
the session scratchpad (`c3-p1.vl` … `c3-p4.vl`, `c3-probe.vl`); re-verify
before implementation and pin each as a test.

```vilan
// P1 — view of an element of an EMPTY list.
mut a = [];
let b = &mut a[0];
```
No machinery consults bounds or emptiness when a subscript view is minted:
`&mut a[0]` lowers to the scalar `(base, key)` pair `[a, 0]` regardless of
`a`'s length. **Observed:** this exact program happens to be a compile error
today — but only because the empty literal's element type never grounds, and
the message is circular ("cannot index List (only a `List` is indexable)").
An empty-at-runtime list of KNOWN element type (see P3) mints the view
silently. **Out of scope here** — this is the *subscript absence* question
(what `a[0]` itself means on a missing element), the same question with or
without a view. Recorded as backlog **I4**, together with the circular
message.

```vilan
// P2 — whole-root reassignment while an element view is live.
mut a = [ 1, 2, 3 ];
let b = &mut a[0];
a = [];
```
**Compile error today** (shipped rule 4):
`cannot reassign 'a' while a view into it is live (rule 4: no invalidating
mutation under a live view).` Liveness is **lexical** — declaration to end
of block, not last-use — so the error fires even if `b` is never read after
the reassignment. That conservatism is deliberate and this proposal keeps it.

```vilan
// P3 / P4 — a mutating call while an element view is live.
mut a = [ 0 ];
let b = &mut a[0];
a.pop();                // P3: method taking &mut self
grow(&mut a);           // P4 also passes the root to a free fn by &mut
b = 99;
print(a[0]);
```
**Silent today, corruption confirmed.** `check_invalidation`'s doc comment
defers it in as many words: *"(Resize / move / drop invalidation, and
index-into-container views, are deferred.)"* Only whole-binding `Assignment`
counts as invalidating. Observed runtime: `b` is the pair `[a, 0]`; after
`pop()` empties the list, `b = 99` **resurrects slot 0** — `a == [99]`, no
error anywhere. (The original discussion used `remove(i)`; `List` has no
`remove` yet — `pop()`/`push()` exhibit the class identically, and E2 covers
whatever removal methods `List` grows.) A bonus finding while probing:
`print(b)` prints the raw `(base, key)` pair (`[ [ 99 ], 0 ]`) instead of
auto-dereffing the scalar view — a transparent-references gap in
argument position, recorded in §5.

```vilan
// The await case (probed 2026-07-09: compiles, prints 99).
async fun mutate_across_await() {
    mut point = Point { x = 1 };
    let view = &mut point;
    await tick();
    view.x = 99;
}
```
**Silent today.** Safe in this exact program only because `point` is
frame-local; see §4 for why it must be rejected anyway.

## 2. E2 — mutating calls are invalidating writes (the static half of C2)

### The rule

While a view rooted at `R` is live, any call that passes `R` — or a place
rooted at `R` — by **`&mut` convention** (the receiver's inferred `borrows`
self, or an explicit `&mut` argument) is an error. Constant vs dynamic index
is irrelevant: the rule never asks *which* element dies, only that the call
*may* move, drop, or reallocate elements.

This is Rust's answer made vilan-shaped: Rust doesn't detect
`remove`-under-borrow dynamically either — `&mut a[0]` exclusively borrows
all of `a`, so *any* `&mut a` use while the element borrow lives is a
compile error. Vilan reaches the same totality through conventions the
analyzer already infers.

### What does and does not invalidate

- **Invalidates:** `a.remove(i)`, `a.push(x)`, `a.clear()`, `a.insert(..)`,
  any user method taking `self` by `&mut`, `free_fn(&mut a)`. `push` is
  included deliberately: it is harmless on the JS backend but reallocates on
  the native backends (F3/F4) — the rule is a language fact, not a backend
  accident.
- **Does not invalidate:** reads and `&self` methods (`a.length()`);
  writes *through the view itself* (`b = 99` writes the element — that is
  the view's purpose); direct writes to a *different field* of a struct
  root (`s.x = 1` while `&mut s.y` lives — field writes change contents,
  not geometry; no `(base, key)` pair is disturbed); calls on unrelated
  containers.
- **Whole-root reassignment stays E1** (shipped, unchanged).

### Diagnostic

Anchored at the call (events anchor; the message names the root), matching
the E1 message's shape:

> `cannot mutate 'a' with '.remove(..)' while a view into it is live (rule 4: no invalidating mutation under a live view).`

## 3. E3 — a view may not live across `await` (C3)

### The rule

1. **Body rule:** inside an async body, an `await` occurring while *any*
   view is live is an error — root-independent (unlike E1/E2, suspension
   invalidates every view: the writer set during a suspension is the whole
   program).
2. **Signature rule:** an `async fun` may not declare `&`/`&mut`
   parameters. The caller's view would be held inside the suspended callee
   across *its* awaits — the same hazard one frame down. Sync callees stay
   free to take views (they cannot suspend), which is what keeps the whole
   analysis local: no call-graph pass, just async bodies and async
   signatures.
3. **Async closures:** an `async { .. }` / async closure may not capture a
   view (binding or parameter) — its body suspends with the capture live.
   The existing escape machinery already rejects view-param captures in
   escape positions; the new scan covers async closure bodies uniformly.

### Why (the three layers, from the discussion)

- **Semantic, real today:** an `await` yields to arbitrary other turns.
  Rule 4's static story works because the analyzer sees every writer
  between a view's creation and its block end; a suspension point makes the
  writer set unknowable. Anything reachable (`Shared`, signals, captured
  state) can be mutated mid-flight: `(base, key)` views get reseated
  (P3/P4's class), object views write into detached copies.
- **Architectural:** every local that survives an `await` becomes a field
  of the continuation object. A view across `await` *is a view stored in a
  struct* — the thing second-class views forbid everywhere else; the async
  frame is just a struct the compiler synthesizes. JS closure scope hides
  this; a state-machine lowering makes it literal.
- **Strategic (F3/F4):** Rust permits references across `await` by paying
  with lifetimes through the generator plus `Pin` (self-referential
  futures). Vilan's bet — second-class views ⇒ no lifetime machinery, no
  pinning, scope-end destruction — survives resumable frames only if views
  never cross suspension points.

### No `Shared` exemption

The backlog's open sub-question, answered **no**. `Shared`'s handle
(captured by value) does pin the cell — memory-safe even natively — but
memory safety was never the only hazard: another turn's `write()` reseats
or removes elements under a held `read()` view, which is exactly P3 through
a different door. Uniformity keeps the rule teachable: *views never cross
`await`; re-acquire after.* The fix is always one line, and re-acquiring is
the semantically honest operation — after a suspension the world may have
changed, and re-reading acknowledges it.

### Diagnostics

Anchored at the `await` (the event), naming the live view(s):

> `cannot hold a view across 'await': 'b' (a view into 'a') is still live here. Re-acquire the view after the await — the awaited turn may change what it points at.`

Signature rule, anchored at the parameter:

> `an async function cannot take '&mut' parameters: the view would be held across its suspension points. Pass a value, or a Shared/handle.`

### Relation to A6

C3 does not block A6 (async turns / optimistic-write → `await` →
reconcile); it is A6's ground rule. A6's reconcile step is built on
*re-reading* state after suspension; C3 turns "state held before the await
is not trustworthy after it" from convention into a compiler-enforced fact.

## 4. Implementation plan

Both events ride `check_invalidation`'s existing scan (post-build; view
origins from `compute_view_origins`, conventions from inferred `borrows`).

- **Phase 1 — E2** (smaller; do first). In `scan_invalidation`, on
  `Expr::Call`: if any argument position (receiver included — the wired
  self argument) passes a place whose `place_root` is a viewed root by
  `Ref`**Mut** convention, record a violation like E1's. Pins per case:
  P3 (`remove(0)`), P4 (dynamic index), `push`/`clear`, a user `&mut self`
  method, `free_fn(&mut a)`, and the guards — `&self` method (no error),
  write through the view (no error), sibling-field write on a struct root
  (no error), unrelated container (no error), view created *after* the
  call (no error; scan order already handles it), nested blocks and loops.
- **Phase 2 — E3.** The same scan learns suspension events (`await`
  expressions — `async_infer` already identifies them) as violations
  against *every* live view; plus the signature rule (async fns reject
  `Ref`/`RefMut` parameter conventions) and the async-closure capture rule.
  Pins: the §1 await probe, view created after the await (no error), await
  in one branch only (error — lexical liveness), `for e in &mut c { await
  .. }` (error; the loop binding is a view — document the restructure:
  collect first or keep the loop synchronous), `Shared` read across await
  (error — the no-exemption decision), async fn with `&mut` parameter,
  async closure capturing a view, and sync functions taking views called
  from async contexts (no error — they cannot suspend).
- **Validation before merging each phase:** the std corpus, examples, and
  LSP suites are the false-positive gate. `std::reactive`/`std::ui` lean on
  `for e in &mut` and `Shared` heavily; if a legitimate std pattern trips
  E2/E3, that pattern — not the rule — gets redesigned, or the finding
  comes back here as a semantics question. Treat any such hit as a
  proposal-level event, not something to special-case in the checker.

## 5. Out of scope, recorded elsewhere

- **Subscript absence semantics** (P1): what `a[0]` — read, write, or view
  — means when the element does not exist. Bounds, not aliasing. Backlog
  **I4**.
- **Scalar views don't auto-deref in argument position** (found by P3's
  probe): `print(b)` for `let b = &mut a[0]` prints the `(base, key)` pair
  itself. Transparent references deref reads and writes; a view passed
  where a VALUE is expected (at least for `any`-typed parameters like
  `print`) leaks the representation. Small, separate fix — C5-adjacent.
- **Field-disjoint borrow splitting** (Rust's simultaneous `&mut s.x` /
  `&mut s.y` refinements): not needed — vilan already permits sibling-field
  *writes* under a field view (§2), and multiple simultaneous views remain
  governed by the existing rules.
- **C2's dynamic remainder** (§6).

## 6. What is left of C2

After E2 lands, the un-catchable-statically residue is writes through
**aliased paths**: two handles to the same `Shared` cell, one writing while
a view through the other is live — plus whatever the C4-era native
destruction semantics add. That is honest runtime-check territory
(generation counters on containers, poisoned views), needs a cost model,
and should be sized only after E2/E3 have been in use — the static rules
may leave the dynamic remainder too rare to justify machinery.
