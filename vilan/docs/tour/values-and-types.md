# Values and types

> Normative rules: spec [§2 Lexical structure](../spec/lexical.md) and [§5 The type system](../spec/types.md).

## Bindings

`let` declares a binding you won't change. `mut` declares one you will.
Types are inferred, and you can annotate when you want to pin one down:

```vilan
import std::print;

fun main() {
	let name = "Ada";
	mut count = 0;
	count += 1;
	let wide: i53 = 1000i53;
	print(i"{name} {count} {wide}");
}
```

One thing to know up front: everything in vilan is a **value**. Assigning
a value to a new binding gives you a copy, not a second name for the same
thing. If that sounds strange coming from JavaScript, start with
[Coming from JavaScript](coming-from-javascript.md), then read
[the memory model](memory-model.md) when you're ready for the full story.

## Primitives

- `bool` — `true` and `false`.
- `str` — immutable strings.
- Signed integers `i8 i16 i32 i53` and unsigned `u8 u16 u32 u53`. A bare
  literal like `42` is `i32`. Other widths take a suffix: `0xFFu8`,
  `60000u16`, `9007199254740992i53`.
- Floats: `f64` (a bare `2.5`, or the `f` suffix) and `f32` (`2.5f32`).
- `BigInt` — arbitrary precision, with the `n` suffix (`7n`).

Why `i53` and not `i64`? Because vilan runs on JavaScript, and JavaScript
numbers are 64-bit floats. Every integer up to ±2^53 is exact in a float.
Beyond that, precision silently disappears. Rather than offer an `i64`
that quietly isn't one, vilan names the type for what it actually
delivers. If you need more than 53 bits, use `BigInt`.

The compiler checks every literal against its type's range, so an
out-of-range literal is a compile error rather than a wrong value.

```vilan
import std::print;

fun main() {
	print(7 / 2);           // 3 — integer division truncates
	print((3.9).as_i32());  // 3 — conversions are explicit, via as_*
	print((300).as_u8());   // 44 — narrowing folds into the target width
	let byte = 0xFFu8;
	print(byte.as_f64() + 0.25);
}
```

Two rules that differ from JS:

- **Integer division truncates toward zero.** `7 / 2` is `3`, and
  `-7 / 2` is `-3`. Float division works the way you expect.
- **There are no implicit conversions between numeric types.** Mixing an
  `i53` and an `i32` in one expression is a compile error. Convert
  explicitly with the `as_*` methods, or suffix the literal.

That second rule has one trap worth memorizing. If `stamp` is an `i53`,
write `stamp + 1000i53`, not `stamp + 1000`. The bare `1000` is an `i32`,
and the mix won't compile.

> **Going deeper.** The `as_*` conversions use Rust's `as` semantics:
> floats truncate toward zero, and integers fold two's-complement into
> the target width, so `(-1).as_u8()` is `255`. Conversions on literals
> fold at compile time. Arithmetic that overflows a type's range is
> undefined behavior — the compiler checks literals, not runtime math.
> Details in spec [§7.2a](../spec/execution.md).

## Strings and interpolation

`"…"` is a plain string, and vilan does **not** interpret `{}` inside it.
To interpolate, prefix the string with `i`:

```vilan
import std::print;

fun main() {
	let name = "John";
	print("Hello, {name}!");    // Hello, {name}!  — a plain string
	print(i"Hello, {name}!");   // Hello, John!
	print(i"literal \{braces\}");
}
```

Concatenation is `+`. The full method list (split, trim, contains, and so
on) is in the [strings reference](../std/strings.md).

## Tuples

`(a, b)` groups a few values without declaring a struct. Take them
apart with a destructuring `let`, or reach one element by position:

```vilan
import std::print;

fun main() {
	let pair = (1, "one");
	let (number, word) = pair;
	print(i"{number} = {word}");
	print(pair.1);
}
```

Tuple types are written the same way: `(i32, str)`. Positional access
(`pair.0`, `pair.1`, chains like `nested.0.1`) types as that element;
through a `mut` binding you can also assign one (`pair.0 = 5`).

## Collections

`List<T>` is built in and has literal syntax. `Map<K, V>` and `Set<T>`
come from std:

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

An empty literal usually needs a type annotation, like
`let xs: List<str> = [];`. There is nothing inside it to infer the
element type from.

Collections are values like everything else, so `let copy = items;`
really copies. If you're used to passing an array around and mutating it
from several places, that's the habit to unlearn. The
[memory model](memory-model.md) chapter shows what to do instead.

When the size is fixed and known, `[T; n]` is a **fixed-length array** — the
length is part of the type, so it can't grow or shrink, and `[i32; 3]` is a
different type from `[i32; 4]`. Write `[value; n]` to fill `n` slots, or a
plain literal under a `[T; n]` annotation:

```vilan
import std::print;

fun main() {
	let buffer = [0; 4];            // [i32; 4] — four zeros
	mut rgb: [u8; 3] = [255u8, 128u8, 0u8];
	rgb[2] = 64u8;                  // indexed like a List
	print(buffer[0]);              // 0
	print(rgb[2]);                 // 64

	mut sum = 0;
	for channel in rgb {
		sum = sum + channel.as_i32();
	}
	print(sum);                    // 447
}
```

Reach for `[T; n]` over `List<T>` when the count never changes — a color, a
matrix row, a lookup table. Everything else (`push`, growing) is what `List`
is for.

> **Going deeper.** `Map` and `Set` key **by value**. Scalar keys (`i32`,
> `str`) work directly; a struct, enum, or `List` key works once it derives
> `Hashable` (`[derive(Hashable)]`), so two equal values are the same key.
> See [collections](../std/collections.md).

## Where's `null`?

There isn't one. A value that might be absent is an `Option<T>`, and the
compiler makes you handle the `None` case before you can use the value.
This is one of the big shifts from JavaScript, and
[Control flow](control-flow.md) shows how natural it becomes. (`null`
technically exists at the host boundary, for externs that can return JS
null, but ordinary vilan code never sees it.)
