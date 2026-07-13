# Glossary

One line per term, with a link to where it's actually taught. Terms are
alphabetical. If you meet a word in the docs that isn't here, that's a
bug in the docs — please add it.

**adopt** — folding a remote value into a [draft](#draft) without
re-sending it. Echoes are ignored, clean fields update, dirty fields win.
[Reactive state](../guide/reactive.md).

**arena** — a container that owns many values and hands out
[handles](#handle) to them. The tool for graphs and cycles.
[The memory model](../tour/memory-model.md).

**binding** — a name introduced by `let` (immutable) or `mut` (mutable).
[Values and types](../tour/values-and-types.md).

**bound** — a requirement on a generic parameter, like `T: PartialEq`:
"any T that can be compared". [Data and traits](../tour/data-and-traits.md).

**boundary** (disposal boundary) — a place where a UI subtree can die: a
mounted root, a list row, a `when`/`swap` body. Each boundary has an
[owner](#owner). [Building UI](../guide/ui.md).

**codec** — the wire format both ends of a connection agree on:
`json_codec()` (readable) or `binary_codec()` (compact).
[Services & RPC](../guide/services.md).

**context** (ambient value) — a value carried invisibly to the code that
needs it, like the current owner or turn. Established with `run`, read
with `get`. [Functions & closures](../tour/functions-and-closures.md).

**contract check** — at connect time, client and server compare a hash of
the service's shape. A stale client fails cleanly instead of corrupting
calls. [Services & RPC](../guide/services.md).

**copy** — what every assignment, argument pass, and field store does to
a value. The receiver gets its own; the original is untouched.
[The memory model](../tour/memory-model.md).

**derive** — an attribute like `[derive(PartialEq, Debug)]` that
generates a trait implementation from a type's shape.
[Data and traits](../tour/data-and-traits.md).

**dirty** — a [draft](#draft) whose local value has edits the server
hasn't confirmed yet. Dirty fields ignore adoption; the user's text wins.
[Reactive state](../guide/reactive.md).

**draft** — a local-first cell for editing server state: typing updates
locally at once, commits in the background, and keeps your text on
failure. [Reactive state](../guide/reactive.md).

**echo** — your own change arriving back through a [mirror](#mirror). A
draft recognizes it and does nothing, so your caret never jumps.
[Reactive state](../guide/reactive.md).

**effect** — code that runs now and again on every change of a signal,
cleaned up automatically by its [owner](#owner).
[Reactive state](../guide/reactive.md).

**entrypoint** — `fun main` in the entry module. It runs automatically;
on node the process exits when it finishes. [Async](../tour/async.md).

**extern** — a declaration binding a host (JavaScript) function, object,
or property so vilan code can call it. [Platforms](../tour/platforms.md).

**frame** — one encoded message on the wire. You only meet it when
building custom transports. [rpc reference](../std/rpc.md).

**handle** — a small copyable id into an [arena](#arena). Storable
anywhere a value is, which [views](#view) are not.
[The memory model](../tour/memory-model.md).

**lang item** — a std declaration the language itself depends on, like
`Option` for `?.` or `Add` for `+`. [Spec appendix](../spec/appendix.md).

**layer** — the platform-specific part of the standard library. Base is
everywhere; the browser layer is browser-only; the process layer is
server-only. [Platforms](../tour/platforms.md).

**local-first** — updating local state immediately and syncing in the
background, instead of waiting on the network. What [drafts](#draft)
implement. [Reactive state](../guide/reactive.md).

**mirror** — an `[expose]`d server signal that every connected client
receives a live copy of. The server writes; every client updates.
[Services & RPC](../guide/services.md).

**monomorphization** — how generics compile: each concrete use gets its
own specialized code, so generic dispatch has no runtime cost.
[Data and traits](../tour/data-and-traits.md).

**owner** — the object that collects subscriptions and disposes them
when its subtree dies. Created by the framework at
[boundaries](#boundary); you rarely touch one directly.
[Reactive state](../guide/reactive.md).

**panic** — aborting the program with a message, for states that should
be impossible. Expected failures are `Result`s instead.
[Control flow](../tour/control-flow.md).

**pattern** — the shape on the left of a `match` arm: a variant to
match, payloads to bind (`Some(let x)`), a literal, or `_`.
[Control flow](../tour/control-flow.md).

**platform** — what a package builds for: node, deno, bun, or the
browser. Decides which std [layers](#layer) are importable.
[Platforms](../tour/platforms.md).

**prelude** — the few names available without imports: the primitive
types, `List`, `void`. Everything else is imported explicitly.
[Spec §4](../spec/names.md).

**safe integer** — an integer JavaScript's 64-bit floats represent
exactly: anything within ±2^53. vilan's `i53`/`u53` are named for this
window. [Values and types](../tour/values-and-types.md).

**service** — a server struct whose `[rpc]` methods clients call and
whose `[expose]`d signals clients [mirror](#mirror).
[Services & RPC](../guide/services.md).

**signal** — a value cell that code can subscribe to. The unit of
reactive state. [Reactive state](../guide/reactive.md).

**spawn** — starting async work without waiting for it: `async expr`.
Gives you a `Promise<T>`. [Async](../tour/async.md).

**subscription** — one live "call me on change" registration on a
signal. Effects manage theirs through [owners](#owner); manual `sub`
hands you the object to dispose. [Reactive state](../guide/reactive.md).

**suspension** — a point where a function pauses and other code runs: a
call to something async, or an explicit `await`. [Views](#view) may not
be held across one. [Async](../tour/async.md).

**trait** — a named capability a type can implement, used as a bound on
generics. Like an interface, but explicit and compile-time only.
[Data and traits](../tour/data-and-traits.md).

**transport** — the thing that carries rpc calls: the reconnecting
WebSocket in production, http or in-process variants for special cases.
[rpc reference](../std/rpc.md).

**turn** — a batch of signal writes that becomes visible at once.
Event handlers and rpc handlers each run in one automatically.
[Reactive state](../guide/reactive.md).

**value semantics** — the rule that data is copied, not shared, unless
you use an explicit sharing tool.
[The memory model](../tour/memory-model.md).

**variant** — one case of an enum, possibly carrying data:
`Shape::Circle(2.0)`. [Data and traits](../tour/data-and-traits.md).

**view** — a short-lived borrow of a place (`&x`, `&mut x`) that aliases
instead of copying. Can't be stored, returned into long-lived state, or
held across a [suspension](#suspension).
[The memory model](../tour/memory-model.md).

**wave** — one settling of a [turn](#turn): every affected watcher runs
once with the final values. [Reactive state](../guide/reactive.md).

**Wire** — the "can travel over the network" capability: scalars, lists
and options of Wire types, and anything with `[derive(Wire)]`.
[Services & RPC](../guide/services.md).
