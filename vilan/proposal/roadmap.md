# Vilan Roadmap

A ranked backlog, most-important first, to be tackled roughly in order. Ranked by:
*unblocks real programs* > *cheap correctness* > *daily DX* > *strategic reach* >
*perf / advanced / cleanup*. Effort is S/M/L. Dependencies are noted inline.

Item numbers are **stable identifiers** — other documents and notes cite them
(`roadmap #9`), so shipped items are removed and their numbers retired, never reused.
The tactical companion is [`backlog.md`](backlog.md) (everything outstanding, indexed);
the per-feature design records live in the sibling proposal documents.

---

## Done (through 2026-07-04)

A compact chronicle — details live in the named proposals and the git history.

- **Language core & tooling (Tiers 1–2, #1–#7):** stdlib essentials (`Result`/`String`/`List`
  combinators, `PartialEq`/`==` dispatch), `Map`/`Set` (primitive keys), CLI subcommands +
  the `vilan.toml` project model, LSP autocomplete, the formatter (`vilan fmt` + LSP), and
  the `vilan test` runner.
- **Memory model:** Phases 1–5 (value semantics, second-class views, inferred `borrows`,
  view-returning `Option<&T>`, `for e in &mut`) plus Phase 6 essentials (`Arena`/`Handle`,
  `Shared<T>`) — `memory-management-rev-1.md` / `-impl-plan.md`. Transparent references
  (`transparent-references.md`). The Phase 6+ tail is deferred → backlog §C.
- **Reactive:** ownership & disposal (`reactive-ownership.md`), batching + the wire turn
  (`reactive-batching.md`), variadic generics / `combine` (`variadic-generics.md`).
- **Browser backend (#8):** `std::dom`/`fetch`, full-stack compilation — subsumed and
  stabilized by the platform model below.
- **Project & platform model (P1–P6):** explicit `vilan.toml` (P1), multi-package
  workspaces (P2), cross-target diagnostics (P3), `[library]` packages L1/L2
  (`library-packages.md`), the `Backend`+`Platform` split with layered std and `deno`/`bun`
  runtimes (`platform-model.md`), `--watch` (P5, `watch-mode.md`).
- **Transport/RPC (P6, the XL):** the whole of `transport-rpc.md` — Wire derives, the
  codec seam (JSON + binary, single-pass envelopes), HTTP/SSE/WebSocket transports with
  the socket multiplex, `[service(Client)]` generation + `Client::connect` with contract
  enforcement, the typed reactive protocol over codecs, the todo app + benchmarks — plus
  the complete follow-ups ladder and the leftover-JSON audit (`p6-followups.md`).
- **Derives** (`[derive(PartialEq, Default, Debug, Json, Wire)]`) — shipped as the
  special-cased subset of the macro engine (#9).
- **Solver stabilization:** the generic-dispatch bug cluster closed with per-case pins
  (`analyzer-refactor.md`, `constraint-queue-plan.md`, `type-solver.md`). Residual small
  gaps are indexed in backlog §B/§H.
- **Try/Lift operators (B11)** — `expr!` assert-or-return and `?.` lifted chains over real
  `Try`/`Verdict`/`Lift` traits (`try-and-lift.md`), adopted across std/examples/generation;
  plus the surrounding stabilization arc: return-position checking (B10,
  `ret-checking.md`), diagnostics source attribution (E1), span precision (E7).

---

## Remaining, ranked

9. **Macro engine** (L; **proposal: `macro-engine.md`; Phases 0–1 SHIPPED
   2026-07-06** — the fueled `js::Node` interpreter + `macro_std` with the 70/70
   equivalence gate, then `macro fun` items, per-file hermetic worlds, and
   `[attr]`/`[derive(X)]` expansion with both caches; a library-defined
   `derive_display` runs end to end. Next = Phase 2, `macro name(..)` invocations) —
   **the frontier.** User-land vilan running inside the compiler:
   `macro fun` items over the `macro_std::meta` reflection surface, hermetic
   per-function isolation (bodies see only `macro_std`, via the general block-scoped
   imports — backlog H2, a prerequisite), `[attr]`/`[derive(X)]` + `macro name(..)`
   invocations, a fueled interpreter (§5), and per-invocation text-level caching made
   sound by enforced determinism (§6). Subsumes the built-in derives + `[service]`
   generation behind a byte-identical goldens gate. Unblocks #15 and backlog G1's
   consumers. Build order: Phase 0 = `macro_std` + the interpreter core.

10. **LSP semantic highlighting** (M) — semantic tokens, precision over the TextMate
    grammar. (Backlog §E carries the rest of the LSP list, including the higher-value
    diagnostics-attribution work E1.)

12. **Fix per-analysis `Box::leak` + incremental analysis** (L) — the leak grows each
    keystroke/compile; true incremental is blocked by the global `entity_id`/`type_id`
    counters. Debounce masks the latency — measure first. (caching plan Tier 2/3)

13. **LSP sub-file incremental parsing** (L) — tree-sitter-style reuse; chumsky is a
    batch parser, so this is the largest, lowest-priority LSP item.

15. **Numeric types `u8`…`i64`/`f32`** (S) — low value for a JS target (collapse to
    `f64`/`BigInt`); do via #9's macro engine, or defer to a non-JS backend (backlog
    F3/F4, where distinct integer widths are real).

**Strategic candidates, not yet ranked/committed** (each wants its own proposal before
building): the **WASM backend** (backlog F3), the **native backend** (backlog F4), the
**language specification** (backlog D1), and the **memory Phase 6+ tail** (backlog §C).
