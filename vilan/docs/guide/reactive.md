# Reactive state

`std::reactive` is vilan's state layer: **signals** hold values, **effects**
react to them, **owners** decide when reactions die, and **turns** decide when
changes become visible. The UI layer (`std::ui`), the RPC mirrors, and the
router are all built on it — understand this chapter and the rest of the
framework follows.

```vilan
import std::print;
import std::reactive::{ Signal, Owner, run_with_owner };

fun main() {
	let count = Signal::new(0);
	let owner = Owner::new();
	run_with_owner(owner, || {
		count.effect(|value: i32| print(value));
	});
	count.set(1);
	count.set(2);
}
main();
```

## Signals

A `Signal<T>` is a mutable cell whose readers can subscribe to changes.

```vilan,fragment
Signal::new(value: T): Signal<T>       // a fresh signal
signal.get(): T                        // current value
signal.set(value: T)                   // write + notify subscribers
signal.set_with(transform: |T| T)      // read-modify-write in one step
```

Signals hold **values** (vilan is value-semantic — see the
[memory model](../tour/memory-model.md)): `get` hands you a copy, and the only
way to change what subscribers see is `set`/`set_with`.

```vilan
import std::print;
import std::reactive::Signal;

fun main() {
	let items: Signal<List<str>> = Signal::new([]);
	items.set_with(|list| {
		mut updated = list;
		updated.push("first");
		updated
	});
	print(items.get().len());
}
main();
```

## Derived state: `map`, `combine`, `flatten`

Derived signals recompute when their sources change — build state as a graph,
not as manual copying.

- `signal.map(transform)` — a signal of the transformed value.
- `combine((a, b, …))` — a signal of the **tuple** of several signals'
  values; fires when any of them changes. Takes 2+ signals.
- `nested.flatten()` — on a `Signal<Signal<U>>`: follows the *current* inner
  signal, detaching from a replaced one.

```vilan
import std::print;
import std::reactive::{ Signal, combine };

fun main() {
	let first = Signal::new("Ada");
	let last = Signal::new("Lovelace");
	let full = combine((first, last)).map(|pair: (str, str)| {
		let (a, b) = pair;
		a + " " + b
	});
	print(full.get());
	first.set("Grace");
	print(full.get());
}
main();
```

A named function can stand in for a closure argument (`signal.map(parse)`) —
see [functions & closures](../tour/functions-and-closures.md).

## Reacting: `sub` and `effect`

Two ways to run code on change:

- `signal.sub(observer): Subscription` — **explicit** lifetime: you keep the
  `Subscription` and `dispose()` it yourself.
- `signal.effect(observer)` — **ambient** lifetime: runs the observer now
  with the current value, re-runs on every change, and registers its
  subscription with the nearest enclosing **owner** (below). This is what UI
  code uses; there is nothing to remember to clean up.

`effect` also fires immediately with the current value; `sub` only fires on
subsequent changes.

## Ownership and disposal

Subscriptions must die when the thing that created them becomes garbage. The
ambient-owner model makes that automatic:

- An `Owner` is a bag of disposables. `owner.dispose()` disposes everything
  it collected (and runs `owner.defer(cleanup)` callbacks).
- `run_with_owner(owner, || …)` runs a block with that owner **ambient**:
  every `effect` inside registers into it, through any depth of function
  calls.
- `get_owner()` reads the ambient owner (e.g. to `defer` custom teardown).
- `comp(|| …)` runs a block under a **fresh** owner and returns
  `(result, owner)` — the building block for component roots.

Building reactive state *outside* any owner is a **compile error** (the
context coverage check): every entry point must establish an owner, so no
subscription can leak. In UI code you never do this by hand — `mount_root`,
`bind_each` rows, `when`/`swap` bodies each establish owners at exactly the
points where subtrees can die (see [Building UI](ui.md)).

```vilan
import std::print;
import std::reactive::{ Signal, Owner, run_with_owner };

fun main() {
	let source = Signal::new(0);
	let owner = Owner::new();
	run_with_owner(owner, || {
		source.effect(|value: i32| print(value));
	});
	source.set(1);
	owner.dispose();
	source.set(2); // not printed: the effect died with its owner
}
main();
```

> **Annotate effect parameters.** Today an `effect` closure's unannotated
> parameter can fail to take the signal's payload type when the body
> destructures it (backlog B23) — write `|current: Option<Task>| …`, not
> `|current| …`.

## Turns: when changes become visible

A **turn** batches signal writes: inside a turn, `set` records; when the turn
*settles*, each subscriber runs **once** with final values — ten writes to one
signal coalesce, and a wave of related writes is observed atomically.

You mostly don't manage turns yourself — the framework establishes them at
its boundaries:

- every UI event handler and `mount_root` build runs in a turn
  (`FlushPolicy::AtSuspension` — settles when the handler's synchronous part
  ends);
- every `[service]` RPC handler runs in a turn (`AtEnd` — transactional).

When you do need one explicitly:

```vilan,fragment
turn(policy, || …)       // run a block in a fresh turn
turn_async(async || …)   // a turn HELD across awaits — a true transaction
batch(|| …)              // join the current turn, or create one
flush()                  // drain the ambient turn early
```

Writes from spawned async work that land *after* a turn settled are grouped
per continuation segment and drained in a microtask — you never observe a
half-applied wave.

## Optimistic writes and local-first drafts

Two lifecycles for "change locally, confirm remotely", with opposite failure
behavior:

**`optimistic(signal, value, commit)`** — paint `value` now, run the async
`commit`, then **confirm or roll back**. Right for one-shot actions (a
delete button): on failure the UI snaps back.

**`draft(initial, commit)` → `Draft<T>`** — a local-first cell for *editing*.
Failure **keeps** the local value (rolling back mid-typing would eat the
user's text); the next push retries.

```vilan,fragment
struct Draft<T> {
	local: Signal<T>,          // bind inputs to this
	state: Signal<DraftState>, // Synced | Dirty | Failed(str)
	…
}
draft(initial: T, commit: async |T| Option<str>): Draft<T>
draft.push(value)   // set local + spawn the commit (never waits on the wire)
draft.adopt(remote) // fold in a remote change
```

The commit closure returns `None` for success or `Some(reason)` for failure —
an RPC-calling closure flows straight in. `push` is safe to call
per-keystroke: a generation counter discards superseded completions (fast
typing over a slow wire), and `adopt` follows three rules — an **echo** of
your own push is a no-op, a **clean** local adopts the remote edit, a
**dirty** local wins (last-write-wins; the eventual push overwrites).

```vilan
import std::print;
import std::reactive::{ draft, Draft, DraftState };
import std::option::Option::{ self, Some, None };
import std::shared::Shared;

fun main() {
	let saved: Shared<List<str>> = Shared::new([]);
	let name = draft("seed", |value: str| {
		saved.write().push(value);
		None
	});
	name.push("edit");         // local is "edit" immediately
	print(name.local.get());
	name.adopt("edit");        // the server echoing it back: no-op
	name.adopt("remote-edit"); // a genuine remote change: adopted (local is clean)
	print(name.local.get());
}
main();
```

In the UI, `bind_draft` wires an `<input>` to a draft — see
[Building UI](ui.md). For the full API including `DraftState`, see the
[reactive reference](../std/reactive.md).

## Keyed reconciliation

`reconcile(old_keys, old_items, new_items, key)` computes a minimal update
plan (`RowStep::Keep`/`Refresh`/`Fresh` + removals) for keyed lists. It is the
pure engine under `ui`'s `bind_each`; use it directly only if you're building
your own list-rendering primitive.

## Traps

- `sub` returns a `Subscription` you must dispose (or `owner.take(sub)`);
  prefer `effect` and let the owner handle it.
- Disposal guarantees no *later* deliveries — a subscriber already queued in
  the currently-draining turn may fire once more.
- `map`/`combine` subscriptions are unowned by design (they live as long as
  their sources); derived signals built inside a disposed subtree don't leak
  observers into it.
- An effect's unannotated parameter may need a type annotation (B23, above).
