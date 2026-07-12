# Functions & closures

## Functions

`fun` declares a function; the body's last expression is its value, `ret`
returns early:

```vilan
import std::print;

fun clamp(value: i32, low: i32, high: i32): i32 {
	if value < low {
		ret low;
	}
	if value > high {
		ret high;
	}
	value
}

fun main() {
	print(clamp(15, 0, 10));
}
main();
```

Generic functions take type parameters with optional bounds:

```vilan,fragment
fun largest<T: PartialOrd>(a: T, b: T): T { … }
```

## Closures

`|params| body` — the parameter types come from context when the closure
flows into a typed position, or from annotations:

```vilan
import std::print;

fun apply(seed: i32, transform: |i32| i32): i32 {
	transform(seed)
}

fun main() {
	print(apply(21, |n| n * 2));
	let label = |count: i32| i"{count} items";
	print(label(3));
}
main();
```

Closure **types** are written `|T| U` (and `|| U` for no parameters,
`|| void` for no result). They appear as parameter types, `let` annotations,
and struct fields.

Closures capture their environment by value at creation (vilan is
value-semantic — see [the memory model](memory-model.md)); captured `Shared`
cells are how a closure shares mutable state with its creator.

## Named functions as closures

A reference to a plain function coerces to a matching closure type — no
wrapping lambda:

```vilan
import std::print;
import std::reactive::Signal;

fun exclaim(text: str): str {
	text + "!"
}

fun main() {
	let words = Signal::new("hello");
	let loud = words.map(exclaim);   // instead of .map(|w| exclaim(w))
	print(loud.get());
}
main();
```

Eligible: plain vilan `fun`s. Not eligible (write the wrapping closure):
generic functions, methods, `async` functions, and externs (a dotted host
global would lose its `this`).

## Async closures

An `async` closure **type** — `async |T| U` — marks a closure value whose
calls are implicitly awaited. The marker lives at two seams only: parameter
types and `let` annotations. It does not exist on struct fields or return
types yet, which produces a standard pattern — **store plain, re-mark at a
`let`**:

```vilan,fragment
struct Draft<T> {
	commit: |T| Option<str>,          // stored plain
}
…
let commit: async |T| Option<str> = self.commit;   // re-marked: calls now await
let outcome = commit(value);
```

The flip side: an async closure flowing into a plain closure parameter is an
error when the parameter returns a value (the caller would receive a promise
typed as `T`) — but legal when it returns `void`: that is **spawn
semantics**, fire-and-forget, and it's what lets UI event handlers and turn
bodies be async without ceremony. See [Async](async.md).

## Context clauses

A parameter's closure type can name ambient **contexts** the closure body
reads — written after the type:

```vilan,fragment
fun mount_root(id: str, body: (|| View) context owner_scope): Owner
fun turn<T>(policy: FlushPolicy, body: (|| T) context turn_scope): T
```

Passing a closure literal into such a position makes it an **injected**
closure: the ambient value (the current `Owner`, the current `Turn`) threads
to it at the call site, through any depth of plain function calls. This is
the machinery behind "every `effect` registers with the nearest boundary" —
your component functions never mention owners, yet ownership flows.

Two consequences worth knowing:

- Closures capture their contexts **at creation**. A closure created outside
  a `run` and called inside it sees nothing — the compiler rejects the shape
  rather than let it misbehave.
- A function that *reads* a context can't be passed around as a plain value
  (the context channel would be severed); the compiler tells you.

## Traps

- Calling a method-call result directly doesn't parse yet:
  `self.hook.read()(a, b)` — bind first: `let hook = self.hook.read();
  hook(a, b)`.
- A closure bound to a local and then called directly
  (`let f = |i| …; f(3)`) doesn't infer its parameter type from the call —
  annotate the parameter.
- Chained element access on a call result (`pair().1`, `read()[i]`) can
  lose the element type — bind, then access.
