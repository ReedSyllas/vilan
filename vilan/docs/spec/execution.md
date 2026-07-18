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

## 7.2a Integer overflow

Arithmetic whose mathematical result exceeds the operand type's range is
**undefined behavior**: a conforming program does not overflow, and an
implementation may produce any value for one that does (the JS backend
yields f64 artifacts — precision loss for the wide types, out-of-range
magnitudes for the narrow ones — without trapping). Literals are
range-checked at compile time (§2.3); runtime operations are not. An
opt-in checked family (`add_safe`, …) is recorded future work; `BigInt`
is the answer where the range itself is the problem.

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
  `Task<T>` where `T` is `expr`'s type (the task handle is opaque, and
  copying it refers to the same task). `async { … }` spawns a block.
- `await expr` — suspend until the `Task<T>` (or raw host `Promise<T>`)
  operand settles; yields `T`.

A spawned computation runs to its first suspension synchronously, then
interleaves with its spawner per the host event loop. Dropping a task
abandons nothing — the computation still runs; only its result is
discarded. A task's failure is **absorbed at the spawn**: it is
delivered to whichever `await` observes the task, and a failed task
that is never observed is reported to the host console with its spawn
origin — it does not terminate the program.

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

## 7.4 Async closure types, adoption, and adaptation

`async |T| U` marks a closure **value** whose calls are implicitly
awaited (suspension points in the caller). The marker is legal on
**parameter types**, **`let` annotations**, **struct fields**, and
**function return types**; calls through a marked field read
(`(self.hook)()`) and through a returned value (`make()()`, or via a
binding) await like any other. An **unannotated binding adopts**
asyncness from any value it holds — its initializer or any `mut`
rebind — so a `let f = || { …await… };` needs no marker at all.

A synchronous closure flowing into an async-typed position is fine
(awaiting a non-promise yields it unchanged).

`sync |T| U` on a **parameter** declares the opposite contract: the
callback completes inside the declaring function's synchronous
protocol, and an async closure argument is an error. (`sync` is a
contextual keyword — it only means the contract directly before a
closure type.) `std::reactive`'s recompute positions (`Signal::map`,
`set_with`, `turn`/`batch` bodies, the UI render callbacks) are `sync`.

**Adaptation.** A **plain**, value-returning closure parameter is
*asyncness-polymorphic*: each call instantiates the function with the
actual asyncness of its closure arguments —

- an async argument instantiates an **async instance**: calls through
  the parameter are awaited **sequentially** (each callback settles
  before the next begins; effects are ordered exactly as the sync body
  orders them), the instance is async, and the caller awaits it;
- sync arguments select the untouched plain instance — no awaits, no
  coloring, identical emission.

An async adapted instance traverses a **snapshot** of any value it
iterates (the receiver as of the call): its awaits admit interleaved
work, which must not tear the traversal. Adaptation follows a closure
through plain parameters transitively (`helper(xs, f)` forwarding `f`
into `map` adapts both), and never crosses these boundaries:

- a `sync` parameter (error, including transitively — the diagnostic
  names the call that made the closure async);
- a host (`external`) function's value-returning closure parameter —
  host code cannot await a vilan closure;
- a trait/generic-dispatched call — there is no statically-known callee
  to instantiate (bind the receiver concretely, or declare the trait
  parameter `async`);
- a module-level initializer — it cannot await (§7.6's J.3 rule).

**The divergence rule** (the remaining stores). An async closure flowing
where a plain closure type is declared on a struct **field** or a
function's declared **return type** is:

- an **error** when that closure returns a value — the reader would
  receive a live promise typed as `T`; declare the field or return type
  `async |…| T` instead;
- **legal** when it returns `void` — *spawn semantics*: the call fires
  the closure and nobody awaits it. This is what allows UI event
  handlers and other void callbacks to suspend freely, and it applies
  to parameters as well (a void parameter spawns rather than adapts).

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
left-to-right evaluation per §7.2; truncating integer division and
two's-complement `as_*` folds (overflow excepted — §7.2a); and `i53`
exactness over the wire across its whole range. Everything
else about the emitted code — names, formatting, module layout, the
`[build]` knobs — is implementation-defined.
