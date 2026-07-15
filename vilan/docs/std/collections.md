# Collections — reference

The container types: `List` (built in), `std::map::Map`, `std::set::Set`,
`std::range::Range`, and the `std::iterator` protocol underneath `for`.

## `List<T>`

Built in, with literal syntax: `[1, 2, 3]`. An empty literal needs a type
annotation (`let xs: List<str> = [];`).

```vilan,fragment
impl List<type T> {
	fun new(): List<T>
	fun push(&mut self, item: T)
	fun pop(&mut self): Option<T>
	fun len(self): i32
	fun is_empty(self): bool
	fun map<U>(self, fn: |T| U): List<U>
	fun filter(self, predicate: |T| bool): List<T>
	fun fold<B>(self, init: B, fn: |B, T| B): B
	fun for_each(self, fn: |T| void)
}
impl List<type T: Add + Default> { fun sum(self): T }
impl List<type T: Mul + Default> { fun product(self): T }
```

Indexing is `list[i]`; iterate with `for item in list` (copies) or
`for e in &mut list` (in-place views — see the
[memory model](../tour/memory-model.md)).

```vilan
import std::print;

fun main() {
	let words = ["alpha", "beta", "gamma"];
	let lengths = words.map(|word| word.len());
	print(lengths.fold(0, |total, n| total + n));
	print(lengths.sum());
}
```

## `Map<K, V>`

```vilan,fragment
impl Map<type K: Hashable, type V> {
	fun new(): Map<K, V>
	fun insert(&mut self, key: K, value: V)
	fun get(self, key: K): Option<V>
	fun contains_key(self, key: K): bool
	fun remove(&mut self, key: K)
	fun len(self): i32
	fun is_empty(self): bool
	fun keys(self): List<K>
	fun values(self): List<V>
}
```

Keys compare **by value**. Scalars work directly; a struct, enum, tuple, or
`List` key works as long as it is `Hashable` — derive it:

```vilan
import std::print;
import std::map::Map;
import std::hash::Hashable;
import std::option::Option::{ self, Some, None };

[derive(Hashable)]
struct Point {
	x: i32,
	y: i32,
}

fun main() {
	mut seen: Map<Point, str> = Map::new();
	seen.insert(Point { x = 1, y = 2 }, "origin-ish");
	// A fresh, distinct Point with equal fields hits.
	match seen.get(Point { x = 1, y = 2 }) {
		Some(let label) => print(label), // origin-ish
		None => print("miss"),
	}
}
```

`keys()` returns the real `K`s (in insertion order), and the key is snapshot
on insert, so mutating the original afterward can't desync the map.

## `Set<T>`

```vilan,fragment
impl Set<type T: Hashable> {
	fun new(): Set<T>
	fun insert(&mut self, value: T)
	fun contains(self, value: T): bool
	fun remove(&mut self, value: T)
	fun len(self): i32
	fun is_empty(self): bool
	fun values(self): List<T>
}
```

Value-keyed like `Map` (element `T` must be `Hashable`); `for x in set`
iterates the elements in insertion order.

## `Hashable`

A key's value is turned into a `Hash` — a canonical key — by `key.hash()`.
`[derive(Hashable)]` implements it for a struct/enum whose fields are all
`Hashable` (scalars, `str`, `bool`, `List`/`Option` of `Hashable`, or another
derived type); a closure, `Set`, `Map`, or `Shared` field is rejected. You can
also hand-write `impl Hashable` to key by a subset of fields, and build your own
container by bounding on `K: Hashable` and keying a `Map<Hash, …>` yourself.

One corner: a float *inside* an aggregate key canonicalizes through JSON, where
`NaN` becomes `null` and `-0`/`+0` collapse to `0`, so those collide. Bare
numeric keys don't have this (they key by JS value directly).

## Range

End-exclusive integer ranges, made for `for`:

```vilan,fragment
Range::new(start: i32, end: i32): Range   // start..end, end excluded
range.next(&mut self): Option<i32>
```

```vilan
import std::print;
import std::range::Range;

fun main() {
	mut total = 0;
	for i in Range::new(1, 5) {   // 1, 2, 3, 4
		total += i;
	}
	print(total);
}
```

## Iterator

The protocol `for` consumes, and the seam for custom sequences:

```vilan,fragment
trait Iterator<T> { fun next(self): Option<T>; }
trait Iterable<T> { fun iter(self): Iterator<T>; }
Iterator::from_fn(fn: || Option<T>): IteratorFromFn<T>   // an iterator from a closure
```

Anything implementing `Iterator`/`Iterable` works in a `for` loop —
`Range` is exactly this.
