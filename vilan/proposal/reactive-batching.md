# Reactive batching — deferred notification & the `batch` turn

**Status:** **proposal** (2026-07-01, not implemented). A batching layer for `std::reactive`:
`Signal::set` keeps committing its value immediately but **defers subscriber notification** to a
flush boundary, and a new `batch(body)` groups a set of writes so their observers fire **once**,
glitch-free. The motivating consumer is the transport/RPC turn (`proposal/transport-rpc.md`): the
wire flush boundary should *be* the reactive flush boundary. Informed by Solid 2.0's reactivity
model, adapted to Vilan's **explicit** tracking — see [What we take from Solid 2.0](#what-we-take-from-solid-20-and-what-we-dont).

## Motivation

Today `Signal::set` is eager and synchronous — it writes the value, then notifies every subscriber
inline:

```vilan
fun set(self, value: T) {
    self.value.write() = value;
    for subscriber in self.subscribers.read() {
        (subscriber.notify)();
    }
}
```

Two problems fall out of that:

1. **It glitches on diamonds.** If `c = combine((a, b))` and a turn writes both `a` and `b`, each
   `set` propagates independently, so `c`'s observer fires **twice** — once with an intermediate
   `(a', b_old)` state. A batched system fires it once, with `(a', b')`.
2. **There is no flush boundary to hang the wire off.** The transport/RPC design
   (`transport-rpc.md`) needs a *turn*: an RPC handler mutates several server signals, the reply is
   produced, and **all** of it — the resulting `Update` frames plus the reply — coalesces into one
   wire flush. With eager `set`, every mutation sends its `Update` immediately, mid-handler, before
   the reply exists. The turn requires notifications to *wait* for a boundary, then fire together.

Both are the same missing primitive: **a batch boundary that coalesces notifications.**

## What we take from Solid 2.0 (and what we don't)

Solid 2.0 makes batching automatic — setters queue and reads return the last committed value until
the microtask flush; `batch()` is removed, `flush()` forces a synchronous settle; effects split
into a *compute* half (tracked reads) and an *apply* half (untracked side effects) so **all**
computes run before **any** applies, giving a glitch-free "clear dependency picture."

Most of that machinery exists to discipline Solid's **automatic** dependency tracking (reads inside
a tracking scope auto-subscribe). **Vilan tracks explicitly** — you pass the closure to `map`/`sub`,
and `map`'s dependency is structural — so we can take the batching win without importing the rest:

| Solid 2.0                                                                 | Here                                                                                                                                                                               |
| ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Batch writes; `batch()` removed; auto-flush at microtask                  | **Adopt** the batching, but as an **explicit** `batch(body)` scope — synchronous by default, no ambient microtask (see [Divergence](#divergence-defer-notification-not-the-value)) |
| `flush()` forces a synchronous settle                                     | Not needed as a separate primitive — a write outside a `batch` already settles synchronously                                                                                       |
| Reads return stale value until flush                                      | **Reject** — root `set` commits its value immediately; only *notification* defers                                                                                                  |
| Compute→apply split (all computes before any applies) for glitch-freeness | **Skip the split** — glitch-freeness comes from dedup-on-enqueue + immediate values (see below), not from a two-phase API                                                          |
| `untrack`, tracking-scope rules, write-in-scope throws                    | **N/A** — explicit/structural tracking has no ambient scope to police                                                                                                              |

The through-line: Solid pays for glitch-freeness by deferring *values* and splitting effects; we get
it more cheaply because our reads are explicit and our values commit eagerly.

## Semantics

- **`set` commits the value immediately, defers the notification.** `self.value` is written at once,
  so `s.set(5); s.get()` is still `5` — no "stale until flush" surprise. What defers is the
  subscriber *notify*: `set` enqueues `self`'s subscribers into a module-level scheduler.
- **Outside a `batch`, the enqueue drains at once.** A lone top-level `set` runs its subscribers
  synchronously, exactly as today — so existing single-write code is behavior-identical.
- **Inside a `batch(body)`, enqueues accumulate and drain once** when the outermost batch returns.
  Nested batches fold into the outermost boundary.
- **Glitch-freeness by dedup-on-enqueue.** The pending queue is a *set* keyed by subscriber id: a
  derived fed by two inputs changed in one batch is pending once, so it recomputes and notifies
  once, after both inputs are committed. Because values are already committed when the derived runs,
  it reads the final `(a', b')` — no intermediate state, without a compute/apply split.
- **`sub`'s immediate fire is unchanged.** Subscribing still runs the observer once with the current
  value (initialization, not a change) — independent of the queue.

### Divergence: defer notification, not the value

This is the one deliberate departure from Solid, and the reason it fits our model. Solid defers the
committed value (so a diamond's inputs are consistent when the graph recomputes) and pays for it with
`flush()`. We don't need to: our observers each watch **one** source (`sub`), and multi-source
observation goes through structural `combine`, which the dedup already collapses to a single notify.
So committing values eagerly can't produce a glitch a reader would see — the only multi-input reader
is the deduped derived. Keeping `get` immediate preserves imperative intuition and keeps the corpus's
lone-`set` behavior identical.

The one edge it leaves: **reading a *derived* value mid-batch is stale.** A `map`/`combine` result
recomputes during the drain (it propagates by notify), so inside a batch, before the boundary, it
still reflects the pre-batch inputs. This is consistent (derived = roots as of the last drain) and
matches Solid's "settle, then read." A future *lazy memo* (pull-based derived that recomputes on
`get`) would make mid-batch derived reads fresh too, and add autodisposal — noted as future, not
required here.

## API

New in `std::reactive`, alongside the existing `next_subscriber_id` counter:

```vilan
// The reactive scheduler: subscribers pending notification, and the current
// batch depth. Module-level, one per program (like `next_subscriber_id`).
struct Scheduler {
    pending: Shared<List<Subscriber>>,
    depth: Shared<i32>,
}

let scheduler: Scheduler = Scheduler {
    pending = Shared::new([]),
    depth = Shared::new(0),
};

// Enqueue a signal's subscribers, deduped by id so a subscriber fed by several
// inputs in one batch fires once. Outside a batch (depth 0) this drains at once,
// so a lone `set` is synchronous — unchanged from today.
fun enqueue(subscribers: List<Subscriber>) {
    for subscriber in subscribers {
        mut seen = false;
        for queued in scheduler.pending.read() {
            if queued.id == subscriber.id {
                seen = true;
            }
        }
        if !seen {
            scheduler.pending.write().push(subscriber);
        }
    }
    if scheduler.depth.read() == 0 {
        drain();
    }
}

// Run every pending notify until quiescent. A notify may enqueue more (a derived
// propagating), which this same loop drains — so a cascade settles in one drain.
fun drain() {
    for !scheduler.pending.read().is_empty() {
        let wave = scheduler.pending.read();
        scheduler.pending.write() = [];
        for subscriber in wave {
            (subscriber.notify)();
        }
    }
}

// The turn: run `body` with notifications deferred, then drain once. Groups a set
// of writes so their observers see them settled, together. Nested batches fold
// into the outermost. Returns `body`'s value.
fun batch<T>(body: || T): T {
    scheduler.depth.write() = scheduler.depth.read() + 1;
    let result = body();
    scheduler.depth.write() = scheduler.depth.read() - 1;
    if scheduler.depth.read() == 0 {
        drain();
    }
    result
}
```

`Signal::set` shrinks to a value write plus an enqueue; `set_with`/`map`/`combine` are unchanged and
inherit batching for free (their internal `set`s go through the scheduler):

```vilan
fun set(self, value: T) {
    self.value.write() = value;
    enqueue(self.subscribers.read());
}
```

## The wire turn (why this exists — `transport-rpc.md`)

The RPC connection wraps inbound-frame handling in a `batch`, so every server-side `set` during the
handler coalesces; when the batch drains, the reactive forwarding closures fire once and `send`
their `Update` frames into the transport buffer, where the reply already sits — then one wire flush:

```vilan
fun on_inbound(self, frame: str) {
    batch(|| {
        match self.protocol.receive(frame) {
            Some(let reply) => self.transport.send(reply),
            None => {},
        }
    });
    self.transport.flush();   // one coalesced write (a WebSocket transport batches; in-process no-ops)
}
```

The reactive flush boundary *is* the wire flush boundary — the unification the transport design was
reaching for. `flush()` stays a transport-trait method (no-op default) as decided there; `batch`
here is what drives it.

## Future: async turns, actions & optimistic updates (#2)

The `batch` above is **synchronous** — `body` runs to completion, then drains. An RPC handler that
`await`s spans ticks, so its turn can't be one synchronous batch: you get an optimistic drain, then
an await, then a reconcile drain. That lifecycle — optimistic write → `await` server → `refresh` the
source of truth → reconcile — is Solid 2.0's *actions / `createOptimistic` / `refresh`* story, and it
maps cleanly onto our `async`/`await` and the client `RemoteSource`. **Deferred as a future
possibility**, to fold into `transport-rpc.md` once this synchronous core lands; it also touches the
no-view-across-await rule (the memory-management proposal) and is its own semantics question (a batch
that spans an await).

## How it lowers

Pure `std` — a module-level `Scheduler` (two `Shared` cells), plus the `enqueue`/`drain`/`batch`
functions and the two-line `set`. No compiler change. `batch<T>(body: || T): T` is an ordinary
generic over a zero-arg closure (the same shape `Owner::take` already relies on).

## Migration & corpus impact

- **Lone-`set` behavior is identical**, so existing programs' output is unchanged — but the
  generated `reactive.js` (and any golden importing `std::reactive`) **will** differ, because the
  reactive runtime itself changed. Regenerate with a fresh debug binary (a stale one silently writes
  wrong goldens) and confirm **runtime output** is byte-identical for the existing corpus before
  accepting the new goldens.
- **`combine`** currently subscribes each input eagerly; under batching its internal `set` on the
  derived defers like any other — no change needed, but its glitch behavior *improves* (the whole
  point). Its subscription-ownership open question stays with `reactive-ownership.md`.
- Nothing calls `batch` today; adopting it at framework boundaries (event dispatch, the RPC turn) is
  additive.

## Test plan

- **Glitch-free diamond:** `c = combine((a, b))`, observer on `c`; `batch(|| { a.set; b.set })` fires
  the observer **once** with both new values; the same two writes *without* `batch` fire twice (pins
  the opt-in boundary).
- **Immediate value:** inside a `batch`, `s.set(5); s.get() == 5` (root value is eager); a derived
  read mid-batch reflects pre-batch inputs and settles after (pins the documented divergence).
- **Cascade in one drain:** a chain `a → map → map → sub` settles in a single drain; the observer
  fires once.
- **Lone set unchanged:** a top-level `set` outside any `batch` notifies synchronously (regression
  against today).
- **Nested batches** fold: an inner `batch` inside an outer one drains only at the outer boundary.
- **Wire turn:** an RPC handler that sets N subscribed signals produces one coalesced flush (in
  `transport-rpc`'s example harness).

## Open questions

- **Naming.** `batch(body)` (we defer + drain) vs. Solid's `flush` (it forces a drain). `batch` reads
  right for a synchronous-by-default model — the scope's job is to *group*, not to *force*. Confirm.
- **Dispose during drain.** A subscriber already pending when its `Subscription` is disposed mid-drain
  still fires once this wave (it's popped from the queue, not the signal list). Acceptable, or should
  `drain` re-check liveness? (Cheap liveness check ≈ the leak-fix concern in `reactive-ownership.md`.)
- **Re-entrant feedback.** An observer that writes a signal it (transitively) observes loops the drain
  — the same infinite loop it would cause synchronously today. Add an iteration cap / dev diagnostic,
  or leave as a user bug?
- **Dedup cost.** The pending set is scanned linearly on enqueue (O(n²) worst case). Fine at current
  scale; a keyed set is a later optimization.
- **Ambient microtask flush** (Solid's default) as a *future* ergonomic layer for UI event handlers —
  deferred for the same reason `reactive-ownership.md` deferred the ambient owner: no magic until it's
  proven against `async` and indirection.
</content>
</invoke>
