# Variadic generics — `combine` and parameter packs

Driving example: `std::reactive::combine`, the **product combinator** of the
reactivity model. It is the one piece of the reactive UI that the type system
can't yet express, so `vilan/examples/reactive-ui/todos.vl` imports it but the
app can't be built (`README.md`: "not yet built — needs variadic generics").

```vilan
// Today's gap: a reactive value depending on N inputs of DIFFERENT types.
let visible: Source<(List<Todo>, str)> = combine((items, filter));
//   combine((Source<List<Todo>>, Source<str>))  ->  Source<(List<Todo>, str)>
visible.derive(|(list, filter)| { /* recompute when EITHER changes */ });
```

`combine` maps a **tuple of sources** to a **source of a tuple**:

```
combine: (Source<A>, Source<B>, …, Source<Z>) -> Source<(A, B, …, Z)>
```

The arity and the element types are both unknown at declaration. That is exactly
a **parameter pack**: one type variable standing for *zero or more* types at
once. This proposal designs packs for Vilan and lands `combine`.

> **Note the call shape.** We take a single **tuple argument** —
> `combine((a, b, c))` — not spread varargs `combine(a, b, c)`. A tuple needs no
> new argument-list grammar (it reuses the tuple expression that already exists),
> and the consumer destructures it with the tuple binders that **just shipped**
> (`derive(|(list, filter)| …)`). The earlier `combine(a, b)` spelling in
> `todos.vl` becomes `combine((a, b))`.

---

## The key simplification: monomorphization makes packs concrete

Vilan monomorphizes — every generic call is specialized to a concrete variant by
its argument types (`transformer.rs`, `current_substitution` / `required_functions`).
So **a pack is never variadic at runtime**: at each call site the argument tuple
fixes the arity *N* and the element types, and the compiler emits one specialized
`combine` for that shape. There is no runtime arity, no fold over types, no
recursion at the type level — just compile-time expansion, the same machinery
that already turns `f<T>` into `f<i32>`.

This is what makes the feature affordable here, where it is famously heavy in a
non-monomorphizing language. The whole design is: **bind a pack to a
`Vec<TypeId>` at the call, then expand.**

---

## Current shape (what's already in place)

- **Tuples are first class.** `Type::Tuple(Vec<TypeId>)` (`type_.rs`), parsed from
  `(A, B)` to `Type::Tuple` in `walk_type_node` (`analyzer.rs:4287`), lowered to a
  JS array; element access and pattern destructuring exist.
- **Generic binders are a `TypeId` identity.** `register_generic_parameters`
  (`analyzer.rs:2643`) turns each `<T>` into a constraint `TypeId`; substitution is
  `SubstitutionContext = HashMap<TypeId, TypeId>` (`type_.rs`). A pack is the same
  idea with the value side widened to `Vec<TypeId>`.
- **Tuple unification already collects bindings.** `reconcile_type`'s
  `(Tuple, Tuple)` arm (`analyzer.rs:5202`) zips element-wise and accumulates
  generic bindings. Pack inference hooks in exactly here.
- **The consumer side is done.** `combine(...).derive(|(list, filter)| …)` relies
  on closure tuple-parameter destructuring, which landed in the last change set.

### One prerequisite bug (found while scoping this)

A generic function returning a tuple does **not** propagate the call's
substitution to a destructured binding:

```vilan
fun pair<A, B>(a: A, b: B): (A, B) { (a, b) }
let p = pair(1, "x");
match p { let (n, s) => n.to_string() }   // ERROR: cannot call 'to_string' on A
```

`p` is typed `(A, B)` (abstract) instead of `(i32, str)`, so the binding `n`
stays generic. This is the same generic-resolution class tracked in
`analyzer-refactor.md`. It matters because `combine`'s result element types come
*from* the pack substitution. The intrinsic-typing route below **side-steps** it
(it computes a concrete result type directly), but the general expression-level
packs (Stage 2) would need it fixed. Filed here as a dependency.

---

## Design

### Surface syntax

```vilan
//        pack decl ─┐        ┌─ pack mapped over a template   ┌─ pack as a tuple
fun combine< ...T >( sources: ( ...Source<T> ) ): Source< ( ...T ) > { … }
```

Three new forms, all opt-in behind `...`:

1. **Pack declaration** `<...T>` — `T` binds to an ordered list of types
   (`GenericParameter { is_pack: true }`).
2. **Pack expansion in a tuple type** `(...F<T>)` — element-wise map of the
   one-hole template `F<T>` over the pack: `(F<T0>, F<T1>, …, F<Tn>)`. The bare
   case `(...T)` is the identity template, i.e. `(T0, …, Tn)`. Expansion is only
   legal inside a tuple type (the only place an unknown-length type list is sound).
3. *(Stage 2)* **Pack expansion in expressions** — see below; not needed to land
   `combine`.

A pack may appear in **at most one** parameter and is bound there; other
positions (the return type here) only *consume* it. This keeps inference
one-directional and decidable.

### Type representation

```rust
// type_.rs
enum Type {
    …
    Pack(Vec<TypeId>),   // an expanded pack: the concrete element list
}
```

- A declared-but-unbound pack `T` keeps its `Type::Generic(constraint_id)`
  identity (so `T` is a name in scope) and is *marked* a pack.
- Substitution gains a sibling map carried alongside `SubstitutionContext`:
  `PackBindings = HashMap<TypeId, Vec<TypeId>>` (pack constraint id → element
  types). Expanding `(...F<T>)` looks `T` up here.

### Inference (arity + elements), at the call

`combine((items, filter))` types the argument tuple as
`(Source<List<Todo>>, Source<str>)`, then unifies it against the parameter type
`(...Source<T>)`:

1. The parameter is a **pack-tuple**: a fixed prefix/suffix of ordinary elements
   (none here) plus one `...Source<T>`. Match the concrete tuple's length to fix
   *N* = 2.
2. For each concrete element `Source<Xi>`, reconcile the template `Source<T>`
   against it (reusing `reconcile_type`), binding the hole: `T_i = Xi`.
3. Record `PackBindings[T] = [List<Todo>, str]`.

Then the return type `Source<(...T)>` expands to `Source<(List<Todo>, str)>` —
**concrete**, so `.derive`'s closure sees concrete element types and the
prerequisite bug never fires.

### Monomorphization

Keyed by the concrete shape, like every other generic call: the variant for
arity *N* with elements `[X0…Xn]` is emitted once (`required_functions`), reused
for an identical later call. Distinct arities are distinct variants — there is no
single `combine` in the output, only `combine` specialized per call shape (and
deduped). Bounded in practice: a program has a handful of `combine` arities.

### The body — two routes

**Route A (recommended to land first): `combine` as an arity-polymorphic intrinsic.**
Declare it in `std::reactive` with the pack signature and an `external`-style
body the compiler supplies per arity — exactly how a bodyless `external` fn is an
intrinsic today, but expanded for the inferred *N*:

```vilan
// std::reactive
external fun combine< ...T >(sources: ( ...Source<T> )): Source< ( ...T ) >;
```

The transformer lowers a resolved `combine` call of arity *N* to:

```js
function combine(sources) {
    const sample = () => sources.map((s) => s.get());   // tuple = JS array
    const result = Source.new(sample());
    for (const s of sources) s.sub((_) => result.update(sample()));
    return result;
}
```

Because tuples are JS arrays and `Source` is uniform in `T`, the **emitted JS is
arity-independent** — a single helper, not one per *N*. Only the *type* is
expanded per arity; codegen is shared. This is the smallest correct thing: it
needs the pack *type* system (decl + tuple expansion + inference) but **no**
expression-level pack expansion.

This mirrors how built-in derives shipped as a special-cased subset of the macro
engine (#9) before the general engine existed — a sanctioned precedent in the
roadmap.

**Route B (Stage 2, only if a second pack use appears): user-writable pack bodies.**
To let users write their *own* pack functions (`zip`, tuple `map`, argument
forwarding), expression-level expansion is needed:

- tuple-spread `(...sources@.get())` — build a tuple by applying a per-element
  expression across the pack;
- pack-`for` `for ...s in sources { … }` — unroll a statement per element.

The honest cost: a heterogeneous per-element map (each `s.get()` returns a
different `Ti`) is rank-2 (the body is polymorphic *per element*), which is a
much larger lift than Route A. It is **not** justified by one function. Defer
until a second variadic need is real, then design it against two callers.

---

## Staged plan — each stage its own commit, **corpus byte-identical**

The corpus (`vilan/test/*.vl` → `.js`) must stay byte-identical through Stage 1
and 2 (no existing program uses `...`).

1. **Parser: `...` forms.** Pack decl `<...T>`; pack expansion `(...Template)`
   inside a tuple type. Add `is_pack` to `GenericParameter`; a `Node` for a
   tuple-type pack element. No semantics yet → corpus untouched.
2. **Type + inference.** `Type::Pack`, `PackBindings`; teach `reconcile_type` /
   the call-binding path to match a pack-tuple against a concrete tuple, fix
   arity, bind elements; expand `(...F<T>)` in `walk_type_node` and during
   substitution. Unit-test arity/element inference in `tests/inference.rs`.
3. **Intrinsic `combine`.** Declare it `external` in `std::reactive` with the pack
   signature; transformer recognizes it and emits the shared helper; result type
   expands concretely. New runtime test: a 2- and 3-input `combine`, assert it
   recomputes on each input. `todos.vl` compiles and runs.
4. **Ergonomics & polish.** `Signal` vs `Source` inputs (combine takes
   `Source<T>`; a `Signal` exposes its `.node` or a `to_source()` — decide and
   document); formatter prints `...T` / `(...F<T>)`; LSP hover/format for packs;
   update `reactive-ui/README.md` (drop the "not yet built" caveat).
5. *(Deferred)* **Stage 2 expression packs** — only against a second caller.

---

## Risks & mitigations

- **Arity-mismatch silently truncating.** `reconcile_type`'s tuple arm zips
  (stops at the shorter), which would hide a wrong-arity call. The pack matcher
  must check lengths explicitly and emit a real diagnostic ("combine expects a
  tuple of sources").
- **Pack in an un-inferable position.** Restrict a pack to be *bound* by exactly
  one parameter; other occurrences only consume it. Reject a pack that can't be
  fixed from arguments (return-only) with a clear error.
- **The prerequisite generic-tuple-destructure bug** (above). Route A avoids it;
  note it as a hard dependency for Route B and link `analyzer-refactor.md`.
- **Monomorphization blow-up.** Per-arity *type* variants only; codegen is shared
  (arity-independent JS). Bounded by the number of distinct `combine` shapes.
- **Type interning (#6 in analyzer-refactor).** `Type::Pack` holds a `Vec`;
  ensure it participates in whatever identity/interning story lands, or keep packs
  out of the interned set initially.

## Verification (every stage)

- Corpus byte-identical (Stages 1–2 add unused grammar; Stage 3 adds only new
  programs).
- `tests/inference.rs`: arity inference, element binding, a wrong-arity error,
  and a **runtime** `combine` test (compile + run under `node`, assert recompute).
- `reactive-ui/todos.vl` builds and runs end to end — the real acceptance test.

## Recommendation

Ship **Route A**: parameter-pack *types* (declaration + tuple expansion +
arity/element inference) plus `combine` as an arity-polymorphic intrinsic with a
shared, arity-independent JS body. It is the minimal change that makes the
signature honestly variadic-generic Vilan, unblocks the reactive UI, and leaves a
clean seam for general expression-level packs (Route B) if and when a second
caller justifies them. Fix the generic-tuple-destructure bug opportunistically;
it is not on Route A's critical path but is on Route B's.
