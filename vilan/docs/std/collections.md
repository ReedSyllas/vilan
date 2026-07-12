# Collections — reference

`List` (built in), `std::map::Map`, `std::set::Set`, `std::range::Range`,
`std::iterator`.

## List<T>

Built in, with literal syntax (`[1, 2, 3]`; empty literals need a type
annotation).

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

## Map<K, V>

```vilan,fragment
impl Map<type K, type V> {
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

Keys compare by host semantics (a JS `Map` underneath) — scalar keys
(`i32`, `str`) behave as expected; **struct keys are a recorded gap** (two
equal struct values are two different keys). Key by an id instead.

## Set<T>

```vilan,fragment
impl Set<type T> {
	fun new(): Set<T>
	fun insert(&mut self, value: T)
	fun contains(self, value: T): bool
	fun remove(&mut self, value: T)
	fun len(self): i32
	fun is_empty(self): bool
}
```

Same key caveat as `Map`.

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
