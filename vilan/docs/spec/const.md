# Spec §9 — Const evaluation

`const expr` evaluates `expr` **during compilation** and replaces it
with its result: the emitted program carries the value as a literal,
never the computation. Const evaluation and macro expansion (§10) run
in the same fueled compile-time interpreter; they are the two phases
that execute vilan code at build time.

## 9.1 The `const` expression

`const` is a prefix operator over an expression. It captures
**greedily**: everything to the end of the surrounding expression folds
(`const 1 + 2 * 3` folds `7`); parenthesize to narrow the extent —
in `(const square(4)) + square(2)` the second call runs at runtime.

A `const` expression may appear anywhere an expression may, including
module-level initializers — where it is also the way to run *logic* at
load position without runtime cost: a `const` initializer ships as a
plain value, participates in no platform coloring (§11.2), and cannot
violate the initializer rules of §7.4 (nothing of it remains to run at
load time).

## 9.2 The const environment

The evaluated expression may use exactly what the compiler can know:

- literals, and the pure operations of the language (§7.2's evaluation
  order applies unchanged);
- functions whose bodies are themselves const-evaluable, transitively;
- imports, and immutable module bindings whose own initializers are
  const-evaluable.

**Host capabilities do not exist at compile time.** A call that
requires the host — the clock, the filesystem, network, randomness, any
`external` function without a compile-time definition — is a compile
error inside `const` ("`now()` is not const-evaluable"), not a deferred
runtime call: the answer would not be a constant.

The one deliberate exception is `std::asset::emit(kind, content)`,
callable **only** during const evaluation: it declares a build asset
(the styling system's CSS, for example) that the build writes beside
the output. Asset emission is deterministic — same inputs, same files.

## 9.3 Failure and resource limits

Const evaluation is total by construction of the budget: each run is
bounded by the interpreter's **fuel** (steps) and **depth** (nesting),
shared with macro expansion and configured by the manifest's `[macro]`
section (§11.4). Exhausting either, or panicking during evaluation, is
a compile error carrying the const expression's span — a runaway
`const` fails the build; it cannot hang it.

## 9.4 Results

The result of a `const` expression is spliced as a literal of its
type: numbers, strings, booleans, lists, tuples, structs and enums of
const-evaluable contents. The expression's *type* is checked exactly
as if it ran at runtime; `const` never changes typing, only when the
computation happens.
