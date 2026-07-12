# Control flow

## `if` / `else`

An expression — both branches yield the value:

```vilan
import std::print;

fun main() {
	let n = 7;
	let label = if n % 2 == 0 { "even" } else { "odd" };
	print(label);
}
```

## `match`

The workhorse. Arms are `pattern => expression` (or a block); payloads bind
with `let`; `_` is the catch-all. Exhaustiveness is checked:

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

`is` tests a pattern as a boolean expression — handy for deriving flags:

```vilan,fragment
let present = entry.map(|current| current is Some(let _task));
```

**Traps**: `match` can't sit directly as an operator operand — bind it to a
local first. A `ret` inside an arm doesn't make the arm divergent for type
unification, and `panic(…)` types as `Any` — annotate the binding a mixed
match flows into.

## Loops

`for` has two forms — iteration and a while-style condition:

```vilan
import std::print;
import std::range::Range;

fun main() {
	for item in ["a", "b", "c"] {
		print(item);
	}

	for i in Range::new(0, 3) {   // 0, 1, 2 — end-exclusive
		print(i);
	}

	mut count = 0;
	for count < 3 {               // while-style: loop while the condition holds
		count += 1;
	}
	print(count);
}
```

`jump break` and `jump continue` control the enclosing loop; `for _ in …`
iterates without binding. Iterating a container by view (`for e in &mut
list`) mutates elements in place — see [the memory model](memory-model.md).

## Early return: `ret`

```vilan,fragment
fun parse(path: str): Route {
	if parts.len() == 0 {
		ret Route::Home;      // early exit
	}
	Route::NotFound           // final expression = the value
}
```

## Option and Result

Absence is `Option<T>`; fallibility is `Result<T, E>`. Both are plain enums
with method conveniences (`unwrap_or`, `map`, `is_some`, …) — the full set
is in the option/result reference. The two operators that make them
pleasant:

**`!` — unwrap or propagate.** Unwraps the good half; on the bad half,
returns it from the enclosing function (which must have a compatible return
type):

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
	let left = to_number(a)!;    // Err returns early, carrying the error
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

**`?.` — lift a continuation into the container.** `option?.field`,
`option?.method()` apply inside the `Some`/`Ok`, yielding `None`/the `Err`
untouched otherwise. A continuation that itself yields the container
flattens (no nesting):

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

`!` and `?.` dispatch through the `Try`/`Lift` traits (`std::operators`), so
your own two-outcome types can join in by implementing them.

## Panics and asserts

`panic(message)` aborts with a message — for unreachable states, not
expected failures (those are `Result`). `assert(condition, message)` panics
when the condition is false; it's also the `vilan test` failure mechanism.
