# Spec §10 — Macros

A macro is a compile-time function that receives program structure as
**data** and returns **source code** to splice into the program.
Expansion happens before name resolution and type checking (§1.2,
phase 3): generated code re-enters lexing and parsing and is then
checked exactly like handwritten code — a macro cannot smuggle
ill-typed code past the compiler.

## 10.1 Declaring and invoking

```vilan,fragment
macro fun derive_display(item: Item): Source
```

`macro fun` declares a macro. Invocation forms:

- **Attribute position**: `[name]` (and `[name(args)]`) on an item
  hands the annotated item to the macro as data; the returned source
  *replaces or augments* the item per the macro's construction API.
  `[derive(A, B)]` is the derive spelling: each named macro receives
  the type and its output is spliced alongside it.
- **`macro { … }` blocks**: an anonymous macro expanded on the spot.
  In item position the returned source splices as items; in expression
  position the block folds to the value of the returned expression
  (for plain values, prefer `const` — §9).
- Macros may call other macros as ordinary functions during expansion.

## 10.2 The macro environment

A macro's body is ordinary vilan, but it compiles against
**`macro_std`** — the compile-time standard library (`source`, the
`meta` item types, collections, strings) — and only that: a macro's
imports are its own, and it cannot reference the surrounding program's
bindings. Macros see **one item at a time**; there is no whole-program
reflection, no ordering guarantee between expansions, and no
communication between macro runs. Like `const` evaluation, each
expansion runs in the fueled interpreter (§9.3): fuel and depth
exhaustion, and panics, are compile errors at the invocation's span.

## 10.3 Inputs and outputs

The annotated item arrives as a `macro_std::meta` value (`Item`, with
accessors such as `as_struct()` yielding names, fields, and types as
data). The macro returns `Source` — text, usually built by
interpolation (`source(i"…")`); literal braces in generated code are
escaped `\{` `\}`.

Returned source is parsed as items (or an expression, for
expression-position blocks) **as if written at the invocation site**:
names in generated code resolve in the annotated item's module, and
imports the generated code needs must be written into the generated
source itself. Errors inside generated code are reported anchored at
the attribute that generated it (the diagnostics standard's macro
rule), with the generated text available to tooling.

## 10.4 Limits

The manifest's `[macro]` section (§11.4) bounds every compile-time
run: `fuel` (interpreter steps per run) and `depth` (nested expansion
— a macro whose output invokes macros). Exceeding either is a compile
error, so a runaway macro fails the build rather than hanging it.

## 10.5 Standard attributes (informative)

`[derive(Wire)]`, `[derive(Hashable)]`, `[service(…)]`, `[rpc]`, and
the other attributes the guides use are macros shipped in the standard
library — the same mechanism as §10.1, not language special cases.
Their semantics are library contracts specified by their reference
pages.
