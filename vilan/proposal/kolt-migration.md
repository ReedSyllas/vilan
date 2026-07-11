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
   DOM/history stub under node) + compile pins. Findings: B19 (a bound
   checked against an unresolved chained generic — annotate the intermediate
   binding), B20 (a named fn doesn't coerce to a closure parameter —
   eta-expand).
4. **A11 — web storage externs.** `localStorage` get/set/remove on the dom
   layer (the client-side JWT home), `sessionStorage` alongside.
5. **K5 — `std::time`.** Minimal v1: epoch millis `now()` (impure host
   capability — correctly NOT const-evaluable), duration arithmetic, and
   basic formatting. Explicitly not a luxon: Kolt's usage decides the
   surface; grow from real call sites.
6. **K6 — transport robustness.** Railway parity for `SocketTransport`:
   reconnect with backoff, request retry policy, and mirror
   RE-SUBSCRIPTION on reconnect (`RemoteSource` must resync after a drop).
   Rides the p6-followups territory.

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

## 5. What the migration tests about vilan itself

Honest expectations: the pilot is the first REAL app pressure on the
turns/context server model, the generated RPC surface, and yesterday's
styling system at once. Expect it to surface ergonomic gaps (error
messages, std holes, fmt behavior on real code) faster than the corpus
ever will — that is the point of driving development with it. Findings
feed this document and the backlog like any other arc.
