# Spec §8 — Contexts

A **context** is a dynamically-scoped value: established for the dynamic
extent of a body, readable from anything that runs within that extent,
invisible outside it. Contexts are vilan's answer to ambient parameters
— the current reactive owner, the current turn, the enclosing nursery —
without global mutable state and without threading a parameter by hand
through every signature.

Contexts are **compiled away**. The implementation threads each
context's value as a hidden parameter through exactly the functions and
closures that transitively read it; there is no runtime storage, no
async-local machinery, and a program that never creates a context pays
nothing. Everything in this chapter is checked at compile time.

## 8.1 The model

`std::context::Context<T>` is the handle (appendix §A.4 — a lang item:
the compiler keys on its operations). A context is created once, at
module level, and referred to by that name:

```vilan,fragment
let flavor: Context<i32> = Context::new();
```

- `flavor.run(value, body)` — establish: `value` is the context's value
  for the **dynamic extent** of `body` (the closure's execution and
  everything it calls, transitively). `run` yields `body`'s value.
- `flavor.get()` — the **strict** read: yields the established `T`.
- `flavor.get_safe()` — the **safe** read: yields `Option<T>` —
  `Some(value)` under an enclosing `run`, `None` otherwise.

`Context::new`, `run`, `get`, and `get_safe` are intrinsics: the
threading pass rewrites their call sites away. They must be applied
directly to the context's **name** (a receiver that is not a named
context is rejected). A context is not otherwise useful as a value:
moving one through a parameter or a field severs the link between its
`run`s and its reads — the reads can then never be covered. `run`'s
body argument must be a closure literal (or an injected closure value,
§8.5).

`Context<T>`'s value type is inferred from its first `run`, exactly as
a `List<T>`'s element type is inferred from `push`.

```vilan
import std::print;
import std::context::Context;

let flavor: Context<i32> = Context::new();

fun describe(): str {
	i"flavor {flavor.get()}"
}

fun main() {
	let inner = flavor.run(7, || {
		print(describe());   // flavor 7 — dynamic extent reaches callees
		flavor.run(9, || describe())
	});
	print(inner);            // flavor 9 — the nearest run wins
}
```

Establishment nests: a read observes the **nearest** enclosing `run`.
Two runs of different contexts are independent.

## 8.2 Reads: strict and safe

The two read forms differ in what they demand of the program, not in
what they return at a covered site:

- A **strict** `get()` demands **coverage** (§8.3): the compiler proves
  every path that reaches it passes through an enclosing `run`. Code
  holding only strict reads carries the bare `T`.
- A **safe** `get_safe()` never demands coverage: uncovered paths
  supply `None`. Code reachable without a `run` carries the value as
  `Option<T>`; at a covered→safe boundary the bare value wraps in
  `Some` automatically.

Strictness is a property of *code regions*, propagated caller-ward: a
function that (transitively) reaches a strict read is strict; a
function whose only demands are safe reads is safe. One function may be
strict for one context and safe for another — each context threads
independently.

## 8.3 Coverage

For every strict read, the compiler checks — over the whole call graph
— that the reading code cannot be entered without the value. It is a
compile error ("context `…` is read here, but this code can be reached
without an enclosing `run`") when a strict read is reachable from:

- the program's top level or the entrypoint `main` (the uncovered
  roots — `main` is semantically the outermost, run-less extent);
- a module-level initializer;
- any caller chain that does not pass through a `run` of that context.

Trait/generic-dispatched calls are covered conservatively: a dispatch
site is treated as reaching every candidate implementation (and the
trait default), so a needy candidate demands coverage of the caller
even if another candidate would have been selected. Dead code —
a function with no callers at all — is exempt (it cannot run
uncovered); a function called only from top level, or taken as a
value, is not.

A function that reads a context **cannot be used as a value**
("`…` reads context `…`, so it can't be used as a value"): an indirect
call would bypass the hidden parameter. Wrap it in a closure literal at
the use site — the closure captures the channel correctly (§8.4).

Safe reads never fence. In uncovered positions they read `None`; this
is what lets library code ask "is there an ambient X?" from anywhere
(the standard library's spawn registration does exactly this — §7.7).

## 8.4 Closures capture at creation

A closure created inside a covered region **captures the context value
at its creation site**, and the capture is what its body reads —
regardless of where the closure is later called:

```vilan
import std::print;
import std::context::Context;

let flavor: Context<i32> = Context::new();

fun main() {
	let snap = flavor.run(3, || {
		|| flavor.get()   // captures 3 at creation
	});
	print(flavor.run(8, || snap()));   // 3 — not 8
}
```

This is the stability rule §7.5 relies on: ambient values do not shift
across suspensions or across deferred invocation, because the closure's
channel was fixed when it was made. (Contrast dynamic-binding systems
where the *call site's* environment decides; in vilan only `run`'s
extent and creation sites decide.)

## 8.5 Injected closures: the `context` clause

Capture-at-creation has one structural consequence: a closure literal
passed **to** an establishing function is created *before* the extent
exists, so it would capture the caller's (often absent) value — exactly
wrong for helpers like `nursery(body)` or `run_with_owner(owner, body)`
whose entire purpose is to run the body under a fresh value.

A **`context` clause** on a closure type solves this by *deferring* the
binding to the call site:

```vilan,fragment
fun with_flavor<T>(body: (|| T) context flavor): T {
	flavor.run(7, body)
}
```

A parameter (or `let` binding) whose closure type carries
`context <name>` declares an **injected** closure:

- A closure **literal** supplied to that position does not capture at
  creation; it takes its own hidden parameter, bound anew at each call.
- Each **call through** the injected value demands the context from the
  *caller* like a strict read, and supplies the caller's value.
- An unannotated local binding holding a closure literal, passed into a
  clause position, **adopts** the clause (as if the literal were
  written inline).

Because the binding is deferred, an injected value may only flow where
the threading can follow it: it can be **called**, **forwarded** to a
parameter with the same clause, or passed as **`run`'s body**. Any
other use — storing it in a field, returning it, putting it in a
collection — is a compile error ("an injected (`context`-typed) closure
can only be called, forwarded …, or passed to `run`").

A clause may name several contexts (`context (a, b)`); the clause must
name context bindings, and it composes with the closure-type markers of
§7.4 (`(sync || T) context turn_scope` is the reactive layer's shape).

## 8.6 Interactions

- **Async** (§7.5): captures are fixed at creation, so a context value
  is stable across every suspension of the extent by construction. An
  awaiting `run` body holds its value across its whole chain.
- **Spawns** (§7.7): every spawn site is an implicit *safe* read of the
  standard library's ambient nursery — inside a nursery's extent the
  value is present and the task registers; outside, the read is absent
  and the spawn stays free-floating.
- **Platforms**: the threading pass runs before emission for every
  target; contexts work identically on process and browser platforms
  (no host storage is involved).

## 8.7 The standard library's ambient values (informative)

`std` builds its ambient machinery on this one mechanism: `owner_scope`
(the reactive disposal owner — `run_with_owner`, `comp`),
`turn_scope` (the current write-batching turn — `turn`, `batch`), and
`ambient_nursery` (`std::task` — nursery registration and the
cancellation signal). Their semantics are library contracts, not
language rules; they are specified by their reference pages.
