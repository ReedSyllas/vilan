# Control flow

> Normative rules: spec [§3 Grammar](../spec/grammar.md) and [§5.10 `!` and `?.`](../spec/types.md).

## `if` / `else`

`if` is an expression. Both branches produce a value, and the whole thing
has that value:

```vilan
import std::print;

fun main() {
	let n = 7;
	let label = if n % 2 == 0 { "even" } else { "odd" };
	print(label);
}
```

There is no ternary operator because `if` already is one.

## `match`

`match` is the workhorse, a `switch` that can take values apart and that
the compiler checks for completeness. Arms are `pattern => expression`.
Payloads bind with `let`, and `_` catches everything else:

```vilan
import std::print;
import std::option::Option::{ self, Some, None };

fun describe(slot: Option<i32>): str {
	match slot {
		Some(let value) => i"got {value}",
		None => "empty",
	}
}

fun main() {
	print(describe(Some(3)));
	print(describe(None));
}
```

If you forget a variant, the compiler tells you. That's most of the
reason enums plus `match` replace flag fields and `null` checks.

When you only need a yes/no answer instead of a full match, `is` tests a
pattern as a boolean:

```vilan,fragment
let present = entry.map(|current| current is Some(let _task));
```

One edge to know: a `match` can't sit directly inside a larger operator
expression. Bind it to a local first.

Arms that never produce a value — a `ret`, a `panic`, a
`jump break`/`continue` — simply don't participate in the match's type.
The other arms decide it:

```vilan,fragment
let value = match slot {
	Some(let text) => text,     // the match is a str
	None => panic("missing"),   // this arm diverges; no annotation needed
};
```

## Loops

`for` covers every loop. It has three forms:

```vilan
import std::print;
import std::range::Range;

fun main() {
	for item in ["a", "b", "c"] {
		print(item);
	}

	for i in Range::new(0, 3) {   // 0, 1, 2 — the end is exclusive
		print(i);
	}

	mut count = 0;
	for count < 3 {               // a while loop: runs while the condition holds
		count += 1;
	}
	print(count);
}
```

There is also a bare `for { … }` for an infinite loop. Loop control is
spelled `jump break` and `jump continue`. Iterating with `for _ in …`
skips the binding.

One more form matters once you care about performance:
`for e in &mut list` iterates *views* of the elements so you can mutate
them in place. That's a [memory model](memory-model.md) topic.

## Early return: `ret`

```vilan,fragment
fun parse(path: str): Route {
	if parts.len() == 0 {
		ret Route::Home;      // early exit
	}
	Route::NotFound           // the final expression is the return value
}
```

## Option and Result

vilan has no `null` and no exceptions. Instead:

- A value that might be absent is an `Option<T>` — either `Some(value)`
  or `None`.
- An operation that might fail is a `Result<T, E>` — either `Ok(value)`
  or `Err(error)`.

Both are plain enums with a rich set of helper methods (`unwrap_or`,
`map`, `is_some`, and friends — the
[full list](../std/option-result.md)). You can always just `match` on
them. Two operators make the common patterns short.

**`!` unwraps, or propagates the failure.** Think of it as "give me the
value, and if there isn't one, return the failure from this function":

```vilan
import std::print;
import std::option::Option::{ self, Some, None };
import std::result::Result::{ self, Ok, Err };

fun to_number(text: str): Result<i32, str> {
	match text.parse_i32() {
		Some(let value) => Ok(value),
		None => Err(text),
	}
}

fun sum(a: str, b: str): Result<i32, str> {
	let left = to_number(a)!;    // an Err returns early, carrying the error
	let right = to_number(b)!;
	Ok(left + right)
}

fun main() {
	match sum("2", "40") {
		Ok(let total) => print(total),
		Err(let bad) => print(i"not a number: {bad}"),
	}
}
```

This is what `try`/`catch` becomes: failures travel up through return
types, visibly, and the caller decides what to do.

`!` returns the failure **as-is**, so the value's error type must already
be the function's — vilan doesn't convert it behind your back. When the
types differ, convert at the value, before the `!`: `.map_err(…)` changes
a `Result`'s error, and `.ok_or(err)` turns an `Option`'s `None` into an
`Err` you supply.

```vilan
import std::print;
import std::option::Option::{ self, Some, None };
import std::result::Result::{ self, Ok, Err };

fun lookup(key: str): Option<i32> {
	if key == "answer" { Some(42) } else { None }
}

fun read(key: str): Result<i32, str> {
	let value = lookup(key).ok_or(i"no entry for {key}")!;  // None -> Err
	Ok(value)
}

fun main() {
	match read("answer") {
		Ok(let value) => print(value),          // 42
		Err(let why) => print(why),
	}
	match read("missing") {
		Ok(let value) => print(value),
		Err(let why) => print(why),             // no entry for missing
	}
}
```

**`?.` reaches inside the container.** It looks like optional chaining
from JS, and on `Option` it plays the same role — with the compiler
checking it:

```vilan
import std::print;
import std::option::Option::{ self, Some, None };

struct Book {
	title: str,
}

fun find(key: str): Option<Book> {
	if key == "hit" {
		Some(Book { title = "dune" })
	} else {
		None
	}
}

fun main() {
	print((find("hit")?.title).unwrap_or("?"));   // dune
	print((find("miss")?.title).unwrap_or("?"));  // ?
}
```

`find("hit")?.title` means: if the option holds a book, take its title
and wrap it back up; if it's `None`, stay `None`. It works on `Result`
too, passing an `Err` through untouched.

> **Going deeper.** Both operators are trait-driven, not hard-coded to
> `Option` and `Result`. `!` dispatches through `Try`, and `?.` through
> `Try` plus the `Lift` marker (`std::operators`). Your own
> two-outcome type can implement them and join in. A `?.` continuation
> that itself produces the container flattens instead of nesting, so
> `find(key)?.shelf()` on an `Option`-returning method stays a single
> `Option`.

## Panics and asserts

`panic(message)` stops the program with a message. Use it for states
that should be impossible, not for expected failures — those are
`Result`s. `assert(condition, message)` panics when the condition is
false, and it's how `vilan test` decides a test failed.
