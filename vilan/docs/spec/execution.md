# Spec §7 — Execution & async

## 7.1 Program start and termination

A program's entry module executes its top-level statements in order. A
function named `main` at the entry module's top level is the
**entrypoint**: it is invoked automatically after the top-level
statements. (An explicit top-level `main();` call is redundant — the
program still runs `main` once.)

On process platforms (node/deno/bun), the process **exits when `main`
completes** — including when async work it spawned is still pending. A
long-lived program must keep `main` open (a listening server does so
inherently; a socket-holding client must await something that ends with
the app). On the browser platform, `main`'s completion leaves installed
handlers and subscriptions live.

`panic(message)` aborts execution with the message; it types as `any`
(§5.1). Failed `assert`s panic.

## 7.2 Evaluation order

Within an expression, evaluation is **left-to-right**: operands before
operators apply, the callee before its arguments, arguments in source
order, the receiver before method arguments. A compound assignment
evaluates its target place once. Short-circuit: `&&` and `||` evaluate
the right operand only when needed. `if`/`match` evaluate exactly the
taken branch; a `match` evaluates its subject once, then tests legs top
to bottom (first matching leg wins; its guard is evaluated only when the
pattern matches).

## 7.3 The async model

vilan is **await-by-default**. Asyncness is a property of *functions*,
inferred, and never written in return types:

1. A function is **async** iff its body contains a suspension point: a
   call to an async function (including async externs), an explicit
   `await`, or a call through an `async`-typed closure value.
2. A **direct call** to an async function is implicitly awaited: the
   caller receives the plain declared value, and the caller becomes
   async (asyncness propagates through the call graph).
3. A call **through a closure value** is never inferred async by itself
   — there is no static callee. The closure *type* carries the marker
   instead (§7.4).

The explicit forms:

- `async expr` — **spawn**: evaluate `expr`'s suspending computation
  concurrently; the spawn expression itself does not suspend and yields
  `Promise<T>` where `T` is `expr`'s type. `async { … }` spawns a block.
- `await expr` — suspend until the `Promise<T>` operand resolves; yields
  `T`.

A spawned computation runs to its first suspension synchronously, then
interleaves with its spawner per the host event loop. Dropping a promise
abandons nothing — the computation still runs; only its result is
discarded.

```vilan
import std::print;
import std::time::{ sleep_for, Duration };

fun step(label: str): str {
	sleep_for(Duration::millis(1));
	label
}

fun main() {
	let pending = async step("b");   // spawned
	print(step("a"));                // awaited inline — prints first
	print(await pending);
}
```

## 7.4 Async closure types

`async |T| U` marks a closure **value** whose calls are implicitly
awaited (and are suspension points in the caller). The marker is legal at
two seams only: **parameter types** and **`let` annotations**. It does
not exist on struct fields or return types; the standard pattern stores
the closure plain and re-marks it at a `let`:

```vilan,fragment
let commit: async |T| Option<str> = self.commit;   // re-marked
let outcome = commit(value);                        // awaited
```

Assigning between the plain and async-marked types is permitted in both
directions at those seams; the marker governs only how calls through the
*binding* behave. A synchronous closure flowing into an async-typed
position is fine (awaiting a non-promise yields it unchanged).

**The divergence rule.** An async closure (a literal whose body suspends,
or an async-typed value) flowing into a **plain** closure parameter is:

- an **error** when the parameter's return type is non-void — the callee
  would receive a live promise typed as `T`;
- **legal** when the parameter returns `void` — *spawn semantics*: the
  call fires the closure and nobody awaits it. This is what allows UI
  event handlers and other void callbacks to suspend freely.

*Implementation note: divergence checking currently tracks
literal-or-binding-initialized arguments; flow through further
indirection is tracked future work, as are async markers on fields and
returns and asyncness-polymorphic higher-order functions.*

## 7.5 Suspension and state

At every suspension point the enclosing function's live state is
captured and restored on resumption — with one carve-out: **views may
not be live across a suspension** (§6.6). Ambient context values (§8,
Phase B) are captured at closure creation and are therefore stable
across suspensions by construction.

Concurrency is cooperative and single-threaded per program: between a
suspension and its resumption, other computations (event handlers,
other spawned work) may run and mutate shared cells; within one
synchronous extent (no suspension), execution is atomic. The reactive
turn machinery (`std::reactive`) is a library discipline layered on
these primitives, not part of the language.

## 7.6 Emission guarantees (observable behavior)

A conforming implementation targeting JavaScript guarantees: the
entrypoint contract of §7.1; `print` writing one line per call to the
host console; panics rejecting/aborting with the given message;
left-to-right evaluation per §7.2; integer arithmetic with the declared
widths (including truncating integer division and two's-complement
`as_*` folds); and `i64` exactness over the wire within ±2⁵³. Everything
else about the emitted code — names, formatting, module layout, the
`[build]` knobs — is implementation-defined.
