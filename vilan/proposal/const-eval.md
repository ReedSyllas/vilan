# `const` — compile-time evaluation as a language feature

Status: **SHIPPED 2026-07-10** — the full v1, same-day as the proposal.
Slices 1–4 (the keyword, mark-and-forward + the free-variable rule, the
evaluation pass, in-place serialization — 21 pins + corpus `const.vl`), then
the **asset channel + const-only bit** (§2–3, the styling prerequisite):
`std::asset::emit` accumulates during `eval_const` only (a capability flag on
the interpreter — macro expansion and the equivalence runner reject it), the
channel dedups by line and orders lexically (SOUND for the styling system's
CSS: `.class` < `@media`, so media blocks take the later cascade position —
argued at `assemble_assets`), and `vilan build` writes `<output>.<kind>`
beside the JS (7 pins + an end-to-end CLI test). The const-only check is the
R-fixpoint over the shared call graph: functions reaching `emit` through
non-const call sites join R, roots (`main`, top-level initializers) never
join — a root's call into R errors AT THAT call site, the outermost runtime
crossing, while `emit` inside R-functions called from `const` chains stays
legal (the styling property-function shape, pinned). Recorded refinements:
indirect/closure-value paths into `emit` are the conservative gap;
`run`/`--watch` write assets beside the canonical output each round (SHIPPED
2026-07-20, hmr.md §11 S0 — single-package `run` and the `--watch` single arm
now call `write_assets`; the workspace paths already did via
`build_workspace_artifacts`); liveness-tied emission (dead-style
elimination), Tier-2 LSP memoization, and deep failure spans as before. Implementation notes that
amended the design: the JS-refugee hint lives in the ANALYZER, not the
parser — `const x = 3` parses fine (assignment is an expression, so it is
`const (x = 3)`), and the forwarding arm catches the `Assign` shape with
the idiom; the `const` marker forwards to its inner expression (no wrapper
entity), so every downstream pass sees a plain subtree; and const
mini-programs skip `rename_for_scopes` so the result binding survives.
The general feature the revised styling system (`proposal/ui-styling.md`)
rides; independently motivated.

## 0. Motivation

vilan already evaluates the whole pure language at compile time — the macro
engine's interpreter runs full vilan in hermetic worlds, is equivalence-gated
against node over the entire corpus, has a depth cap, a curated host table
(`Math.*` and friends), and turns panics into diagnostics. Today that power is
reachable only by producing *source text*. `const` exposes it for producing
**values**:

```vilan
let TABLE = const build_crc_table();      // computed at compile time: a literal in the JS
let CARD = const display(flex) + padding(space(4));   // the styling use case
mut cache = const build_initial();        // compile-time initial value, runtime-mutable
```

Payoffs beyond styling: lookup tables, precomputed scales, parsed static
config, wire-format hashes (`contract_hash` can stop being compiler magic and
become plain vilan), and — through the asset channel (§3) — compile-time
*emission* of non-code build outputs. Every such value costs zero at runtime:
the emitted JS contains the result literal, not the computation.

## 1. The model

`const` is a **weak-precedence expression keyword**: it captures the largest
expression to its right within the current bracket/comma context and
evaluates it at compile time.

```vilan
let result = const 1 + 2;            // the JS contains `3`
let TABLE = const build_crc_table(); // module scope: the shared-constant idiom
f(const heavy_default(), runtime);   // argument position; stops at the comma
let narrowed = (const f()) + g();    // parenthesize to narrow the capture
```

- **One mechanism, no declaration form.** `let NAME = const expr` *is* the
  constant declaration — bindings stay ordinary `let`/`mut` (tree-shaken by
  F6, clone-sited like any binding), annotations sit on the binding as usual,
  and `mut x = const initial()` expresses a compile-time initial value for
  runtime-mutable state, which a `const` declaration could not. (`const NAME
  = expr` as sugar is deliberately not shipped — one way to say it; recorded
  as a later nicety if the corpus begs.) `const { .. }` needs no special
  case: blocks are expressions, so multi-statement compile-time computation
  falls out.
- **Evaluation**: the captured expression is evaluated at compile time by the
  macro interpreter (the worlds machinery — one evaluator, not a second
  dialect). `let` alone stays runtime; `const` is the *guarantee* (and the
  capability gate, §2) — an optimizer may fold plain initializers someday,
  but `const` promises it and errors when it can't.
- **Free variables must be const-known**: an import, a literal, or an
  immutable binding whose own initializer is a `const` expression (chaining;
  `mut` disqualifies). A parameter or runtime local errors at the reference —
  "`n` is a runtime value; a `const` expression reads only
  compile-time-known bindings". Calls are unrestricted (§2's no-coloring
  rule): only free *variables* need the judgement.
- **Serialization is in place**: the result literal replaces the expression
  at its site. A `const` inside a loop re-materializes per iteration —
  never worse than the computation it replaced (that call also produced a
  fresh value per iteration; the computation is gone, the allocation
  unchanged), and no aliasing questions arise against value semantics.
  Sharing is spelled with an ordinary binding at whatever scope you mean;
  hoisting-with-read-only-proof is a recorded optimization, not v1.
- **No function coloring.** Any function reachable from a const initializer is
  const-callable — the interpreter is total over the pure language, so there
  is no `const fn` annotation and no ecosystem split (the Rust lesson,
  avoided; this is the Zig-shaped design, available to vilan because the
  evaluator predates the feature). A const expression that reaches an
  unavailable capability fails with a **spanned static error**, not a marker
  check.
- **The result must be plain data**: numbers, strings, bools, lists, maps,
  tuples, structs, enum values — transitively. A closure, view, `Shared`
  cell, or promise in the *result* is a static error at the expression
  (internal use during evaluation is fine — the interpreter models all of it;
  only the surviving value is constrained). Value semantics makes the
  snapshot natural.
- **Failures are diagnostics**: a panic during evaluation (`Thrown` — e.g.
  the checked-subscript message), the depth cap (`Depth`), an unavailable
  capability (`Unsupported`), or a non-data result — all report at the
  `const` expression with the failure message. Deep-span fidelity (pointing
  inside the callee) is a recorded refinement shared with macro diagnostics.
- **Dependencies**: const expressions form a value dependency graph through
  the const-known bindings they read (imports included); evaluation follows
  it in deterministic order (module topological order, then binding order,
  then expression order within a body); a cycle is an error.

## 2. The capability model

The const world *is* the macro world: pure vilan plus the curated host table
(math, string/collection intrinsics) — no io, dom, fetch, timers, process.
One new bit on top:

**Const-only functions.** A few std internals are legal *only* on call paths
rooted in a `const` expression — the first being `std::asset::emit` (§3),
whose whole point is a compile-time effect. Enforcement is static
reachability over the existing call graph (`src/call_graph.rs`): a call path
from runtime code into a const-only function errors at the offending call
site with what it means ("styles are compile-time values — build them in a
`const` expression", worded per API). v1 keeps the bit **std-internal** (users cannot
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

Const evaluation runs per expression at compile time; the interpreter's
speed is corpus-proven (the equivalence suite runs whole programs), and the
worlds cache precedent applies: memoize per expression on the
dependency-closure source.
v1 ships without incremental memoization (evaluate on each compile).

**Tooling split** (settled with the user): the LSP **evaluates explicit
`const` expressions** — they are opt-in contracts, bounded in number by the
user's own hand, and their diagnostics (`space(37)` blowing a scale's
bounds) belong live in the editor — under the existing analysis debounce
and the fuel cap (an editor must survive a `while true` const mid-edit; a
capped miss reports "did not finish within the compile-time budget" like
any other evaluation failure). What the LSP **never runs is G3's inference
sweep**: inference is silent-fallback optimization by design, so it
produces no diagnostics and nothing user-visible — there is nothing for an
editor to surface; it is a build-time pass only (`vilan check` doesn't need
it either: it cannot produce errors). A design invariant keeps the
LSP-side evaluation cheap and deferrable: **no downstream pass depends on
const *values*** — the type of `const expr` is the type of `expr`, so
hover/completion/navigation never wait on evaluation, and the debounced
pass can trail typing without blocking anything. (The sharp asymmetry with
macros: the LSP must expand those, because they create items and types.
Const generics would break this invariant — a second reason they are out
of scope beyond v1 sizing.) `vilan check` evaluates explicit consts as
`build` does — check means "will it build". Incremental memoization of the
LSP-side evaluation rides the Tier-2 caching arc.

## 5. Out of scope (v1)

- Const *generics* / const parameters / `const` depending on an enclosing
  generic's type parameters. (A `const` inside a generic function body is
  legal only if its initializer is independent of the type parameters.)
- User-declared const-only functions.
- `const` *parameters* (a callee demanding compile-time arguments) — the
  expression form makes call-site `f(const ..)` free, but parameter-side
  requirements are const-generics territory, out with them.
- Cross-crate/library const export beyond what value serialization already
  gives (a library's `const` re-evaluates in the consumer's compile — fine,
  deterministic).
- Floating point: no divergence to manage — the interpreter's f64 *is* JS's
  f64 (same representation, equivalence-gated), stated for the record.

### Recorded v2: inferred `const`

`let a = 1 + 2;` folding without the keyword (backlog G3). No fundamental
blocker; recorded here so v1's design doesn't foreclose it. The rules that
keep it sound:

- **Inference is transparent; `const` stays the contract.** The explicit form
  ERRORS when evaluation fails; inference silently falls back to runtime on
  ANY failure — capability, fuel, non-data result, or a **panic**. The panic
  case is load-bearing: a dynamically-dead `if false { xs[5] }` evaluates to
  a panic but runs fine — folding it would reject a working program.
  Fallback preserves observable behavior exactly, panics included.
- **Same eligibility as the explicit form**: const-known free variables
  (which, with the plain-data rule, is also what makes internal mutation
  non-escaping — external state is unreachable without referencing it), the
  const capability world, plain-data results.
- **Const-only functions never infer.** `asset::emit` requires an explicit
  `const` root — otherwise whether a style compiles depends on optimizer
  mood. Inference folds values; it never creates const contexts.
- **Budgets are the v2-sized work**: an evaluation fuel cap (a missed fold
  beats a hung compiler) and a serialized-size cap (a 10 KB table literal
  replacing a 20-character call is a regression nobody asked for — explicit
  `const` is the opt-in for big results). Heuristics with knobs.
- **Debug ergonomics**: folded computation vanishes from stack traces; the
  `[build]` presets fit naturally — debug skips inference, release infers.
- **The LSP never runs inference** — silent fallback means there is nothing
  to surface in an editor; the sweep is a build-time optimization pass only
  (§4's tooling split).

## 6. Implementation sketch

1. **Grammar**: `const` keyword as a weak-precedence expression prefix
   (captures to the end of the bracket/comma context; lexer keyword, parser
   arm, formatter, TextMate). Parser nicety, specced here: statement-initial
   `const IDENT =` gets the JS-refugee hint — "vilan has no const
   declarations — write `let x = const ..`".
2. **Analyzer**: mark const expressions; type-check them normally (the type
   system is unchanged — const-ness is an expression property, not a type);
   enforce the const-known free-variable rule; build the dependency order;
   run the capability reachability check.
3. **Const pass**: post-analysis, evaluate marked expressions in dependency
   order via the interpreter (through the existing `transform_to_ast` path,
   as macros do); collect asset emissions; convert failures to spanned
   diagnostics.
4. **Serialization**: result value → `js::Node` literal in place (numbers,
   strings, arrays, maps, the struct/enum runtime shapes the transformer
   already defines); reject non-data results with the §1 error.
5. **Channel**: dedup/order/write per §3; `vilan build --watch` regenerates.
6. **Pins**: value classes round-trip (incl. nested enums/maps); capability
   failure spans; the free-variable rule (runtime local, `mut`, and chained
   const-known cases); weak-precedence shapes (`const a + b`, argument
   position, parenthesized narrowing); cycle detection; panic-at-const spans;
   in-place semantics in a loop; determinism of the channel across binding
   reorderings; a `const` used from both server and client layers.

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
