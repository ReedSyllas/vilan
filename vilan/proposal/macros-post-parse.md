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

## 4. Open questions for the discussion

1. Should an expansion be able to *opt into* exporting an import (re-export
   from generated code)? (Draft answer: no — generated re-exports are spooky;
   a macro can generate the item itself.)
2. Do `macro { .. }` blocks (item position) get the same scope wrapper?
   (Draft: yes — same splice channel, same rule.)
3. Does the wrapper affect the world/expansion caches' keys? (Draft: no —
   caching keys on source text; the wrapper is walk-time structure.)
