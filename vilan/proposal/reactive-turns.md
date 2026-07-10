# Reactive turns — flush is scoped, not global (A6 redesigned)

Status: **SHIPPED 2026-07-09** — `get_safe` (§5.1), the Turn machinery +
server boundary (§5.2–5.3), the `std::ui` boundary, and CONTINUATION
SETTLING all landed the same day; §5.5's optimistic-reconcile follow-on
remains recorded. The `AtSuspension` "async-lowering hook" turned out
unnecessary: for an async extent, `turn`'s own drain fires at the body's
first suspension (the body returns its promise there), and a write landing
AFTER the turn settled schedules one **microtask drain** — each continuation
segment settles as one coalesced wave (per-set settling would have
re-glitched multi-input observers), with no compiler insertion at all. The
policies therefore CONVERGE for async extents in v1 (`FlushPolicy` states
intent and keeps the API stable). **`turn_async` + `optimistic` shipped
same-day**, closing §5.5: `turn_async(body)` is the true held-across-await
transaction — its body is `async`-typed (J2 SHIPPED 2026-07-10: `async || T`
closure types make calls through the value implicitly awaited; the original
spawn-then-flatten workaround is gone), holds every notification (the turn never
reaches `settled` mid-flight, so the continuation microtask never fires
early), and settles once — same-signal writes coalesce to their final value.
`optimistic(signal, value, commit)` is the reconcile lifecycle: paint now,
await the commit, then confirm or roll back, returning the outcome. **A6 is
COMPLETE**; the cadence split for directly-awaiting `turn` bodies remains
the one recorded refinement. Three implementation findings amended the design:

1. **Injected bodies, not captured** (`turn` AND `batch`): a batch body is a
   literal at the call site, created BEFORE the extent exists —
   capture-at-creation would hand it the caller's (usually absent) turn. Both
   enter through `run`, which supplies the turn to the deferred literal;
   `batch`'s join arm re-establishes the CURRENT turn (same queue, outer
   settle).
2. **Drain affinity — the one runtime device.** A notify fired during a
   drain may `set` (a derived recomputing), but notifiers are closures
   created anywhere; compile-time capture cannot hand them the draining
   turn. A `set` with no ambient turn joins the currently draining one
   (a module-level stack, pushed/popped around the synchronous drain loop —
   it can never cross an `await` or interleave extents). This is what keeps
   cascades coalescing (the glitch-free dedup) inside their own settle.
3. **The boundary must sit where user code is called DIRECTLY.** Stored
   handler closures capture at REGISTRATION (nothing), so wrapping an outer
   dispatch in a turn cannot reach them. `[service]`-generated routes wrap
   their bodies in `turn(AtEnd, ..)` — the generated literal contains the
   direct call into the user's handler method, so the turn threads
   compile-time into real handler code. MANUAL `dispatcher.on(|req| ..)`
   handlers self-`batch` (one line, documented). **The `std::ui` boundary
   SHIPPED the same way** (same-day follow-up): the host stores only a plain
   ADAPTER — `View.on` takes a clause-typed handler and registers
   `|| turn(AtSuspension, || handler())`, so each DOM dispatch (and each
   `bind_value` write-back, and `mount_root`'s initial build) runs in its
   own turn with zero user ceremony. Enabled by two B15 extensions shipped
   with it: clauses on `let` annotations (a named injected closure —
   forwards, `run`-body, and direct calls all work), and clause ADOPTION —
   an unannotated closure-literal binding passed into a clause position
   adopts the clause (`let add = || ..; .on("click", add)`, the idiomatic
   pattern both example apps already used).

Supersedes A6's original sketch ("auto-`flush` on the next microtask") — the
microtask hook dissolves into boundary-established turns. Prerequisite
sub-slice: `get_safe` (ambient-owner.md §2.1's recorded tail — shipped with
strict/safe flavors on the threading pass).

## 0. The problem

Today's scheduler (`std/src/reactive.vl`) is one module-level value: a single
`pending: Shared<List<Subscriber>>`. `set` commits its value immediately and,
inside a `batch`, defers only the *notification*; `flush()` drains the global
queue to quiescence.

The failure, found in review before A6 could bake it in:

> An HTTP server has two requests in flight from two clients, each mutating
> signals its own client subscribes to. Request A finishes and flushes —
> and drains **B's** pending notifications too, pushing B's half-done state
> to B's subscribers mid-request.

Three observations sharpen it:

- **It is latent today only by accident.** A handler that never awaits runs
  to completion, so no second request can interleave with its batch. The
  moment handlers suspend (any I/O between writes), interleaving begins.
  A6 exists precisely to let handlers span awaits — the original A6 sketch
  (a global microtask auto-flush) makes the failure *routine*: whichever
  microtask fires first drains every request's queue.
- **Global flush is non-composable even single-client.** Any library calling
  `flush()` mid-operation publishes a stranger's half-settled state.
- **The diagnosis:** the global queue conflates *cadence* (when
  notifications settle: microtask, batch end) with *identity* (whose writes
  settle together). `batch` gets away with it synchronously because a
  synchronous extent has one implicit owner; suspension breaks exactly that
  implication.

## 1. The model: a turn is the ambient transaction

A **`Turn`** owns a pending-notification queue and a flush policy. It is
established for a dynamic extent through `std::context` — the same machinery
as `owner_scope`, with the same property that makes it correct here: hidden
parameters are captured by continuations, so **a request's turn follows its
own awaits while interleaved requests keep theirs** (proven by the A5
substrate probes). This is the compile-time, statically-verified equivalent
of the `AsyncLocalStorage` pattern Node SSR frameworks use against this same
global-singleton bug class.

```vilan
turn_scope: Context<Turn>

// Establish a fresh turn for `body`'s dynamic extent (B15 injected closure).
fun turn<T>(policy: FlushPolicy, body: (|| T) context turn_scope): T

// Drain the AMBIENT turn's queue to quiescence (per-turn wave budget).
fun flush()

// Join the ambient turn if one is established (a no-op wrapper — preserving
// today's nested-batch outermost-flush semantics exactly), else a fresh
// at-end turn. `batch` dissolves into the model instead of being a sibling.
fun batch<T>(body: || T): T
```

- `Signal.set` reads the ambient turn via **`get_safe`**: established →
  enqueue the notification there; not established → notify inline (today's
  non-batch behavior, unchanged — top-level init and timers stay legal).
- `flush()` drains only the ambient turn. Request A can no longer touch
  request B's queue *by construction*.
- Dedup — "one settle fires each subscriber once", the glitch-freeness —
  becomes per-turn (a subscriber may be pending in two turns at once when a
  shared signal changes in both; dedup keys on `(turn, subscriber)`).
- The drain-wave budget (cascade cutoff) moves from the global scheduler to
  the turn.

## 2. Cadence: flush policy rides the turn

The original A6 wanted an auto-flush so users stop hand-writing `batch`.
Turns subsume it without any global hook, because **boundaries** establish
turns:

| Boundary | Establishes | Default policy |
|---|---|---|
| `std::ui` event listeners (`on_click`, …) | a turn per event dispatch | `AtSuspension` |
| `mount_root` / `comp` initial run | a turn per mount | `AtSuspension` |
| `serve_connected` / RPC dispatch | a turn per request/message | `AtEnd` |
| explicit `turn(policy, body)` | opt-in | caller's choice |

Policies:

- **`AtSuspension`** — flush the turn's queue at each `await` boundary and
  at extent end. The UI mental model: optimistic paint before the await,
  settle again after. (This is the "async turns" half of A6's name: each
  synchronous segment between suspensions settles as a unit.)
- **`AtEnd`** — transactional: one settle when the extent completes. The
  server default: a request's subscribers see its writes as one wave.

Writes with no ambient turn notify inline — so code that never opts in
behaves exactly as today, and the "forgot to batch" problem disappears for
UI code because the *boundary* owns the turn, not the user.

## 3. Interactions with the shipped model

- **`owner_scope` is orthogonal** and composes: a closure needing both
  writes `(|| void) context (owner_scope, turn_scope)` — the multi-context
  clause shipped with B15.
- **C3/E3 (view-invalidation) is satisfied by construction**: a `Turn` is an
  ordinary value (`Shared`-backed queue); context threading carries values,
  never views, across awaits.
- **`ReactiveServer`/SSE realtime sync is the live testbed**: its dispatch
  is exactly the boundary that must establish a per-session turn, and the
  P6 realtime example is the regression program for the two-clients
  scenario.
- **Eager value commit is unchanged** — see the honest limit in §4.

## 4. The honest limit: notification isolation, not value isolation

`set` commits the value immediately; turns isolate the *notification waves*.
For the motivating scenario — each client subscribed to its own signals —
that is a complete fix: B's subscribers are simply not in A's queue. But two
turns writing the **same shared** signal still interleave at the value
level: a subscriber notified by A's flush reads whatever B has committed so
far. That is inherent to shared mutable state under eager commit.
Alternatives recorded and *not* taken here: per-turn value staging
(copy-on-write signal values with commit-on-flush) is a real cost and the
wrong default for signals that are *meant* to be shared; the
optimistic-write → `await` → reconcile lifecycle (A6's second half) is the
application-level answer and builds ON turns as a separate slice.

## 5. Implementation plan

1. **`get_safe`** (the A5 tail, ambient-owner.md §2.1's sketch): the
   possibly-established context read. The hidden parameter for
   `get_safe`-reachable regions carries `Option<T>`; strict-`get` regions
   keep the bare flavor and the existing coverage fence; covered→safe
   boundaries `Some`-wrap; safe-only roots synthesize `None`. Pins for the
   flavor split, the boundary wrap, and the fence staying intact for
   strict reads.
2. **`Turn` + `turn_scope` + policies in `std::reactive`**: per-turn
   queue/dedup/budget; `set` routes via `get_safe`; `flush` drains the
   ambient turn; `batch` rewritten as join-or-create (its existing corpus
   behavior must hold byte-for-byte at the OUTPUT level — the goldens will
   change with the std source and are regenerated only after run
   verification, per the standing discipline).
3. **Boundaries**: `std::ui` event listeners and `mount_root` wrap in
   `AtSuspension` turns; `serve_connected`/RPC dispatch wraps in `AtEnd`.
4. **The isolation regression**: a corpus program with two interleaving
   async tasks, each writing its own signal and flushing — asserting each
   subscriber fires only on its own turn's settle (the two-requests
   scenario, distilled); plus the `AtSuspension` optimistic-paint shape.
5. **A6's remaining half** (optimistic-write → reconcile) stays a recorded
   follow-on riding turns.

## 6. Out of scope

- Per-turn value staging (§4 — recorded, not taken).
- Cross-turn ordering guarantees for shared signals (last-flush-wins is
  accepted and documented).
- Scheduler fairness beyond the per-turn wave budget.
