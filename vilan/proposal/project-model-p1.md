# P1 — Explicit `vilan.toml` (typed declarative manifest)

Status: **implemented** (2026-06-22). The schema, validation, target precedence,
`none` target, and `NAME.vl` ≡ `NAME/lib.vl` resolution all landed; the decisions
in §8 reflect what shipped. Deferred follow-ups (path-dependency *loading*,
server-side manifest completions) are tracked on the roadmap.

Roadmap: "Next up — project & platform model", item **P1**. P2 (multi-package
workspace + per-package targets) and beyond build on this; P1 deliberately stops
at a *single package*, parsing — but not yet resolving — the workspace and
dependency schema so the file format is stable before P2 wires it up.

---

## 1. Goal

Replace the ad-hoc, partly-magic manifest handling with a **typed, declarative,
validated** `vilan.toml`. Today the manifest is read as an untyped `toml::Table`
and the project shape is inferred (entry defaults to `main.vl`; the package's
module root is silently the entry file's parent directory; the target comes only
from the `--target` flag, never the manifest). P1 makes all of that explicit and
checked, and closes one concrete resolution gap (`NAME.vl` ≡ `NAME/lib.vl`).

The bar (per the engineering principles): a settled schema on paper, then a typed
parser with validation diagnostics and regression tests pinning each rule —
*before* P2 depends on the format.

## 2. What exists today (the magic we are removing)

Grounded in the current code:

- **Manifest parsing** — `crates/vilan-cli/src/main.rs`: `project_from_manifest`
  reads `vilan.toml` as a `toml::Table` and branches structurally:
  `[server]`+`[client]` ⇒ `Project::FullStack`; otherwise `[package] entry`
  (default `"main.vl"`) ⇒ `Project::Single`. `[build]` ⇒ `BuildOptions`. No
  `name`, `description`, `target`, or `dependencies` are read. Unknown keys are
  silently ignored (no validation).
- **Module root** — `analyzer.rs::analyze` sets `pkg_root = entry_path.parent()`.
  The package's own modules (`import pkg::foo`) resolve against that directory.
  Never declared; inferred from the entry path.
- **Module file resolution** — a referenced module `foo` always loads
  `{root}/foo.vl` (`std_module_path` + the load loop, ~analyzer.rs:9492). There
  is **no** `foo/lib.vl` fallback for submodules; only the *package root* of std
  is a `lib.vl`. So `NAME.vl` and `NAME/lib.vl` are **not** interchangeable today.
- **Target** — `Target` is `Node | Browser` (`crates/vilan-core/src/target.rs`),
  default via clap `--target node`. Not sourced from the manifest. `Platform`
  (`Core | Node | Browser`) gates std modules by target.
- **Existing manifests** (must keep parsing): `examples/math` (`[package] name +
  entry`), `examples/reactive-ui` (`[client] entry` only), `examples/fullstack`
  & `examples/todo` (`[server]`+`[client]`, optional `[build]`).

## 3. Target schema

```toml
# ── A package (a buildable/importable unit) ───────────────────────────
[package]
name = "math"              # required; a valid Vilan identifier (it is how the
                           #   package is imported once P2 lands)
description = "..."        # optional, free text
root = "src"               # optional; default "src". The package's source root.
entry = "main.vl"          # optional; default "main.vl". Resolved against `root`
                           #   → src/main.vl. Used by build/run.
target = "node"            # optional; "node" | "browser" | "none". Default "node".

[package.dependencies]
geometry = "1.2"                         # shorthand: a version string (registry)
shapes   = { version = "0.3", path = "../shapes" }   # local path dependency
http     = { version = "1.0", registry = "vilanhub" }

# ── A workspace root (groups packages) ────────────────────────────────
[project]
packages = ["packages/core", "packages/web"]   # paths to package dirs
[project.dependencies]                          # inherited by every member
serde = "1.0"

# ── Codegen options (unchanged from today) ────────────────────────────
[build]
preset = "release"
```

A manifest is **exactly one of**: a *package* (`[package]`) or a *project root*
(`[project]`) — setting both is an error in P1 (Q4). `[server]`/`[client]` stay
valid through P1 for backward-compat; P2 replaces them with per-package targets
(a member package with `target = "browser"`).

## 4. Design

### 4.1 Typed manifest in `vilan-core`

Add `crates/vilan-core/src/manifest.rs` with serde-derived structs, deserialized
from the TOML text (not hand-walked):

```rust
pub struct Manifest {
    pub package: Option<Package>,
    pub project: Option<Project>,
    pub build:   Option<BuildOptionsToml>,   // existing knobs, now typed
    // legacy, deprecated in P2:
    pub server:  Option<EntryOnly>,
    pub client:  Option<EntryOnly>,
}
pub struct Package {
    pub name: String,                // required; validated as an identifier
    pub description: Option<String>,
    pub root: Option<PathBuf>,       // default "src"
    pub entry: Option<PathBuf>,      // default "main.vl", resolved against root
    pub target: Option<TargetName>,  // node | browser | none
}
pub struct Dependency { version: String, registry: Option<String>, path: Option<PathBuf> }
//  serde untagged: a bare string  -> Dependency{ version, .. }   (registry dep)
//                  a table        -> the full form               (registry or path)
pub struct Project { packages: Vec<PathBuf>, dependencies: Map<String, Dependency> }
```

It lives in `vilan-core` (not the CLI) because the **LSP** needs the same parse
to learn a file's target and module root — that is exactly E6/P3. `Manifest`
exposes `parse(text) -> Result<Manifest, Vec<Error>>` and a `validate()` that
returns span-less `Error`s (TOML spans are coarse; a clear message suffices).

The CLI's `project_from_manifest` / `parse_build_options` collapse onto this:
read text ⇒ `Manifest::parse` ⇒ `validate` ⇒ build the existing `Project` enum.

### 4.2 Target from the manifest

`Target` gains `None` (a pure library: only the `Core` platform layer is
reachable; emitting requires picking a concrete host, so `build`/`run` for a
`none` package errors "no target host — set `target` or pass `--target`"). The
effective target is: `--target` flag (explicit override) ► `[package].target` ►
`Node` (default). `Target::parse` learns `"none"`; `Platform::is_available_for`
returns *Core-only* for `None`.

### 4.3 The package source root

The package's module root is now the explicit `[package].root` (default `"src"`),
resolved relative to the manifest directory — *not* inferred from the entry's
parent. `entry` (default `"main.vl"`) and every `pkg::foo` reference resolve
against it: `{manifest_dir}/{root}/main.vl`, `{manifest_dir}/{root}/foo.vl`. In
`analyze`, `pkg_root` becomes `manifest_dir.join(root)` instead of
`entry_path.parent()`.

### 4.4 `NAME.vl` ≡ `NAME/lib.vl`

In the module load loop (analyzer.rs ~9492), resolve a module `name` by trying
`{root}/{name}.vl` then `{root}/{name}/lib.vl`, first existing wins; error if
both exist (ambiguous). One small helper, applied uniformly to Std and Pkg
origins. This is the one resolution gap P1 closes outright.

### 4.5 Validation diagnostics

`validate()` checks:
- `name` is present and a valid Vilan identifier.
- `entry` (resolved against `root`) exists on disk; `root` exists.
- `target` is recognized (`node`/`browser`/`none`).
- exactly one of `[package]` / `[project]` is present (not both, not neither).
- **dependencies**: well-formed; a declared *registry* dependency (a bare version
  string, or a table without `path`) is **rejected** with a clear "registry
  dependency resolution is not yet supported" (Q1, Option A). *Path* dependencies
  (`{ path = ".." }`) are accepted by the schema; their actual loading is the
  multi-package machinery, deferred — see the roadmap step in §8.

Unknown top-level keys ⇒ a warning (serde `deny_unknown_fields` would be too
strict given forward-compat; prefer collect-and-warn).

## 5. Implementation steps

1. `manifest.rs`: structs + `parse` + `validate` + unit tests; re-export from `lib.rs`.
2. `Target::None` + `Target::parse("none")` + `Platform::is_available_for(None)` (Core-only) + display name; update the `--target` help text.
3. CLI: replace `project_from_manifest`/`parse_build_options` ad-hoc reads with `Manifest`; thread `target` (flag ► manifest ► default).
4. Module root: `pkg_root = manifest_dir.join(root)` (default `src`); `entry` resolved against it.
5. Module loader: `{name}.vl` ∥ `{name}/lib.vl` resolution + ambiguity error.
6. Validation wiring: surface `validate()` errors through the CLI's normal diagnostic path; LSP calls `Manifest::parse` for target/root discovery (full LSP cross-target work stays P3).
7. **Manifest autocomplete** (LSP/extension): completion + hover for `vilan.toml` keys and enum values — see §6.
8. Migrate the example manifests to the new schema (the `src` root default moves entry files under `src/`) — see §7.
9. Regression + inference/CLI tests (see §7's test list).

## 6. Manifest autocomplete (LSP/extension)

Now that the manifest is a typed schema, surface it in the editor:

- **Static path (ship first):** a JSON Schema for `vilan.toml` plus VS Code's
  `tomlValidation` / the Even Better TOML contribution point, giving key
  completion, enum values (`target = node|browser|none`), and hover docs with no
  server work. Cheapest, immediate.
- **Server path (follow-on):** teach `vilan-lsp` to recognize `vilan.toml`
  (a second `documentSelector`) and serve `completion`/`hover` from the same
  `Manifest` schema it already parses — keeps one source of truth and extends to
  context-aware completions (e.g. existing package paths under `[project].packages`).

If the server path slips, it moves to the roadmap (§8); the static JSON Schema is
in P1 regardless.

## 7. Test plan

- **manifest.rs unit tests**: each schema form (string vs table dependency;
  `[package]` only; `[project]` only; `[build]`); each validation error (missing
  or non-identifier `name`, missing `entry`/`root`, unknown `target`, both
  sections present, neither present, a declared registry dependency).
- **Path-dependency acceptance**: a `{ path = ".." }` dep parses without error
  (loading still deferred — §8), while a registry dep errors.
- **Backward-compat**: each migrated `examples/*/vilan.toml` builds with
  byte-identical output to before (`examples/math`, `examples/fullstack`).
- **`NAME.vl` ≡ `NAME/lib.vl`**: a fixture importing `pkg::foo` resolves
  identically whether `foo` is `foo.vl` or `foo/lib.vl`; both-present ⇒ error.
- **Target precedence**: manifest `target = "browser"` builds for browser; a
  `--target node` flag overrides it; `target = "none"` + `build` errors.
- **Corpus 69/69 + inference suite** stay green.

## 8. Decisions (settled) & deferred follow-ups

Resolved from review:

- **Q1 — dependencies.** Registry dependencies → **error** ("not yet supported")
  so they're never silently ignored. Path dependencies are accepted by the schema;
  their *loading* is deferred (§8 follow-up below).
- **Q2 — `name` in P1.** Parsed and validated; a package still self-references via
  `pkg::`. Cross-package `name::..` imports are P2. `name` is inert beyond
  validation in P1.
- **Q3 — module root.** New explicit `[package].root` key, default `"src"`.
  `entry` (default `"main.vl"`) resolves against it → `src/main.vl`; `pkg::foo` →
  `src/foo.vl`. `pkg_root` is `manifest_dir.join(root)`, no longer `entry.parent()`.
- **Q4 — `[package]` + `[project]`.** Mutually exclusive in P1; setting both is a
  validation error.
- **Q5 — default `entry`.** Stays `"main.vl"` (no `src/` prefix in the value — the
  `src/` comes from `root`).
- **Q6 — `target = "none"`.** Included in P1 (parse + Core-only gating).
- **Q7 — `name`.** Required.
- **Q8 — legacy `[server]`/`[client]`.** Untouched in P1; P2 replaces them.

Deferred to the roadmap:

- **Path-dependency loading** (Q1): the schema accepts `{ path = ".." }` in P1,
  but resolving a local path dependency means loading another package's modules
  under its `name` namespace — the multi-package machinery. Add as a roadmap step
  alongside / just after P2 (per-package targets), reusing its loader.
- **Server-side manifest completions** (§6) if not landed in P1.

(Roadmap to be updated with these two follow-ups.)

---

Once §7 is settled I'll fold the answers into §3–§5 and implement against §6.
