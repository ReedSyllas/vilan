# Return-position type checking (backlog B10)

Status: **implemented with this note** (2026-07-04). Pins: the two `#[ignore]`s named in
B10, un-ignored, plus the per-case suite below.

## The gap (bigger than B10 recorded)

B10 said the solver never constrains a `ret` against the enclosing signature. Probing
showed the gap is wider: **the tail expression isn't checked either**. `fun bad(): i32 {
"nope" }` compiled clean — `Constraint::ReturnType` runs `infer_type(body, expected =
declared)` which *directs* inference (return-position generic binding) but never
*verifies* the result. Every "Expected X, but got Y" in the suite came from let-annotation
and argument checking; return position had none.

## Semantics (settled by probe, pinned)

1. **`ret` returns from the nearest enclosing callable** — function, closure, or `async`
   block (probed: a `ret` in a closure exits the closure; in an `async {}` it settles the
   block). The check is therefore scoped per-callable.
2. **In a function with a declared return type `R`:** the tail and every `ret v` check
   `typeof(v)` against `R` through the same constraint (`reconcile_type` — the same
   unification the let-annotation check uses, so generic returns bind, not just match).
   A **bare `ret`** checks a synthesized void value against `R` — so it is legal exactly
   when `R` is void, and errors as `Expected i32, but got void instead.` otherwise. No
   special case: bare `ret` is `ret <void>`.
3. **In a function with no declared return type (void):** nothing is checked — neither
   the tail (existing behavior: `fun f() { 5 }` compiles, the value is discarded) nor any
   `ret v`. Consistency with the tail is the rule; a void function's return values are
   discarded, not diagnosed.
4. **In closures and `async` blocks (v1: unchecked):** their return types are *inferred*,
   not declared, so there is nothing declared to check against. A `ret` inside one is
   skipped by this check (the boundary pushes an unchecked frame). The follow-up — `ret`
   *participating* in closure return inference (today `|x| { ret 5; }` has a void tail
   and types as a void closure) — is its own slice, pinned `#[ignore]`d.

## Mechanism

- `resolve_return_type` gains the missing half: after `infer_type` resolves, `reconcile_type`
  against the declared type; `None` → the standard mismatch diagnostic at the value's span.
  This alone fixes the tail.
- The analyzer walks with a `return_type_stack: Vec<Option<TypeId>>` — `Some(R)` pushed
  around a function body walk when a return type is declared, `None` for undeclared-void
  functions, closures, and `async` blocks (the boundary that makes `ret` inner-scoped).
- `Node::FuncReturn` pushes `Constraint::ReturnType` for its value (or a synthesized
  `Expr::Void` entity spanned at the `ret` itself) against the innermost `Some(R)`, and
  seeds `expected_types` — so `ret` is a first-class return position: return-directed
  generic binding (`ret List::new()`) works exactly as it does for the tail.

## What turning the check on surfaced

Three fixes fell out of enforcement, all root-caused:

- **The nine operator-trait defaults were ill-typed** — `{ panic("not implemented yet"); }`
  with a semicolon makes the panic a *statement* and the block's tail void, defeating the
  existing never-typing (`panic(..)` calls type as `Any` — a mechanism whose own comment
  anticipates exactly this "sole body of a function with any return type" case). Dropping
  the semicolons restores the intended pattern; behavior identical (panic throws).
- **`reconcile_type` had no `(Trait, Trait)` arm** — a trait-typed `self` returned through
  a trait-typed signature (`impl Iterator<type T> with Iterable<T> { fun iter(self):
  Iterator<T> { self } }`) had never reached a *checking* position before. Same-id traits
  now reconcile their arguments pairwise, like the nominal `Struct`/`Enum` arms.
- **`reconcile_type` had no `(Mapped, Mapped)` arm** — a parameter typed `(U in T:
  List<U>)` returned through an identically-written mapped return walks as two distinct
  binder ids, so the arm reconciles *structurally* (sources and templates recurse; the
  binders' alpha-renaming bindings are dropped from the result).

## Excluded (recorded, not drifted into)

- Closure-`ret` participation in closure return inference (above; `#[ignore]` pin).
- A never type: `ret`/`panic` as expressions still type void; `match` arms mixing a
  `ret` arm with value arms keep today's behavior (the arm unification is untouched).
