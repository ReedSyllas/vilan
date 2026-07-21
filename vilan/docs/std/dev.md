# Dev / HMR — reference

`std::dev` is the app-facing surface of **hot module replacement** — the
live-update loop `vilan run --watch` runs for a full-stack app (see
[The dev loop](../guide/dev-loop.md) for the whole picture). Browser-only,
and every hook here is a **no-op outside a hot reload**: importing it costs
nothing in a production build, so you can leave the calls in place.

```vilan,fragment
fun hmr_active(): bool                       // is a hot-reload session live?
fun on_teardown(cleanup: || void)            // run before the next swap
fun stash<type T>(key: str, value: T)        // carry plain data across a swap
fun take<type T>(key: str): Option<T>        // recover it (None outside HMR / on a miss)
```

## `hmr_active`

True only while a `run --watch` browser round has installed its dev runtime;
`false` in a normal `build`. Everything else in the module already guards on
it, so you rarely call it directly — reach for it when you want a whole block
to exist only during development.

```vilan,browser
import std::print;
import std::dev;

fun main() {
	if dev::hmr_active() {
		print("editing live — the dev channel is connected");
	}
}
```

## `on_teardown`

Register a cleanup to run **before the next hot swap re-evaluates the
bundle**. The swap's teardown disposes the UI root and closes the live rpc
socket for you; `on_teardown` is the sanctioned patch for anything the swap
can't see on its own — a raw `set_interval`, a bare spawned task, a handle to
something outside the reactive system. Without it, that stray keeps running
after the swap: harmless (it writes into disposed cells) but wasteful.

```vilan,browser
import std::dev;
import std::shared::Shared;

let connection: Shared<bool> = Shared::new(true);

fun main() {
	// Something the reactive runtime doesn't own — close it before a swap so
	// the re-evaluated bundle starts from a clean slate.
	dev::on_teardown(|| connection.write() = false);
}
```

A plain browser refresh is always the complete reset — seed state lives only
in the page's heap — so `on_teardown` is about tidiness within a session, not
correctness across one.

## `stash` / `take`

The manual carryover channel — Vite's `hot.data`, made type-safe. `stash` a
value under a key before a swap; `take` it back in the re-evaluated bundle.
Module-level bindings carry across a swap automatically (see
[the dev loop](../guide/dev-loop.md)); `stash`/`take` is for the rest — a
value minted inside a function that you want to survive one edit.

```vilan,browser
import std::print;
import std::dev;
import std::option::Option::{ self, Some, None };

fun main() {
	// On the first boot `take` is None; after a hot swap it returns what the
	// previous bundle stashed. `take` is non-destructive — the value stays put
	// for the swap after this one too (Vite's persistent `hot.data`).
	let seen: Option<i32> = dev::take("visits");
	let visits = match seen {
		Some(let n) => n + 1,
		None => 1,
	};
	print(i"loaded {visits} time(s) this session");
	dev::stash("visits", visits);
}
```

**The transfer bound.** `T` must be **transferable-as-value**: plain data the
new bundle can adopt by reference without inheriting old code — scalars, `str`,
`List`, `Option`, `Result`, tuples, and structs/enums built from them. A
closure, a `View`, a resource, or a reactive cell (`Signal`/`Shared`) carries
code or identity the new bundle can't adopt, so stashing one is a **compile
error at the call site** — the type system enforces what Vite leaves to
convention:

```vilan,fragment
let live: Signal<i32> = Signal::new(0);
dev::stash("live", live);
// error: `Signal<i32>` cannot cross a hot swap — a closure, view, resource,
// or reactive cell (`Signal`/`Shared`) carries code or identity the new
// bundle cannot adopt; stash only plain data
```

To carry the value inside a `Signal`, stash its payload — `stash("n",
signal.get())` — and re-seed a fresh cell from `take` on the other side. That
is exactly what the compiler does for you for a *module-level* `Signal`
binding; `stash`/`take` just gives you the same move by hand for the cases it
can't reach.

`take` returns `None` on a first boot, a plain browser refresh, or a key
nothing has stashed. One honest caveat: unlike the *automatic* module-binding
carryover — which fingerprints each binding's type and fresh-inits on a
mismatch — the manual stash is a plain keyed slot. If an edit changes the
type you `take` at a key, the old value comes back in its old shape; the
transfer bound guarantees it is still plain data, but reading it as the new
type is on you (Vite's `hot.data` has the same contract, unchecked). When in
doubt, pick a new key — or refresh, which clears the stash entirely.
