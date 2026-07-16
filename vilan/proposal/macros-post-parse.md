# Macros as a post-parse stage? — discussion (2026-07-16)

Status: **DISCUSSION — no decision.** Raised by the user alongside the
derive-import-leak bug; this note frames the question so the decision can be
argued on paper. Context: `macro-engine.md` (the engine), the measured
tree-interchange rejection (2026-07-07), and the expression-lifting rewrite
(`lift.rs`), which quietly established the repo's first post-parse Node→Node
stage.

## 1. What "post-parse" would even mean here

The engine already runs *after parsing and before analysis* — expansion is
positionally a post-parse stage today. The real architectural axes are:

- **Interchange**: expansion output is SOURCE TEXT, re-parsed and spliced.
  Tree interchange (macros produce/receive `Node` values) was measured and
  deliberately not taken: 0.8% of a real build; the caches erase it on
  re-analysis; batching parses is the recorded cheap fallback.
- **Scoping**: spliced items land *in the module* — so an expansion's
  self-carried imports become module imports. This is the **derive-import
  leak** (live, verified: `JsonValue` resolves after `[derive(Json)]` with no
  import; std's `rpc.vl` once depended on exactly such a leak).
- **Provenance**: text splices carry synthetic spans; expansions know their
  originating item, but diagnostics into generated code still speak internal
  vocabulary (backlog E8).

The leak is a **scoping** problem. It is orthogonal to interchange: a
tree-based engine that splices items into the module scope leaks identically.
So "make macros post-parse/tree-based" would not, by itself, fix the bug —
and the bug can be fixed without touching interchange.

## 2. Options

**A. Keep the engine; add an expansion scope wrapper (recommended).**
Expansion output parses exactly as today, but the spliced items arrive
wrapped in a scope-carrying node (the `LiftGroup` precedent: a parse-shaped
wrapper that changes only resolution). The walk pushes a child scope for the
expansion: its `import`/`use` bindings resolve *inside the wrapper only*,
while the items it defines (impls, functions, structs) register into the
module as they must. Prerequisite: sweep std's derives/`[service]` for code
that depends on a leaked name (`rpc.vl`'s old dependence) and make each
expansion self-contained — which they nominally already are ("outputs
self-carry imports" was the design). Small surface; no engine rewrite; kills
the leak class.

**B. Full tree interchange as a formal post-parse stage.** Macros produce
`Node` trees; splicing is structural; scoping and provenance ride the tree.
Buys: richer provenance for E8, no re-parse (already negligible), and a
cleaner substrate IF macros ever need to *inspect* trees (semantic queries
are the recorded beyond-v1 item). Costs: the engine's construction API
(`macro_std::build`, quote/join, the str-returning derives) is text-shaped
and load-bearing — all five derives plus `[service]` would migrate; the
measured verdict said the win doesn't pay today. This is the right shape
*eventually* if semantic queries or heavy metaprogramming arrive; nothing
today forces it.

**C. Status quo.** Leaves the leak. Not acceptable — user code can silently
depend on invisible names.

## 3. Recommendation

Take **A** now (it is the derive-leak fix, properly scoped), and record **B**
as the migration path gated on the first macro feature that genuinely needs
trees (semantic queries / tree-pattern matching), not on hygiene or
performance — both of which A and the existing caches already cover. The
expression-lifting rewrite is a useful precedent for A's mechanics: a
resolution-only wrapper node, exhaustively handled in `for_each_child`, with
the formatter unaffected (expansions never reach `vilan fmt`).

## 4. The real question (user, 2026-07-16): analyzer-integrated macros

The sharper proposal: macros don't emit source at all — they run against the
analyzer and **mutate the analysis graph directly** (register entities,
impls, scope bindings, types). Claimed wins: no parse time, no import
leaking, and — the substantive one — *more capabilities*.

**What the capability win really is.** Today's derives are recursive-
SYNTACTIC by necessity: a macro cannot ask "does this field's type implement
`Wire`?" because it runs before analysis exists. Analyzer integration makes
macros *semantic* — trait queries, resolved types, reachability. That is a
genuinely new power class, and it is already the recorded trigger for the
engine's beyond-v1 work (macro-engine.md §11 "semantic queries").

**The hazards, in order of severity:**

1. **Staging becomes a fixpoint entanglement.** A macro that reads analysis
   (does `Point: Wire` hold?) while other macros' outputs *change* analysis
   (an impl that makes it hold) turns expansion order into a semantic
   observable — the chicken-and-egg every mainstream design dodges: Rust
   keeps proc macros token→token *specifically* so they can't observe types;
   Zig gets semantic comptime only by making the whole analysis LAZY and
   demand-driven. Today's clean staging (expand to fixpoint, then analyze
   once) would need a real replacement, not an amendment.
2. **The API surface is the compiler's most-churned internals.** The graph
   (entities, `Expr`, constraints, scopes) changed shape twice THIS WEEK
   (`LiftRegion`, `prepped_conditions`). Exposing it as the macro contract
   either freezes it or breaks user macros every release. Emission-based
   macros ride a small, stable surface (source text; someday `Node`).
3. **Caching and the LSP.** Worlds and expansions cache on *text keys* —
   deterministic, replayable, and what keeps per-keystroke re-analysis
   cheap. A graph-mutating macro's input is *analyzer state*; invalidation
   becomes graph-shaped, and hermeticity (the property that made
   library-defined macros safe to run at all) is gone.
4. **Provenance gets worse, not better.** Emitted source is inspectable —
   error previews exist, spans exist. Graph mutations have no source to
   show; E8's "diagnostic points into generated code" problem becomes
   "diagnostic points into code that never existed anywhere".

**The middle path that captures the win without the hazards** — split the
proposal's two halves:

- **Semantic READS (queries), staged:** macros keep *emitting*, but gain a
  read-only reflection API over a completed analysis of the non-generated
  program — expand in waves: analyze, expand the macros whose queries are
  answerable, re-analyze, repeat to fixpoint (with a cycle diagnostic when a
  macro's query depends on its own output — the honest version of the
  chicken-and-egg, surfaced instead of resolved by luck). This is Zig's
  discipline grafted onto the existing engine, and const-eval already shares
  the interpreter, so the reflection vocabulary has a home.
- **Graph WRITES stay the compiler's:** what macros "write" remains
  declarative — emitted items, plus (cheaply, now) the scope wrapper for
  hygiene. No parse-time argument survives measurement (0.8%, cached), and
  the leak dies with the wrapper either way.

Recommendation: adopt the wrapper now (§2 A); design the staged-query
reflection API as the engine's v2 when a macro genuinely needs a semantic
answer (the `[derive(Wire)]` all-fields check is the standing candidate);
keep direct graph mutation off the table as a *contract*, even if the
staged expander uses it internally.

## 5. Open questions for the discussion

1. Should an expansion be able to *opt into* exporting an import (re-export
   from generated code)? (Draft answer: no — generated re-exports are spooky;
   a macro can generate the item itself.)
2. Do `macro { .. }` blocks (item position) get the same scope wrapper?
   (Draft: yes — same splice channel, same rule.)
3. Does the wrapper affect the world/expansion caches' keys? (Draft: no —
   caching keys on source text; the wrapper is walk-time structure.)
