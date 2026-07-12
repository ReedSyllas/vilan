# Values and types

## Bindings

`let` binds immutably; `mut` allows reassignment and mutation. Types are
inferred; annotate when you want to pin one:

```vilan
import std::print;

fun main() {
	let name = "Ada";
	mut count = 0;
	count += 1;
	let wide: i64 = 1000i64;
	print(i"{name} {count} {wide}");
}
```

Everything is a **value** — a binding of an existing value is a copy (see
[the memory model](memory-model.md)).

## Primitives

- `bool` — `true` / `false`.
- `str` — immutable strings.
- Sized integers: `i8 i16 i32 i64 u8 u16 u32 u64`. A bare integer literal
  is `i32`; other widths take a suffix (`0xFFu8`, `60000u16`,
  `9007199254740992i64`). Literals are range-checked at compile time.
- Floats: `f64` (bare `2.5`, or suffix `f`) and `f32` (`2.5f32`).
- `BigInt` — arbitrary precision, `n` suffix (`7n`).

Integer division **truncates toward zero** (`7 / 2 == 3`, `-7 / 2 == -3`);
float and BigInt division don't. Mixed-width arithmetic doesn't coerce —
convert explicitly with the `as_*` methods (Rust-`as` semantics: truncate,
fold into the target width):

```vilan
import std::print;

fun main() {
	print(7 / 2);           // 3
	print((3.9).as_i32());  // 3
	print((300).as_u8());   // 44 — folded into u8
	let byte = 0xFFu8;
	print(byte.as_f64() + 0.25);
}
```

**Trap**: an `i64` in a binary operation needs a suffixed literal on the
other side — `stamp + 1000i64`, not `stamp + 1000` (the bare literal is
`i32`).

## Strings and interpolation

`"…"` is a plain string. The `i` prefix interpolates `{expr}`; escape
literal braces as `\{`/`\}`:

```vilan
import std::print;

fun main() {
	let name = "John";
	print("Hello, {name}!");    // Hello, {name}!  — plain string, no magic
	print(i"Hello, {name}!");   // Hello, John!
	print(i"literal \{braces\}");
}
```

Concatenation is `+`. The full method set (split, len, contains, …): the
strings reference *(Phase 2 pages)*.

## Tuples

`(a, b)` groups values; destructure with `let (x, y) = pair;`:

```vilan
import std::print;

fun main() {
	let pair = (1, "one");
	let (number, word) = pair;
	print(i"{number} = {word}");
}
```

Tuple types are written `(i32, str)`. Elements are also reachable as
`.0`/`.1` — but not chained directly on a call result (bind first; see
[gotchas](../appendix/gotchas.md)).

## Collections

`List<T>` is built in with literal syntax; `Map<K, V>` and `Set<T>` are
imported and construct with `::new()`:

```vilan
import std::print;
import std::map::Map;
import std::set::Set;

fun main() {
	mut items: List<i32> = [1, 2, 3];
	items.push(4);
	print(items.len());
	print(items[0]);

	mut scores: Map<str, i32> = Map::new();
	scores.insert("ada", 100);

	mut seen: Set<i32> = Set::new();
	seen.insert(7);
	print(seen.contains(7));
}
```

An empty literal usually needs an annotation (`let xs: List<str> = [];`) —
there's nothing to infer the element type from. Collections are values like
everything else: `let copy = items;` copies (`Map`/`Set` key semantics for
struct keys are a recorded gap — keys are best kept scalar for now).

## `null` is not part of the model

Absence is `Option<T>` (`Some(value)` / `None`) — see
[Control flow](control-flow.md) for the idioms. The `null` type exists only
at host boundaries (externs that can return JS null).
