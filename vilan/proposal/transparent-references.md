# Transparent references — implicit place, explicit value

**Status:** **implemented** (2026-06-21). Landed: **R5** (assign through a view with no `*`,
plain + compound), **R6** (`*` rejected as an assignment target), **R1** (annotation view-ness must
match the initializer), **R7** (no `mut` view binding), **R8** (no implicit borrow at `&`/`&mut` call
arguments — the method `self` receiver is exempt; this also fixed a pre-existing broken-codegen bug
for bare scalars). **A1 (`Shared.write: &mut T borrows self`) is done** (backlog A1) — it built on
R5 here. The one remaining follow-up is constructing/matching an `Option<&mut T>` view **inline as a
transient** (`match Some(&mut a)`) — see Open questions. The conformance example is committed as
`vilan/test/transparent-references.vl` + the `transparent_references_*` / `r8_*` inference tests.

This is a *surface* change to how second-class views are read, written, and bound; it does **not**
change the lifetime/escape model (second-class views, `borrows`, the position-default conventions)
at all.

## Motivation

Today a `&mut T` view needs an explicit `*` for every interaction — both reads (`*v`) and writes
(`*v = x`, `*obj.slot() = x`). The write-side `*` is pure ceremony: there is only one sensible
meaning for "assign to a view." Removing it makes the common patterns — `&mut` parameters, in-place
container mutation, and especially `Shared::write()` — read naturally:

```
self.value.write() = value;          // not  *self.value.write() = value
self.subscribers.write().push(sub);  // not  (*self.subscribers.write()).push(sub)
a.write().count = a.write().count + 1;
```

while keeping value extraction explicit, so copy-vs-alias is never guessed by the compiler.

## The one idea

A view is a distinct type from the value it refers to, and the language never silently converts
between them. Instead:

- A view is **implicitly a place**: you assign *through* it and project *through* it (`.field`,
  `.method`) with no operator.
- A view's **value is explicit**: `*v` reads a *copy* of the referent (an rvalue). It is the only
  way to cross from view to value, and it can never appear on the left of `=`.

So `*` stops meaning "dereference (read or write)" and starts meaning, unambiguously, "the value in
here, copied out." Writes lose their `*`; reads keep theirs.

## Rules

Let an expression's type be either a **value type** `T` or a **view type** `&T` / `&mut T`.

- **R1 — Distinct types, no coercion.** `&T` / `&mut T` are not `T`. There is no implicit
  conversion in either direction. `let b: i32 = x` (view into value slot) and
  `let b: &mut i32 = *x` (value into view slot) are type errors.
- **R2 — Borrow.** `&e` / `&mut e` borrows a *place* `e`, producing a view. `&mut` requires `e` to
  be a mutable place. `&mut e` where `e` is already a view (a view of a view) is an error. *(As
  today.)*
- **R3 — A bare view is the view.** A view-typed binding, parameter, pattern capture, or call
  result denotes the view value. Binding it (`let a = x`), passing it to a view parameter (`f(x)`),
  or returning it forwards/aliases the same referent — subject to the unchanged second-class/escape
  rules.
- **R4 — Member access projects a place.** For `e : &[mut] U`, `e.field` and `e.method(args)`
  auto-deref `e` to its referent place and project; the result is a **place** (no copy). This is
  what lets `cell.write().push(z)` and `a.write().count = …` read cleanly.
- **R5 — Assignment writes through.** If the target of `=` / `op=` is a view-typed place, the
  assignment writes (or read-modify-writes) through to the referent: `x = v`, `x += v`. Compound
  operators act directly on the place, like any place — no `*`.
- **R6 — `*` is value extraction, rvalue-only.** For `e : &[mut] U`, `*e` is an rvalue: a copy of
  the referent value (a deep copy for aggregates, per value semantics). It is required wherever a
  view is used as a plain value (`*x + 1`, `i"{*x}"`, `f_value(*x)`, `*x == *y`, `let b = *x`).
  `*e` may **not** be an assignment target (`*x = v` is ill-formed — write `x = v`).
- **R7 — No rebinding a view.** A view binding cannot be made to refer elsewhere, so `mut a = &mut x`
  is an error; use `let`. (Referent mutability lives in `&mut` vs `&`, so `mut` on a view binding
  would be meaningless anyway.)
- **R8 — No auto-ref.** A value place is not implicitly borrowed for a view parameter: `f(a)` where
  `f(x: &mut U)` is an error; write `f(&mut a)`. Only an existing view forwards bare (R3).

### Why uniform `*` on reads (the live decision)

R1 + R6 require `*` on *every* view-as-value read, not only at bindings. The alternative
("auto-deref a view wherever the context can only be a value — arithmetic, interpolation, by-value
args — and require `*` only where a binding could go either way") is rejected: it reintroduces
exactly the context-driven, type-directed coercion this proposal exists to remove, for the sake of
dropping a `*` that the current corpus already writes. Uniform `*` keeps one rule — *`*` is the only
view→value crossing* — and makes `*x = v` ill-formed, which is what enforces "no `*` on the left of
`=`."

## Conformance example

This is the canonical regression test (your example, with R6's `*` on the value-reads):

```
mut a: i32 = 10;
let b: &mut i32 = &mut a;
let c: &mut i32 = b;            // R3: alias — c forwards the same referent

b = 20;                        // R5: write-through
print(i"a = {a}, b = {*b}, c = {*c}");   // a = 20, b = 20, c = 20

fun f(x: &mut i32) {
    x += 10;                   // R5: write-through (compound op on the place)
}

f(&mut a);                     // R2: fresh borrow of `a`
print(i"a = {a}, b = {*b}");   // a = 30, b = 30

f(b);                          // R3: forward the existing view
print(i"a = {a}, b = {*b}");   // a = 40, b = 40

fun g(x: &mut i32): &mut i32 borrows x {
    x                          // R3: forward (return the view)
}

g(c) /= 10;                    // R5: write-through the returned view
print(i"a = {a}, b = {*b}");   // a = 4, b = 4

match Some(&mut a) {
    Some(x) => { x += 8 }      // R3 capture + R5 write-through; x : &mut i32
    None => {}
}
print(i"a = {a}, b = {*b}");   // a = 12, b = 12
```

Contrast (all type errors, by R1/R6):

```
let b: i32 = x;        // view into a value slot — write `*x`
let b: &mut i32 = *x;  // value into a view slot — drop the `*`
*x = 5;                // `*x` is an rvalue — write `x = 5`
f(a);                  // R8 — write `f(&mut a)`
```

## Lowering

No new runtime representation: a view is still a `(base, key)` pair (scalar place) or the object
reference (aggregate), exactly as today. Only where `*` sits in the AST moves.

- `x = v` / `x op= v` (view-typed place) lowers like today's `*x = v` did: a scalar place is a slot
  write `base[key] = v`; an aggregate whole-write copies fields in place. The analyzer routes this
  by the target's *view type* instead of by a `Dereference` node.
- `*x` (rvalue) lowers like today's `*x` read: `base[key]` for a scalar, the referent for an
  aggregate (value semantics inserts the deep copy at the binding site via the existing clone-site
  analysis).
- `x.member` derefs the view to its referent place, then projects — e.g. `cell.write().push(z)` →
  `cell.v.push(z)` (the `SharedValue` intrinsic already yields `cell.v`).
- A view-returning **call** used as an assignment target or `op=` target is bound to a temp once, so
  `g(c) /= 10` evaluates `g(c)` a single time (the transformer already does this for `*call`).

## Resolved `Shared::write` (A1) — done

`fun write(self): &mut T borrows self` shipped on top of R5. Every call site stays clean — no `*`:
`self.value.write() = value` (R5 write-through), `self.subscribers.write().push(sub)` (member access
on a pointee-typed view), `a.write().count = …`. Implementation (see backlog A1): `external`
functions record `borrows` (so `c.write()` is a recognized view), and the `SharedValue` intrinsic
split into read/`SharedWrite`. Both still lower to `cell.v` — so member access works with no
auto-deref — and a write *through* the `SharedWrite` view rebinds the slot (`cell.v = v`) rather
than `Object.assign`-merging (which is wrong for `Shared<List<…>>`). Codegen is byte-identical to the
old `write(): T`. The `borrows self`-vs-ref-counted-cell question (whether `Shared`'s view is exempt
from a future no-view-across-`await` rule) is recorded under memory item C, decided when that rule
lands.

## Migration & test matrix

Mechanical, and small: in the existing view corpus, **drop `*` from assignment left-hand sides**
(`*x = …` → `x = …`); leave `*` on value-reads untouched; add the conformance example above.

- Corpus to re-baseline: `view-basic.vl`, `view-field.vl`, `view-primitive.vl`, `view-conventions.vl`,
  `borrows.vl`, `borrows-inferred.vl`, `for-views.vl`, `for-mut-container.vl`, `option-view.vl`,
  `subscript.vl`, `side-effect-let.vl`, plus `std::shared` / `std::reactive` once A1 lands.
- Implementation surface:
  - **Parser:** allow a bare view expression as an assignment target; keep parsing `*e` (now only
    valid as an rvalue).
  - **Analyzer:** R1 no-coercion checks; R4 member-access auto-deref of a view subject (today
    `resolve_field_accessor` requires `Type::Struct` — generalize to deref a view first); R5 route
    view-typed assignment targets to write-through; R6 reject `*e` as an assignment target; R7/R8.
  - **Transformer:** key write-through off the target's view type rather than a `Dereference` node.

## Open questions for the spec (the reason D1 exists)

- **Pattern syntax for `Option<&mut T>`.** `Some(x)` binds `x : &mut T` (a view, by R3); there is no
  `ref` / `&mut` in patterns. Confirmed. **Inline transient confirmed and shipped (2026-07-14, backlog
  §C.5):** constructing an `Option<&mut T>` *inline* and matching it is allowed, not only *returning*
  one (the Phase-5 `Arena::get` shape). The direct form `match Some(&mut a) { … }`, the conditional
  form `match if c { Some(&mut x) } else { None } { … }`, and forwarding a bare view parameter
  (`match Some(p) { … }` for `p: &mut T`) all bind the capture to the view and write through; because
  the transient never outlives the `match`, a view of a *local* is sound here (unlike a returned view,
  which must project a parameter). Storing the same constructor in a `let` still escapes and is
  rejected.
- **Equality / ordering on views.** `x == y` requires `*x == *y` (value comparison) under R1/R6;
  confirm there is no view-identity comparison.
- **`*` on an aggregate.** It is a deep copy by value semantics; confirm it participates in the
  existing copy-elision (move when the source is dead) rather than always cloning.
