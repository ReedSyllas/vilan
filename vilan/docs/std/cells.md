# Cells — reference

The two sharing tools: `std::shared::Shared` (one shared mutable cell) and
`std::arena::Arena` (stable identities for graphs). When to reach for
which: [the memory model](../tour/memory-model.md).

## `Shared<T>`

A heap cell two places can hold at once — the escape hatch from
value-semantics copying.

```vilan,fragment
impl Shared<type T> {
	fun new(value: T): Shared<T>
	fun read(self): T                      // a COPY of the contents
	fun write(self): &mut T                // a writable view of the contents
}
```

```vilan
import std::print;
import std::shared::Shared;

fun main() {
	let log: Shared<List<str>> = Shared::new([]);
	let record = |entry: str| {
		log.write().push(entry);
	};
	record("first");
	record("second");
	print(log.read().len());
}
```

- `read()` copies: mutating the result is lost
  (`shared.read().push(x)` — the classic trap). Mutate through `write()`.
- `write()` returns a view — use it within the same statement
  (`cell.write() = v`, `cell.write().push(item)`); it obeys the usual view
  rules (no storing, no holding across `await`).
- Copying the `Shared` value itself copies the *handle* — both handles see
  one cell. That's the point.

## `Arena<T>` + `Handle<T>`

A **generational arena**: insert values, get back small copyable
`Handle<T>` keys. Handles are plain values — storable in struct fields and
lists (which views are not), so nodes can reference each other:

```vilan,fragment
struct Handle<T> { … }   // slot index + generation; copy freely

impl Arena<type T> {
	fun new(): Arena<T>
	fun insert(&mut self, value: T): Handle<T>
	fun get(self, handle: Handle<T>): Option<T>        // None once removed
	fun set(&mut self, handle: Handle<T>, value: T): bool
	fun remove(&mut self, handle: Handle<T>): Option<T>
	fun contains(self, handle: Handle<T>): bool
	fun len(self): i32
	fun is_empty(self): bool
}
```

```vilan
import std::print;
import std::arena::{ Arena, Handle };
import std::option::Option::{ self, Some, None };

struct Node {
	label: str,
	edges: List<Handle<Node>>,
}

fun main() {
	mut nodes: Arena<Node> = Arena::new();
	let a = nodes.insert(Node { label = "a", edges = [] });
	let b = nodes.insert(Node { label = "b", edges = [a] });
	// Close the cycle: a → b.
	match nodes.get(a) {
		Some(let node) => {
			mut updated = node;
			updated.edges.push(b);
			nodes.set(a, updated);
		},
		None => {},
	}
	print(nodes.len());
}
```

- **Generational** means deletion-safe: removing a value and reusing its
  slot bumps a generation counter, so a stale handle `get`s `None` instead
  of aliasing the new occupant.
- `get` returns a **copy** of the value; mutate by `get` → modify → `set`
  (or design nodes so edges/fields update independently).
- Traversal is re-`get` per step — the arena stays mutable while you walk.
