# Variadic generics (mapped tuples) — implementation plan

Implements `variadic-generics.md`. Core scope = everything needed to **define and
call `combine` without `keyof`**. Deferred items have their own section. Each
stage is one commit; every stage after Stage 0 is **corpus byte-identical** (it
adds grammar/types unused until a program opts in).

## Current shape (what we build on)

- **Tuples.** `Type::Tuple(Vec<TypeId>)` (`type_.rs`); `Node::Tuple` walked to
  `Type::Tuple` at `analyzer.rs:4287`; lowered to a JS array; element access and
  pattern destructuring shipped (`compile_pattern`, `transformer.rs`).
- **Generics.** `register_generic_parameters` (`analyzer.rs:2643`) →
  per-binder constraint `TypeId`; `SubstitutionContext = HashMap<TypeId, TypeId>`
  (`type_.rs`); monomorphization via `current_substitution` / `required_functions`
  (`transformer.rs`).
- **Unification.** `reconcile_type`'s `(Tuple, Tuple)` arm (`analyzer.rs:5202`)
  zips element-wise and collects generic bindings — the inference hook.

---

## Stage 0 — Flat tuple lowering (the one breaking stage)

Make nested tuples store flat; nesting becomes a purely static (type-level)
notion. **Distinct types are preserved** — this stage changes *layout only*, not
the type relation.

- **Construction.** Lowering a tuple literal/`Tuple` expr flattens each element
  whose static type is itself a tuple, splicing its (statically-known) element
  slots: `(a, 3)` with `a: (i32, i32)` → `[a[0], a[1], 3]`. Width comes from the
  element's resolved `Type::Tuple` arity (recursively).
- **Access.** Every tuple read computes a **flat offset** from the static type:
  field access `x.i`, subscript, and `compile_pattern`'s `PropertyIndex` chain all
  walk the nested type to a flat index. (`compile_pattern` currently emits
  `subject[i][j]` for nested tuples — it now emits a single flattened index.)
- **Sub-tuple extraction.** A binder/field that selects a *tuple* element
  (`let (a, rest) = x`, `rest: (B, C)`) materializes a reslice `[x[k], x[k+1], …]`.
- **Codegen impact.** NOT byte-identical: regenerate goldens for files with a
  genuinely nested tuple. Today that is exactly `vilan/test/destructuring.vl`
  (`(1, (2, "z"))`: `[1,[2,"z"]]` → `[1,2,"z"]`, access `$b[1][0]` → `$b[1]`).
- **Verify.** Rebuild the debug binary first ([[golden-regen-rebuild-debug]]),
  regenerate, then **run the entire corpus under `node`** and confirm identical
  stdout. Add a nested-tuple lowering test asserting the flat array shape.

> If a wider audit finds more nested-tuple goldens, regenerate them here too; the
> commit body must list every regenerated file and confirm behavior is unchanged.

## Stage 1 — Arity & element bounds (`T: (..)`, `(2..)`, `(..N)`, `(..: Trait)`)

- **Parser.** A tuple-bound form in generic-bound position: `(` range `)` with an
  optional `: Trait` element bound. Range = `lo?..hi?` (both optional) or a bare
  `..`. A new bound `Node` (e.g. `Node::TupleBound { lo, hi, element: Option<…> }`).
- **Analyzer.** Represent it as a constraint on the binder (a `TupleBound` kind
  parallel to the existing trait bounds in `register_binder`, `analyzer.rs:2676`).
  At a call, once the argument's concrete tuple type is known, check arity ∈ range
  and (if present) each element against the element bound; emit a clear diagnostic
  on mismatch ("`combine` expects a tuple of 2 or more sources").
- **No codegen.** Byte-identical. Tests: an in-range call compiles; out-of-range
  and element-bound violations error.

## Stage 2 — Mapped tuple types (`(U in T: F<U>)`) + inference

- **Parser.** A mapped-type node: binder name, source tuple type `T`, single-hole
  template referencing the binder.
- **Type.** Add `Type::Mapped { binder, source: TypeId, template: … }` (symbolic
  while `T` is abstract). `walk_type_node`: build it; when `source` resolves to a
  concrete `Type::Tuple`, **expand** element-wise to a concrete `Type::Tuple`.
- **Substitution.** Where generic substitution rewrites types, expand a
  `Type::Mapped` whose `source` became concrete (so `combine`'s parameter type
  becomes a real tuple in each monomorphization).
- **Inverted inference.** In the call-binding path / `reconcile_type`: unifying a
  concrete argument tuple against a `Type::Mapped` parameter binds the binder
  element-wise (single hole), yielding `T`. Reuse the `(Tuple, Tuple)` machinery.
  - *Concrete-constructor template* (`Source<U>`): structural unify each slot.
  - *Trait template* (`Readable<U>`, the form `combine` needs): invert by impl
    resolution — find `U` such that the slot type implements the trait. Sound
    only because the trait has no blanket impl (one impl per type); reject /
    diagnose an ambiguous (multi-impl) inversion rather than guess.
- Tests: forward expansion (`(U in (i32,str): Source<U>)` → `(Source<i32>,
  Source<str>)`), inversion (`combine((a,b,c))` binds `T = (A,B,C)`), and a
  trait-template inversion against two distinct implementors.

## Stage 3 — Tuple comprehensions + `for`

- **Parser.** Value comprehension `(x in xs = e)`; tuple `for (_, x) in xs { … }`.
  (Reuse the binder grammar for the `(_, x)` slot.)
- **Analyzer.** Type-check the body **once** against the abstract element type
  (the source tuple's mapped element / its bound): the comprehension's type is the
  mapped tuple of the body's element type; the `for` body checks with `x` at the
  element type, void result.
- **Transformer.** **Unroll at monomorphization** (when the source tuple type is
  concrete): the comprehension emits a flat array literal of the per-element
  expressions; the `for` emits N statements with `x` bound to each `xs[i]`.
- Tests: a comprehension round-trips; a `for` over a 3-tuple emits 3 bodies.

## Stage 4 — `Readable` bridge + `combine` in `std::reactive`

- **`Readable<T>` trait** (its own sub-commit) — the read interface a `Source` and
  a `Signal` both satisfy, accepted as `combine`'s slot type. Chosen over
  `Into<Source<U>>` because the reflexive blanket `Into` impl breaks
  target-directed dispatch today (verified: `let x: i32 = w.into()` resolves to the
  identity impl); a bespoke trait has one impl per type, so per-element inversion
  is unambiguous (see proposal). Revisit `Into` only if blanket-impl dispatch is
  fixed ([[analyzer-stabilization]]).

  ```vilan
  trait Readable<T> { fun as_source(self): Source<T>; }
  impl Source<type T> with Readable<T> { fun as_source(self): Source<T> { self } }
  impl Signal<type T> with Readable<T> { fun as_source(self): Source<T> { self.node } }
  ```

- **`combine`** — `combine<T: (2..)>(sources: (U in T: Readable<U>)): Source<T>`,
  keyof-free body (convert each slot via `as_source`, snapshot-recompute) from the
  proposal. Depends on Stage 2's trait-template inversion.
- Update `todos.vl` to `combine((items, filter))` (the `Signal`s now satisfy
  `Readable`).
- Tests: a 2- and 3-input `combine` runtime test (assert recompute on each input
  change), with a mix of `Signal` and `Source` inputs; `reactive-ui/todos.vl`
  builds and runs.
- Update `reactive-ui/README.md` (drop the "not yet built" caveat).

---

## Deferred stages (own proposals/commits when taken up)

- **`keyof` + indexed write** — a `keyof T` index type and `v[key] = …`, lowering
  to a constant index per unrolled iteration; restricted to statically-known
  indices. Upgrades `combine` from O(N)-per-update to O(1) per-slot.
- **Spread parameters** — `fun f(...items: T)` + the `f(a, b, c)` call convention
  collecting into a tuple `T`.
- **Copy elision on flatten** — move/liveness analysis to drop the construction
  copy when a source operand is dead.
- **Multi-hole mapped templates** — relax the single-hole inference restriction.

## Risks (recap, see proposal for detail)

- Stage 0 is the only non-byte-identical stage — gate it on a full corpus *run*,
  not just a golden diff.
- `Type::Mapped` must survive substitution and expand at the right point; get this
  wrong and a `combine` parameter stays symbolic into codegen.
- Single-hole inference is a hard invariant — reject multi-hole templates with a
  diagnostic rather than infer ambiguously.
- Watch the generic-tuple-destructure propagation bug for `combine` used inside
  another generic function (`analyzer-refactor.md`).

## Sequencing & checkpoints

Stages are ordered by dependency: 0 (layout) → 1 (bounds) → 2 (mapped types +
inference) → 3 (comprehension/for) → 4 (combine). Stages 1–3 are independently
mergeable and byte-identical; the feature is only *observable* at Stage 4. Each
stage: `cargo fmt`, full `cargo test`, and (Stage 0) a corpus behavior run.
