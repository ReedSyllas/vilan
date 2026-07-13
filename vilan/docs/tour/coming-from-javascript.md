# Coming from JavaScript

vilan compiles to JavaScript and runs on node and in the browser. If you
know JS or TypeScript, most of the syntax will feel familiar. A handful of
ideas are genuinely different, though. This page is the five-minute
orientation. Each topic links to the chapter that teaches it properly.

## The three big shifts

**1. Values are copied, not shared.**

In JavaScript, objects and arrays are passed around by reference. Two
variables can point at the same object, and a change through one is
visible through the other. In vilan, assigning or passing a value gives
the receiver its own copy. Nothing is shared unless you ask for sharing
explicitly.

```vilan
import std::print;

struct Point { x: i32, y: i32 }

fun main() {
	mut a = Point { x = 1, y = 2 };
	mut b = a;      // b is a copy
	b.x = 99;
	print(a.x);     // 1 — a is untouched
}
```

This removes a whole class of spooky-action bugs. When you do want two
places to see the same state, you say so with a tool built for it. The
[memory model](memory-model.md) chapter walks through all of them.

**2. There is no `null` or `undefined`.**

Absence is a real type called `Option`. A value that might be missing is
`Option<T>`, and the compiler makes you handle the missing case before
you can touch the value. The same idea covers errors: a function that can
fail returns `Result`, not a thrown exception. [Control
flow](control-flow.md) shows the idioms, and they are shorter than they
sound.

**3. `await` is implicit.**

Calling an async function just gives you the value. You don't write
`await`, you don't see `Promise<T>` in return types, and a function
becomes async automatically when it calls something async. You only reach
for the explicit keywords when you *don't* want to wait. The
[async chapter](async.md) explains the whole model.

## A quick phrasebook

| You write in JS/TS              | You write in vilan                              |
| ------------------------------- | ----------------------------------------------- |
| `function f(x) { … }`           | `fun f(x: i32): i32 { … }`                      |
| `const x = …` / `let x = …`     | `let x = …` / `mut x = …`                       |
| `x === y`                       | `x == y` (type-checked equality)                |
| `` `Hello ${name}` ``           | `i"Hello {name}"`                               |
| `[1, 2, 3]`                     | `[1, 2, 3]` (a `List<i32>`)                     |
| `{ x: 1, y: 2 }`                | a `struct` value: `Point { x = 1, y = 2 }`      |
| `(x, y) => x + y`               | `\|x, y\| x + y`                                |
| `switch` / discriminated unions | `enum` + `match`                                |
| `x?.field`                      | `x?.field` (on `Option`, and it's type-checked) |
| `null` / `undefined`            | `Option<T>` (`Some(v)` / `None`)                |
| `throw` / `try` / `catch`       | `Result<T, E>` + `!` to propagate               |
| `await fetchThing()`            | `fetch_thing()` (awaiting is implicit)          |
| `class` with methods            | `struct` + `impl` block                         |
| interfaces / duck typing        | `trait`s, checked at compile time               |

## Things that look the same and mostly are

Closures (`|x| x * 2` instead of `x => x * 2`), string concatenation with
`+`, `if`/`else`, comments with `//`, modules as files. The standard
library is small and imported explicitly, so files start with a few
`import` lines, like ES modules.

## Things to unlearn

- **Don't mutate through a shared reference.** `list.push(x)` works on
  *your* list. If you were relying on "everyone sees the update", you
  want a `Signal` (reactive state) or a `Shared` cell, both explicit.
- **Don't check for `null`.** Match on the `Option`.
- **Don't wrap things in `try`/`catch`.** Errors are values. Look at the
  `Result` and decide.
- **Don't sprinkle `await`.** It's already there.

## Where to go next

Read [Hello vilan](hello-vilan.md) to get a program running, then follow
the tour in order. When you start building UI or talking to a server, the
[guides](../guide/) take over. And whenever something surprises you, check
the [gotchas page](../appendix/gotchas.md) — if it surprised us first, it
is written down there.
