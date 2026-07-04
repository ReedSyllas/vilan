# Variadic generics via mapped tuples over flat storage

> **Status: implemented** — see [`variadic-generics-plan.md`](variadic-generics-plan.md)
> (shipped, commits bc360e9…3d00f5c): `combine` works end to end and is load-bearing in the
> reactive UI and the todo example. `keyof`/spread-params/elision remain deferred (the
> plan's own deferred section).

> Supersedes the parameter-pack exploration (previous commit). The driving
> example is unchanged — `std::reactive::combine`, the product combinator the
> reactive UI needs (`reactive-ui/todos.vl` imports it; `README.md` flags it as
> "not yet built — needs variadic generics"). The mechanism is different: instead
> of dedicated pack syntax, three orthogonal, independently-useful tuple features
> — **arity-bounded generics, mapped tuple types, and tuple comprehensions** — out
> of which `combine` falls as an ordinary function.

```vilan
//              ┌─ T is a tuple of arity ≥ 2
fun combine<T: (2..)>(sources: (U in T: Source<U>)): Source<T> {
//                             └─ for each element U of T, this slot is Source<U>
    let snapshot = || (source in sources = source.get());   // a tuple comprehension
    let derived = Source::new(snapshot());
    for (_, source) in sources {                            // iterate a tuple
        source.sub(|| derived.update(snapshot()));
    }
    derived
}

let visible = combine((items, filter));        // Source<(List<Todo>, str)>
visible.derive(|(list, filter)| { … });        // destructures with binders shipped already
```

`combine` is not special-cased: it is a normal function written in the language
over a mapped-tuple parameter. The same features give `zip`, a tuple `map`,
element-wise `into`, etc.

---

## The load-bearing mechanism: monomorphization → compile-time unrolling

Iterating a heterogeneous tuple, mapping a template type per element, indexing a
slot whose type depends on the index — these are normally **dependent typing**.
In Vilan they are not, because **`T` is concrete at every monomorphization**, so
every tuple map / loop **unrolls at compile time**:

```vilan
for (_, source) in sources { source.sub(…); }   //  T = (A, B, C)
// emits, in the A,B,C variant:
sources[0].sub(…); sources[1].sub(…); sources[2].sub(…);
```

The body is *type-checked once* against the abstract element type `U` (its trait
bound, if any), then *expanded N times* against the concrete elements when the
call fixes `T`. This is exactly how generic functions already work — analyze
once, specialize per call — and it is what makes the whole feature affordable.

---

## Runtime model: flat storage, but **distinct types**

**Flat lowering.** A nested tuple lowers to a flat array at construction, using
each element's statically-known width:

```vilan
let a = (1, 2);
let b = (a, 3);
// today:  const b = [a, 3];           ->  [[1,2], 3]
// flat:   const b = [a[0], a[1], 3];  ->  [1, 2, 3]
```

So `(A, B, C)`, `((A, B), C)`, and `(A, (B, (C, ())))` are the **same value at
runtime** — coercing between them is zero-cost *when requested*.

**Types stay distinct (the deliberate choice).** Equal runtime layout does *not*
make them the same type. `(Point, Color)` with `Point = (i32, i32)`,
`Color = (i32, i32, i32)` is **not** interchangeable with `(i32, i32, i32, i32,
i32)` — nesting is a real type boundary, so a 5-int tuple cannot silently satisfy
a `(Point, Color)` parameter. Flattening happens **only** through an explicit
spread, a comprehension, or an explicit annotation — never implicitly. This keeps
the perf win without dissolving the type system's structure.

**Costs.** Construction copies the flattened elements (a later optimization elides
the copy when a source operand is dead — *deferred*, needs liveness/move
analysis). Extracting a sub-tuple is the reverse — `let (a, rest) = x` where
`rest: (B, C)` reslices `[x[1], x[2]]` (copy or view).

---

## Surface syntax

### Arity & element bounds — `T: (..)`

A tuple bound on a generic: an arity range, optionally with a per-element bound.

```vilan
T: (..)            // any tuple (incl. () and 1-tuples)
T: (2..)           // arity ≥ 2
T: (..10)          // arity ≤ 10
T: (2..10)         // 2 ≤ arity ≤ 10
T: (..: Display)   // any tuple, every element bound by Display
T: (2..: Into<bool>)   // ≥ 2 elements, each Into<bool>
```

The element bound is what lets a uniform body type-check (`item.to_string()`
needs `(..: Display)`; `item.into()` needs `(..: Into<_>)`).

### Mapped tuple types — `(U in T: F<U>)`

A tuple type built by mapping a **single-hole** template over `T`'s elements:

```vilan
(U in T: Source<U>)   //  T = (A, B)  ->  (Source<A>, Source<B>)
(_ in T: bool)        //  ignore the element, every slot is bool
(U in T: Readable<U>) //  trait template => an EXISTENTIAL slot (any impl)
```

A template is either a **concrete constructor** (`Source<U>` — the exact slot
type) or a single **trait** (`Readable<U>` — an existential slot, "any type
satisfying it", the form `combine` uses). Map preserves arity. The single-hole
restriction keeps inference decidable (see below); multi-hole templates
(`Map<U, V>`, `U::Assoc`) are **deferred**.

### Tuple comprehensions (value) — `(x in xs = e)`

The value-level twin of the mapped type — `:` ascribes a type, `=` binds a value:

```vilan
(source in sources = source.get())   //  (s0.get(), s1.get(), …)
(item in items = item.into())
```

### Tuple iteration — `for (_, item) in items`

Yields `(key, element)` per element; the body is unrolled per element at
monomorphization. The `key` slot is a placeholder in the core (`_`); a usable
`keyof`-typed key for in-place indexing is **deferred** (below).

### Explicit flatten / concat — spread `..`

The *only* way nested becomes flat, in both type and value position:

```vilan
(..T, U)       // type:  append U to the elements of tuple T
(..a, b)       // value: a = (x, y)  ->  (x, y, b)
```

---

## Inference

`combine((a, b, c))` must infer `T` **backwards** through the mapped parameter
type: given the concrete argument `(Source<A>, Source<B>, Source<C>)`, solve
`(U in T: Source<U>)` for `T = (A, B, C)`. With a single-hole template this is
element-wise unification — reconcile each concrete slot `Source<Ai>` against the
template `Source<U>` to bind `U = Ai`, collect `T` — reusing the existing
`reconcile_type` tuple path (`analyzer.rs:5202`). Multi-hole templates would make
the inversion ambiguous, hence the single-hole rule.

For a **trait template** (`Readable<U>`) the inversion is by impl resolution
instead of structural unification: find `U` such that the concrete slot type
implements `Readable<U>`. Unambiguous precisely because `Readable` has no blanket
impl (one impl per type) — the reason a bespoke trait is chosen over `Into`
(below).

---

## Source vs Signal inputs — a bespoke `Readable<T>` trait

`combine`'s inputs are heterogeneous in a second way: `todos.vl` passes
`Signal`s, but a `Signal<T>` is a writable root, not a `Source<T>` (it *wraps*
one in `.node`). Two ways to let a slot accept either:

1. **A bespoke trait** both implement — `Readable<T>`, with one bridge method.
2. **`Into<Source<U>>`** — `Signal` converts to its `Source`; `Source` is covered
   for free by the reflexive blanket `impl type T with Into<T>` (`std/into.vl`).

Option 2 is more elegant *in principle* (no new trait; `Source` needs no impl),
and would be my pick **but for a verified blocker**: target-directed dispatch
through the blanket impl doesn't work today. With both `Into<Wrap>` (the reflexive
identity) and an explicit `Into<i32>` in scope, `let x: i32 = w.into()` resolves to
the *identity* impl and errors `Expected i32, but got Wrap` — the annotation
doesn't steer impl selection. Resolving `Signal<A>: Into<Source<A>>` hits exactly
this: the identity `Into<Signal<A>>` competes and wins. Fixing it is a separate
analyzer change in the generic-resolution cluster ([[analyzer-stabilization]]).

A **bespoke trait sidesteps it entirely** — `Readable<T>` has no blanket impl, so
each type has exactly one impl and per-element resolution is unambiguous (verified:
a `first<R: Readable<i32>>` dispatches cleanly for two distinct implementors).
**Recommendation: Option 1.** It also formalizes the README's claim that
"`Source<T>` is the read interface every reactive value implements." Revisit
Option 2 if/when blanket-impl dispatch is fixed — `combine` would then need no
change beyond dropping the trait.

```vilan
// std::reactive — the read interface both a Source and a Signal satisfy.
trait Readable<T> {
    fun as_source(self): Source<T>;
}
impl Source<type T> with Readable<T> { fun as_source(self): Source<T> { self } }
impl Signal<type T> with Readable<T> { fun as_source(self): Source<T> { self.node } }
```

## Worked: the reactive combinators

```vilan
// product — the driving example, keyof-free. A slot is any Readable<U> (a trait
// template => an existential slot, inverted through its single impl per type).
fun combine<T: (2..)>(sources: (U in T: Readable<U>)): Source<T> {
    let nodes = (s in sources = s.as_source());     // (Source<U0>, Source<U1>, …)
    let snapshot = || (node in nodes = node.get());
    let derived = Source::new(snapshot());
    for (_, node) in nodes {
        node.sub(|| derived.update(snapshot()));
    }
    derived
}

// a homogeneous map, for contrast — element bound gives each item its method
fun labels<T: (..: Display)>(items: T): (_ in T: str) {
    (item in items = item.to_string())
}
```

`combine`'s result type is concrete at the call (inference fixes `T`), so the
downstream `.derive(|(list, filter)| …)` sees a concrete tuple and destructures
with the binders that already shipped.

---

## Deferred aspects (in scope conceptually, out of scope for the core)

- **`keyof` + indexed per-slot write** — a real key type usable as
  `v[key] = source.get()`, turning `combine`'s O(N)-per-update snapshot recompute
  into an O(1) per-slot write. Sound only where the index is statically known
  (an unrolled loop / literal); a `keyof` value that escapes to a runtime-dynamic
  position collapses `v[key]` to the union of element types. The core uses the
  recompute form and needs none of this.
- **Spread parameters — `fun f(...items: T)` / `f(1, "hi", true)`** — a second,
  varargs call convention alongside the single-tuple `combine((a, b, c))` form.
  Orthogonal; the core ships only the tuple-argument form.
- **Copy elision on flatten** — reuse a dead source operand in place
  (`const b = [a[0], a[1], 3]` without copying `a`). Needs liveness/move analysis;
  correctness holds without it (it just copies).
- **Multi-hole mapped templates** — `(U, V in T: Map<U, V>)` and associated-type
  templates. The core is single-hole.
- **Implicit structural flatten coercion** — *explicitly rejected* (see "distinct
  types"); flattening is always opt-in.

---

## Risks & mitigations

- **Flat lowering is a breaking codegen change.** It is the one stage that is
  *not* corpus-byte-identical: any file with a genuinely nested tuple regenerates
  its golden. Today that is exactly one file (`vilan/test/destructuring.vl`:
  `(1, (2, "z"))` → `[1,2,"z"]` instead of `[1,[2,"z"]]`). All tuple access is
  compiler-generated from static types, so the rewrite is mechanical; validate by
  *running* every corpus program and diffing behavior, not just the `.js`. (Heed
  [[golden-regen-rebuild-debug]] — rebuild the debug binary before regenerating.)
- **Symbolic mapped types through generics.** While `T` is abstract (inside
  `combine` before specialization), `(U in T: Source<U>)` is a symbolic type that
  must survive substitution and expand to a concrete `Type::Tuple` when `T` is
  fixed. Needs a `Type::Mapped`-style representation, expanded in the same place
  generic substitution happens.
- **Mapped-type inference inversion** is the one genuinely new analysis; the
  single-hole rule keeps it to element-wise unification.
- **Generic-tuple-destructure propagation bug** (`pair<A,B>(): (A,B)` leaves a
  destructured binding abstract — see `analyzer-refactor.md`). Not on the core's
  critical path (combine's `T` is concrete at the call), but it bites a `combine`
  nested inside another *generic* function; track alongside.
- **Type interning (#6 in analyzer-refactor).** `Type::Tuple`/`Type::Mapped` hold
  `Vec`s; keep them out of any interned identity set initially, or make them
  participate explicitly.

## Verification

- Flat-lowering stage: regenerate affected goldens; **run** the whole corpus under
  `node` and confirm identical behavior.
- Every later stage adds unused grammar/types → corpus byte-identical until a
  program opts in.
- New `tests/inference.rs` cases: arity-bound check + error, mapped-type inference
  (forward and inverted), a comprehension, and a **runtime** `combine` (2- and
  3-input, asserting recompute on each change).
- Acceptance: `reactive-ui/todos.vl` builds and runs end to end.

## Recommendation

Build the keyof-free core: flat lowering → arity/element bounds → mapped tuple
types + inference → comprehensions + tuple `for` → `combine` in std. Defer
`keyof`/indexed-write, spread parameters, and copy elision until a second caller
or a measured need justifies them. See `variadic-generics-plan.md` for the staged
plan.
