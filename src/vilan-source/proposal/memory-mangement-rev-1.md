# Memory Management (Revision 1)

Goals, unchanged: deterministic, GC-free memory that a developer can *read off
the source*, that unlocks in-place optimization even on the JS backend, and that
ports to WebAssembly / small Rust modules later **without exposing lifetimes**.

This revision rests on four rules. The first two govern *owned values* (the easy
half). The last two govern *references* (where every sharp edge lives).

1. **Values are copied. Copies are semantic.**
2. **Elision is an optimization — never observable.**
3. **References are second-class views.**
4. **Safety rule: no *invalidating* mutation under a live view** — not Rust's
   aliasing-XOR-mutability.

The combination is "mutable value semantics": you get Rust-grade safety (no
use-after-free, no double-free, no iterator invalidation) without a borrow
checker and without ever writing a lifetime.

---

## 1. Values are copied. Copies are semantic.

Assignment copies. There are no hidden aliases between bindings — primitives and
structs behave identically.

```vilan
mut a = 10;
a = 20;

mut b = a; // copy
b = 30;
print(i"{a}, {b}"); // 20, 30
```

```vilan
mut a = Point::new(7, 5);
mut b = a; // copy

a.x = 10;
b.x = 20;
print(i"{a}, {b}"); // Point { x = 10, y = 5 }, Point { x = 20, y = 5 }
```

Collections are values too: `mut b = a` on a `List` is semantically a full copy,
so `a` and `b` never observe each other's writes. (How often that copy is
*actually performed* is rule 2.)

**The payoff of "semantic":** because every assignment is a copy *in the
language*, using a value after you "moved" it is always fine. There is no
use-after-move, no partial-move tracking, no `?`-on-a-moved-value — the entire
class of move errors that Rust diagnoses simply cannot occur. You never reason
about ownership transfer to write correct code; you reason about it only to
predict performance.

---

## 2. Elision is an optimization — never observable.

A semantic copy that the compiler can *prove* is the original's last use is
compiled to a move (storage reuse), with no clone.

```vilan
mut a = Point::new(4, 6);
mut b = a; // developer sees a copy; `a` is dead after this, so the compiler reuses it
b.x = 10;
print(b); // Point { x = 10, y = 6 }
```

```js
const a = [4, 6];
const b = a; // no copy — `a` is never read again
b[0] = 10;
console.log(b);
```

When the original *is* read later, the copy is real:

```vilan
mut a = Point::new(4, 6);
mut b = a;
b.x = 10;
print(i"{a}, {b}"); // a is still alive → a real copy was needed
```

```js
const a = [4, 6];
const b = structuredClone(a);
b[0] = 10;
console.log(`${a}, ${b}`);
```

Because the result is identical either way, **the elision analysis only has to
be sound, never complete.** When it can't prove last-use (a conditional consume,
a value escaping through a view — see rule 4), it copies. Worst case is a missed
optimization, never a miscompile. This is the property that keeps the analysis
from sprawling: it can be as simple or as smart as we like, independently of
correctness.

> Model choice: this document specifies **static last-use elision** — every
> clone is visible in the output and has zero runtime bookkeeping. The
> alternative (reference-counting with copy-on-write, à la Swift/Koka) clones
> more precisely but adds refcount traffic and less predictable cost. Static
> elision is the better fit for "memory you can read off the source"; refcounting
> is reserved for genuine sharing (see *Shared mutable state*).

---

## 3. References are second-class views.

`&x` makes a **readonly view** of `x`; `&mut x` makes a **writable view** (which
requires `x` to be `mut`). `*v` reads through any view, and writes through a
writable one.

```vilan
mut a = 10;
let c = &mut a; // writable view
*c = 40;
print(i"{a}, {c}"); // 40, 40
```

Field and method access through a view auto-derefs, so `*` is only needed to
read or replace the *whole* pointee:

```vilan
mut p = Point::new(1, 2);
let v = &mut p;
v.x = 10;              // auto-deref; same as (*v).x = 10
*v = Point::new(7, 8); // replace the whole value
```

### One vocabulary, position-dependent defaults.

`own` (a copy), `&` (readonly view), and `&mut` (writable view) mean the same
thing in every position. Only the *unannotated default* changes, to match the
common case — and `&mut` is always explicit, so mutation is never silent:

| position  | bare default           | written out                            |
| --------- | ---------------------- | -------------------------------------- |
| binding   | own — `let b = a`      | `&a`, `&mut a`                         |
| parameter | readonly view — `x: T` | `own x: T`, `x: &mut T`                |
| return    | own — `: T`            | `: &T borrows X`, `: &mut T borrows X` |

So a parameter inspects its argument without copying by default; you write `own`
only when the function must *store or return* the value — and if you forget, the
compiler rejects it ("can't store a view") rather than silently cloning. This is
what keeps reads of large aggregates cheap without leaning on elision: a getter
`fun value_of(self, …)` views `self`, it never deep-copies it.

### Second-class: a view may not outlive the thing it views.

A view exists **only as a function parameter or a local binding**. It may *not*
be returned, stored in a struct field, or placed in a collection. That single
restriction removes lifetimes — a view is structurally incapable of escaping the
scope of its target, so there is nothing to annotate or infer.

```vilan
struct Holder { view: &i32 } // illegal: a field cannot hold a view
let xs: List<&i32> = ...;     // illegal: a collection cannot hold views
```

(The one *sound* escaping case — returning a view derived from an argument — is
recovered by the *Projections* section below, not forbidden.)

### Mutating through a view: parameters and methods.

"Let a function or method modify my value in place" is just a writable-view
parameter:

```vilan
fun scale(target: &mut i32, factor: i32) {
	*target *= factor;
}

mut n = 5;
scale(&mut n, 4);
print(n); // 20
```

A mutating method takes `&mut self`; the receiver is viewed at the call site and
must be `mut`:

```vilan
impl Option<type T> {
	fun insert(&mut self, value: T) {
		*self = Some(value); // replaces the whole value in place
	}
}

mut a = None;
a.insert(5);
print(a); // Some(5)
```

The method lives on `T` and `&mut self` cannot escape the call, so no
first-class reference type is ever needed.

### Why second-class is right for the native future.

A non-escaping view is precisely a Rust `&`/`&mut` whose lifetime is always
elidable, so second-class views lower to Rust references *for free* — the
WASM/Rust backend never surfaces a lifetime, because there are none. On the JS
backend a view lowers to a `(base, key)` pair (see *Backend lowering*).

---

## 4. Safety rule: no *invalidating* mutation under a live view.

Rust forbids `&` and `&mut` from coexisting at all. That is stronger than memory
safety needs, and it is the restriction this language declines. Aliasing is
fine. Mutating *through* aliases is fine. The only thing forbidden is mutation
that would **invalidate an outstanding view** — that is, **resize, reassign,
move, or drop** a value while a view into it (or into its interior) is live.

The hole this closes is invisible on the GC'd JS backend and fatal on native:

```vilan
mut list = [ 1, 2, 3 ];
let v = &mut list[0];  // writable view into element 0
list.push(4);          // ILLEGAL: structural mutation while `v` is live
                       // (in native, push may reallocate and dangle `v`)
*v = 10;
```

For second-class views the check is purely lexical — a view's live range is its
enclosing block — so this is a cheap static analysis, no runtime exclusivity
machinery.

**Permitted** (these don't invalidate anything):

```vilan
// two writable views to the same scalar — aliased writes are fine
mut a = 0;
let c = &mut a;
let d = c; // copying a view is fine; it's still second-class
*c = 40;
*d = 50;
print(i"{a}, {c}, {d}"); // 50, 50, 50

// iterate by writable view; element writes don't resize the list
mut list = [ 1, 2, 3 ];
for &mut item in list {
	*item *= 10;
}
print(list); // [ 10, 20, 30 ]

// view a payload in place; we mutate through it, we don't reassign `val`
mut val = Some(10);
match val {
	Some(&mut x) => { *x = 20; }
	None => {}
}
print(val); // Some(20)
```

**Forbidden** (each would invalidate a live view):

```vilan
for &item in list { list.push(*item); } // resizing `list` while iterating it
match val { Some(&x) => { val = None; *x } None => 0 } // reassigning `val` under a readonly view of its payload
```

> Note the deliberate looseness versus Rust: two simultaneous *writable* views to
> one value are allowed. That is memory-safe (the owner still frees it exactly
> once) — it just reintroduces the aliasing footgun Rust prevents. We accept
> that trade for ergonomics; we do **not** accept invalidation, which is the
> part that is actually unsound.

**Interaction with rule 2:** a live view counts as a use. Eliding `mut b = a`
into a storage-reuse of `a` while `let v = &a` is still live would corrupt `v`,
so "last use" means "last use *including through views*." (Sound by
construction: when in doubt, copy.)

---

## Shared mutable state (the escape hatch)

Value semantics + second-class views **cannot** express genuinely shared,
mutable, multiply-owned heap data: graphs, doubly-linked lists, observer
networks, caches keyed into from many places. No amount of cleverness changes
that — a value has one owner, and a view can't escape to become a second one.

Rather than weaken the core, shared mutability is **opt-in and visible in the
types**, via one of two tools.

### Preferred: an arena with handle indices.

The arena owns every node as one plain value; cross-node "references" are
`NodeId` handles, not pointers. The whole graph is therefore an ordinary owned
value — copyable, movable, freed in one shot — with no aliasing at all.

```vilan
// A directed graph.
struct NodeId {
	index: u32,
	generation: u32, // bumped when a slot is reused, so stale handles are caught
}

struct Node {
	value: i32,
	edges: List<NodeId>,
}

struct Graph {
	nodes: List<Node>,
}

impl Graph {
	fun new(): Graph {
		Graph { nodes = List::new() }
	}

	fun add(&mut self, value: i32): NodeId {
		let index = self.nodes.length();
		self.nodes.push(Node { value, edges = List::new() });
		NodeId { index, generation = 0 }
	}

	fun connect(&mut self, from: NodeId, to: NodeId) {
		// mutate one node in place, addressed by handle
		self.nodes[from.index].edges.push(to);
	}

	fun set_value(&mut self, id: NodeId, value: i32) {
		self.nodes[id.index].value = value;
	}

	fun value_of(self, id: NodeId): i32 {
		self.nodes[id.index].value // a copy of a scalar — cheap
	}

	fun neighbors(self, id: NodeId): List<i32> {
		mut result = List::new();
		for edge in self.nodes[id.index].edges {
			result.push(self.value_of(edge));
		}
		result
	}
}

fun main() {
	mut graph = Graph::new();

	let a = graph.add(1);
	let b = graph.add(2);
	let c = graph.add(3);

	graph.connect(a, b);
	graph.connect(a, c);
	graph.connect(b, c);

	// One write through the arena is visible to everyone holding `b`.
	graph.set_value(b, 20);

	print(graph.neighbors(a)); // [ 20, 3 ]
}
```

Why this is the recommended shape:

- **Sharing without aliasing.** A `NodeId` is a `u32` pair, freely copied, stored
  in fields and lists (it is a value, not a view) — yet there is exactly one
  owner of the actual node data: the arena.
- **Safe deletion.** Removing a node bumps that slot's `generation`; a handle
  carrying the old generation is detected as stale on access (returns `None` /
  traps) instead of silently reading a recycled node — no dangling, no GC.
- **Portable and serializable.** Handles are integers, so an arena round-trips to
  disk or across the WASM boundary unchanged; pointers never would.
- **Cycles are free.** `a → b → c → a` is just handles in lists; there is nothing
  to leak.

Doubly-linked lists, trees with parent pointers, and observer graphs are all the
same pattern: store the handle, not the thing.

### When there is no single owner: `Shared<T>`.

If ownership is genuinely diffuse — several independent subjects each holding the
*same* mutable observer, with no natural arena — use a reference-counted cell.
This is the one place reference identity and shared mutation are explicit in the
type, so the cost is legible at the use site.

```vilan
struct Counter { count: i32 }

fun main() {
	let shared = Shared::new(Counter { count = 0 });

	let a = shared.clone(); // +1 refcount — same underlying cell
	let b = shared.clone(); // +1

	a.write().count += 1;
	a.write().count += 1;

	print(b.read().count); // 2 — the write is shared

	// `shared`, `a`, `b` each release a refcount as they leave scope; the cell
	// is freed when the last release hits zero.
}
```

- `read()` / `write()` apply rule 4 *dynamically*: a `write` while any other view
  is live traps — the runtime form of "no invalidation under a live view". The
  returned handle is itself second-class.
- Reference counting is GC-free but **cannot reclaim cycles**; a cycle of
  `Shared<T>` must be broken with a `Weak<T>` edge. This is the single place the
  model asks the developer to think about ownership — which is exactly why arenas
  are preferred when an owner exists.

### A reusable arena in std: `Arena<T>` and `Handle<T>`

The graph above hand-rolls its slot/generation bookkeeping. That logic is the
same for every container that wants stable, deletion-safe identities, so it
belongs in std as one generic struct — *any* struct becomes the `T`, with no
per-type code:

```vilan
// First-class key. Two integers — copied freely, and (unlike a view) storable
// in struct fields and collections. That storability is the entire point.
struct Handle<T> {
    index: u32,
    generation: u32,
}

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

struct Arena<T> {
    slots: List<Slot<T>>,
    free: List<u32>,        // emptied indices, reused on the next insert
}

impl Arena<type T> {
    fun insert(&mut self, value: T): Handle<T> {
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index];
                slot.value = Some(value);
                Handle { index, generation = slot.generation }
            }
            None => {
                let index = self.slots.length();
                self.slots.push(Slot { generation = 0, value = Some(value) });
                Handle { index, generation = 0 }
            }
        }
    }

    // Checked projection: a view into the slot, but only while the handle's
    // generation still matches. `borrows self` is the provenance-return feature;
    // rule 4 forbids invalidating the arena while the returned view is live.
    fun get(&self, handle: Handle<T>): Option<&T> borrows self {
        let slot = &self.slots[handle.index];
        if slot.generation == handle.generation {
            match &slot.value { Some(&value) => Some(value), None => None }
        } else {
            None
        }
    }

    fun remove(&mut self, handle: Handle<T>): Option<T> {
        let slot = &mut self.slots[handle.index];
        if slot.generation == handle.generation {
            slot.generation += 1;       // bump → every existing handle to this slot goes stale
            self.free.push(handle.index);
            slot.value.take()
        } else {
            None
        }
    }
}
```

Two properties make this the canonical "survive invalidation" tool, and both
fall out of the model rather than being bolted on:

- **The handle is first-class; the view is not.** A `Handle<T>` is two integers,
  so it is exempt from rule 3's ban on references in fields and collections —
  which is exactly what lets a `Node` hold `List<Handle<Node>>` for its edges.
  The view returned by `get` stays second-class and rule-4-policed; the *key* you
  keep around is a plain value.
- **You traverse by re-`get`, so you can mutate while traversing.** Holding a key
  and re-taking a view on each access (rather than holding a `&T` across a
  mutation) means `arena.insert(...)` mid-traversal is legal — you simply weren't
  holding a live view at that instant. Handles sidestep, by construction, the one
  thing rule 4 would forbid.

**The escape ladder.** "Keep the value past an invalidating mutation" has three
rungs for three different needs, cheapest first:

1. **Clone past the view** — `let snapshot = *first;`, then mutate freely. A
   *detached value*: no identity, blind to later updates and deletion. Zero std
   machinery; the right answer when you only wanted the data.
2. **`Arena<T>` + `Handle<T>`** — a *durable identity* into the live container:
   re-`get` sees updates, deletion surfaces as `None`.
3. **`Shared<T>`** — genuinely diffuse co-ownership; dynamic exclusivity, `Weak`
   for cycles.

Clone gives a snapshot, the arena gives an identity, shared gives co-ownership.
Most "I want it to survive the push" cases are really rung 1.

**Trait, later.** A `Store<T>` trait (`insert`, `get(...) borrows self`,
`remove`) would let generic algorithms span any arena-like backing — but it
earns its keep only once a *second* implementation exists (a `Slab`, a typed
component store). Ship `Arena<T>` concrete; promote the interface to a trait the
moment a second store appears, not before. (Same discipline as deferring the
generic effect engine until async gave context a second instance.)

**One language prerequisite:** `Handle<T>` uses `T` purely for type safety (so a
`Handle<Node>` can't be passed to an `Arena<Edge>`) — it appears in no field, so
the compiler must permit an unused type parameter (equivalently, a zero-size
`Phantom<T>` marker). Either mechanism works; the only non-option is an untyped
handle, which loses cross-arena safety.

---

## Projections: returning a view into an argument

Rule 3 forbids *returning* a view, only passing one. That ban is what deletes
lifetimes — but it also blocks the most useful shape there is: a function or
subscript that hands back a view into one of its arguments. `arena.node(id):
&Node`, `list[i]: &T`, `tree.find(key): &Value`, and the mutable iterator
`next(&self): &T` all need exactly this. Without it, the only way to mutate
through a returned thing is `Shared<T>`, which is far too heavy for ordinary
indexing and iteration.

The relaxation is: **a function may return a view, provided the compiler can
prove which argument(s) the view borrows.** That provenance is inferred and
frozen onto the signature — the same way async-ness and context-ness are
inferred effects in this compiler — and written (or shown) as a `borrows`
clause:

```vilan
impl List<type T> {
    fun at(&mut self, index: i32): &mut T borrows self {
        &mut self[index]   // a subscript access produces a view tagged @ {self}
    }
}

mut list = [ 1, 2, 3 ];
let v = list.at(0);   // v: &mut i32 @ {list}
*v = 10;              // writes back into list[0]
print(list);          // [ 10, 2, 3 ]
```

### Why this is lighter than lifetimes

Rust's lifetimes encode two things: **provenance** ("which storage does this
alias") and **duration** ("for how long; what outlives what"). All the pain —
variance, `'a: 'b` outlives relations, higher-ranked bounds, lifetime
parameters threaded through every type — is duration. This model's only safety
rule is *no invalidating mutation under a live view*, and enforcing it needs
provenance and **liveness**, but never duration:

- **Provenance** is an analysis fact, a set of origin roots (`v @ {list}`), not
  a type. No variance, no outlives lattice, nothing in the signature but the
  inferred `borrows` summary.
- **Liveness** is already computed — it is the same last-use analysis that
  drives copy elision (rule 2).

Enforcement is then just: while any value with origin `{x}` is live, `x` may not
be invalidated (resized, reassigned, moved, or dropped). And this is the alias
fact rule 4 needs *anyway*:

```vilan
let v = &mut list[0];   // v @ {list}
list.push(4);           // push may realloc → invalidates {list} while v is live → error
*v = 10;
```

Without provenance the compiler couldn't police that. So tracking origins is not
new machinery added to permit returns — it is the information rule 4 already
depends on, now made explicit at the return boundary. Dangling temporaries need
no separate check either: `let v = make_temp().at(0)` tags `v @ {temp}`, and
`temp` dropping while `v` is live is just drop-under-live-view — already a
rule-4 violation. The compiler either extends the temporary's life or rejects
it; no duration arithmetic.

### The line that must hold

Relax *return*; keep the ban on **storing a view in a struct field or
collection**. Return is tractable because provenance maps parameters to
arguments at a known call site, where the concrete origins exist. The moment a
view lives in a field, the aggregate carries a provenance tag and origins flow
through types — the on-ramp back to lifetimes-in-types. Return: yes. Storage in
aggregates: still no.

### What this buys

Mutable iteration over a *user-defined* container works through ordinary
external iteration, with no GAT and no `Shared<T>`:

```vilan
fun next(&mut self): Option<&mut T> borrows self   // "borrows self" is the whole annotation

for &mut item in tree {     // each item: &mut T @ {iter} @ {tree}
    *item *= 10;            // mutates real storage
}                           // mutating `tree` mid-loop = push-under-live-view = rule-4 error
```

Because there is no lifetime *type* to be generic over — only an origin summary
that says "borrows self" — the lending-iterator problem that forces Rust into
GATs simply does not arise. And unlike an internal/callback iteration protocol,
provenance-return carries **no control-flow tax**: `break`, `continue`,
`return`, and `?` inside the loop behave normally.

### Storage vs. computed projections

The model above returns a view into storage that actually exists. A *computed*
projection — a bit-packed field, an insert-on-write `dict[key]`, a setter that
must re-validate an invariant after the write — has no slot to point at; the
"view" is a temporary that must be **written back** through code after mutation.
That case is handled by lowering the access to a coroutine/closure transform:
the accessor runs up to a `yield`, the caller's `*v = …` is spliced in as the
continuation, then the after-`yield` code reconstitutes the value (this is
Swift's `_modify` / Hylo's `subscript`). It is the same surface syntax
(`: &T borrows self`); only the lowering differs — return-the-address for
storage, coroutine for computed. The computed case is **deferred** until a
feature needs it (bit fields, insert-on-write); the storage case above is the
one to build first.

---

## Backend lowering on JavaScript

The model is defined for a backend with addresses and deterministic destruction
(native / WASM linear memory). JavaScript has neither, so three things need a
stated lowering.

### Views lower to a `(base, key)` pair

A view is *not* an address on JS; it is a base object plus an accessor key, and
`*v` means `base[key]`:

- `&point.x` → `base = point` (already an object), `key = "x"`
- `&list[i]` → `base = list` (already an array), `key = i`
- `&local` → a bare local has no container, so **box it** into a one-slot cell
  `{ v: 0 }`; then `base = cell`, `key = "v"`. Reads of the local become
  `cell.v`.

So boxing a primitive local is not a separate mechanism — it just gives a bare
local a base so it fits the `(base, key)` shape. Two consequences keep this
cheap:

- **Box only when viewed.** A local is boxed only if it is ever the target of
  `&`; ordinary `mut` locals stay raw JS numbers and pay nothing. (Structs and
  lists are already mutable objects, so `&field` / `&element` need no extra
  allocation.)
- **The pair is usually scalar-replaced.** Because views are second-class and
  cannot escape (rule 3), the compiler passes `base` and `key` as two ordinary
  values rather than materializing a pair object — e.g. `insert(self_base,
  self_key, value)`. The only real allocation is the cell for a viewed primitive
  local, and that cell is reclaimed by the host GC — no manual cleanup.

**Rejected alternative — a global pool + index.** Representing a viewed primitive
as an index into one shared array looks like it saves allocations, but every
cost is a problem already triaged away: freeing a slot requires knowing when the
local *and all its views* are dead (the deferred destruction problem, now on a
common operation); reusing slots needs a free-list plus generations to avoid
ABA — i.e. re-implementing `Arena<T>` in codegen for every program; and a global
index maps to nothing on the native side, forcing two unrelated view lowerings.
The `(base, key)` cell, by contrast, is GC-reclaimed on JS and maps directly to
a stack/heap slot address on native.

### Rule 4 is enforced on JS even though it has no runtime meaning there

JS arrays never realloc or dangle, so a rule-4 violation is harmless on JS and
UB only on native. The checker runs **on the JS target anyway**, so the model is
single-conformance: *if a program compiles for JS, it has no aliasing UB on
native.* Every JS build doubles as a native-portability check. The cost is
rule-4 friction on a backend that does not strictly need it — accepted
deliberately, in exchange for predictability and one set of rules everywhere.

### Deterministic destruction is deferred

GC-free deterministic destruction (the `Shared<T>` refcount release, closing a
`File`, any user `drop`) has no native JS equivalent and would need
`using` / `try-finally` instrumentation at every owning scope
(`FinalizationRegistry` is non-deterministic and not acceptable). This is
**deferred** — and choosing the `(base, key)` cell above is what keeps the defer
valid, since GC reclaims cells with no drop obligation. Pure value types and
arenas are all GC-reclaimable, so deferral costs nothing today.

**Tripwire:** revisit before the first type with a *non-memory* drop obligation
(`Shared<T>`, `File`, sockets) targets the JS backend — at that point the
`using` / `try-finally` lowering must exist.

---

## Summary

| Aspect                       | Rule                                         | Cost it avoids                                   |
| ---------------------------- | -------------------------------------------- | ------------------------------------------------ |
| Owned values                 | semantic copies + last-use elision           | move checker, use-after-move, lifetimes          |
| Sharing a value mutably      | second-class `&` views                       | first-class references, lifetimes                |
| Safety                       | no invalidating mutation under a live view | aliasing-XOR-mutability (Rust's restrictiveness) |
| Shared/cyclic mutable graphs | arena + handles (or `Shared<T>`)             | a tracing garbage collector                      |

Everything a typical program does — locals, structs, collections, passing things
to functions to be modified, iterating mutably — lives in rules 1–4 with no
annotations. The genuinely hard cases (shared mutable graphs) are *opt-in,
localized, and visible in the types*, instead of forcing their complexity onto
the whole language. That is the difference between "simple until it isn't" and
"a small core with a clearly-marked advanced corner."

## Open questions

- **`borrows` clause surface** — is provenance inferred and silent, or written on
  the signature? And does subscript projection (`self[i] @ {self}`) need any
  annotation, or is it always inferable?
- **Computed projections** — when to build the coroutine-lowered case (bit
  fields, insert-on-write); the storage case ships first.
- **`Store<T>` trait** — extract only once a second backing (a `Slab`, a typed
  component store) exists.
- **Deterministic destruction** — deferred; see the tripwire in *Backend
  lowering*.
