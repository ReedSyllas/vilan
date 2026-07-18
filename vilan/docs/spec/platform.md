# Spec §11 — The platform model & manifests

A **platform** is a host a build targets: `node` (the default), `deno`,
`bun`, or `browser`. The first three form the **`@process`** family.
One package may build for several platforms at once (§11.4's entries);
the compiler proves, per entry, that no reachable code requires a
capability its platform lacks.

## 11.1 Layers

The standard library is layered:

- the **base** layer — platform-neutral, available everywhere;
- the **browser** layer (`std::dom`, `std::ui`, `std::router`,
  `std::storage`) — browser builds only;
- the **process** layer (`std::fs`, `std::http`, `std::db`,
  `std::process`, `std::rpc_server`) — `@process` builds only.

A library may declare the same shape for itself (`[library.layer]`,
§11.4): a neutral root plus per-platform overlay roots.

## 11.2 Coloring and the reachability check

Every function is **colored** with the platform requirement it implies
— seeded by the layer its externs and std calls live in, flowing
callee-to-caller through the call graph (the same inference shape as
asyncness, §7.3), including through generic instantiations: a generic
function's requirement is judged **per instantiation**, so `save<T>`
colors process-only only for the `T`s whose code actually reaches a
process capability.

The check is on **reachable code, not imports**: importing a module is
free; each entry is checked along the call paths that start at its
`main` (and its reachable initializers). A path that crosses onto a
platform the entry does not build for is a compile error naming the
chain from the entry to the crossing. Module-level initializers obey
the same rule: a binding's initializer is analyzed, colored, and
bundled only if something reachable references the binding. `const`
initializers evaluate at build time (§9) and never color anything.

## 11.3 Fences

`[platform("browser")]` (one platform, a family like `"@process"`, or
several) on a function declares the platforms it promises to run on.
The promise is checked on **every** compile, whatever the build's
entries: if code the fenced function reaches requires a layer one of
the fenced platforms lacks, the error lands **at the fence** with the
offending chain — not at some distant entry in a dependent build.
Fences add no runtime behavior; they are checked declarations.

## 11.4 Manifests (`vilan.toml`)

The manifest declares what a directory builds. Sections:

- **`[package]`** — an application or plain package: `name`,
  `description`, `root` (source root; default `src/`), `entry` (the
  entry file, when there is exactly one), `target` (a platform;
  default `node`), and `dependencies` (name → `{ path = "…" }`; path
  dependencies only — registry dependencies are future work).
- **`[entry.<name>]`** — one build entry per table: `path` (default
  `<root>/<name>.vl`) and `target` (default `node`). A package with
  entries builds each for its own platform; reachability (§11.2) is
  what lets one source tree serve several.
- **`[library]`** — a dependency-only package: `name`, `description`,
  `root`, `dependencies`, and **`[library.layer.<name>]`** overlays
  (`root`, `platform = ["…"]`) for per-platform sources.
- **`[project]`** — a workspace: `packages = ["member", …]` (paths);
  building the project builds every member against its own manifest.
- **`[build]`** — codegen options: `preset` (`"debug"` | `"release"`)
  and the per-feature overrides `indent`, `spaces`, `debug-names`.
  Build options never change program semantics (§7.6), only the
  emitted text.
- **`[macro]`** — the compile-time interpreter budget: `fuel` (steps
  per macro/const run) and `depth` (nested expansion), §9.3/§10.4.

## 11.5 Build products

Each entry emits `dist/<name>.js` for its platform (browser entries
first, so a server that ships bundles finds them fresh), plus
`dist/<name>.css` when const evaluation emitted style assets (§9.2).
`vilan run` builds all entries and starts the one `@process` entry;
`vilan check` checks every entry, always. The emitted text beyond
§7.6's guarantees is implementation-defined.
