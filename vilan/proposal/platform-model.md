# Build model: backends, platforms, and layers

Status: **implemented** (2026-06-23, the platform-model stabilizing slice; see [[roadmap]]
"recently shipped"): `Target` split into `Backend`+`Platform`, library layers declare
`PlatformPattern` sets, std's `src/node` moved to `src/process` under `[library.layer]`, CLI
`--platform`/`--backend`. Proven by the follow-on `deno`+`bun` runtimes (the std `process`
layer serves node/deno/bun unchanged) and the §4.2 completeness check (`vilan check`). **Supersedes library-packages
L3** and **refines L1/L2**. A merge of two designs: the *layer model* (layers declare
the platforms they serve, with version first-class, and the cross-platform contract is
intrinsic to resolution) and the *axis model* (`Target` splits into an emitter
**backend** and a **platform**, with a registry for platform identity). The goal: a
build model stable enough that a new runtime (Deno/Bun), a runtime *version* (Node 24 vs
26), or a language **backend** (WASM) is *additive*, not a rewrite.

The **initially supported** set is deliberately small — platform `node:24` (the current
LTS) and `browser`, backend JS ES2022 — with everything designed so adding more is a
registry / family / layer edit, never a refactor.

---

## 1. The problem

One `Target` enum (`Node | Browser | None`) encodes four independent things, so every
new axis forces it open and ripples through the compiler:

| variable | what it really is | pipeline stage |
|---|---|---|
| **Language** (JS / WASM) | the **backend** — which emitter runs + output ES level | codegen |
| **Runtime** (Node / Deno / Bun / Browser) | the **platform** — what it runs on + how to launch | resolution + runner |
| **Version** (Node 24/26, ES2025) | *two things*: the platform's **runtime version**; the backend's **ES level** | resolution / codegen |
| **APIs** (DOM, Date, `node:http`, `solid-js`) | **not a target axis** — *layers* (host APIs) + *dependencies* | the layer system + the dependency graph |

(This mirrors TypeScript's settled split of `target` (output ES level) vs `lib`
(assumed API surface) vs the runtime — evidence the decomposition is real. Vilan can do
it more cleanly because the layer + dependency systems already model the API surface.)

## 2. The model

A **build configuration** is a `(backend, platform)` pair; everything else is derived.

- **Backend** — *how* to emit. `Js { ecma }` | `Wasm { … }`. The emitter reads this and
  nothing else. A new backend is isolated to the emitter; layers may optionally gate on
  it (binding modules that differ JS-vs-WASM — §5).
- **Platform** — *where* it runs: a `runtime:version` identity (`node:24`, `browser`).
  It selects which **layers** are reachable and, via the registry, how to launch.
- **Layers** — the API/binding surface (below).
- **Dependencies** — installed libraries (the P1/L1 graph), unchanged. A dependency is
  itself a library with layers and may require capabilities.

The **API surface** a program may use is *derived* — the platform's reachable layers
plus declared dependencies — never a field on the target.

## 3. Layers declare the platforms they serve

The pivot from L1/L2: a layer doesn't get its meaning from a platform's ordered list —
**each layer declares the set of platform patterns it applies to**, and a module
resolves to the *most-specific* layer matching the build platform. This is what makes
runtime *and version* fall out of one mechanism.

```toml
[library]
name = "std"
platform = "*"                    # base modules (no layer) serve every platform

[library.layer.process]
platform = ["@process"]           # a family: the process-having runtimes (node, + deno/bun as added)
[library.layer.browser]
platform = ["browser"]
root = "src/browser"              # directory form (below)

# Illustrative — version- or runtime-specific layers, declared only when a binding diverges:
# [library.layer.node-26]
# platform = ["node:26"]          # a version-specific override
```

- A **platform pattern** is a `runtime` + optional `:version` (`node:24`, `node:*`,
  `browser`), or a **family** (`@process`) the registry expands to a pattern set.
- A layer's files join it by **filename suffix** — `http_sys.node.vl` is module
  `http_sys` in layer `node` — or by a **directory** (`root = "src/<layer>"`, or the
  `(layer)/` paren-dir sugar) when many siblings share one layer. Plain `src/foo.vl`
  (no suffix) is the base layer, serving the library's `platform` default (`*`).
- **Layers must be declared** (capability and runtime alike); only a base-only library
  omits the block. This is *required*, not tidy: with the `NAME ≡ NAME/lib.vl` rule, a
  layer directory and a nested-module directory are both subdirectories — and a suffix
  like `.node` is only a layer because the manifest says `node` is one. Declaration is
  what disambiguates a *layer* from a *module*, and gives the checker (§4.2) the
  complete set.

"Capability" (`process`, `browser`) is just the role-name for a layer whose pattern
covers *several* platforms; a "runtime layer" (`node`, `deno`) covers one. Same
mechanism — a declared layer with a pattern — so there is no separate concept.

## 4. Resolution makes the contract intrinsic

### 4.1 Building (a fixed platform)

For a build `(backend B, platform P)`, a module `M` resolves to the **most-specific
layer** whose pattern matches `P` (and whose backend constraint, if any, matches `B`)
that provides `M`. Specificity: exact version (`node:24`) ▸ runtime wildcard (`node:*`)
▸ family / multi-runtime ▸ base (`*`). If no matching layer provides `M` but some
*non-matching* layer does, that's the recoverable **cross-platform diagnostic**
(L1/L2's gate, now pattern-driven); if no layer provides it, an ordinary unresolved
import.

So `import std::http::Server` for `node:24`: `http` resolves via the `process` layer
(`@process` ∋ node); `http`'s `import pkg::http_sys` resolves `http_sys` to the most
specific layer matching `node:24` — the shared `process` binding, unless a `node:26`
override (etc.) is more specific. Version and runtime overrides are just
higher-specificity layers.

### 4.2 Checking (no fixed platform) — the contract *is* resolution

A build resolves *one* `_sys`. But the contract — every runtime supplies it, and they
agree — must be checkable **without a platform** (editing `std`, or a library `check`).
That isn't a separate pass: it's the same resolution, run over the **importing file's
layer's pattern set** instead of one platform. An import `pkg::M` from a layer covering
patterns `S` is valid iff:

- **Completeness** — for every pattern in `S`, some matching layer provides `M`. So
  `http.process.vl` (covering `@process`) importing `pkg::http_sys` requires `http_sys`
  to resolve for every runtime in `@process` — catching "added the node binding, forgot
  deno" the day deno joins the family.
- **Uniformity** — the variants those patterns resolve to share one type signature, so
  the shared interface type-checks against all of them — catching a drifted `_sys`.

The scope is the import site's layer: a `pkg::M` from a *shared* layer must hold across
all the runtimes it serves; a `pkg::M` from a *runtime* layer is that runtime's own
business (a node-only module isn't flagged for missing a deno variant). One rule covers
both building and checking — there is no separate "contract check" feature.

## 5. Host bindings: interface vs implementation

When a host API differs per runtime, split the module into a shared **interface** and
per-runtime **`_sys` implementations** behind a uniform contract (Rust's `std::sys`):

```rust
// http.process.vl — shared, written once
import pkg::http_sys::{ RawServer, create_server, listen };
struct Server { raw: RawServer }
impl Server { fun start(self) { listen(self.raw, ...) } }
```
```rust
// http_sys.node.vl                       // http_sys.deno.vl
[extern("node:http", "createServer")]     [extern("Deno", "serve")]
external fun create_server(..): RawServer;   external fun create_server(..): RawServer;
```

Runtimes share an *interface*, not an *API*; a shim that doesn't conform is a type
error *in its own layer*, never a silent mismatch. When two runtimes can't share even
an interface, drop the shared `http` and ship full per-runtime `http.node.vl` /
`http.deno.vl` — same mechanism.

**The common case needs no split.** `node:http`, `node:fs/promises`, and `process.*`
work on Node, Deno (node: compat), and Bun, so `std::http`/`fs`/`process` are single
modules in the **`process`** layer with their existing `node:` bindings, serving every
runtime in `@process`. The `_sys` split is for genuine divergence (e.g. swapping in
`Deno.serve`) — reserved, not required up front.

**Backend (JS/WASM) shares this seam.** A WASM build imports host functions
differently, so a binding can also gate on backend: `http_sys.wasm.vl`, or a layer with
`backend = ["wasm"]`. The shared interface never changes across backends — only its
`_sys` does. With JS the only backend today, no binding gates on it yet.

## 6. Concrete `std`

```
std/
  vilan.toml                      # [library] + [library.layer.process|browser]
  src/
    option.vl list.vl string.vl number.vl json.vl time.vl ...   # base (platform *)
    (process)/                    # `process` capability layer (@process: node[/deno/bun])
      http.vl  fs.vl  process.vl  # public interfaces
      http_sys.vl                 # portable node:-compat binding (the default)
    (browser)/                    # `browser` capability layer
      dom.vl  ui.vl
    # http_sys.deno.vl / http_sys.node-26.vl — suffix overrides, only if a binding diverges
```

The migration from L2 is small: the three `src/node/` modules move into a `process`
layer (their `node:` bindings are portable), `dom`/`ui` stay in a `browser` layer, and
the manifest declares the two layers. Runtime/version override files appear only when a
binding actually diverges.

## 7. Representation

### 7.1 Types (`vilan-core`)

```rust
enum Backend { Js { ecma: EcmaVersion } /*, Wasm { .. } */ }     // ES2022 is the only EcmaVersion today
struct Platform { runtime: String, version: Version }            // node:24, browser
// A layer's matcher: the platform patterns (+ optional backend) it serves.
struct Layer { name: String, patterns: Vec<PlatformPattern>, backend: Option<Backend>, source: LayerSource }
// PackageSpec.target_roots: Vec<(Target, PathBuf)>  →  layers: Vec<Layer>
```

`Target` (one `Copy` enum) splits into `Backend` + `Platform`. Resolution matches a
build's `(backend, platform)` against each layer's patterns, most-specific wins.

### 7.2 Manifest

A **library** declares its layers (§3); base-only libraries omit the block. An **app**
picks a platform; the backend defaults from it:

```toml
[package]
name = "api"
platform = "node:24"            # → @process layers + base; launcher node; backend js (es2022)
# backend = "js"               # optional; "wasm" later
```

### 7.3 CLI

`--target node` stays as sugar for `--platform node:24 --backend js`; `--platform` /
`--backend` expose the cross-product. `vilan run` uses the platform's launcher from the
registry (so a future `deno`/`bun` run needs no per-runtime CLI code).

### 7.4 Platform registry — the identity half

The layer model handles *which code*; the registry handles *what a platform is*, and is
where the supported set lives — small now, extensible by design:

- **Runtimes** (built-in): `node` (default/only version **24**, the LTS) and `browser`;
  plus `none` (the degenerate platform — base layer only, no launcher/backend,
  check-only). Each defines its launcher, default backend, the ES level it supports, and
  name validation (`noed:24` is an error, not a silent miss). *Adding `deno`/`bun`, or a
  new node version, is a registry entry.*
- **Backends** (built-in): `js` with `ecma = es2022` (the only value today). *Adding
  `wasm`, or a new ES level, is a registry entry.*
- **Families**: `@process = node` today (the process-having runtimes; `deno`/`bun` join
  it when added). A layer says `@process` once, so a new runtime is a one-line family
  edit, not per-library churn — and `process` is shared vocabulary, not a pattern list
  each library re-types.

Custom/declared platforms (a project defining its own runtime) are deferred (§9).

## 8. How it lands on L1/L2 and supersedes L3

- **L1/L2 are most of this.** `PackageSpec` layers, `search_roots`/`available_roots`,
  and `gate_library_imports` *are* the layer machinery — they need the key generalized
  from `Target` to a declared layer with a pattern set, and resolution to do
  most-specific-match instead of `[overlay(T), base]`.
- **The one codegen branch** (`Target::Node ⇒ process.exit`) becomes a platform/backend
  **host-profile** capability (`has_process_exit`).
- **L3 dissolves**, and so does its runtimes-vs-capabilities fork: capabilities are
  multi-platform layers and runtime layers are single-platform layers — one mechanism.
- **Version** (Node 24 vs 26) and **backend** (JS/WASM), which L3 didn't address, both
  fall out: version is part of a platform pattern; backend is the emitter axis layers
  can optionally gate on.

## 9. The stabilizing slice (now) vs deferred

**Now** (contained; corpus stays byte-identical — comparable to L2's threading):

1. `Backend` (`Js { ecma: es2022 }`) + `Platform` (`runtime:version`) replace the
   conflated `Target`; `none` is the degenerate platform.
2. Re-key layers from `Target` to declared layers with **platform pattern sets**;
   resolution does most-specific-match; the cross-platform diagnostic is pattern-driven.
3. Re-layer `std`: `src/node/` → the `process` layer; declare the `process`/`browser`
   layers; registry runtimes `node:24` + `browser` (+ `none`), backend `js:es2022`, and
   the `@process` family (= `node` today).
4. Generalize the transformer's one branch to a host-profile capability.

**Near-term follow-on** (small, high value): the platform-agnostic **contract check**
(§4.2) — it needs only the declared layers; the rest is comparing already-computed
signatures.

**Deferred** (additive, each isolated — and the whole point of the model): `deno`/`bun`
runtimes (a registry line + `@process` membership + any `_sys` overrides); more node
versions; the **WASM** backend; ES-version *downleveling*; declared/custom platforms;
version *ranges* (`node:>=24`) beyond exact + wildcard.

## 10. Test plan

- **Resolution** — most-specific match: a version override layer beats `@process` beats
  base; the portable `process` binding serves the `@process` family.
- **Cross-platform** — `std::http` (process layer) is a clean cross-platform error for a
  `browser` build (recoverable, no cascade); `std::dom` for a `node` build.
- **Declared layers** — a declared suffix/dir is a *layer*; an undeclared `src/util/lib.vl`
  is a *module*; a base-only library with no layer block still resolves.
- **Contract check** — a `process` interface importing `pkg::http_sys` errors when a
  served runtime lacks it (incomplete) or its signature diverges (non-uniform); a
  runtime-layer-only import isn't flagged for other runtimes (import-site scope).
- **`std` re-layering** — corpus 69/69 byte-identical.
- **Registry/families** — a synthetic runtime added to `@process` (one registry line) is
  served by `std::http` with no library change; an unknown runtime name is rejected.

## 11. Open questions

**Settled (from review):**

- **Layers declare the platforms they serve** (pattern sets), with version first-class,
  most-specific-wins resolution, and the cross-platform **contract check intrinsic to
  resolution** (§4) — adopted from the layer model.
- **`Backend` and `Platform` are separate** (the emitter axis vs the runtime), with a
  **registry** for platform identity and **families** (`@process`) for additivity — kept
  from the axis model.
- **Layers are declared explicitly**; base-only libraries omit the block.
- **Supported set**: platforms `node:24` + `browser` (+ `none`), backend JS ES2022;
  everything else additive.

**Still open:**

- **Q1 — backend in patterns or its own field?** Recommend **separate field**, with
  layers optionally constraining on it (`backend = ["wasm"]`) — an emitter choice, not a
  capability. Confirm.
- **Q2 — layer file forms.** Support *both* the `module.layer.vl` suffix (co-locates
  thin variants) and the `(layer)/` directory (bulk), as shown? *(Recommend both; suffix
  for `_sys` overrides, dir for many-module capability layers.)*
- **Q3 — version syntax now.** Exact (`node:24`) + wildcard (`node:*`) + families
  (`@process`) in the first cut; ranges (`node:>=24`) deferred? *(Recommended.)*
- **Q4 — specificity ties.** When two layers match a build equally specifically and both
  provide `M`, error (ambiguous) vs. a declared order? *(Recommend: error — a library
  bug.)*
- **Q5 — slice scope.** Land the backend/platform split + layer re-key + `std`
  re-layering together, or types-first then `std`? Both keep the corpus identical.

---

Once §11 is settled I'll implement the stabilizing slice (§9) against §10, keeping the
corpus byte-identical; then runtimes, versions, and the WASM backend are additive
registry/backend/layer work.
