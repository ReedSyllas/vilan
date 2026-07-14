# Platform coloring — function-granular platform checking

**Status: proposed.** The successor granularity to `platform-model.md`: that
document made *modules and layers* the unit of platform truth; this one makes
**functions** the unit for application code, with layers remaining the
declaration mechanism at library boundaries. Motivated concretely by the
`[service]` placement problem (below) and by backlog §E.8's diagnostic
standard. Companion reading: `platform-model.md` (the layer model this builds
on), `library-packages.md` (L1/L2), `transport-rpc.md` §4.2 (the service
macro this liberates).

## 1. The problem

A `[service]` struct must live in a `common` library package today, because
both the browser and the server compile it — and platform checking is
module-granular, so *everything* in that module must resolve on *both*
platforms. The bodies of `[rpc]` methods are compiled by a build that will
never run them, and they pay for it:

```vilan
// common/src/lib.vl — compiled into BOTH builds
impl TodoStore {
    [rpc]
    fun add(self, label: str): i32 {
        self.db.insert(label)      // ✗ cannot exist: `std::db` is @process,
        …                          //   and this module also serves the browser
    }
}
```

So every server resource reaches the service through injected values and
closures, threaded from `server/main.vl` into struct fields. Kolt does this
dance today; the walkthrough and todo examples do it in miniature. The
`context` API cannot rescue it: `let db: Context<Db> = Context::new()` in
`common` already *names* `Db`, and naming a `@process` type from
browser-compiled code is the violation. Contexts solve value plumbing, not
the type-level wall.

The deeper statement of the problem: **the compiler checks platform at the
granularity of "what is compiled together" instead of "what actually runs
on this platform."** The client needs the service's *contract* — signatures,
exposed element types, the hash, the generated stub. Only the server needs
the bodies. The module system currently cannot express that split without
splitting packages.

## 2. The design in one paragraph

Every function gets an inferred **platform requirement** — the set of
platforms it can run on — seeded by std's layer declarations and propagated
through the call graph, exactly as async-ness is inferred today (same
`CallGraph`, same fixpoint shape, same dispatch-candidate treatment for
generics and traits). A build **checks requirements only on code reachable
from its entry point**. Unreachable code is still parsed, analyzed, and
type-checked — it just isn't platform-admitted. An `[rpc]` body that calls
`std::db` is `@process`-colored; the browser build compiles the same module,
generates the same stub and hash from the signatures, and never reaches the
body — so nothing errors. Reach it, and the error names the chain.

## 3. Rules

### 3.1 Color is on code, not on types

- A **function's** requirement = the intersection of the platforms of
  everything it transitively calls or reads, seeded by definition site: a
  function defined in a layer module (e.g. `std/src/process/db.vl`) requires
  that layer's platforms; base-layer and user code start unconstrained.
- A **type is colorless.** `struct Store { db: Db }` is declarable anywhere;
  a field, parameter, or return type may name a platform-specific type
  freely. Color flows through the only way to *obtain* such a value — calling
  colored constructors/functions — so a browser-reachable path can never
  actually hold a `Db`. (Types are erased in the JS backend; there is nothing
  a type name alone can do at runtime.)
- A **module-level initializer** (a `let` at module scope) is code: its
  requirement is checked like a function reachable from every entry that
  loads the module. `let db: Context<Db> = Context::new()` is fine anywhere —
  `Context::new` is neutral; it is `db.run(open_db(), …)` that is colored.

### 3.2 Propagation

- Direct calls and reads: union into the caller, per the call graph.
- Trait/generic dispatch: color via **dispatch candidates**, exactly as
  `async_infer` does — an impl's member for the method, the trait default,
  and the same-named-members over-approximation when a multi-bound parameter
  loses precision. Over-approximate is sound (it can only reject more).
- Monomorphization: a generic function's requirement is computed **per
  instantiation** — `save<T: Persist>(x: T)` is `@process` only for the `T`s
  whose `Persist` impl is.
- Closures, v1 rule: a closure literal's body color **propagates to the
  function that creates it**. Creating a `@process` closure marks the creator
  `@process`. This is deliberately conservative: it keeps closure *values*
  colorless as data (no coloring of fields/params holding them), and it is
  exactly right for the service macro (`dispatcher()` creates the route
  closures containing user bodies, so `dispatcher()` is `@process`; the stub
  creates none). Loosening to invocation-site coloring — letting neutral code
  *store* colored closures it never calls — is recorded as a future step; it
  is the platform analogue of J2's async-type-in-field limitation and should
  probably land with it.

### 3.3 Checking

- Each build has an entry (today: the package `target`'s main). Every
  function **reachable** from that entry must admit the build's platform.
  Reachability is the analyzer's existing notion (the one behind
  reachable-subset std loading and abstract-fn removal).
- Everything else in loaded modules is parsed, name-resolved, and
  type-checked as today — type errors in unreached server code still fail a
  client build. Only the *platform admission* check is reachability-scoped.
  (The machinery half-exists: off-platform modules are already "still loaded
  for typing" with the error reported at the import — analyzer.rs's
  cross-platform import path. This proposal moves the report from the import
  site to reachable use, and deletes it when there is no reachable use.)
- Cross-package imports stop being platform checkpoints for applications: a
  browser app may depend on a `@process`-targeted package, reach its neutral
  items, and never its colored ones.

### 3.4 What layers still do

Declared layers remain the **boundary contract** for `std` and for published
`[library]` packages: a library *promises* platforms per layer, and
`check_library_contract` keeps verifying every module of every layer against
its promise (definition-site, as today — a library must not compile-or-not
depending on who imports it). Coloring consumes those declarations as its
seeds. Application packages — the ones with entries — get inference. A
private path-dependency library may choose either: declare layers and be
contract-checked, or declare none and let its items color inferentially
through the app's build.

### 3.5 Macros

Macro-world code executes at compile time on the host; it is outside the
coloring domain entirely (it already lives in a separate hermetic world).
Macro *output* is ordinary source and colors ordinarily. `[service]`
generation needs no changes: stub + hash derive from signatures; bodies and
`dispatcher()` color `@process` by inference in each world that compiles
them.

### 3.6 Diagnostics (the §E.8 standard, built in from day one)

A violation is reported at the **user's code with the chain**, never at a
std frame:

```
Error: `render_page` cannot run on `browser`
  it requires @process because it calls `std::fs::read_to_string`
    render_page (client/src/main.vl:41)
    → load_template (client/src/main.vl:88)
    → std::fs::read_to_string (@process)
  reachable from the browser entry via `main → route → render_page`
```

The chain is the propagation path the fixpoint already walked; rendering it
is bookkeeping (record one predecessor per colored node, like async_infer's
divergence notes). An error inside macro-generated code names the generating
item (`the route generated for [rpc] fun add`), using the macro engine's
provenance.

## 4. What this unlocks

### 4.1 The service moves home

```vilan
// server/src/store.vl — the service lives WITH its resources
[service(TodoClient)]
struct TodoStore {
    [expose] todos: Signal<List<Todo>>,
}

impl TodoStore {
    [rpc]
    fun add(self, label: str): i32 {
        let db = db_scope.get();          // a Context<Db>, established in main
        db.insert(label)                   // direct std::db use — @process body
    }
}
```

The client package depends on the server package (or a `service` package the
server also uses), imports `TodoClient`, and compiles: the stub and contract
hash are signature-derived; `add`'s body and `dispatcher()` are
`@process`-colored and unreachable from the browser entry. **The injected
closure/value pattern becomes a choice, not a requirement** — and `context`
becomes the natural way to hand resources to handlers (capture-at-creation
through `dispatcher()` already works; the wire-turn machinery proved it).

### 4.2 Single-package full-stack (the package-targets endgame)

Once checking is per-entry-reachability, the three-package ceremony is not
load-bearing. A manifest can declare two entries in one package:

```toml
[package]
name = "todo"

[entry.client]
target = "browser"

[entry.server]
# target defaults to node; path defaults to <name>.vl under the root
```

`vilan build` compiles each entry against its platform; functions color by
inference; the service struct sits wherever it reads best; `common` becomes
taste. (The `EntrySection` remnant in the manifest parser is the fossil of
the old `[server]`/`[client]` sections — this is that idea done right.)

Design, settled at implementation (2026-07-13):

- **Keys.** `[entry.<name>]` takes `target` (same vocabulary and validation
  as `[package] target`; the early sketch's `platform =` key is spelled
  `target` so the manifest has one word for it) and `path` (resolved
  against the package `root`, like `[package] entry`; default `<name>.vl`
  — so `[entry.server]` alone means `src/server.vl`). `target` defaults to
  node. Entries therefore live under the source root by construction, and
  `pkg::` resolves from any of them.
- **Validation.** Entries require a `[package]`; they are mutually
  exclusive with the single-entry keys `[package] entry`/`target` (one
  manifest, one way to say it). An entry name must be a valid identifier
  (it names `dist/<name>.js`); `path` must be relative and free of `..`;
  at least one entry when the table appears.
- **Lowering.** A multi-entry package lowers onto the existing workspace
  orchestration: one build unit per entry, all sharing the package's
  source root, dependencies, and `[build]` options, with outputs at
  `<package dir>/dist/<name>.js` (assets — e.g. a browser entry's CSS —
  beside them, as today). Build order is *semantic*, not
  declaration-order: browser-class entries build first (stable among
  themselves), so a process entry that serves bundles always finds them
  fresh. A `[project]` member may itself declare entries and contributes
  one unit per entry; duplicate output names across a workspace are
  rejected at lowering. `--platform`/`--stdout` don't apply to multi-entry
  builds (exactly as they don't to workspaces).
- **`vilan run` / `vilan check`.** The workspace rules apply unchanged:
  `run` builds everything and runs the single node-platform entry (zero or
  several is an error naming the ambiguity); `check` checks every entry,
  always (§7 decision 4).
- **Editor.** A file matching an entry's path analyzes under that entry's
  target; any other file under the root has no single platform (it may be
  reached from several entries), so its platform is inferred from its own
  imports — harmless, because a module has no `main` and thus no
  admission walk; it still gets hover colors, which are
  platform-independent.
- **The legacy `[server]`/`[client]` pair is retired.** No manifest in the
  tree uses it; `validate` now rejects it with a migration hint pointing
  at `[entry.server]`/`[entry.client]` (the serde fields survive solely to
  render that hint), and the CLI/LSP lowering paths are gone.

## 5. What does NOT change

- `std`'s layers, the resolution order, and the platform registry
  (`platform-model.md` §§3–7) — unchanged; they are the seeds.
- `check_library_contract` for `[library]` packages — unchanged.
- Wire safety: `[derive(Wire)]`'s recursive-syntactic field check stays
  exactly as conservative as it is (analyzer-stabilization notes why);
  coloring neither relaxes nor depends on it.
- Type checking of unreached code, the docs gate, goldens: everything still
  compiles end to end; only platform admission narrows to reachable code.
- Existing multi-package apps keep building bit-for-bit: package targets
  become entry seeds, imports that check today keep checking.

## 6. Delivery

- **Phase 1 — the inference + reachability check** (the substance): the
  coloring fixpoint over `CallGraph` (per-instantiation, dispatch-candidate
  rules), reachability-scoped admission replacing import-site rejection for
  app builds, chain-rendering diagnostics. Manifest untouched. Exit
  criterion: the todo example's service moves into `server/` with direct
  `std::db`-style access (via a small on-disk store; the example keeps
  `todos.json`), client build green, and the error chain renders for a
  deliberate violation.
- **Phase 2 — polish** *(SHIPPED)*: LSP surfacing (colors as hover info;
  violations as live diagnostics under the document's entry), error-format
  bake-off, `--platform` interaction audit (`vilan check` of a multi-entry
  package checks every entry). Landed as: chains label library frames with
  their module (`boot (server::store) → exists (std::fs)`); `vilan check`
  audited (it funnels through the same pipeline as `build`, proven on a
  violating file); live editor diagnostics pinned (manifest `target` drives
  the platform, scratch files infer theirs from imports); and hover shows a
  function's requirement via `platform_color::requirements` — an
  entry-independent per-function fixpoint (multi-source BFS per layer label,
  caller-ward over the same edge expansion as the admission walk) whose
  witness links render a shortest via-chain, e.g. ``requires the `process`
  layer of `std` (via `save → write_file (std::fs)`)``.
- **Phase 3 — multi-entry packages**: the `[entry.<name>]` manifest form,
  `vilan build`/`run` orchestration, the walkthrough rewritten as one
  package (docs: services guide, platform guide, walkthrough — same
  commits).
- **Recorded, not scheduled**: invocation-site closure coloring (with J2's
  field-typed async closures), inferred layer synthesis for publishing a
  colored library, wasm as a third color exercising >2-platform sets.

Tests, per case (the harness patterns exist): seed pickup from a layer
module; propagation through direct call / trait impl / trait default /
multi-bound fallback; per-instantiation generics (same function admitted and
rejected under different `T`s); closure-creation coloring (`dispatcher()`
shape); module-initializer coloring; elision (colored body unreachable from
the entry compiles + runs); the moved-service e2e over `local_rpc` and over
the http mount; chain-rendering `assert_fails_spanning` pins; a
`[library]` contract check unchanged under the new pass.

## 7. Decisions (settled with the user, 2026-07-13)

1. **Terminology**: "platform requirement" in diagnostics and the spec;
   "coloring" in the guide, where the async rhyme helps teach it.
2. **Explicit annotation**: none in v1 — pure inference. A declared
   `[platform(…)]` fence (checked against inference, like a type
   annotation) is cheap to add later where teams want boundaries.
3. **Dependency direction pre–phase 3**: blessed — a client may depend on
   the package that defines the service; it is temporary scaffolding until
   single-package entries land.
4. **`vilan check` on multi-entry packages**: checks all entries, always —
   the contract-check spirit. `--platform` narrows a `build`, not a
   `check`.
