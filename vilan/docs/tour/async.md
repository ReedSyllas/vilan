# Async

vilan's model is **await-by-default**: calling an async function just gives
you the value — no keyword, no promise type in your signature. Asyncness is
*inferred* (a function that awaits anything is async, and so are its
callers), and you reach for the explicit forms only to opt *out* of waiting.

```vilan
import std::print;
import std::time::{ sleep_for, Duration };

fun fetch_label(): str {
	sleep_for(Duration::millis(1));   // suspends — so fetch_label is async
	"ready"
}

fun main() {
	print(fetch_label());   // implicitly awaited; main becomes async too
}
```

The declared return type stays the plain value (`str`, not a promise) —
inference carries the asyncness, signatures carry the meaning.

## The explicit forms: `async` and `await`

- `async expr` — **spawn**: start the work, don't wait. Types as
  `Promise<T>`.
- `async { … }` — spawn a block.
- `await promise` — unwrap a promise you're holding.

```vilan
import std::print;
import std::time::{ sleep_for, Duration };

fun step(label: str): str {
	sleep_for(Duration::millis(1));
	label
}

fun main() {
	let pending = async step("concurrent");   // running; we haven't waited
	print(step("first"));                     // awaited inline
	print(await pending);                     // now collect the spawned one
}
```

Fire-and-forget is spawn with the promise dropped: `let _done = async
save(entry);`. `Promise::all(promises)` (in `std::promise`) awaits a list.

## Async closures

Calls through a **closure value** aren't statically known, so asyncness
can't infect through them — instead the closure *type* carries the marker:
`async |T| U`. Calls through an async-typed closure value are implicitly
awaited like direct calls.

The marker lives at parameters and `let` bindings only (not struct fields or
return types yet), giving two working patterns:

```vilan,fragment
// 1. An async-friendly callback parameter — sync closures pass fine too
//    (awaiting a plain value just resolves):
fun draft<T: PartialEq>(initial: T, commit: async |T| Option<str>): Draft<T>

// 2. Stored plain, re-marked at a let when it's time to call:
let hook: async || void = self.stored_hook;
hook();   // awaited
```

An async closure flowing into a **plain** closure parameter is allowed only
when the parameter returns `void` — that's **spawn semantics** (the call
fires and nobody waits), and it's why UI event handlers can await freely.
With a non-void return it's a compile error: the caller would receive a
promise disguised as a `T`.

## Timers

`std::time`: `sleep_for(duration)` / `sleep(millis)` suspend;
`Duration::millis/seconds/minutes/hours/days` construct durations;
`now(): Instant` reads the clock. See the time reference *(Phase 2)*.

## What async does NOT do

- **No callback coloring in signatures** — you never write `Promise<T>` as
  a return type; promises appear only where you spawned and kept one.
- **No implicit concurrency** — everything is awaited in order unless you
  spawn. One suspension at a time per call chain, same as JS.
- **No view across a suspension** — a `&`/`&mut` view held across an await
  is rejected; re-derive after (see [the memory model](memory-model.md)).

## Traps

- On node, a **completed `main` ends the process** — a long-lived client
  (holding a socket, waiting for events) must keep `main` open
  (`sleep_for` a long duration, or await something that ends with the app).
- Turns interact with suspension: a UI turn settles at the handler's first
  suspension (`AtSuspension`) — writes before and after an await land in
  separate waves unless you use `turn_async` (see the
  [reactive guide](../guide/reactive.md)).
- Spawned work's signal writes drain per continuation segment — coalesced,
  but after the originating turn. Don't expect a spawned write to be
  visible synchronously after the spawn.
