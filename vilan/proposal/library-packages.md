# Library packages (replaces roadmap P4)

Status: **L1 implemented** (2026-06-23); L2/L3 sketched below. The `[library]`
manifest, target-layered resolution, the per-module layer-availability gate
(replacing P2/P3's coarse per-dependency gate), and the dependencies-must-be-
libraries rule all landed; `examples/fullstack/common` migrated to `[library]`.
The decisions in §7 reflect what shipped. `std` is untouched (its migration is L2),
so the corpus stays byte-identical.

Supersedes the roadmap's P4 ("target-varying modules" via `[[module_override]]`).
A `[library]` package with **target layers** turns out to subsume P4 *and* the
P2/P3 platform-gating model *and* the `std` special-casing — so the per-module
override config is unnecessary.

---

## 1. Goal

Give the language a proper notion of a **library**: an importable unit with a
public surface and no app-shaped baggage (no `entry`, no single host `target`),
that can serve **multiple targets** by layering its source — a shared base plus
per-target overlays. The same import path (`geometry::transform`) resolves to a
different source file per build target, structurally, without per-module config.

This is the concept `std` has always needed and never had. Today `std` is
special-cased three ways — no manifest, no entry/target, and a hardcoded
name→platform map (`Platform::of_std_module` literally matches `"http" => Node`).
A library with target layers replaces all three; `std` becomes an ordinary
library (its migration is step **L2**).

## 2. Why this replaces P4 (and unifies P2/P3)

- **P4 (target-varying modules)** wanted `import std::http` to resolve to
  `http.node.vl` vs `http.deno.vl` per target, via a `[[module_override]]` table.
  With layers, you just put `http.vl` in the `node` layer and a different `http.vl`
  in the `deno` layer — same import path, different source, **no config**. (The
  roadmap already rejected the sibling-file convention `http.node.vl` as having no
  clean implementation; directory layers *are* that implementation.)
- **P2's platform gating** (`Platform::of_std_module`, the hardcoded name map) and
  **P3's cross-target diagnostic** become one mechanism: a module's availability for
  a target is simply **whether it exists in a layer reachable for that target**. No
  name map — the directory a module lives in *is* its platform.
- **P2's coarse dependency-target compat rule** (`gate_dependency_import`: a dep
  must be `none` or the build target) dissolves into the same per-module check — a
  library serves all targets via layers; only the specific modules that have no
  layer for your target are flagged.

## 3. The model

A library declares a base root (shared by all targets) plus optional per-target
overlay roots:

```toml
[library]
name = "geometry"          # required identifier — how dependents import it
description = "..."         # optional
root = "src"               # the base (shared) layer; default "src"

[library.target.node]
root = "src/node"          # node-only overlay

[library.target.browser]
root = "src/browser"       # browser-only overlay

[library.dependencies]
units = { path = "../units" }
```

A library has **no `entry`** (it isn't built as an app) and **no single host
`target`** (it serves all). `[library]` is mutually exclusive with `[package]`
(an app) and `[project]` (a workspace).

### 3.1 Layered resolution

For a build target `T`, a library module `M` resolves by searching, in order, the
**`T` overlay root then the base root** — first match wins, so a `T`-overlay module
*shadows* a base module of the same name:

- `geometry::transform` with `transform.vl` only in `src/` → every target.
- `geometry::gpu` with `gpu.vl` only in `src/browser/` → browser only.
- `geometry::clock` with `clock.vl` in **both** `src/node/` and `src/browser/` →
  the build-target's version (the P4 case, structurally).

### 3.2 Availability = layer presence (the unified gate)

When user code does `import geometry::M` for a build target `T`:

- `M` found in `[T-overlay, base]` → resolve and load.
- `M` not found there but present in **some other** overlay (`T'`) → a recoverable
  cross-target error at the import: "`geometry::M` is in the `node` layer and isn't
  available when building for `browser`" — the exact P3 behavior, now driven by
  layout instead of a hardcoded map.
- `M` found in no layer → an ordinary unresolved-import error (a typo).

This is `Platform::of_std_module` generalized: the loader checks which layer dir a
module is in, rather than consulting a name table. And it **replaces** P2's
per-dependency `gate_dependency_import` — a library is universally importable; the
gate is per-module, at the import (P3's machinery, already in place).

## 4. Step decomposition

Splitting per the project's "build the general thing, then migrate the special
case onto it" instinct, so the risky `std` refactor lands on a proven mechanism:

- **L1 — `[library]` manifest + target-layered resolution (this proposal).** The
  general mechanism, proven on *user* libraries. Migrate the `examples/fullstack`
  `common` package from `[package] target = "none"` to `[library]`. `std` is
  untouched (stays special-cased) this step.
- **L2 — `std` becomes a library.** Give `std` a `[library]` manifest, reorganize
  its 5 platform modules into `node`/`browser` layers (`src/node/{fs,http,process}`,
  `src/browser/{dom,ui}`), delete `Platform::of_std_module`, load `std` through the
  library machinery, and collapse the std-specific gate into the general one.
- **L3 (optional) — open-ended target layers.** Decouple library layer *names*
  from the fixed codegen target enum, so a library can offer a `deno`/`bun` layer.
  Needs a story for how `--target` and host codegen handle unknown targets.

## 5. L1 design

### 5.1 Manifest

Add a `Library` section to `vilan-core::manifest`: `name` (required identifier),
`description`, `root` (default `src`), `target: Map<String, LayerSpec { root }>`,
and `dependencies`. `validate()`: exactly one of `[package]` / `[project]` /
`[library]`; `name` is an identifier; each layer's `root` exists; the layer names
are recognized build targets (L1: `node`/`browser`; L3 relaxes this).

### 5.2 Resolved spec (`analyze`'s input)

P2's `PackageSpec { root, target, dependencies }` gains layers. A library
contributes, per build target, an **ordered search path** of roots
(`[overlay(T), base]`) plus the full layer set (so the loader can tell
cross-target from not-found). Concretely, `PackageSpec.root: PathBuf` becomes a
small `roots` structure: a base plus a `target → root` map. The loader's
`resolve_module_file` searches the ordered roots; the cross-target check consults
the other layers. A plain `[package]`/legacy dependency is the degenerate case
(base only, no overlays) — byte-identical behavior.

### 5.3 Gating

The loader's existing P3 seeding-site gate generalizes: when user code imports
`lib::M`, resolve `M` against the library's `[overlay(T), base]`; if absent there
but present in another layer, emit the spanned cross-target error (reusing the P3
path). `[library]` dependencies use this per-module gate **instead of** P2's
`gate_dependency_import` (a library has no single target to compare). `std`'s
`gate_platform_imports` is untouched in L1 (it migrates in L2).

### 5.4 Resolution (`manifest::resolve_workspace`)

A dependency may now be a `[library]` (the canonical case) as well as the legacy
`[package] target = "none"`. `resolve_workspace` reads `[library]` deps, builds the
layered `PackageSpec`, and recurses through `[library.dependencies]`. Dependencies
**must be libraries** (you can't depend on an app); a `[package]` dependency is an
error (with a migration hint). `common` migrates to `[library]`.

### 5.5 No app/CLI surface change

Apps (`[package]`) and the build/run/workspace flow are unchanged — an app still
has one `target`, and importing a library just gives it that target's layer.

## 6. L1 test plan

- **Layered resolution** — a user library with `base` + `node` + `browser` layers:
  a node app importing a `node`-layer module resolves it; a browser app importing
  the same path resolves the browser version (a varying module); a base module
  resolves for both.
- **Availability** — a browser app importing a `node`-only-layer module gets one
  spanned cross-target error (not a cascade), and the rest types cleanly (P3).
- **Not-found vs cross-target** — importing a module in *no* layer is an ordinary
  unresolved error; importing one in another target's layer is the cross-target
  message.
- **Dependencies-are-libraries** — a `[package]` dependency errors with a hint; a
  `[library]` dependency (incl. transitive, with its own `[library.dependencies]`)
  resolves; a cycle still fails.
- **Migration + regression** — `examples/fullstack` `common` as `[library]` builds
  byte-identically; **corpus 69/69 + inference unchanged** (`std` untouched in L1).

## 7. Decisions (settled) & deferred

Settled from review:

- **Q2 — dependencies must be libraries.** You depend on libraries, not apps; a
  `[package]` dependency is an error (with a migration hint). This lets the
  per-module layer gate fully **replace** P2's `gate_dependency_import`. `common`
  migrates `[package] target="none"` → `[library]` (examples-only breaking change).
- **Q4 — base `lib.vl` may not re-export target-specific items.** Enforced
  concretely: a library's base `lib.vl` re-exports resolve against the **base layer
  only** — re-exporting a module that lives in a target overlay is a clear error
  ("`http` is target-specific; import it by path, or use a per-target `lib.vl`"),
  rather than a `lib.vl` that silently works for some targets and breaks others.
  **Per-target `lib.vl`** (a target overlay can carry its own `lib.vl`, resolved
  against `[overlay, base]`, to re-export target-specific items) is the expected
  eventual direction — **deferred** (not in L1; likely lands with or after L2).

Accepted as recommended (no objection raised):

- **Q1 — `[library]` is a distinct table** (mutually exclusive with `[package]` /
  `[project]`): no `entry`, no single host `target`.
- **Q3 — layer semantics:** search `[overlay(T), base]`, overlay shadows base;
  availability = presence; cross-target = present in another layer.
- **Q5 — per-target root:** explicit `[library.target.<t>] root`, defaulting to
  `src/<t>` when the section is present without a `root`.
- **Q7 — single-target library declaration:** deferred to L2/ergonomics (a
  node-only library expresses itself by putting everything in the `node` layer; a
  cleaner single "doesn't support browser" diagnostic can come later).

Deferred (tracked, to be done):

- **Q6 — open-ended target layers** (`deno`/`bun`, decoupled from the codegen
  target enum) — **L3**. L1/L2 wire only the `node`/`browser` layers. Confirmed
  this needs doing eventually.

---

With §7 settled I'll implement **L1** against §6, keeping the corpus byte-identical
(`std` untouched), then take `std` onto it in **L2**.
