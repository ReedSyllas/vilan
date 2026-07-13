# Numbers — reference

The sized numeric family (`std::number`), generic `min`/`max` (`std::math`),
and random values (`std::random`). Literal syntax and conversion semantics:
[Values and types](../tour/values-and-types.md).

## The family

| Type | Width | Literal |
|---|---|---|
| `i8 i16 i32 i53` | signed | bare = `i32`; others suffixed (`100i53`) |
| `u8 u16 u32 u53` | unsigned | suffixed (`0xFFu8`) |
| `f64` | float | `2.5` or `10f` |
| `f32` | float | `2.5f32` |
| `BigInt` | arbitrary | `7n` |

`i53`/`u53` are the **wide** integers, named for the precision they
actually deliver: they are f64-backed on the JS backend, and every value
in ±2^53 (JavaScript's safe-integer window) is exact. There is no `i64` —
a type that silently loses precision past 2^53 would be lying about its
width; for genuinely bigger integers use `BigInt`.

Literals are range-checked at compile time (an out-of-range `i53` literal
is a compile error, not a rounded value). Integer division truncates
toward zero. No implicit width coercion — convert with `as_*`. Arithmetic
that overflows a type's range is **undefined behavior** (spec §7.2) — on
JS it manifests as f64 artifacts; a checked `add_safe` family is recorded
future work.

## Methods

Integers (per type; shown for `i32`):

```vilan,fragment
impl i32 {
	fun abs(self): i32
	fun pow(self, exponent: i32): i32
	fun min(self, other: i32): i32
	fun max(self, other: i32): i32
	fun rem(self, m: i32): i32     // the % operator's method
	fun diff(self, other: i32): i32
}
```

Floats add the usual math surface:

```vilan,fragment
impl f64 {
	fun abs(self): f64
	fun sqrt(self): f64
	fun pow(self, exponent: f64): f64
	fun floor(self): f64
	fun ceil(self): f64
	fun round(self): f64
	fun min(self, other: f64): f64
	fun max(self, other: f64): f64
	fun sin(self): f64      // cos, tan, asin, acos, atan …
}
```

Every numeric type implements `Default` (zero), the operator traits, and
comparison.

## Conversions: `as_*`

Every numeric type converts to every other with Rust-`as` semantics —
truncate toward zero (floats), fold two's-complement into the target width
(integers):

```vilan
import std::print;

fun main() {
	print((3.9).as_i32());    // 3
	print((-1).as_u8());      // 255 — folded
	print((300).as_u8());     // 44
	let wide = 9007199254740992i53;
	print(wide.as_i32());
	print((255u8).as_f64() / 2.0);
}
```

Conversions on literals fold at compile time.

## std::math

```vilan,fragment
fun min<T: Ord>(a: T, b: T): T
fun max<T: Ord>(a: T, b: T): T
fun minmax<T: Ord>(a: T, b: T): (T, T)   // (smaller, larger)
```

## std::random

```vilan,fragment
fun range<T: Random>(low: T, high: T): T   // uniform in [low, high)
// implemented for i32, u32, f64
```

```vilan
import std::print;
import std::random;

fun main() {
	let roll = random::range(1, 7);   // 1..=6
	print(roll >= 1 && roll <= 6);
}
```

Not cryptographic — for tokens and ids use `std::crypto`
(`random_uuid`, `random_bytes`; see [misc](misc.md)).
