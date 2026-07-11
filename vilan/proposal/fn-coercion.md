# Named functions as closure values (backlog B20)

Status: **SHIPPED 2026-07-11**. Found building A10: `current_path().map(parse)`
failed ("Expected |str| U, but got fn parse(str): Route instead") and needed
eta-expansion `.map(|path| parse(path))` — pure ceremony, since on the JS
backend the named function IS the value being constructed.

## 1. The rule

A reference to a named function may appear wherever a value of **closure
type** is expected, when all of these hold:

1. **It is a plain vilan `fun`** — not `external`. Extern binding forms
   (`method`/`get`/`set`, dotted paths like `document.getElementById`) have
   no sound value form: a dotted global loses its `this` when detached, and
   the emission for `__`-helper externs is call-shaped. Eta-expansion keeps
   the call form and stays the answer there.
2. **It is not generic.** A generic function has no single value — which
   instantiation is meant would need expected-type-driven instantiation of a
   function *value* (monomorphization of values, not calls). Deferred; the
   existing mismatch error (which prints the generic signature) remains.
3. **It is not a method** (no `self` parameter). `x.method` as a value means
   receiver capture — real closure creation, B18-adjacent. Deferred.
4. **It is not `async`.** A call through a plain closure value is not awaited
   (the J2 indirect-call gap), so the coerced value would leak a raw promise.
   Asyncness lives per-binding (`async_values`), not in `Type::Closure`, so
   the type-level coercion cannot see the async side; until it can, async
   functions keep the mismatch error. (A SYNC function flowing into an
   `async ||`-annotated parameter is fine and needs nothing: the await it
   receives resolves a plain value.)
5. **It reads no contexts.** Already enforced downstream: the context pass
   rejects a needs-context function used as a value ("reads context …, so it
   can't be used as a value") — coercion does not bypass it.
6. **The signature reconciles** with the closure type: same parameter count,
   parameter types and return type unify — binding the CALLEE's generics, so
   `map<U>`'s `|str| U` against `fn parse(str): Route` binds `U = Route`.
   The return type is the declared annotation when present, otherwise the
   body's inferred type (the same rule call-typing uses).

## 2. Semantics

The reference denotes the function itself as an immutable callable value —
eta-equivalent to `|a1, .., an| f(a1, .., an)`, and on the JS backend it IS
the function object. Like closures, it is exempt from value-semantics
cloning (immutable, so sharing is indistinguishable from copying).

The coercion applies anywhere types unify: call arguments (the motivating
`map(parse)`), annotated `let`s, struct-literal fields, return positions.
It is one-way — nothing in the surface language is *annotated* as a function
type, so closure→function never arises.

## 3. Implementation shape

`Type::Function(id)` already exists (function references type as it); the
coercion is a pair of symmetric arms in `reconcile_type` and `compare_type`:
`Function` meeting `Closure` converts the eligible function's signature to a
`Type::Closure` and recurses (parameter `TypeId`s exist on the declaration;
the return is the declared `TypeId` or the body's type). Ineligible
functions don't convert, so they keep today's mismatch error. Symmetric
because call sites reconcile in both argument orders. Codegen needs nothing
new: a function reference already emits as the (renamed) identifier and
marks the function reachable.

## 4. Out of scope (recorded)

- **Generic functions as values** (rule 2) — wants expected-type-driven
  instantiation; revisit if eta-expansion proves a real burden.
- **Methods as values / bound methods** (rule 3).
- **Extern functions as values** (rule 1).
- **Async functions as values** (rule 4) — unlocks when asyncness moves into
  (or alongside) the closure type.
- **Unannotated function-typed bindings**: `let f = parse;` keeps
  `Type::Function` (no closure-typed position forces the coercion). Calling
  such a binding works as before; passing it later coerces at that later
  position. Nothing new is promised about function-typed VALUES beyond the
  coercion into closure-typed slots.
