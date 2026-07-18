# Async

> Normative rules: spec [§7 Execution & async](../spec/execution.md).

If you know async/await in JavaScript, here is the whole model in one
line: vilan keeps the machinery and deletes the keywords. Calling an
async function just gives you the value. You don't write `await`, you
don't mark functions `async`, and you never see `Promise<T>` in a return
type. The compiler figures out which functions suspend and awaits the
calls for you.

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

`fetch_label` sleeps, so it's async. `main` calls it, so `main` is async
too. The return type stays the honest `str`. Asyncness spreads through
the call graph on its own, the way it always wanted to.

## Opting out of waiting: `async` and `await`

The explicit keywords exist for the one thing implicit awaiting can't
express: *not* waiting.

- `async expr` **spawns**: start the work, don't wait for it. It gives
  you a `Task<T>` — a handle to the running work.
- `async { … }` spawns a block.
- `await task` collects a task you spawned earlier.

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

So in JS you mark the async case and waiting is explicit. In vilan you
mark the *concurrent* case and waiting is the default. Fire-and-forget
is just spawning and dropping the task: `let _done = async
save(entry);`. To wait on many at once, `Task::settle_all(tasks)` from
`std::task`.

A task's failure can't crash the program from the outside: if the
spawned work panics, a later `await` receives the panic, and a task
nobody ever awaits reports the error to the console — with the name of
the function that spawned it — and execution continues.

## Async closures

This section matters once you store async callbacks. Until then, skim.

A call through a closure *value* can't be seen by the compiler's
asyncness inference (there's no fixed callee to look at). Two things
close the gap: the type carries the marker — `async |T| U`, written at
any contract position — and unannotated bindings *adopt* asyncness from
the closure they hold. Calls through either are awaited implicitly,
like direct calls.

```vilan,fragment
// 1. An async-friendly callback parameter — sync closures pass fine too
//    (awaiting a plain value just resolves):
fun draft<T: PartialEq>(initial: T, commit: async |T| Option<str>): Draft<T>

// 2. A struct field storing an async callback — reads await when called:
struct Poller {
    tick: async || i32,
}

// 3. A function handing one back — `make()()` and
//    `let go = make(); go()` both await:
fun make(): async || i32
```

The marker is accepted at parameters, `let` annotations, struct fields,
and function return types. An unannotated `let` (or a `mut`, through
every rebind) holding an async closure needs no marker at all — the
binding adopts the closure's asyncness.
[Functions & closures](functions-and-closures.md) covers the same seams
from the closure side.

### Higher-order functions adapt

A plain, value-returning closure **parameter** does something better
than refuse: it *adapts*. Passing an async closure instantiates an
async copy of the function — its calls through the parameter are
awaited — while every sync call site keeps the untouched original.
`map` is one function, not two:

```vilan,norun
import std::print;
import std::time::sleep;

fun fetch_len(url: str): i32 {
	sleep(1);
	url.len()
}

fun main() {
	let urls = ["ab", "cdef"];
	print(urls.map(|url| fetch_len(url)));   // awaited per element: [2, 4]
	print(urls.map(|url| url.len()));        // the plain instance, no awaits
}
```

The contract is **sequential**: each callback settles before the next
begins — a 100-element `map` whose callback takes a second takes a
hundred seconds. When the elements are independent, opt into
concurrency by starting them all first:

```vilan,norun
import std::print;
import std::time::sleep;

fun fetch_len(url: str): i32 {
	sleep(1);
	url.len()
}

fun main() {
	let lens = ["ab", "cdef"]
		.map(|url| async fetch_len(url))   // all in flight (List of tasks)
		.map(|t| await t);                 // settle in order: total ≈ max
	print(lens);
}
```

An adapting function traverses a *snapshot* of its receiver — the list
as of the call — so work interleaved during the awaits can't tear the
iteration. Adaptation follows the closure through plain parameters
(`fun helper(xs, f) { xs.map(f) }` adapts end-to-end), but it cannot
cross a host (`external`) boundary or a trait/generic dispatch, and it
never touches a parameter marked **`sync`**:

```vilan,fragment
// The callback completes inside the reactive graph's synchronous
// protocol — an async closure here is refused, not adapted:
fun map<U>(self, transform: sync |T| U): Signal<U>
```

`Signal::map`, `turn`, `batch`, and the UI render callbacks are `sync`
positions: move async work into `turn_async`, `Draft`, or a spawned
`async` block instead.

The remaining boundaries keep the refusal rule. An async closure
flowing where a plain closure type is declared on a struct **field** or
a function's declared **return type** is a compile error if that
closure returns a value, because the reader would receive a promise
disguised as the value (declare the field or return `async || T`
instead). If it returns `void`, it's allowed anywhere — the call just
becomes fire-and-forget. That's why UI event handlers can await freely
with no ceremony.

## Nurseries: structured spawning

A dropped task keeps running with nobody responsible for it. When every
spawn should be accounted for, run the work in a **nursery** from
`std::task`:

```vilan,norun
import std::print;
import std::time::sleep;
import std::task::nursery;

fun announce(label: str, ms: i32) {
	let _ = async {
		sleep(ms);
		print(label);
	};
}

fun main() {
	let value = nursery(|n| {
		announce("slow", 20);   // a helper's spawn is still inside the extent
		announce("quick", 10);
		print("body");
		7
	});
	print(value);   // body, quick, slow, 7 — nothing outlives the nursery
}
```

The contract:

- **Everything joins.** `nursery(body)` returns only when the body *and
  every task spawned in its dynamic extent* have settled — spawns made
  by the body, by functions it calls, and by the tasks themselves
  (grandchildren). No plumbing: the extent is ambient, like a context.
- **The value passes through.** The nursery's value is the body's
  value. Spawns outside any nursery keep their free-floating behavior.
- **First failure wins, the rest are absorbed.** If the body throws,
  that failure is the nursery's; otherwise the earliest-settled task
  failure is, re-raised from the `nursery` call with the name of the
  function that spawned the task. Either way the other tasks are
  observed and their failures discarded — a losing task can never crash
  the program later.

### Cancellation

The body's handle cancels the whole extent:

```vilan,norun
import std::print;
import std::time::sleep;
import std::task::{ nursery, Task };

fun fetch_from(source: str, ms: i32): str {
	sleep(ms);   // stands in for a real request
	source
}

fun main() {
	let winner = nursery(|n| {
		let a = async fetch_from("mirror-a", 300);
		let b = async fetch_from("mirror-b", 10);
		let first = Task::race([a, b]);   // first settled wins
		n.cancel();                       // abort the loser's IO
		first
	});
	print(winner);   // mirror-b — and mirror-a's sleep was cut short
}
```

`n.cancel()` fires the nursery's host `AbortSignal`. std's IO carries
that signal automatically — a `sleep` or an in-flight `fetch` inside
the extent rejects promptly instead of running out — and those
cancellation rejections are *echoes*, absorbed at the join rather than
treated as failures. The first real failure cancels the same way, so
one task's error stops its siblings' work early. Details:

- Cancellation doesn't preempt: your code runs until its next
  (cancellable) suspension. Pure-compute loops can poll
  `n.is_cancelled()` between chunks.
- Code after `n.cancel()` in the body still runs, and the body's value
  is still returned — cancel kills the *children*, not the body. (If
  the body itself is suspended on IO when cancellation lands, that IO
  rejects and the cancellation becomes the nursery's outcome.)
- Nurseries nest: an outer cancel reaches every inner nursery's IO.
- To hand the signal to a host API std doesn't wrap, read
  `std::task::ambient_signal()`.

Prefer a nursery whenever spawned work has an owner that should wait
for it; keep bare spawns for genuine fire-and-forget.

## Timers

From `std::time`: `sleep_for(duration)` and `sleep(millis)` suspend.
`Duration::millis/seconds/minutes/hours/days` build durations. `now()`
reads the clock. Details in the [time reference](../std/time.md).

## What async does NOT do

- **No promise-colored signatures.** Return types are the plain values.
  A `Task<T>` appears only where you spawned and kept one.
- **No hidden concurrency.** Everything waits, in order, unless you
  spawn. Same single-threaded event loop as JS underneath.
- **No views across a suspension.** A `&`/`&mut` view held across an
  await is rejected. Re-derive after — see
  [the memory model](memory-model.md).

## Traps

- On node, **the process exits when nothing is left to do**: once
  `main` finishes, only live host handles (a running timer, a socket, a
  listening server) keep it alive. A dropped task that is merely
  *pending* — awaiting something that will never wake — is silently
  abandoned at exit, and whether a spawn outlives `main` depends on
  what it holds. Joining spawns in a nursery makes the question moot; a
  long-lived client must keep `main` open by awaiting something that
  ends with the app.
- Don't expect a spawned write to be visible immediately after the
  spawn. Spawned work interleaves with yours per the event loop, like
  any promise.

> **Going deeper.** The reactive layer batches signal writes into
> "turns", and turns interact with suspension: a UI turn settles at the
> handler's first await, and writes after it land in later waves unless
> you use `turn_async`. That's a [reactive guide](../guide/reactive.md)
> topic, not a language rule.
