# Analysis reuse — the E3 arc (leak closure, the prelude checkpoint)

> **Status: Phase 1 SHIPPED 2026-07-21; Phase 2 awaiting the user's read.**
> Backlog E3 (L), reframed by the 2026-07-21 scout; numbers re-measured that
> day.
>
> **Phase 1 outcome + two corrections to §0** (implementation, same day): the
> three uncached sites are closed — the two macro parse sites now route
> through the content-addressed `parse_cached` (gensym stamping bakes the
> site number into the text, so the content key was already site-composite;
> "stamped parses fresh" was optimization-only, per git archaeology), and the
> `run_service` leak was **removed at the root** (nothing ever borrowed the
> input — it was only hashed). Corrections: (1) the scout's "only
> `reactive.vl` uses `fresh(`" was a grep false-positive (`Refresh(`
> matched) — **std ships no gensyms**, so the per-keystroke exposure was
> user-gensym-macro projects, not every UI project; (2) a hypothesized fourth
> site (`flush_rust_fallback`) was measured and disproved — the scout's three
> were complete. Instrumentation: `leak_tally` (14 named sites, thread-local
> counters), and the harness is un-`#[ignore]`d asserting on COUNTERS — the
> measured split on a changing entry is **357 B/analysis of named leak vs
> ~60 KiB/analysis of RSS allocator churn**, which is §0's inference made
> fact and Phase 2's whole motivation: the churn *is* the re-analysis, and
> only the checkpoint removes it.

## 0. What the scout established (corrections to E3's framing)

- **The leak is real and reproduces**: 44.8 KiB/analysis RSS growth (harness
  `measure_per_analysis_leak`, 200 changing analyses, no plateau) ≈ 43.8 MiB
  per 1000 keystrokes, ×(1 + open dependents) since `reanalyze_dependents`
  re-analyzes every other open document per surviving edit.
- **But the named `'static` leaks are small**: the entry source
  (`document.rs:310`) + entry AST (`lib.rs:367`) + per-package display names
  total a few KiB/analysis. Most of the 44.8 KiB is allocator retention from
  rebuilding and dropping a whole `Program` (the reachable std) every call.
  Freeing the named leaks alone recovers little.
- **The real unbounded leak lives in macro expansion, and the harness never
  reaches it**: `parse_generated` is uncached for expression-position macros
  (`macros.rs:1284`), gensym-stamped item macros (`:1331`), and `[service]`
  inputs (`:1000`) — each leaks its parse per analysis. Only `reactive.vl`
  uses gensyms today, so *every UI project* (kolt) re-leaks the reactive
  framework's expansions on every keystroke. The module-loader and
  world/expansion caches are already content-keyed and bounded — the old
  "leaks per module" note is fixed; these three sites are the stragglers.
- **There are no global id counters.** `Id`/`TypeId`/`SourceId` are
  per-analysis fields reset each run. The incremental blocker is that ids are
  minted **densely in one whole-program traversal order** — an edit renumbers
  everything after it, so no prior analysis is partially reusable. Changing
  that touches ~1400 reference sites across every post-parse stage.
- **The LSP's per-keystroke floor** is one full analysis of the reachable std
  (~150 ms measured on a trivial 330-byte file), regardless of the edit.

## 1. The reframe

E3's two halves are really three, in sharply different weight classes:

1. **Close the true leaks** — small, bounded, do now (§2).
2. **Stop re-analyzing the unchanged prelude** — the actual latency *and*
   RSS-churn win, achievable **without touching the id model** (§3).
3. **True incremental analysis** (stable/generation-scoped ids, the ~1400-site
   cross-cut) — explicitly **deferred** (§4); Phase 2's ceiling decides if it
   is ever needed.

## 2. Phase 1 — leak closure + honest instrumentation (S)

- Content-key the three uncached `parse_generated`/`run_service` sites, same
  pattern as the existing `PARSES`/`EXPANSIONS` caches. A stamped expansion's
  text is identical across keystrokes for an unchanged site, so caching turns
  per-keystroke leaks into bounded per-distinct-content entries — the same
  transition the module loader already made.
- Add per-`Box::leak`-site byte counters (a tiny `leaked_bytes(site)` tally,
  test-only surface) so leak claims are *measured*, not RSS-inferred.
- Re-shape the harness: keep the RSS number as a report, but **assert on the
  counters** (bounded leaked-bytes per analysis after warmup) — RSS is too
  noisy to gate on. Un-`#[ignore]` it as the Phase-1 pin.
- Success: counted leak per changing-analysis ≈ entry-source + entry-AST only
  (file-size-proportional, freed... no — still leaked, but *named and
  measured*; eliminating them entirely is Phase 2's side effect for std and a
  recorded refinement for the entry).

## 3. Phase 2 — the prelude checkpoint (M)

The floor cost is re-analyzing identical std sources every call. Ids are
deterministic — identical inputs analyzed in identical order mint identical
dense ids — so the analyzer state **after the always-loaded prelude (and the
workspace's stable dependency set) is byte-reproducible**. Therefore:

- **Snapshot** the analyzer immediately after prelude + dependency loading,
  **clone per analysis**, and analyze only the entry (and changed package
  modules) on top. The clone is indistinguishable from a fresh re-analysis by
  construction (same ids, same maps), so no downstream stage can tell.
- **Keying/invalidation**: the checkpoint is keyed by the content hashes of
  everything folded into it (std sources, dependency sources, manifest shape,
  macro-limit config). Any miss → rebuild the checkpoint (exactly today's
  cost, once) and cache it. The LSP holds one checkpoint per project; the E12
  watch loop holds one per leg.
- **Preconditions to verify at S-time** (the implementation order's first
  job): the `Analyzer` is field-inventory cloneable (maps/vecs of ids and
  leaked `&'static` refs clone shallowly and remain valid — leaked data is
  immortal by definition); no field hides analysis-order state that differs
  between "cloned" and "re-run" (the S2a lesson: enumerate fields
  exhaustively, no catch-all assumptions); thread-local overlays stay
  orthogonal.
- **Expected win**: the ~150 ms trivial-file floor collapses toward
  clone-cost + entry-only analysis; RSS churn (the fragmentation driver)
  drops by the same factor; E12's parse cache composes (parse skipped, now
  analysis skipped too — a watch round approaches entry-proportional).
- **Measure**: the leak harness's wall clock per analysis before/after, plus
  a kolt-shaped fixture (a project importing `std::ui`/`reactive`) so the
  gensym path and a realistic reachable set are in the measurement.

## 4. Phase 3 — deferred: true incremental (stable ids)

Only if Phase 2's ceiling (entry-file-proportional analysis) still hurts on
real projects. It is an XL cross-cut (~1400 id sites, every post-parse
stage, the dense-`SourceId`-indexes-`sources` assumption) and its payoff
over Phase 2 is limited to *large single files* — the one thing the
checkpoint can't shrink. Recorded, not planned. If it ever activates, the
design starts from generation-scoped ids with the checkpoint as generation 0.

## 5. Order and gates

Phase 1 → Phase 2, each suite-gated with the harness pin tightening as it
goes; docs untouched (internal); `caching-plan` gains a pointer to this file.
Phase 2 lands behind a differential guard in the spirit of the house rule:
a test that analyzes a corpus of programs both ways (fresh vs
checkpoint-cloned) and asserts identical diagnostics + identical emitted JS —
the "no downstream stage can tell" claim, pinned rather than argued.
