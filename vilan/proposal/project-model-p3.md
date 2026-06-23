# P3 — Cross-target imports diagnose, don't break typings

Status: **plan, awaiting review** (open questions in §6 to settle before implementing)

Roadmap: "Next up — project & platform model", item **P3**. An error-recovery
requirement on P2's target gating. Builds on P1 (the target model) and P2 (the
per-package targets + the cross-package compat rule).

---

## 1. Goal

A cross-target import — a `browser` build reaching for `std::http`, or a package
importing a dependency of an incompatible target — should produce **one clear
diagnostic at the import**, and the analyzer should keep typing the rest of the
file **as if the import were allowed**, so a single mistake doesn't cascade.

Today it cascades badly. A browser build of a Node program
(`import std::http::{ Server }; import std::fs::read_file_to_str`) reports **18
diagnostics**: the two real ones (`std::http`/`std::fs` not available) are buried
under sixteen follow-ons — `cannot find 'http' in the imported path`, `cannot find
'read_file_to_str'`, `cannot find type 'Server'`, `cannot call a non-function
value`, … — because the gated module is *skip-loaded*, so none of its symbols
bind. The two real diagnostics also carry **no source span** (they render at
`EMPTY_SPAN`).

P3's target for that same program: **2 diagnostics**, each pointing at its
`import` line, and nothing else.

## 2. What happens today (the cascade)

- **Platform std gate** — in the loader (`analyzer.rs`, the worklist loop), a std
  module whose platform layer isn't available for the build target pushes a
  diagnostic (`EMPTY_SPAN`) and **`continue`s — skipping the load entirely**. The
  module's symbols never bind, so every downstream use is an "unresolved" error.
  Worse, the gate fires per-load with no span, and (since loads are skipped) the
  user never even sees a single coherent message.
- **Cross-package compat** — `manifest::resolve_workspace` **hard-fails** (returns
  `Err`) when a dependency's target is incompatible (P2's `none`-or-same rule). In
  the CLI that's one clean error (the build stops). But in the **LSP** it degrades
  to `unwrap_or_default()` → an *empty* workspace → `import dep::x` is unresolved →
  the same cascade, in the editor.
- Diagnostics render against the entry file's source text (`compile_to_js` /
  `analyze_source`), so a span only renders correctly when it lies in the entry
  file (SourceId 0). In the LSP the open document *is* the entry, so import-site
  spans there are always correct.

## 3. Design — gate at the import, never skip-load

One principle: **the loader never skip-loads a module for target reasons.** A
cross-target module is loaded for *typing* (its signatures bind, so downstream
resolves), and the violation is reported as a recoverable **error** at the import
site. The build still fails (you can't emit a browser bundle that calls Node
`http`) — codegen is gated on an empty error list — but the *type* pass runs to
completion, so there's no cascade.

The check moves to the **user-code import seeding points**, where both the import
span and the "is this the user's own import vs. a transitive std-internal one" are
known:

### 3.1 Platform std gate (the common case)

- **Remove the worklist gate-skip.** Std modules load regardless of target.
- When seeding std modules **from user code** — the entry's `import std::..` and
  each user package module's `import std::..` (the `Pkg`/`Dep` origins in the
  loop) — check `Platform::of_std_module(name).is_available_for(target)`. If not,
  push one error **with the import's span**. Still enqueue the load.
- Std-internal seeding (a std module's own `pkg::`/`std::` refs, the std `lib.vl`,
  the always-loaded core primitives) is **not** gated — so `std::http` loading and
  pulling in `std::fs` transitively does not spawn a second, spurious diagnostic
  for `fs` the user never imported.

This naturally yields exactly one diagnostic per *user-written* cross-target
import, each spanned, and the rest of the file types cleanly.

### 3.2 Cross-package target gate

- **`resolve_workspace` stops hard-failing on the compat rule.** It still detects
  cycles (those stay fatal) and still records every `path` dependency with its
  declared `target` in the `PackageSpec` — it just no longer rejects an
  incompatible one. (The data to gate is already on `PackageSpec.target`.)
- In the loader, when seeding a dependency **from user code** (`import <dep>::..`
  in the entry or a package module), check the dependency's target against the
  build target (`none` or equal = ok). If incompatible, push one spanned error.
  Still enqueue the load, so `dep::item` resolves and the file types cleanly.
- The CLI build still fails (the error blocks codegen); the LSP now shows one
  squiggle at the import instead of an unresolved-name cascade.

### 3.3 Spans

`collect_module_refs` currently returns `Vec<&str>` (names only). Extend it to
carry each import's span (`Vec<(&str, Span)>`), threaded to the seeding sites so
the gate diagnostics point at the `import`. Callers that don't need the span
ignore it. Entry-file imports (SourceId 0, and the LSP's open document) render
precisely; a gated import inside a *non-entry* package module carries that
module's span (correct in the LSP when that file is open; in the CLI it renders
against the entry text, an existing multi-file-diagnostic limitation — §6 Q3).

## 4. Implementation steps

1. **Spanned module refs** — `collect_module_refs` returns `(name, span)` pairs;
   update its call sites (most map `(n, _) → n`).
2. **Drop the worklist gate-skip** — std modules always load. (Corpus byte-identical
   for in-target builds; the *behavior change* is only for cross-target builds,
   which were already failing.)
3. **Platform gate at user-code seeding** — a small helper checks a std import's
   availability and pushes a spanned error; applied at the entry's `std::` seeding
   and the `Pkg`/`Dep` modules' `std::` seeding. De-duplicate (one error per
   `(source, module)`).
4. **Cross-package gate** — `resolve_workspace` drops the compat hard-fail (keeps
   cycle detection); add the dependency-target check at the `<dep>::` user-code
   seeding, pushing a spanned error.
5. **CLI/LSP** — no signature changes (both already pass the workspace through).
   Verify the LSP surfaces the single import-site diagnostic; verify the CLI build
   still fails on a cross-target import (now with 1–2 clean diagnostics).
6. **Tests** (see §5).

## 5. Test plan

- **No cascade (the headline)** — the browser-build-of-a-Node-program fixture
  yields exactly the cross-target errors (one per offending `import`) and **no**
  `cannot find` follow-ons; assert the count and that downstream names resolve.
- **Spanned** — the diagnostic's span covers the `import` (not `EMPTY_SPAN`).
- **Transitive not double-reported** — importing `std::http` (which pulls `std::fs`)
  reports `http` only, not `fs`.
- **In-target unaffected** — a Node build importing `std::http` is clean; a browser
  build importing `std::dom` is clean. **Corpus 69/69 + inference byte-identical.**
- **Cross-package** — a `browser` package importing a `node` dependency reports one
  import-site error and the dependency's symbols still resolve (no cascade); a
  cycle still hard-fails; a `none` dependency imports cleanly from any target.
- **LSP** — analyzing a browser file importing `std::http` returns one diagnostic,
  and hover/go-to-def on `Server` still resolve (typing succeeded).

## 6. Decisions (settled) & remaining notes

Settled from review:

- **Q1 — recoverable error.** A cross-target import enters `program.diagnostics`
  and **blocks codegen** (the build fails — it genuinely can't emit), but the type
  pass continues so there's no cascade. Not a warning: a broken bundle never emits.
- **Q2 — gate at the import site.** Both the platform-std and the cross-package
  violation are reported on the offending `import`, with that import's span (an
  editor squiggle), via the loader's user-code seeding — not at the manifest level.
- **Q3 — span every import site.** `collect_module_refs` carries import spans so
  every cross-target import points at its `import` line. Precise in the LSP; a
  non-entry module's span rendering against entry text in the CLI is a pre-existing
  multi-file-diagnostic limitation (no worse than today). A general per-diagnostic
  source association is a separate, larger cleanup — out of scope.

Remaining notes (proceeding with the recommendation unless flagged):

- **Q4 — `none` libraries reaching for platform std** are caught by the same §3.1
  gate (a `none` build makes every platform layer unavailable) and surfaced as one
  import-site error. Same mechanism, no special case.
- **Q5 — loading-for-typing safety.** Loading e.g. `std::http` purely to type-check
  it should be safe (`external` signatures), but a platform module's source could
  reference something genuinely absent off-target. The plan includes a **test sweep**
  over the platform std modules (`http`/`fs`/`process`/`dom`/`ui`) under the opposite
  target to confirm loading-for-typing stays clean; if one isn't, gate that module's
  *body* walk (load signatures only). Resolved during implementation.

---

With §6 settled, I'll implement against §4/§5, landing it so the corpus stays
byte-identical and the headline fixture drops from 18 diagnostics to the 1–2 real
cross-target ones, each spanned at its `import`.
