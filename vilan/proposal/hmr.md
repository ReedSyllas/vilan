# Hot module replacement — closing the dev loop (A13)

> **Status: DRAFT 2026-07-20 — for review.** Backlog A13 (L; proposal first; before
> A7, ahead of F5/F7 — user calls 2026-07-18). Goal: edit a source file and the
> running browser app updates without a full reload, reactive state preserved.
> Sequenced ahead of A7 (SSR/hydration) because the two share their hardest
> groundwork — stable identities for state and a transfer classification — and HMR
> exercises both without also needing serialization (§4). This document settles the
> design; facts about the existing machinery were verified against the code
> 2026-07-20 (file references inline).

## 0. What exists, and what that dictates

The dev loop today (all verified):

- **Watch** re-runs the *whole command* on any `.vl` change — a 300 ms poll, no
  incremental path (`watch-mode.md`; `crates/vilan-cli/src/main.rs`,
  `watch_loop`). `run --watch` kills and restarts the one Node child each round.
- **Emission** is one flat JS bundle per workspace leg (`dist/<name>.js`) plus a
  CSS sidecar (`dist/<name>.css`). There is **no dev static server and no emitted
  HTML** — the user's server leg serves the bundle from disk, and the HTML page is
  hand-authored source. The client boots via `main`'s body inlined at the bundle's
  tail (an async IIFE when `main` awaits).
- **Module state** emits as flat top-level `let` with **stable, source-derived
  names by default** (the `Readable` name style; only the `[build]` release preset
  mangles). Module bindings are enumerated by the analyzer
  (`module_level_bindings`) — the compiler statically knows every one.
- **The reactive runtime is std vilan**, not a JS prelude: `Signal` is two
  `Shared` cells (value + subscriber list), teardown is `Owner`-scoped, and
  `ui.mount_root` returns the root `Owner`. Disposing that owner plus clearing
  the container element is a complete unmount; nothing does both today.
- **K6 reconnect** already lets a client survive a server restart: `SocketDuplex`
  outlives the socket, redials with backoff, re-attaches mirrors, and resyncs
  their caches from the server's current values. Contract drift closes the duplex
  permanently.
- **Const-eval assets are build-only**: the `run` paths discard `const_assets`
  and never call `write_assets` (`const-eval.md` records the gap). A13's CSS
  hot-swap needs them on disk each watch round — the G2 tail is slice 0.

Two consequences drive the whole design:

1. **Whole-bundle swap is the honest v1.** There is no per-module emission and no
   component re-render unit to lean on (Solid's HMR lesson: fine-grained
   reactivity means *identity* is the feature, not module boundaries). Rebuilding
   everything is what watch already does, and full rebuilds are fast (§7 of the
   caching plan bought that). Per-module swap would require module-boundary
   emission for a payoff — preserving *local* UI state — that module boundaries
   alone don't deliver anyway. Evaluate later, don't presume (§8).
2. **Change detection by output bytes, not input analysis.** Each watch round
   rebuilds every leg (unchanged philosophy); then the *artifacts* are compared:
   server bundle bytes changed → restart the server child; client bundle bytes
   changed → push a swap; only the CSS sidecar changed → push a CSS hot-swap; no
   bytes changed → do nothing. No dependency tracking, no per-leg watchers, and
   the classification is exact by construction — the same byte-identity principle
   the corpus gate runs on.

## 1. Surface

**HMR is part of `run --watch`** for a workspace with a browser leg — no new
subcommand, no new flag to learn; `--no-hmr` opts out (plain restart-the-server
behavior, exactly today's). Rationale: `run --watch` already *means* "the dev
loop"; a separate `vilan dev` would be a second name for the same intent.
Instrumentation (§5) applies only to bundles built by an HMR-active `run
--watch`, so `build` output — and every golden — is byte-identical to today.

A single-package browser app cannot `run` today (no Node leg to execute); it is
out of v1's scope and recorded in §8 (the dev channel's static serving could
grow to cover it).

## 2. The dev channel

The CLI hosts a tiny HTTP endpoint on `127.0.0.1` (default port **35917**;
`--hmr-port` overrides) with three routes, hand-rolled on `std::net::TcpListener`
in keeping with the dependency-free watcher — SSE needs no websocket handshake,
no SHA-1, no crate:

- `GET /events` — **Server-Sent Events**. On each watch round the CLI pushes one
  event describing what changed: `{ kind: "swap" | "css" | "reload", version }`.
  `version` is a monotonically increasing build counter.
- `GET /bundle/<leg>.js` and `GET /asset/<leg>.css` — the current artifacts,
  served from `dist/` with `Access-Control-Allow-Origin: *` (the page origin is
  the user's server, not the CLI).

The browser side is the **dev runtime**: a small JS shim prepended to
HMR-instrumented client bundles. It installs itself once as a
`window.__VILAN_HMR__` singleton (a re-evaluated bundle reuses the live
instance), connects an `EventSource` to the embedded port, and reacts:

- `swap` → fetch the new bundle, run the swap protocol (§3).
- `css` → find the stylesheet `<link>` whose href ends in the sidecar's name and
  bump a cache-busting query param — no reload, no swap. (Requires the `<link>`
  idiom; an app that inlines its CSS gets a full `swap` instead — the byte-diff
  already classifies this correctly, since inlined CSS changes the bundle.)
- `reload` → `location.reload()` — the escape hatch the CLI can always fall back
  to, and the dev runtime's own response to any swap failure.

On connect, the CLI sends the current `version`; the dev runtime compares it to
the version embedded in its own bundle and immediately requests a swap if stale.
This heals the fresh-tab-staleness case for free: the common serving idiom reads
`dist/client.js` once at server boot, so a new tab after a client-only edit gets
a stale bundle — which then swaps itself forward on its first heartbeat.

## 3. The swap protocol

On a `swap` event the dev runtime, in order:

1. **Capture** — read every exposed binding's transfer value (§4) from the live
   registry into a seed map `{ key → { fingerprint, value } }`. A getter that
   throws skips its binding (fresh init instead).
2. **Teardown** — run the registered teardown list: dispose each recorded root
   `Owner` and `clear()` its container (registered by `mount_root`, §5), close
   each live `SocketDuplex`'s socket (registered at dial). Disposal clears
   subscriber lists, so any microtask still in flight from the old turn scheduler
   notifies into emptiness — inert by construction.
3. **Evaluate** — `import()` the fetched bundle text via a Blob URL (bundles are
   module scripts; top-level `let` is module-scoped, so old and new never
   collide). The new bundle's instrumented initializers consult the seed map
   (§4), its inlined `main` re-runs, remounts the UI, and re-dials RPC — a fresh
   duplex against the still-running server, so mirrors resync exactly as K6
   reconnect does today.
4. **On any failure** — teardown already ran, so don't limp: `location.reload()`.

What this preserves and what it doesn't (v1, stated honestly):

- **Preserved**: module-level state (the transfer set, §4) and everything the
  server holds — which in the full-stack idiom is most durable state; mirrors and
  `Draft` cells resync from the server on the fresh duplex.
- **Reset**: state minted *inside* functions during mount — ephemeral UI signals,
  half-typed uncommitted input, focus, scroll. Fine-grained reactivity gives
  these no stable identity to key on; inventing one (positional component
  identity) is the A7-adjacent refinement, §8. Un-pushed dirty `Draft` text is
  lost with them — recorded, with A14's debounced auto-push as the mitigation.

## 4. Identity and transfer — the A7 groundwork

**Identity.** Every module-level binding gets a compiler-minted key:
`package::module_path::binding_name` — stable across builds by construction
(source-derived), and correctly *not* stable across a rename (a renamed binding
is a new thing; it fresh-initializes). Alongside the key, a **fingerprint**: a
stable hash of the binding's canonical structural type. A seed entry is adopted
only when key *and* fingerprint match; an edit that changes a binding's type
falls back to fresh init for that binding, silently correct instead of adopting
a stale shape.

**Transfer is in-heap, not serialized.** The old and new bundle share one JS
realm, so transfer passes values by reference — no Wire bound, no codec, no
derive requirement. What makes a value *safe* to pass is that it carries no old
code: the **plain-data classification const-eval already defines** (scalars,
`str`, lists, options, structs/enums of plain data — no closures, promises,
views, resources) is reused as the transfer test, applied per binding type at
compile time:

- plain-data binding → transfer the value itself;
- `Signal<T>` / `Shared<T>` with plain `T` → transfer the **payload**
  (`.get()` / `.read()`); the new bundle constructs a fresh cell seeded with it —
  old subscribers must not survive, only the value does;
- anything else (a closure-holding struct, a `View`, a resource — module-level
  resources are loan-only and never drop, so the old bundle's is simply
  abandoned to the realm) → not exposed, fresh init.

**Why this is the A7 groundwork.** Hydration needs the same two artifacts —
stable state keys and a which-values-can-cross classification — plus
serialization, because SSR crosses a process boundary. HMR proves the identity
and classification halves in-heap; A7 adds `Wire` on top. That is the reason A13
goes first, made concrete.

## 5. Compiler emission (HMR builds only)

A `BuildOptions { hmr: bool }` flag, set only by an HMR-active `run --watch` —
never by `build`, so goldens and release output are untouched. When set, for the
browser leg:

- **Prepend the dev runtime** (a fixed JS shim, like `__shared_new` — small,
  reviewed, no external fetch) with the port and build version embedded.
- **Wrap each transferable module binding's initializer**:
  `let counter = __hmr_adopt("app::counter", FP, () => 0);` — adopt returns the
  seed value on key+fingerprint match, else runs the thunk. For signal/shared
  bindings the transformer emits the seed-the-payload form.
- **Expose each transferable binding** at the module tail:
  `__hmr_expose("app::counter", FP, () => counter)` — for signals, the getter
  the transformer emits reads the payload. Getters are closures over the live
  bindings, so capture at swap time reads current values.
- **Registration hooks**: `mount_root` and the duplex dial register with the dev
  runtime's teardown list. Delivered as a `std`-internal hook that is a no-op
  when `window.__VILAN_HMR__` is absent — one guarded call each, zero cost in
  production bundles (and dead-code-free there is a nice-to-have, not a
  requirement, since production bundles aren't HMR-instrumented anyway; the hook
  compiles to a host-global check).

The interpreter needs no `__hmr_*` arms: HMR emission never runs under the
equivalence gate (it is `run --watch`-only), and the gate's builds don't set the
flag. Pin that assumption with a test asserting `build` output is byte-identical
with and without a watch-mode compile in the same process.

## 6. Full-stack coordination

Per watch round, after rebuilding all legs (browser legs first, as today):

- **Server bundle changed** → kill + restart the Node child (existing
  machinery), then push the round's client event if any. The client survives via
  K6 reconnect; if the shared contract drifted, the client bundle necessarily
  changed too (shared source), so the same round pushes a `swap` — the fresh
  duplex dials the new contract and never hits the drift-close. A server-only
  edit leaves the client connected through one backoff cycle, exactly as today.
- **Client bundle changed, server didn't** → push `swap`; the server keeps
  running and its port stays warm.
- **Only a CSS sidecar changed** → push `css`.
- **Compile error** → push nothing; report in the terminal as today (the running
  app keeps its last good build — the standard HMR contract).

## 7. Classification, risks, non-goals

- **Closure rule**: not a model change — no new alias kind, no epoch event, no
  language semantics at all. This is tooling plus dev-only emission.
- **Zombie risk**: anything the old bundle scheduled outside owner tracking
  (a raw `set_interval` extern, a bare spawned task) keeps running after
  teardown. v1 records this; the practical mitigation is that std's own
  machinery (effects, subscriptions, the duplex) is teardown-registered, and a
  stray timer's writes land in disposed cells. If it bites in practice, the
  refinement is owner-tracking timers — independently worth considering.
- **Server-side HMR**: a non-goal, permanently — restart is the model for the
  Node leg; the process is cheap and correctness is free.
- **Security**: the dev channel binds `127.0.0.1` only and serves only `dist/`
  artifacts.

## 8. Recorded refinements (not v1)

- **Local-state identity** (positional/component keys) — the piece that would
  preserve in-flight UI state; shared design space with A7's resumable
  hydration. Evaluate after v1 ships and the loss is felt (or isn't).
- **Per-module swap** via module-boundary emission — only worth it if whole-
  bundle re-eval ever gets slow; measure first.
- **Single-leg browser dev**: grow the dev channel's static serving into a tiny
  dev server (`index.html` + bundle) so `run --watch` works without a Node leg.
- **Watch precision**: watch exactly `Program.sources` (the `watch-mode.md`
  refinement) — orthogonal, becomes more attractive as HMR tightens the loop.

## 9. Open questions — calls wanted before S1

- **(a) Surface**: HMR default-on under `run --watch` with `--no-hmr` opt-out
  (recommendation), vs opt-in `--hmr`, vs a `vilan dev` subcommand.
- **(b) Adoption miss** (key present, fingerprint changed): silent fresh init
  with a dev-runtime console note (recommendation — the binding's type changed;
  fresh is correct), vs full reload for the whole swap.
- **(c) Un-pushed `Draft` state**: accept the v1 loss (recommendation; A14's
  debounced auto-push shrinks the window), vs teardown-flush dirty drafts before
  swap (couples HMR to Draft semantics and can push half-typed state).

## 10. Slices (suite-gated, docs same commit, per-case pins)

1. **S0 — the G2 tail**: `run` and `run --watch` write assets each round
   (`write_assets` on the run paths). Pins: a CLI test per path; sidecar bytes
   refresh on a watch round. Ships alone — it also fixes `run`'s missing-CSS gap
   today.
2. **S1 — the dev channel + live reload**: SSE endpoint, artifact routes,
   byte-diff classification in the watch round, dev-runtime shim with
   `reload`-on-any-change and `css` hot-swap. No state carryover yet — this
   slice alone is live-reload + CSS-without-reload, a complete DX win at low
   risk. Pins: unit tests for the byte-diff classifier and SSE framing; an
   end-to-end CLI test driving a round and asserting the pushed event.
3. **S2 — the swap**: identity + fingerprints, `__hmr_adopt`/`__hmr_expose`
   emission, teardown registration (`mount_root`, duplex), Blob-import swap,
   failure → reload. Pins: transformer unit tests per emission shape (value /
   signal / shared / excluded); headless DOM-stub e2e (the A10 harness): boot,
   mutate module state, swap in an edited bundle, assert carryover + new code
   live + old subscriptions dead; the build-output-unchanged pin (§5).
4. **S3 — full-stack proof**: the §6 coordination matrix pinned (server-only /
   client-only / shared-edit / css-only / compile-error), kolt as the
   real-world exercise. Docs: the tour's dev-loop page + `run --watch` reference;
   `documentation.md`'s gate applies.
