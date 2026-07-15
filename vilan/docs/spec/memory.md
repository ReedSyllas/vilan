# Spec §6 — The memory model

Four rules (design: `proposal/memory-management-rev-1.md`). Rules 1–3 and
rule 4's static half are normative and enforced; rule 4's dynamic
remainder is future work, marked below.

## 6.1 Rule 1 — values are copied; copies are semantic

Every binding, assignment, argument pass, field initialization, and
return **copies** the value. After `mut b = a`, mutating `b` never
affects `a` — for primitives, structs, enums, tuples, lists, and every
other value type alike:

```vilan
import std::print;

struct Point { x: i32, y: i32 }

fun main() {
	mut a = Point { x = 4, y = 6 };
	mut b = a;
	b.x = 10;
	print(a.x);   // 4 — b is a semantic copy
}
```

## 6.2 Rule 2 — elision is an optimization, never observable

An implementation may skip a copy (reuse the storage) when no conforming
program can tell — e.g. when the source is never used again. Elision must
not change any program's output; a live view of the source counts as a
use. (This rule licenses the JS backend to alias under the hood; it
grants programs nothing.)

## 6.3 Rule 3 — references are second-class views

`&place` / `&mut place` create a **view**: an alias of a place, readonly
or writable. Views are values of view type (`&T`, `&mut T`) but are
deliberately second-class — a view may not outlive the thing it views:

- A view may be a **parameter** (the caller's place is lent for the
  call), a **short-lived local**, or a **return projecting a parameter**
  (§6.5). One vocabulary, position-dependent defaults: `&mut self` on a
  method, `bump(&mut c)` at a call site, `x: &mut T` in a signature all
  carry the same convention.
- A view may **not** be stored in a struct field, a collection element,
  or a `Signal`/`Shared` payload; may not be returned except through a
  `borrows` projection; may not be captured by a closure that outlives
  the place; may not cross an `await` (§6.6).

Mutating through a view writes the viewed place; reading its value
requires an explicit `*`. A view in **value position** — passed where a
value is expected, used as an operator's operand, or bound to a value
type — is a compile error, never a silent coercion to the pointee (so the
`(base, key)` representation of a scalar view can't leak); write `*v` to
copy the value out. Iteration by view (`for e in &mut list`) binds each
element as a view — assignment and field writes go through; `*e` reads the
element. The parameter conventions:

| Convention | Written | Meaning |
|---|---|---|
| bare | `x: T` | by value (a copy — rule 1) |
| own | `own x: T` | by value, explicitly (documentation of intent) |
| ref | `&x` / `x: &T` | readonly view |
| ref mut | `&mut x` / `x: &mut T` | writable view |

## 6.4 Rule 4 — no invalidating mutation under a live view

While a view of a place is live, mutations that would **invalidate** the
view (replacing the aggregate that contains the place, removing the
element it points into, resizing past it) are forbidden. The compiler
enforces the statically-decidable half: assignments to the viewed root or
an enclosing place while a view is live are rejected.

*Implementation note: the dynamic remainder (aliasing reached through
calls, container-internal invalidation) is tracked future work;
`Shared.read()/write()` apply rule 4 dynamically at the cell (a `write`
view is exclusive for its statement).*

## 6.5 Projections: `borrows`

A function may return a view **into one of its parameters** — the one
sanctioned escape from rule 3's return ban. The projected parameter is
named by a `borrows` clause, which is **inferred** when the body makes it
evident (a method returning a view of `self` needs no clause):

```vilan,fragment
fun write(self): &mut T borrows self;   // Shared::write — explicit
fun get(&mut self, i: i32): &mut T      // inferred: borrows self
```

At the call site the returned view obeys the same second-class rules,
with the borrow anchored to the projected argument: the argument's place
is treated as viewed while the result is live (rule 4 applies to it).
Returning a view of a **local** is always an error (it would dangle).

`Option<&T>` is permitted as a return type for "a view, maybe" (map
lookups); the `Some` payload obeys the same anchoring. An `Option<&mut T>`
may also be built **inline as a transient** and matched in the same
expression — `match Some(&mut a) { Some(let v) => … }`, including the
conditional form `match if c { Some(&mut x) } else { None } { … }` and
forwarding a bare view parameter (`match Some(p) { … }` for `p: &mut T`).
Because the transient never outlives the `match` that consumes it, its
payload may view a **local** (unlike a returned projection). Binding the
same constructor to a `let` stores the view and is rejected.

## 6.6 Views and suspension

**A view may not be live across an `await`.** Between suspension and
resumption other code runs and may invalidate any place; rather than
extend rule 4 across turns, the language forbids the shape. Re-derive
the view after the suspension:

```vilan,fragment
let row = &mut rows[index];
send(row.id);              // suspends
row.text = "sent";         // ✗ error: view live across await

send(rows[index].id);      // ✓ re-derive
rows[index].text = "sent";
```

This applies to every suspension point: calls to async functions
(implicitly awaited, §7), explicit `await`, and calls through async
closure values.

## 6.7 Library escape hatches (informative)

`Shared<T>` (one shared cell; `read()` copies, `write()` yields an
exclusive statement-scoped view) and `Arena<T>`/`Handle<T>` (stable,
generation-checked identities — handles are plain values, storable where
views are not) are std types built on these rules, not extensions of
them. See [cells](../std/cells.md).
