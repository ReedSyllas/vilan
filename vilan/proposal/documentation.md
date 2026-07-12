# Documentation (D1a) — the user-facing reference

## 1. Problem

The language is getting hard to use because of the frameworks we've built on
top of it: reactive signals with ownership and turns, the UI layer, services
and RPC with mirrors, typed styles, the router, drafts. None of this exists
anywhere except the std sources and the proposal documents — so anyone (the
author included) who forgets something simple has to re-read and re-understand
source code. Proposals are design documents: they record *why* and *how it was
built*, not *how to use it*, and they are frozen at shipping time while the
code moves on.

D1 splits in two:

- **D1a (this proposal): the user-facing reference** — how to *use* the
  language, the std library, and the frameworks. The active pain; built first.
- **D1b (later): the formal language specification** — normative grammar and
  semantics, backlog §D.1. The tour chapters written here become its informal
  companion; nothing in D1a blocks on it.

## 2. Goals and non-goals

Goals:

- A reader who knows programming (JS/TS-level background assumed, no Rust
  required) can build a full-stack vilan app from the docs alone.
- Look-up speed: "what were `bind_each`'s parameters again?" is answered by a
  signatures-first reference page, not by reading `ui.vl`.
- Docs that cannot rot: every example is compiled by the test suite.

Non-goals (recorded, not built now):

- A doc generator extracting reference pages from source comments — worth
  doing eventually (E-series backlog), but v1 is hand-written; generation can
  replace the mechanical parts later without changing the structure.
- A rendered website. Markdown in-repo is the v1 venue; a static-site build
  can be added on top without rewriting anything.
- Versioning/`since` annotations — there are no releases yet; git is the
  history.

## 3. Venue and layout

A markdown book at `vilan/docs/`:

```
docs/
  README.md          — index, how to read, conventions
  tour/              — Part I: the language (informal; D1b's future companion)
  guide/             — Part II: framework guides (task-oriented, narrative)
  std/               — Part III: API reference (signatures-first, per module)
  appendix/          — Part IV: gotchas, glossary
```

Two registers, deliberately separate:

- **Guides** answer "how do I build X" — narrative, one topic end to end,
  examples that grow.
- **Reference** answers "what does this API do" — one page per module (small
  related modules share a page), each item as: signature, one-paragraph
  semantics, a minimal example, traps. A page opens with an at-a-glance table.

Guides link into reference pages rather than duplicating signatures; reference
pages link back to their guide for the concepts.

## 4. Compile-tested examples

Every fenced code block in `docs/` is part of the test suite. A harness test
(`crates/vilan-core/tests/docs.rs`) walks `vilan/docs/**/*.md`, extracts
fenced blocks, and compiles each with the existing `inference.rs` machinery:

- ` ```vilan ` — a complete program; must compile for the node target.
- ` ```vilan,browser ` — complete program, browser target.
- ` ```vilan,norun ` — compiles but is not expected to be runnable
  (e.g. requires a live server).
- ` ```vilan,fragment ` — NOT compiled (shows a diff, a signature, or an
  intentionally-wrong example). Kept rare and always labelled in prose.

Convention: prefer complete small programs — they double as copy-paste
starting points. The harness reports the doc file and heading on failure so a
broken example is a one-jump fix. This is the same discipline as the corpus:
a std or language change that breaks a doc example fails CI until the doc is
updated — the mechanism that keeps hand-written docs honest.

(Runtime *output* assertions are deliberately out of scope for v1 — compile
coverage is the rot-prevention; behavior is pinned in `inference.rs` where it
belongs.)

## 5. Maintenance policy

Definition of done for any std/framework/language change grows one item:
**update the affected docs page(s) in the same commit**. CLAUDE.md gets a line
under Testing. Backlog entries that ship features name the doc sections they
touched (the way they already name pins).

## 6. Table of contents

### Part I — The language tour (`docs/tour/`)

1. **Hello vilan** — install/build, `vilan build`/targets, `vilan.toml`
   (`[package]`, `[build]`, `target`, dependencies), packages & workspace
   layout, `import` (`std::`/`pkg::`/`<dep>::`).
2. **Values and types** — primitives, sized numerics + literal suffixes
   (`1000i64`), strings + interpolation (`i"…{expr}…"`), tuples, `List`/
   `Map`/`Set` literals, `let`/`mut`/type annotations.
3. **Functions and closures** — `fun`, closure types (`|T| U`), named-fn
   coercion (`map(parse)`), async closure types and their seams (parameter /
   `let` only), context clauses (`(|| T) context owner_scope`).
4. **Data and traits** — `struct`/`enum`/`impl`, generics and bounds,
   `impl X<type T: Bound>`, traits + operator traits (`Add`, `PartialEq`, …),
   derives (`[derive(Wire, PartialEq, Debug)]`).
5. **Control flow** — `match`/`is`/binding patterns, `Option`/`Result`
   idioms, `!` and `?.`, `jump`, `ret`, loops.
6. **The memory model** — value semantics, views (`&`/`&mut`) as
   second-class borrows, `own`, `Shared<T>`, `Arena`/`Handle`, the
   view-across-await rule; when to reach for which.
7. **Async** — implicit await on direct calls, `async expr` = spawn,
   `Promise<T>`, `sleep`/`sleep_for`, the J2 rules in practice (why a stored
   callback is re-marked at a `let`).
8. **Macros and const** — attribute macros (`[derive]`, `[service]`,
   `[macro]` knob), macro blocks, `const` evaluation (styles as the worked
   example).
9. **Platforms** — base/browser/process std layers, what's available where,
   full-stack packages (client + server + common), assets.

### Part II — Framework guides (`docs/guide/`)

10. **Reactive state** — `Signal` (get/set/sub), derived state
    (`map`/`combine`/`flatten`), effects, ownership & disposal (ambient
    owner, boundaries), turns & `FlushPolicy` (what settles when), `batch`,
    `optimistic`, `Draft` local-first cells.
11. **Building UI** — `view`/`View` chaining, static vs bound setters,
    events as turn boundaries (`on`/`on_event`), two-way binds (`bind_value`,
    `bind_draft`), lists (`bind_each` + keys), conditionals (`show` vs `when`
    vs `swap`), `mount_root`, component functions.
12. **Styling** — `const` typed styles, `Length`/`Color`/`space`, `styled`,
    dynamic values via `style_var`, how rules are emitted.
13. **Routing** — routes as nested enums, `parse`/`href` as an inverse pair,
    `link`, `current_path` + `swap`, `navigate`, deep links & the history
    fallback.
14. **Services and RPC** — `[service]`/`[rpc]`/`[expose]`, what `Wire` means
    (+ `derive(Wire)` rules), codecs (json/binary), generated clients &
    `connect`, mirrors (`RemoteSource`) and live sync, `ConnectionState` +
    reconnection behavior, pending-call semantics (reject, fail-fast, never
    blind-retry), the server side (`serve_service`), auth via `context`.
15. **Persistence and the server** — `std::db` (sqlite: prepare/run/first,
    column accessors incl. `big_integer`), the http server, `fs`,
    `process` (args/env/exit).
16. **A full-stack walkthrough** — the kolt shape end to end: workspace
    layout (client/server/common), auth handshake, one mirrored entity,
    routed pages, a draft-based editor, headless e2e testing patterns (the
    DOM-stub harness).

### Part III — std reference (`docs/std/`)

One page per group; every public item gets an entry.

- `collections.md` — list, map, set, range, iterator
- `option-result.md` — option, result (+ `Try`/`Lift` operators)
- `strings.md` — string, display, debug, into
- `numbers.md` — number (sized ints, floats, BigInt), math, random
- `traits.md` — compare, default, operators
- `cells.md` — shared, arena
- `time.md` — Instant, Duration, sleep, timers
- `encoding.md` — json, wire, binary, bytes, base64
- `reactive.md` — the full API surface (guide ch. 10's lookup counterpart)
- `style.md` — the style API surface
- `rpc.md` — transports, dispatcher, frames (guide ch. 14's counterpart)
- `net.md` — fetch, ws
- `browser.md` — dom, ui (API tables), router, storage
- `process.md` — db, http, fs, process, rpc_server
- `misc.md` — io (print/assert/panic), promise, context, crypto, jwt, asset

### Part IV — Appendices (`docs/appendix/`)

- `gotchas.md` — the recorded idiom traps as a checklist (match as operand,
  bind-then-call for stored closures, annotate effect params, i64 literal
  suffixes, `desc` is an SQL keyword, …) — seeded from the backlog/memories,
  grown as findings land.
- `glossary.md` — turn, owner, boundary, mirror, view, world, …

## 7. Delivery plan

Phased; each phase lands as a normal reviewed commit and is immediately
useful:

- **Phase 1 — skeleton + the pain points.** `docs/` scaffolding + the
  extractor test; guides 10–14 (reactive, UI, styling, routing, services/RPC)
  and their reference counterparts (`reactive.md`, `browser.md`, `rpc.md`,
  `style.md`); tour chapters 3, 6, 7 (closures/async/memory — the three the
  frameworks lean on hardest).
- **Phase 2 — the std sweep + the rest of the tour.** Remaining Part III
  pages; tour 1–2, 4–5, 8–9; guide 15.
- **Phase 3 — the walkthrough + appendices.** Guide 16, gotchas, glossary.

Phase 1 is the bulk of the value; 2 and 3 are steady-state fill.

## 8. Open decisions

1. **Venue**: in-repo markdown book at `vilan/docs/` as above — or a single
   big reference file, or jump straight to a rendered site?
2. **Example gating**: compile-test all doc examples (the §4 harness) — or
   keep docs untested prose?
3. **Phase-1 priority**: frameworks-first as in §7 — or the std sweep first,
   or a thinner everything-at-once pass?
4. **Tour now vs later**: write the informal tour chapters as scheduled — or
   defer all language-description writing to D1b (the spec) and keep D1a
   framework/std-only?
