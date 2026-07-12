# Strings — reference

`str` (built in, immutable), plus the text-facing traits `Display`, `Debug`,
and `Into`.

## str

Concatenate with `+`; interpolate with `i"…{expr}…"` (see
[Values and types](../tour/values-and-types.md)).

```vilan,fragment
impl str {
	fun len(self): i32
	fun is_empty(self): bool
	fun trim(self): str
	fun to_uppercase(self): str
	fun to_lowercase_ascii(self): str
	fun contains(self, needle: str): bool
	fun starts_with(self, prefix: str): bool
	fun ends_with(self, suffix: str): bool
	fun replace(self, from: str, to: str): str       // all occurrences
	fun repeat(self, count: i32): str
	fun split(self, separator: str): List<str>
	fun substring(self, start: i32, end: i32): str   // end-exclusive
	fun code_at(self, index: i32): u32               // UTF-16 code unit
	fun parse_i32(self): Option<i32>                 // declared in std::option
}
```

```vilan
import std::print;

fun main() {
	let path = "/w/3/task/7";
	let parts = path.split("/").filter(|part| !part.is_empty());
	print(parts.len());
	print(parts[0].to_uppercase());
	print("task".repeat(2));
}
```

`str` also implements `PartialEq`/`Ord` (lexicographic `==`, `<`) and
`Default` (`""`).

## Display — user-facing text

```vilan,fragment
trait Display {
	fun to_string(self): str;
}
fun format<T: Display>(value: T): str
```

Implement `Display` for values that have a natural user-facing rendering;
`format(value)` (from `std::display`) is the generic entry point.
Interpolation accepts anything already `str`-shaped — call
`format`/`to_string` explicitly for custom types.

## Debug — developer-facing text

```vilan,fragment
trait Debug {
	fun debug(self): str;
}
```

`[derive(Debug)]` generates a structural rendering (`Point { x: 1, y: 2 }`
style) for structs and enums — the standard tool for logging and error
paths (`error.debug()` on an `RpcError`).

## Into — conversions

```vilan,fragment
trait Into<T> {
	fun into(self): T;
}
```

The generic conversion seam: implement `Into<Target>` on a source type,
bound helpers as `T: Into<Target>`. (Numeric width conversions don't use
this — they're the `as_*` methods on the numbers; see
[numbers](numbers.md).)
