# The memory model

> Normative rules: spec [§6 The memory model](../spec/memory.md).

vilan is **value-semantic**: an assignment, a function argument, a struct
field, a signal payload — each is logically its own copy. Nothing aliases
unless you reach for one of the explicit sharing tools, and each tool exists
for one situation. The decision tree:

1. Default: plain **values** (`let` / `mut`).
2. Mutate in place across a call: a **view** (`&mut`).
3. Shared mutable state with independent lifetime: **`Shared<T>`**.
4. Graphs, cycles, stable identities: **`Arena` + `Handle`**.

## Values (`let`, `mut`)

`let` binds immutably; `mut` allows reassignment and field mutation. A plain
binding of an existing value is a **copy** — mutating the copy leaves the
original alone:

```vilan
import std::print;

struct Counter { value: i32 }

fun main() {
	mut original = Counter { value = 10 };
	mut copy = original;   // copies
	copy.value = 99;
	print(original.value); // 10 - the original value is unchanged
}
```

Passing to a function is the same: the callee gets its own value, and
mutations inside don't leak out. (An `own c: Counter` parameter documents
"takes the value" explicitly; it's the default behavior with a name.)

## Views (`&`, `&mut`)

A view is a **borrow of a place** — it aliases, it does not copy. `&mut x`
is writable, `&x` readonly:

```vilan
import std::print;

struct Counter { value: i32 }

fun bump(&mut c: Counter) {
	c.value += 10;
}

impl Counter {
	fun increment(&mut self) {
		self.value += 1;
	}
}

fun main() {
	mut c = Counter { value = 10 };
	c.increment();
	bump(&mut c);
	print(c.value); // 21 - both mutated the original through views
}
```

Views are deliberately **second-class**: they live as parameters and
short-lived locals; they cannot be stored in structs, returned into
long-lived state, or held across an `await`. That confinement is what makes
them safe without a lifetime system.

Iterating by view mutates elements in place — plain `for x in list` copies
each element, `for e in &mut list` binds writable element views (`*e` reads
the value; assigning writes through):

```vilan
import std::print;

fun main() {
	mut xs: List<i32> = [1, 2, 3];
	for e in &mut xs {
		e = *e * 10;
	}
	print(xs[2]); // 30
}
```

A method may *return* a view projecting its receiver (`fun get(&mut self,
…): &mut T`) — the borrow is inferred, and the returned view obeys the same
short-lived rules at the call site. `Option<&T>` works for "a view, maybe"
(a map lookup).

## `Shared<T>` — the shared mutable cell

When two places genuinely need to see the same mutable state — a closure and
its creator, two data structures — `Shared<T>` is the tool: a heap cell with
value reads and in-place writes.

```vilan
import std::print;
import std::shared::Shared;

fun main() {
	let count = Shared::new(0);
	let bump = || {
		count.write() = count.read() + 1;
	};
	bump();
	bump();
	print(count.read()); // 2 — the closure and main share the cell
}
```

- `read()` returns a **copy** of the value.
- `write()` returns a writable view *of the cell's contents* — assign to it
  (`cell.write() = v`) or mutate through it (`cell.write().push(item)`),
  within the same statement.

`Shared` is everywhere in framework internals (signals, transports) and in
closures that accumulate. If you're reaching for it to "avoid a copy" on a
hot path, measure first — values are cheap.

## `Arena` + `Handle` — graphs and identity

Cyclic or many-to-many structures don't fit values-plus-views. An `Arena<T>`
owns a collection of slots; a `Handle` is a small copyable id into it —
edges are handles, and everything stays value-semantic on the outside:

```vilan,fragment
let nodes: Arena<Node> = Arena::new();
let a: Handle = nodes.insert(Node { … });
let b: Handle = nodes.insert(Node { edges = [a] });
nodes.get_mut(a).edges.push(b);   // a cycle, no aliasing
```

Use it for trees with parent pointers, graphs, entity systems — anywhere
you'd want "references to each other".

## The async rule

**No view may live across an `await`.** Between suspension and resumption,
other turns run and may mutate or replace what the view aliased; the
compiler rejects the shape. Re-derive after the await instead — read the
`Shared` again, re-index the list:

```vilan,fragment
let row = &mut rows[i];
send(row.id);            // suspends —
row.text = "sent";       // ✗ rejected: view held across await

send(rows[i].id);        // ✓ re-derive after the suspension
rows[i].text = "sent";
```

## Traps

- "Why didn't my mutation stick?" — you mutated a copy. Either take `&mut`
  at the seam, or restructure around `Shared`/a signal's `set_with`.
- `shared.read()` is a copy: `shared.read().push(x)` mutates the copy and
  is lost. Write through `write()`: `shared.write().push(x)`.
- Signals follow value semantics too: mutate lists with
  `signal.set_with(|list| …)`, never by mutating a `get()` result.
