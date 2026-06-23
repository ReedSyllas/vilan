# P2 — Multi-package workspace + per-package targets

Status: **implemented** (2026-06-22). The loader gained isolated per-package
namespaces, the CLI a `[project]` workspace model (legacy `[server]`/`[client]`
lowered onto it) with path-dependency loading, the target-compatibility rule, and
cycle detection; both examples migrated to workspaces. The decisions in §7 reflect
what shipped. Deferred: `[project.dependencies]` inheritance and registry-dependency
loading (only `path` deps resolve).

Roadmap: "Next up — project & platform model", item **P2**. Builds on P1's typed
manifest. Supersedes the legacy `[server]`/`[client]` form (backlog F1) and
addresses backlog E6. Folds in **path-dependency loading** (parsed-only in P1).

---

## 1. Goal

Graduate the full-stack vision from *one package with two entries* (`[server]` +
`[client]`) to a **workspace of packages**, each targeting its own platform, that
import one another by name:

```
fullstack/
  vilan.toml            # [project] packages = ["client", "server", "common"]
  common/  vilan.toml   # [package] name = "common"  target = "none"
           src/lib.vl   #   shared domain logic (core std only)
  server/  vilan.toml   # [package] name = "server"  target = "node"
           src/main.vl  #   import common::greeting
  client/  vilan.toml   # [package] name = "client"  target = "browser"
           src/main.vl  #   import common::greeting
```

`common` is a pure library (`target = "none"`); `server` and `client` each depend
on it and `import common::greeting`. Building the project builds each member for
its own target.

The deliverable: a package can depend on another (local path dependency), import
its public items under a chosen name, and a project builds all its members — with
target compatibility enforced and a clean per-package module namespace.

## 2. What exists today (post-P1)

- **Manifest** (`vilan-core::manifest`): `[package]` (name/root/entry/target),
  `[package.dependencies]` (path deps *parse* but don't load; registry deps
  error), `[project]` (packages/dependencies — *parsed, not resolved*). Building a
  `[project]` errors "not supported yet". Legacy `[server]`/`[client]` still build.
- **Loader** (`analyzer::analyze`): two roots — `std_root` and a single
  `pkg_root`. A worklist of `(Origin, name)` where `Origin ∈ {Std, Pkg}` loads
  modules into **one flat namespace** (`module_scopes: HashMap<&str, Id>`), with a
  single `pkg` scope (self-reference) and a `std` scope. `collect_module_refs`
  scans top-level imports for `pkg::<m>` / `std::<m>` refs. `pkg_root` is passed in
  by the front-end (CLI from `[package].root`, LSP from the manifest).
- **Targets**: `Target ∈ {Node, Browser, None}`; `Platform::of_std_module` + a
  per-module gate against the build target. One target per `analyze` invocation.
- **Front-end split**: manifest/workspace resolution lives in the CLI/LSP; they
  pass `std_root`, `pkg_root`, `entry_path`, `target` into `analyze`. `analyze`
  itself reads only module files, never manifests. *P2 keeps this split.*

Two structural facts shape the work: the module namespace is **flat** (two
packages with a module `utils` would collide), and there is **one global `pkg`
scope** (a dependency's own `pkg::` self-refs would wrongly resolve against the
importer). Both must generalize.

## 3. Semantics

### 3.1 Packages, dependencies, and import names

A package declares dependencies it may import:

```toml
[package]
name = "server"
target = "node"

[package.dependencies]
common = { path = "../common" }   # the KEY `common` is the import name
```

`import common::greeting` then resolves `common` to that package's public items.
A dependency must itself be a `[package]` (have a manifest with `[package]`).

The **import name is the dependency key** (`common` above), not the dependency's
own `[package].name` — this allows aliasing and decouples a package's identity
from how a dependent spells it (Q2 settles whether they must also match).

### 3.2 The three namespace roots

After P2 an import path's first segment is one of:

- `std::…` — the standard library (shared, as today).
- `pkg::…` — **the current package's own** sibling modules (self-reference;
  package-relative, as today — but now each package has its *own* `pkg`).
- `<dep>::…` — a declared dependency, by its import name.

A dependency's modules referencing `pkg::sibling` resolve **within that
dependency**, never against the importer. `pkg` is always package-local.

### 3.3 Per-package targets and compatibility

Each package has a target. A dependency is loadable by an importer iff:

> `dep.target == none` **or** `dep.target == importer.target`

A `none` library is universal (core std only — importable by any package); a
`node`/`browser` library is importable only by a package of that same target. A
`browser` package importing a `node` package (or vice versa) is a hard error in
P2; **P3** softens the diagnostic so it doesn't cascade. Within one build (one
package + its loadable deps) every module is gated by the building package's
target, which by the rule above is always satisfiable.

### 3.4 Building a workspace

- `vilan build <project-dir>` builds **every** member package for its own target,
  emitting `dist/<package>.js` under the project root (replacing the legacy
  `dist/{client,server}.js`).
- `vilan build <package-dir>` builds a single member (its own target).
- `vilan run <project-dir>` builds the project, then runs its **single
  `node`-target package** (Q3 covers ambiguity / `default-run`).
- `vilan check` type-checks every member.

### 3.5 The dependency graph

Edges come from `[package.dependencies]` path entries. The graph must be **acyclic**
(a cycle is a clear error). `[project] packages` lists the build set and the root
for shared config (`[project.dependencies]` inherited by members — Q4). A package
may depend on another whether or not both are `[project]` members, as long as the
path resolves (Q1 settles whether non-member deps are allowed in P2).

## 4. Architecture

### 4.1 Front-end resolves the graph; `analyze` consumes it

Keeping P1's split, the **front-end** (CLI/LSP) resolves the dependency graph from
manifests and passes a description into `analyze`. New input — a set of package
specs reachable from the entry package:

```rust
pub struct PackageSpec {
    pub import_name: String,  // how the *entry* package addresses it (`common`)
    pub root: PathBuf,        // its source root (manifest dir + `root`)
    pub target: Target,       // its declared target (for the compat rule)
    // its own dependencies, so transitive `dep::dep2::…` resolves
    pub dependencies: Vec<(String, /* index into the spec table */ usize)>,
}
```

`analyze` gains a `packages: &[PackageSpec]` parameter (the entry package's own
root/target stay as today). `analyze` reads only `.vl` files under those roots —
never manifests. The CLI/LSP own all manifest reading, path resolution, the
compatibility check, and cycle detection, and produce the (validated) spec table.

> This keeps `analyze` filesystem-light and unit-testable: a test hands it a spec
> table over temp dirs (as `tests/module_resolution.rs` already does for one
> package).

### 4.2 Loader generalization: per-package namespaces

The loader's worklist key changes from `(Origin, &str)` to `(PackageId, &str)`,
where `PackageId` identifies the entry package, `std`, or a dependency. Each
package gets:

- its **own module set** (no flat-name collision — `module_scopes` becomes keyed
  by `(PackageId, name)`),
- its **own `pkg` scope** (self-reference), childed so its modules see the
  builtins, and
- a **namespace entity** registered under its import name in the importer's scope,
  so `<dep>::module::item` resolves (mirroring how `pkg`/`std` are module entities
  today).

Loading a dependency seeds the worklist with that dependency's reachable modules
(its `pkg::` siblings within itself, its own `<subdep>::` refs mapped through its
dependency list, and `std::` refs). Std stays a single shared package.

### 4.3 `collect_module_refs` over arbitrary roots

Today it scans for one fixed root (`pkg` or `std`). Generalize it to return
`(root, module)` pairs for *every* import whose first segment is a known namespace
(`pkg`, `std`, or a declared dependency name), so the loader can seed the right
package. An unknown root is left for normal name resolution to diagnose (e.g. a
typo'd dependency name → "cannot find `foo`").

### 4.4 CLI: workspace build/run

- `project_from_manifest` learns the `[project]` case: resolve members, build the
  per-package dependency graph (validate paths, targets, acyclicity), and produce
  per-package build jobs. Replace the bespoke `FullStack` enum with a general
  `Project::Workspace { members: Vec<PackageBuild> }`; `Single` stays for a lone
  package / bare file. Legacy `[server]`/`[client]` lowers onto the workspace path
  (a synthesized two-member workspace) so it keeps working unchanged.
- Each member compiles via the existing `compile_to_js`, now also handed its spec
  table (built from its `[package.dependencies]`).
- Output: `dist/<name>.js` per member; `run` selects the node member.

### 4.5 LSP

`resolve_project_context` already finds a file's package and target. Extend it to
also build the file's package spec table (its declared deps), so cross-package
go-to-definition / hover resolve into a dependency's source. (Deep cross-package
LSP polish can lean on P3; P2 wires the resolution so imports at least type-check
in-editor.)

## 5. Implementation steps

1. **Spec plumbing** — add `PackageSpec` + the `packages` parameter to `analyze`
   and `analyze_source` (empty slice = today's single-package behavior, so all
   existing callers/tests pass unchanged at first).
2. **Per-package `pkg` scope** — refactor the single `pkg_scope` into one per
   loaded package; key `module_scopes` by `(PackageId, name)`. (Pure refactor;
   single-package output must stay byte-identical — corpus 69/69.)
3. **Dependency loading in `analyze`** — register each `PackageSpec` as a namespace
   entity, load its modules under its own `pkg`, gate by the building target.
4. **`collect_module_refs` generalization** — collect refs under any known root;
   seed the worklist per package.
5. **CLI workspace resolution** — `[project]` → members + dep graph (paths,
   compat rule, cycles); `Project::Workspace`; per-member spec tables; legacy
   `[server]`/`[client]` lowered onto it; `dist/<name>.js`; `run` selects node.
6. **LSP** — spec table from the file's package deps.
7. **Migrate examples** — `fullstack` + `todo` become real workspaces
   (`common`/`server`/`client`); the legacy form keeps a regression test.
8. **Tests** (see §6).

## 6. Test plan

- **Loader (unit, over temp dirs)** — extend `tests/module_resolution.rs`:
  cross-package import resolves; a dependency's `pkg::` self-ref stays within it;
  two packages with same-named modules don't collide; a `none` dep imports from
  both a node and a browser package; an incompatible-target dep errors; a missing
  dependency name errors; a dependency cycle errors.
- **CLI (integration, over temp workspaces)** — `build` a project emits one JS per
  member for the right target; `run` selects the node member; a bad path /
  incompatible target / cycle reports clearly.
- **Manifest** — dependency-graph validation unit tests (compat rule, cycles).
- **Backward-compat** — legacy `[server]`/`[client]` builds byte-identical to P1;
  single-package and bare-file builds unchanged; **corpus 69/69 + inference**
  stay green.

## 7. Decisions (settled) & remaining defaults

Settled from review:

- **Q1 — visibility = explicit deps.** A package imports `common` **only** if its
  `[package.dependencies]` lists it (path dep). `[project] packages` is just the
  build set + shared config; membership alone grants no visibility. Gives an
  explicit acyclic graph and unifies with path-dependency loading.
- **Q3 — `vilan run` on a project** runs the project's single `node`-target
  package; zero or more than one is an error that points at a future
  `[project] default-run = "<package>"` to disambiguate. `default-run` itself is
  deferred until there's actually a second node member to choose between.
- **Q6 — strict cross-target rule.** A dependency is importable iff its target is
  `none` or equals the importer's target; a cross-platform import is a hard error.
  P3 later softens the *diagnostic* (no cascade) but keeps the rule. No type-only
  cross-target surface in P2.
- **Q8 — migrate both examples.** Convert *both* `fullstack` and `todo` to real
  workspaces (`common`/`server`/`client`). Since no example then exercises the
  legacy `[server]`/`[client]` path, add a **synthetic legacy-form regression
  test** (CLI integration over a temp project) so backward-compat stays covered.

Remaining defaults (recommended; flag if you disagree, else I proceed with them):

- **Q2 — import name = the dependency key** (`shared = { path = "../common" }` →
  `import shared::…`), defaulting to the dependency's own `[package].name` but
  freely overridable (aliasing, as Cargo allows).
- **Q4 — defer `[project.dependencies]` inheritance.** Parse it, but members
  declare their own path deps for now; wire inheritance when registry deps land.
- **Q5 — non-member path deps allowed.** The path is the edge; `[project] packages`
  only controls what `build <project>` compiles.
- **Q7 — flat output** `dist/<name>.js` at the project root (matches today's
  `dist/{client,server}.js`).

---

With §7 settled, the build order is: spec plumbing (`PackageSpec` + empty-slice
no-op) → per-package `pkg` scopes (pure refactor, corpus byte-identical) →
dependency loading → CLI workspace + `run`-selects-node → migrate both examples +
synthetic legacy regression. Each step keeps the corpus green.
