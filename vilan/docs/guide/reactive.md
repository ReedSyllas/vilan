# Reactive state

`std::reactive` is vilan's state layer. If you've used signals in Solid or
Preact, you'll be at home immediately. If you're coming from React, think
of a signal as a piece of state that components subscribe to directly —
there is no re-render, no dependency array, no memoization dance. When a
signal changes, exactly the code that watches it runs.

Four ideas make up the layer, and this chapter takes them in order:

- **Signals** hold values.
- **Effects** run code when signals change.
- **Owners** decide when effects die.
- **Turns** decide when changes become visible.

The UI layer, the rpc mirrors, and the router are all built on these, so
this chapter pays for itself quickly.

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
```

## Signals

A `Signal<T>` is a mutable cell whose readers can subscribe to changes.

```vilan,fragment
Signal::new(value: T): Signal<T>       // a fresh signal
signal.get(): T                        // current value
signal.set(value: T)                   // write + notify subscribers
signal.set_with(transform: |T| T)      // read-modify-write in one step
```

Signals hold **values**. vilan copies, so `get` hands you a copy, and the
only way to change what subscribers see is `set` or `set_with`. To update
a list inside a signal, transform it:

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
```

(If you tried `items.get().push("first")`, you'd be mutating a copy. The
[memory model](../tour/memory-model.md) chapter explains why that's a
feature.)

## Derived state: `map`, `combine`, `flatten`

Build state as a graph and let it recompute itself:

- `signal.map(transform)` gives a signal of the transformed value.
- `combine((a, b, …))` gives a signal of the **tuple** of several
  signals' values. It fires when any of them changes. Takes two or more.
- `nested.flatten()` on a `Signal<Signal<U>>` follows whichever inner
  signal is current, and detaches from a replaced one.

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
```

A named function can stand in for the closure (`signal.map(parse)`) —
see [functions & closures](../tour/functions-and-closures.md).

## Reacting: `effect` and `sub`

Two ways to run code on change. **Use `effect` by default.**

- `signal.effect(observer)` runs the observer now with the current
  value, re-runs it on every change, and cleans itself up automatically
  when its surrounding UI (or other owner) goes away. Nothing to
  remember.
- `signal.sub(observer): Subscription` is the manual version. It fires
  only on *later* changes, and you keep the `Subscription` and call
  `dispose()` on it yourself.

One current sharp edge: annotate an effect's parameter when the body
takes it apart. Write `|current: Option<Task>| …`, not `|current| …`.
The compiler sometimes fails to infer it (a tracked bug, B23), and the
error it produces ("cannot access field on type T") isn't obvious.

## Ownership: who cleans up

Every effect is a subscription, and subscriptions must die when the
thing that created them goes away — otherwise a page you navigated off
keeps reacting forever. That's a memory leak in any reactive system.
vilan's answer is **owners**, and the good news is that in normal app
code you never manage them: the UI layer creates owners exactly where
subtrees can die (a mounted root, a list row, a conditional block), and
every `effect` you create automatically registers with the nearest one.

For tests, or when you're building your own machinery:

- `Owner::new()` makes an owner; `owner.dispose()` disposes everything
  registered with it.
- `run_with_owner(owner, || …)` runs a block with that owner ambient.
  Every `effect` inside — however deep in function calls — registers
  into it.
- `get_owner()` reads the ambient owner, e.g. to attach custom cleanup
  with `owner.defer(…)`.

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
```

Creating reactive state *outside* any owner is a compile error. That
sounds strict, but it's the property that makes leaks impossible by
construction, and in practice `mount_root` already gave you an owner
before your first line of UI code ran.

> **Going deeper.** Ownership flows through the `context` mechanism
> ([functions & closures](../tour/functions-and-closures.md)): the
> `owner_scope` context carries the current owner, and closure
> parameters marked `context owner_scope` receive it invisibly. `comp`
> runs a block under a fresh owner and returns `(result, owner)` — it's
> the primitive under `mount_root`.

## Turns: when changes become visible

If an event handler sets five signals, you want watchers to see the
final state once, not five intermediate states. vilan batches writes
into **turns**. Inside a turn, `set` just records. When the turn
settles, each affected watcher runs once with the final values.

You mostly never manage turns, because the framework opens them at its
boundaries: every UI event handler runs in one, every `mount_root` build
runs in one, and every rpc handler on the server runs in one. This is
like React's automatic batching, generalized.

For the rare explicit cases:

```vilan,fragment
turn(policy, || …)       // run a block in a fresh turn
turn_async(async || …)   // a turn HELD across awaits — a true transaction
batch(|| …)              // join the current turn, or create one
flush()                  // drain the ambient turn early
```

> **Going deeper.** A plain `turn` settles when the body's synchronous
> part ends, so an async handler publishes a wave at its first await and
> further waves per continuation. `turn_async` instead holds every
> notification until the whole async body completes — a transaction.
> Writes that land after a turn settled (from spawned work) are grouped
> per continuation segment and drained in a microtask, so you never
> observe half a wave.

## Optimistic writes and local-first drafts

Two ready-made lifecycles for "update the UI now, confirm with the
server after". They differ in what happens on failure, and the
difference is the point:

**`optimistic(signal, value, commit)`** paints the value immediately,
runs your async commit, and on failure **rolls back**. Use it for
one-shot actions like a delete button — if the delete failed, the row
should come back.

**`draft(initial, commit)`** is for *editing*. It keeps the user's text
on failure (rolling back mid-typing would eat their input) and retries
naturally on the next push. Bind an input to a draft and every keystroke
can safely commit through an rpc:

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

The commit closure returns `None` on success or `Some(reason)` on
failure, so an rpc-calling closure drops straight in.

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
```

> **Going deeper.** `push` is per-keystroke safe: a generation counter
> means a slow older commit that lands late is discarded rather than
> clobbering a newer one. `adopt` follows three rules — an **echo** of
> your own push changes nothing, a **clean** local adopts the remote
> edit, and a **dirty** local wins (last-write-wins: the remote value is
> remembered so your eventual push knowingly overwrites it). The
> [reactive reference](../std/reactive.md) states all of it precisely,
> and `bind_draft` in [Building UI](ui.md) is the input-side wiring.

## Keyed reconciliation

`reconcile(old_keys, old_items, new_items, key)` computes a minimal
update plan for keyed lists (keep this row, refresh that one, these are
gone). It's the pure engine underneath `ui`'s `bind_each`. You'd only
call it directly to build your own list-rendering primitive.

## Traps

- `sub` gives you a `Subscription` to dispose manually. Prefer `effect`
  and let the owner handle it.
- Disposal stops *future* deliveries. A watcher already queued in the
  currently-settling turn may fire one final time.
- Annotate effect parameters that get destructured
  (`|current: Option<Task>|`) — the B23 workaround from above.
- Derived signals (`map`/`combine`) live as long as their sources, by
  design. They don't need owners, and they don't leak into disposed
  subtrees.
