# Kolt → vilan — the migration driver

Status: **LIVING DOCUMENT** (2026-07-11). Kolt (`~/code/kolt`) is the user's
real-time web project planner and the north star that motivated much of
vilan; this document is the gap analysis and sequencing that makes it the
explicit driver of backlog priority. Update it as slices land and as the
pilot discovers gaps reading could not.

## 0. What Kolt is

A real-time project planner / management tool, ~5.3k LOC:

- **Client**: Solid 1.9 + Vite, Tailwind v4 + CVA, `@solidjs/router` (nested
  layouts, `/w/ORG/WS/*` scheme planned), luxon (dates), jose (JWT
  handling), fuse.js (fuzzy search), lucide (icons), motionone (animation).
  27 components.
- **Server**: Deno. In-memory SQLite (`@db/sqlite`, schema-in-code — the
  persistence layer is deliberately young). JWT auth (jose HS512, JWK
  persisted to disk) + bcrypt. `AsyncLocalStorage<Client>` for ambient
  per-connection state. A class-based RPC surface annotated with
  `/** @rpc reducer */` doc comments, driving a codegen step that emits
  `api.metadata.ts` / `socket.metadata.ts` — binary type descriptors — over a
  hand-rolled combinator serializer (`$u8 … $list, $tuple, $Result`). A
  WebSocket client ("Railway") with reconnect/backoff and request
  correlation. Server-side signals (`signal.ts`) with per-client observation
  (`remote-interface.ts`, `state.ts`).
- **Architecture principle** (the user's, deliberate): the server is the
  ONLY point of database contact. The client never touches storage; the
  server owns authorization; the database technology is swappable
  (in-process or external) behind the server's interface.

## 1. The solved column

Kolt hand-built, in TypeScript, most of what vilan now generates — the
migration deletes these subsystems rather than porting them:

| Kolt subsystem | vilan counterpart (shipped) |
|---|---|
| `shared/serializer.ts` (runtime combinator codec) + `*.metadata.ts` (codegen'd binary type descriptors) — BOTH of the "type data at runtime / build a compiler plugin" paths | `[derive(Wire)]` + codecs; types are real, so neither path exists |
| The metadata tables as version guards | `contract_hash` — a stale bundle is a clean error |
| `Railway` (WS stub, request correlation, typed methods) | `[service(Client)]`-generated `Client<Transport>` + `SocketTransport` |
| `/** @rpc reducer */` doc-comment annotations | `[service]` / `[expose]` attributes — checked, not commentary |
| `AsyncLocalStorage<Client>` ambient connection state | `std::context` — the same pattern, compile-time-verified, capture-at-creation across `await` |
| `signal.ts` + per-client observation + `state.ts`/`remote-interface.ts` | `std::reactive` + turns (per-request flush isolation was A6's motivating scenario) + `ReactiveServer`/`RemoteSource` mirrors |
| Solid client | `std::ui` — fine-grained reactive by ancestry |
| Tailwind + CVA | `std::style` (A8) — variants are `match` over consts |
| `shared/result.ts` (`$Result`) | `std::result`, Wire-codable |
| Deno runtime | platform `deno:2` |
| Server-only-DB as team discipline | platform layers: client code importing a `@process` module is a COMPILE ERROR — the principle becomes structural |

## 2. The gaps (each is a backlog item; critical-path order)

1. **K3 — `std::crypto` / auth primitives.** JWT HS512 sign/verify and
   password hashing. WebCrypto (`crypto.subtle`) is on node AND deno, so
   this is an extern-binding module: HMAC import/sign/verify (JWT = header
   + payload base64url + HMAC), PBKDF2 via `deriveBits` for passwords (v1;
   bcrypt/argon2 need npm deps — recorded beyond-v1), `randomUUID`/random
   bytes. Async externs throughout (subtle is promise-based — J-machinery
   handles it). Passkeys/WebAuthn: recorded beyond-v1 on the same module.
2. **K4 — SQLite bindings.** A platform-layered extern module: deno
   `jsr:@db/sqlite`, node `better-sqlite3` — one vilan interface, per-layer
   impls (the platform model's `_sys` seam). Doubles as the proving ground
   for runtime jsr/npm dependencies flowing through extern module imports.
   Server-layer only, per the architecture principle — which vilan enforces.
   Kolt's persistence is young (`:memory:`, schema-in-code), so a minimal
   exec/query/prepare surface suffices; no ORM ambitions.
3. ~~**A10 — `std::ui` router.**~~ — **SHIPPED 2026-07-11**
   (`proposal/router.md`): the enum-route model — routes are nested ENUMS
   mirroring nested layouts (`layout_main`/`layout_workspace` become plain
   functions matching them; `/w/{org}/{ws}/..` is a variant payload), with a
   hand-written `parse`/`href` pair over `segments`. `std::router`
   (`current_path`/`navigate`/`link` over `Routable`), `View.swap` (the
   general dynamic-subtree boundary), `View.on_event` + `std::dom::Event`.
   Runtime semantics pinned headless (`crates/vilan-cli/tests/router.rs`, a
   DOM/history stub under node) + compile pins. Findings: B19 (a method
   generic grounded only by a closure's return froze abstract — FIXED
   2026-07-11, same day; `map(..)` chains into `swap` need no annotation),
   B20 (a named fn didn't coerce to a closure parameter — SHIPPED
   2026-07-11, same day, `proposal/fn-coercion.md`: `map(parse)` is now the
   idiom, no eta-expansion).
4. **A11 — web storage externs.** `localStorage` get/set/remove on the dom
   layer (the client-side JWT home), `sessionStorage` alongside.
5. ~~**K5 — `std::time`.**~~ — **SHIPPED 2026-07-11.** `Instant` (epoch
   millis) + signed `Duration`, both `[derive(Wire, PartialEq)]` — a
   timestamp is plain data and rides rpc payloads; `now()`, operators
   (`instant ± duration`, `duration ± duration`; elapsed is the named
   `later.since(earlier)` — the operator traits return `Self`), unit
   constructors/truncating accessors, `describe()` ("1d 4h"), `to_iso()`
   via host `Date`, `sleep`/`sleep_for` (the K6 backoff shape). Base layer
   (every host has the clock), so it compiles for node AND browser; `const
   now()` is rejected ("unknown host call `Date.now`") — the impure
   capability stays runtime. **Unblocked by making `i64` a Wire scalar**
   (found here: the derive rejected `millis: i64`): its own
   serializer/deserializer channel, JSON as a number, binary as the f64 bit
   pattern (the runtime's i64 IS an integral f64 — exact to 2^53, which
   epoch millis and row ids fit). Note: Kolt today has NO live date call
   sites (luxon is declared, unused) — the surface follows the immediate
   needs (task timestamps, K6 backoff); grow from real call sites stands.
   Corpus `time.vl` (node-run; interpreter-excluded — host clock) + 5 pins.
6. ~~**K6 — transport robustness.**~~ — **SHIPPED 2026-07-11**
   (`proposal/transport-robustness.md`). Railway parity, typed:
   `ConnectionState` as a SIGNAL (`Connected`/`Reconnecting`/`Closed` —
   Kolt's client binds a "reconnecting…" banner to it), reconnect with
   doubling backoff (250ms→4s cap, 10 attempts, budget resets on success;
   initial connect dials the same way instead of hanging on a dead server),
   `Transport.call` returns `Result` (in-flight calls REJECT with
   `RpcError::Transport("connection lost")` — the Railway dangle, fixed;
   calls while down fail fast with "not connected"; requests are never
   blind-retried — `create_workspace` is not idempotent, the app owns
   retry), and mirror re-subscription: the duplex survives (Shared cells
   swap inside), the generated `connect` registers a reconnect hook that
   re-verifies the contract (drift closes for good), re-`__attach`es under
   the fresh connection id, and rebinds every `RemoteSource` positionally —
   the server's current-value Update resyncs each watched mirror. Verified
   end-to-end (`crates/vilan-cli/tests/transport_robustness.rs`): SIGSTOP
   catches a call in flight, SIGKILL drops the socket, a restarted server
   with DIFFERENT state resyncs the mirror and calls resume. Finding: B21
   (a dependency-`[service]` consumer without a direct `std::rpc` import
   mistypes the generated connect — pinned; the probe carries the one-line
   workaround).

**Non-blocking, recorded here rather than as items**: canvas 2D externs
(whiteboards — a later dom-layer extension), SVG elements in `std::ui`
(lucide replacement; may just work via `view(tag)` — verify in the pilot),
fuzzy search (fuse.js is a pure algorithm — port or bind when needed),
animation (std::style transitions cover the basics; motionone parity is far
future), automations/webhooks (plain server code once K4 lands).

## 3. The pilot slice — COMPLETE (2026-07-11)

Built as `~/code/kolt/vilan/` (a 4-package workspace: `common`, `server`,
`client`, `probe`), verified end-to-end against a running server:
**register → authenticated workspace create → forged-token rejected → the
change lands live on a SECOND connection's mirror → persisted in SQLite and
reloaded on restart.** Every §2 gap it touched is now shipped (K3 crypto/jwt,
K4 db, A11 storage). The server keeps `common` platform-neutral by holding its
DB logic in `Shared<|..| R>` HOOK closures the rpc methods call — so
`std::db` never leaks past the `@process` layer. Findings fed back: B17
(fixed before the pilot), B18 (method-call-result call parse gap — worked
around), E10 (`pkg::`/`std::` module name collision — worked around by
renaming), and confirmation that RPC stubs must be called from an async
context (a sync helper matches an unresolved promise — a sharp edge worth an
eventual diagnostic). What the pilot did NOT need yet: JWT itself (session
tokens are a `session` table row — revocable, simpler than JWT for a
single server; JWT waits for stateless multi-node), routing (one screen +
a `show` toggle), K5 time, K6 robustness.

## 3b. The original pilot sketch (kept for the record)

A vertical slice, built as a fresh vilan workspace beside the TS app —
`kolt/vilan/` — sharing nothing at runtime (no protocol compatibility
goal; the TS app keeps running while slices grow):

> **Register → login (hashed password, JWT) → authenticated socket
> connect → ONE live entity (the workspace list) synced across tabs via a
> `RemoteSource` mirror → styled with `std::style` → persisted in SQLite
> across server restarts.**

Exit criterion: two tabs, one login, live sync between them, and the list
survives a server restart. Build order inside the slice — each gap built
exactly when it blocks: K3 crypto externs → K4 sqlite binding → the
`[service]` + auth handshake (token check at connect, `std::context`
carrying the authenticated account through handlers) → client UI (login
form + list; A11 storage for the token) → persistence wiring.

What the pilot deliberately defers: routing (one screen + a conditional is
enough — A10 lands when the second PAGE does), K5 time, K6 robustness
(manual refresh on drop is acceptable for the slice), and every
non-blocking row above.

## 4. After the pilot

Sequencing by dependency, not by component count: A10 router + K5 time
(the app shell), K6 transport robustness (before real use), then component
migration in Kolt's own feature order (workspaces → tasks → filters/search
→ canvas/whiteboards last, behind the canvas externs). The todo.md
ambitions (orgs, automations, passkeys) map onto the same items — nothing
in it demands machinery beyond §2 plus recorded beyond-v1 notes.

**The second screen — SHIPPED 2026-07-11** (kolt 2a717fb): the client is
routed on the A10 model — `Route::{Home, Workspace(i32), NotFound}` +
`parse`/`href` over `segments`, pages swapping on
`current_path().map(parse)` (the B20 coercion in anger), rows as typed
`link`s, a live workspace detail page reading the mirror (a deep link
populates when the first sync lands), sign-out navigating home. The server
needed NOTHING — the pilot's catch-all already was the history-API
fallback. New permanent asset: `vilan/e2e/run.sh`, a headless CLIENT e2e
(real bundle + real server under node; DOM/history/storage stubs + node's
native WebSocket wrapped to resolve the relative URL) covering fresh-visit
register→create→navigate→popstate and deep-link-reload→sign-out. The rpc
probe passes untouched. Finding: none — the whole slice compiled first
try; A10/B19/B20 landed exactly the shapes this screen needed.

## 5. What the migration tests about vilan itself

Honest expectations: the pilot is the first REAL app pressure on the
turns/context server model, the generated RPC surface, and yesterday's
styling system at once. Expect it to surface ergonomic gaps (error
messages, std holes, fmt behavior on real code) faster than the corpus
ever will — that is the point of driving development with it. Findings
feed this document and the backlog like any other arc.
