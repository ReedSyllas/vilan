# std::reactive — reference

Signals, effects, ownership, turns, and the higher-level cells. Concepts and
usage patterns: the [reactive guide](../guide/reactive.md).

Import what you use:

```vilan,fragment
import std::reactive::{
	Signal, Source, Subscription, Disposable, combine,
	Owner, owner_scope, get_owner, run_with_owner, comp,
	Turn, FlushPolicy, turn_scope, turn, turn_async, batch, flush,
	optimistic, draft, Draft, DraftState,
	reconcile, ReconcilePlan, RowStep,
};
```

## At a glance

| Item | Kind | One line |
|---|---|---|
| `Signal<T>` | struct | mutable cell with subscribers |
| `Source<T>` | trait | anything readable + subscribable (`get`/`sub`/`effect`) |
| `Subscription` | struct | an explicit subscription; `Disposable` |
| `combine` | fn | tuple-signal over 2+ signals |
| `Owner` | struct | disposal bag; the lifetime unit |
| `run_with_owner`, `comp`, `get_owner`, `owner_scope` | fns/context | establish/read the ambient owner |
| `turn`, `turn_async`, `batch`, `flush`, `FlushPolicy`, `turn_scope` | fns/context | write batching |
| `optimistic` | fn | paint → commit → confirm-or-rollback |
| `draft`, `Draft<T>`, `DraftState` | fn/struct/enum | local-first editing cell |
| `reconcile`, `ReconcilePlan`, `RowStep` | fn/structs | keyed list diffing engine |

## Signal

```vilan,fragment
impl Signal<type T> {
	fun new(value: T): Signal<T>
	fun set(self, value: T)                 // write + notify
	fun set_with(self, transform: |T| T)    // read-modify-write
	fun map<U>(self, transform: |T| U): Signal<U>
}
impl Signal<type T> with Source<T> {
	fun get(self): T
	fun sub(self, observer: |T| void): Subscription
	// from the trait default:
	fun effect(self, observer: |T| void)    // fires now + on change; owner-registered
}
impl Signal<Signal<type U>> {
	fun flatten(self): Signal<U>            // follow the current inner signal
}
```

- `set` notifies through the ambient turn when one exists (writes coalesce);
  outside any turn it notifies immediately.
- `map`'s result is a live derived signal; its internal subscription is
  unowned (lives as long as the source).
- `effect` requires an ambient owner — calling it outside every owner is a
  compile error (context coverage). It fires once immediately.
- `sub` does **not** fire immediately, and its `Subscription` is yours to
  dispose (or hand to `owner.take`).

## combine

```vilan,fragment
fun combine<T: (2..)>(sources: (U in T: Signal<U>)): Signal<T>
```

A signal of the tuple of the sources' current values, firing when any source
changes. Variadic over tuples of signals of mixed element types:

```vilan
import std::print;
import std::reactive::{ Signal, combine };

fun main() {
	let flag = Signal::new(true);
	let count = Signal::new(2);
	let both: Signal<(bool, i32)> = combine((flag, count));
	let (_on, current) = both.get();
	print(current);
}
```

(Bind the tuple before taking elements — chained access on a call result,
`both.get().1`, doesn't type yet; see the gotchas appendix.)

## Subscription, Disposable

```vilan,fragment
trait Disposable { fun dispose(self); }
struct Subscription { … }        // impl Disposable
```

Disposing a subscription guarantees no *later* deliveries; a delivery already
queued in the currently-draining turn may still land once.

## Owner

```vilan,fragment
impl Owner {
	fun new(): Owner
	fun take<T: Disposable>(self, item: T): T   // adopt a disposable; returns it
	fun defer(self, cleanup: || void)           // run cleanup at dispose
}
impl Owner with Disposable {
	fun dispose(self)   // dispose everything collected + run defers
}

let owner_scope: Context<Owner>
fun get_owner(): Owner                                        // read the ambient owner
fun run_with_owner<T>(owner: Owner, body: (|| T) context owner_scope): T
fun comp<T>(body: (|| T) context owner_scope): (T, Owner)     // fresh owner + result
```

`body` parameters marked `context owner_scope` receive the ambient owner
implicitly — your component functions thread ownership without mentioning it.
Establish owners at **disposal boundaries** (places where a subtree can die),
not per object; in UI code the framework's boundaries (`mount_root`,
`bind_each` rows, `when`/`swap` bodies) already do this.

## Turns

```vilan,fragment
enum FlushPolicy { AtEnd, AtSuspension }
let turn_scope: Context<Turn>

fun turn<T>(policy: FlushPolicy, body: (|| T) context turn_scope): T
fun turn_async<T>(body: (async || T) context turn_scope): T   // held across awaits
fun batch<T>(body: (|| T) context turn_scope): T              // join or create
fun flush()                                                   // drain the ambient turn now
```

Inside a turn, signal writes are recorded and each subscriber runs once with
final values when the turn settles. `turn` settles at the end of the body's
synchronous extent; `turn_async` holds every notification until the async
body fully completes (a transaction). Framework boundaries establish turns
for you: UI event handlers and `mount_root` (`AtSuspension`), RPC service
handlers (`AtEnd`). Writes landing after a settle (from spawned work) drain
in per-segment microtasks.

## optimistic

```vilan,fragment
fun optimistic<T, E>(signal: Signal<T>, value: T, commit: async || Result<T, E>): Result<T, E>
```

Paint `value` into `signal` now, await `commit`, then reconcile: the
confirmed value on `Ok`, the previous value **rolled back** on `Err`. Returns
the outcome for error UX. For continuous editing, use `draft` instead —
rollback is wrong mid-typing.

## Draft — local-first cells

```vilan,fragment
enum DraftState {
	Synced,       // local matches the last pushed/adopted value
	Dirty,        // local edits not yet confirmed (in-flight included)
	Failed(str),  // last push errored; local KEPT, not rolled back
}

struct Draft<T> {
	local: Signal<T>,           // bind inputs to this; read like any signal
	state: Signal<DraftState>,  // bind a status label to this
	…                           // internals: synced value, generation counter
}

fun draft<T: PartialEq>(initial: T, commit: async |T| Option<str>): Draft<T>

impl Draft<type T: PartialEq> {
	fun push(self, value: T)    // set local + SPAWN the commit (returns immediately)
	fun adopt(self, remote: T)  // fold in a remote value
}
```

- `commit` returns `None` on success, `Some(reason)` on failure. The
  parameter is `async`-typed so an RPC-calling closure flows in directly; a
  plain synchronous closure works too.
- `push` is per-keystroke-safe: local-first (never waits on the wire), and a
  generation counter ensures only the **newest** push settles `state` —
  a slow older commit landing late is discarded.
- `adopt` rules: value equal to the last synced value (an **echo** of your
  own push) → no-op; **clean** local (no unpushed edits) → adopt into
  `local`; **dirty** local → local wins, the remote value is remembered so
  the eventual push knowingly overwrites (last-write-wins).
- On failure, `state` carries the reason and `local` keeps the user's text;
  the next `push` retries naturally.

UI wiring: `View.bind_draft(draft)` — see the [browser reference](browser.md).

## reconcile — keyed list diffing

```vilan,fragment
enum RowStep {
	Keep(i32),     // reuse old row at index (moved into the new order)
	Refresh(i32),  // same key, changed value: rebuild, dispose old index
	Fresh,         // a new row
}
struct ReconcilePlan {
	steps: List<RowStep>,  // one per NEW item, in the new order
	removed: List<i32>,    // old indices gone entirely
}
fun reconcile<T: PartialEq, K: PartialEq>(
	old_keys: List<K>, old_items: List<T>, items: List<T>, key_of: |T| K,
): ReconcilePlan
```

The pure engine under `ui.bind_each`; duplicate keys claim the first
surviving row once. Reach for it directly only when building a custom
list-rendering primitive.
