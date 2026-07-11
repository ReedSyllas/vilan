# `const` — compile-time evaluation as a language feature

Status: **PROPOSAL** (2026-07-10). The general feature the revised styling
system (`proposal/ui-styling.md`) rides; independently motivated. Semantics
here are the contract; the keyword and binding-form details are settled,
smaller spellings (attribute forms, const blocks) are explicitly deferred.

## 0. Motivation

vilan already evaluates the whole pure language at compile time — the macro
engine's interpreter runs full vilan in hermetic worlds, is equivalence-gated
against node over the entire corpus, has a depth cap, a curated host table
(`Math.*` and friends), and turns panics into diagnostics. Today that power is
reachable only by producing *source text*. `const` exposes it for producing
**values**:

```vilan
const TABLE = build_crc_table();          // computed once, at compile time
const CARD = display(flex) + padding(space(4));   // the styling use case
```

Payoffs beyond styling: lookup tables, precomputed scales, parsed static
config, wire-format hashes (`contract_hash` can stop being compiler magic and
become plain vilan), and — through the asset channel (§3) — compile-time
*emission* of non-code build outputs. Every such value costs zero at runtime:
the emitted JS contains the result literal, not the computation.

## 1. The model

```vilan
const NAME = expression;        // module scope and local scope
const NAME: Type = expression;  // annotation as with `let`
```

- **Evaluation**: the initializer is evaluated at compile time by the macro
  interpreter (the worlds machinery — one evaluator, not a second dialect).
  `let` stays runtime-by-default; `const` is the *guarantee* (and the
  capability gate, §2) — an optimizer may fold `let` someday, but `const`
  promises it and errors when it can't.
- **No function coloring.** Any function reachable from a const initializer is
  const-callable — the interpreter is total over the pure language, so there
  is no `const fn` annotation and no ecosystem split (the Rust lesson,
  avoided; this is the Zig-shaped design, available to vilan because the
  evaluator predates the feature). A const expression that reaches an
  unavailable capability fails with a **spanned static error**, not a marker
  check.
- **The result must be plain data**: numbers, strings, bools, lists, maps,
  tuples, structs, enum values — transitively. A closure, view, `Shared`
  cell, or promise in the *result* is a static error at the binding (internal
  use during evaluation is fine — the interpreter models all of it; only the
  surviving value is constrained). Value semantics makes the snapshot natural.
- **Serialization**: the transformer swaps the initializer for the result's
  JS literal (`const TABLE = [ 0, 79764919, .. ];`). Module-level `const`
  bindings tree-shake like any binding (F6).
- **Failures are diagnostics**: a panic during evaluation (`Thrown` — e.g.
  the checked-subscript message), the depth cap (`Depth`), an unavailable
  capability (`Unsupported`), or a non-data result — all report at the
  `const` binding with the failure message. Deep-span fidelity (pointing
  inside the callee) is a recorded refinement shared with macro diagnostics.
- **Dependencies**: a const initializer may read other consts (imported
  included); evaluation follows the value dependency graph in deterministic
  order (module topological order, then binding order); a cycle is an error.

## 2. The capability model

The const world *is* the macro world: pure vilan plus the curated host table
(math, string/collection intrinsics) — no io, dom, fetch, timers, process.
One new bit on top:

**Const-only functions.** A few std internals are legal *only* on call paths
rooted in a const initializer — the first being `std::asset::emit` (§3),
whose whole point is a compile-time effect. Enforcement is static
reachability over the existing call graph (`src/call_graph.rs`): a call path
from runtime code into a const-only function errors at the offending call
site with what it means ("styles are compile-time values — bind this with
`const`", worded per API). v1 keeps the bit **std-internal** (users cannot
declare const-only functions) and requires direct call chains — a const-only
function passed indirectly (through a closure value) is conservatively
rejected. This is one capability bit on a handful of internals, not function
coloring: ordinary functions remain callable from both worlds with no
annotation.

## 3. The asset channel — compile-time emission

```vilan
// const-only: appends a line to the build's `kind` asset.
fun emit(kind: str, line: str): void;    // std::asset
```

During const evaluation, `emit` accumulates `(kind, line)` pairs in the
compiler. After compilation the channel, per kind:

1. **Deduplicates by line** — independent const evaluations compose into one
   output with no cross-binding coordination (the property that makes atomic
   CSS plateau).
2. **Orders deterministically** — a kind-specific rule (CSS: base < pseudo <
   media, then lexical), so outputs are byte-stable regardless of evaluation
   or caching order.
3. **Writes `<out>.<ext>` beside the compiled `.js`** (e.g. `dist/client.css`).

The channel is styling-agnostic: A7 SSR wants it for critical CSS, and any
compile-time codegen (license manifests, service worker precache lists) rides
the same mechanism. A two-target build (client + server bundles) evaluates
consts per compile; dedup makes the union coherent, and the CSS lands beside
the client bundle.

**Liveness over-approximation, recorded**: v1 evaluates every `const` and
keeps every emitted asset line, even if F6 later drops the binding from the
JS (assets are collected before assembly-time reachability). Tying emission
to binding liveness — which would give dead-style elimination for free — is
the recorded refinement, mirroring F6's own recorded over-approximations.

## 4. Cost and caching

Const evaluation runs per binding at compile time; the interpreter's speed is
corpus-proven (the equivalence suite runs whole programs), and the worlds
cache precedent applies: memoize per binding on the dependency-closure source.
v1 ships without incremental memoization (evaluate on each compile); the LSP
path leans on the existing debounce until Tier-2 caching lands.

## 5. Out of scope (v1)

- Const *generics* / const parameters / `const` depending on an enclosing
  generic's type parameters. (A `const` inside a generic function body is
  legal only if its initializer is independent of the type parameters.)
- User-declared const-only functions.
- `const` in non-binding positions (const blocks, const arguments).
- Cross-crate/library const export beyond what value serialization already
  gives (a library's `const` re-evaluates in the consumer's compile — fine,
  deterministic).
- Floating point: no divergence to manage — the interpreter's f64 *is* JS's
  f64 (same representation, equivalence-gated), stated for the record.

## 6. Implementation sketch

1. **Grammar**: `const` keyword; binding form beside `let` (lexer keyword,
   parser arm, formatter, TextMate).
2. **Analyzer**: mark const bindings; type-check initializers normally (the
   type system is unchanged — const-ness is a binding property, not a type);
   build the const dependency order; run the capability reachability check.
3. **Const pass**: post-analysis, evaluate initializers in dependency order
   via the interpreter (transform the initializer expression through the
   existing `transform_to_ast` path, as macros do); collect asset emissions;
   convert failures to spanned diagnostics.
4. **Serialization**: result value → `js::Node` literal (numbers, strings,
   arrays, maps, the struct/enum runtime shapes the transformer already
   defines); reject non-data results with the §1 error.
5. **Channel**: dedup/order/write per §3; `vilan build --watch` regenerates.
6. **Pins**: value classes round-trip (incl. nested enums/maps); capability
   failure spans; cycle detection; panic-at-const spans; determinism of the
   channel across binding reorderings; a `const` used from both server and
   client layers.

## 7. Alternatives rejected

- **Rust-style `const fn` coloring** — an ecosystem-wide annotation burden
  vilan doesn't need; the interpreter's totality is the asset, use it.
- **Macros as the value channel** (the styling proposal's first draft) —
  produces source text, so every consumer pays the DSL toll: no hover, no
  go-to-def, no typed diagnostics inside the block, custom highlighting.
  Superseded by this proposal exactly to avoid that toll.
- **Build scripts** (a `build.vl` executed by the CLI) — a second program
  with its own capability story, non-composable with the module graph, and
  invisible to the type checker. The asset channel gives the useful half
  (emission) inside the language.
