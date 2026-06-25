# Transport / RPC library (roadmap P6)

Two Vilan processes communicate and move data **without hand-written serializers** —
client↔server and server↔server. The largest remaining *Next up* item (XL); this
proposal settles the model and a phased plan before any build.

## 1. Requirements (from the roadmap)

- **No hand-written serializers** — both **data** and **invocations** encode/decode
  automatically.
- **Pluggable transports** — HTTP / WebSocket / in-process as built-ins, *custom
  transports first-class* (not privileged over built-ins).
- **A permission / exposure system** — the remotely-callable surface is explicit and
  small; nothing is reachable by default.
- **The reactive north star** — a `Signal` as a remote handle (the reactive README's
  vision): the server holds the writable `Signal`, the client sees a read-only
  `Source` whose `.sub(..)` subscribes over the transport.

## 2. Approach: typed procedures, not an IDL

The TypeScript world splits into **schema-first** (gRPC/protobuf, Cap'n Proto — an IDL +
codegen) and **type-first** (tRPC — the language's own types *are* the contract, no
IDL). Vilan already has the type-first ingredients, so we take that path:

- **Shared types are the schema.** A `[library]` package (like `common` in the
  full-stack example) holds the procedure signatures and payload types, imported by
  **both** sides. No separate IDL, no drift between a schema and the code.
- **`[derive(Json)]` is the data codec.** The `Json`/`FromJson` round-trip
  (`std::json`) already serializes scalars, `str`, `bool`, `List`, `Option`, derived
  structs (faithful field objects), and derived enums (externally tagged). The RPC
  layer reuses it rather than inventing a wire format.
- **A derive generates the invocation glue.** What's left — decode an invocation,
  dispatch to the right method, encode the result — is mechanical and is *generated*
  from the shared contract (Phase 2), the way `Json`/`Debug` impls are generated today.

The model is **three layers**, each independently swappable:

```
  service  — declares the callable surface; generates dispatcher (server) + stub (client)
  codec    — data ⇆ bytes        (JSON via the existing derives; binary later)
  transport — bytes ⇆ the wire   (HTTP / WebSocket / in-process / custom)
```

It is **peer-symmetric**: "client" and "server" are just *who hosts a service* vs *who
calls it*. Server↔server is the same mechanism with an HTTP/WS transport between two
Node processes; client↔server is the same with the browser calling over HTTP.

## 3. The wire model

An **invocation** is `(procedure name, arguments)`; a **reply** is a result or an
error. Encoded through the codec (JSON shown):

```jsonc
// request envelope
{ "method": "get_user", "args": [42, { "include_email": true }] }
// reply — success                        // reply — failure
{ "ok": { "id": 42, "name": "Ada" } }     { "err": { "kind": "unauthorized", "message": "…" } }
```

- **Procedure name** is a string (debuggable, JSON-native; a numeric id is a later
  compaction). `args` is positional — the dispatcher knows each method's parameter
  order, so it `from_json`s argument *i* at the *i*-th parameter's type.
- The envelope is itself a derived type (`RpcRequest`, `RpcReply`), so the codec
  handles it uniformly.

## 4. Transport — the pluggable seam

A request/response transport is one method; making it a trait is what lets HTTP,
WebSocket, in-process, and *custom* transports all satisfy the same contract:

```vilan
trait Transport {
    // Send an encoded request, get the encoded reply. `Promise<T>` is explicit by
    // choice: a round-trip is where the caller should `await` deliberately (see §7).
    // (Auto-await now works through a trait-bounded call, so an inferred-async
    // `call(self, request: str): str` would also type-check — the explicit `Promise`
    // is the clearer transport contract.)
    fun call(self, request: str): Promise<str>;
}
```

Built-ins:

- **HTTP** (`HttpTransport`) — the default client↔server transport. `call` POSTs the
  request to an endpoint and reads the reply body. *Needs `std::fetch` to gain
  POST/body* (§10) — it is GET-only today.
- **In-process** (`LocalTransport`) — `call` runs the server dispatcher in the same
  process. The substrate for **unit tests** (no network) and for composing services
  within one server.
- **WebSocket** (`SocketTransport`) — a *bidirectional* transport (Phase 3), the
  substrate for subscriptions/streaming and the reactive north star. It extends the
  base with a server→client message channel.

A custom transport (message queue, IPC pipe, WebRTC, a test double) is just an
`impl Transport` — first-class, no registry.

## 5. Codec — data ⇆ bytes

The codec is abstracted so a compact binary format can replace JSON later, but **JSON
is the default and only codec at first**:

```vilan
trait Codec {
    fun encode<T: Json>(self, value: T): str;
    fun decode<T: FromJson>(self, bytes: str): Result<T, RpcError>;
}
```

`JsonCodec` delegates to `to_json` / `from_json`. **Constraints inherited from the
derives** (so the proposal is honest about what crosses the wire today):

- ✅ scalars, `str`, `bool`, `List<T>`, `Option<T>`, derived structs and enums (nested).
- ⛔ **`Map<K, V>`** — no JSON impl yet; payloads use a derived struct or `List<Pair>`
  until Map serialization lands (backlog I1).
- ⛔ **`List<List<T>>`** — the dispatch-time monomorphization gap (browser-backend
  memory) means a collection directly nested in a collection doesn't round-trip; wrap
  the inner list in a one-field struct for now.

These are *codec* limits, not RPC limits — they lift as the derives improve.

## 6. Service & exposure — the permission boundary

The **service is the entire remotely-callable surface**; nothing else is reachable.
A service is a trait, shared in `common`, marked `[service]`:

```vilan
// common/src/lib.vl — the contract, imported by BOTH sides
[derive(Json)] struct User { id: i32, name: str }

[service]
trait Accounts {
    fun get_user(id: i32): User;
    [rpc(auth)]                         // gated on a caller identity (§6.1)
    fun rename(id: i32, name: str): User;
}
```

`[service]` generates two things from the trait's method signatures (Phase 2):

- **Server dispatcher** — `Accounts::dispatch(impl, request) -> reply`: decode the
  envelope, **match the method name**, `from_json` each argument at its parameter
  type, call the user's `impl Accounts for ServerState`, `to_json` the result into an
  `ok`/`err` reply. A method *not* in the trait has no arm — it is unreachable, which
  *is* the attack-surface guarantee.
- **Client stub** — `Accounts::connect(transport) -> impl Accounts`: each method
  `to_json`s its args, builds the envelope, `await`s `transport.call`, decodes the
  reply (§7). Calling `accounts.get_user(42)` reads like a local call.

### 6.1 Permissions

- **Opt-in, not opt-out.** Only `[service]` traits are exposed; within them, only the
  listed methods. A function elsewhere in `common` (or the server) is never callable.
- **Per-method gates** via attributes: `[rpc(auth)]` requires an authenticated caller;
  the dispatcher resolves a caller identity/session from the transport (a header, a
  connection) and rejects unauthenticated calls with `err: unauthorized` *before*
  invoking the impl. `[rpc(Visibility.Readonly)]` (from the north star) marks
  exposed-but-not-mutating members for the reactive phase.
- The **codec rejects malformed input** (decode → `Result`), so a hostile payload is a
  clean `err`, never a panic or a type-confusion.

## 7. Client stubs, async, and errors

The async constraint from the substrate (§ investigation) shapes the stub shape:

```vilan
// Generated client stub (sketch)
impl Accounts for AccountsClient {
    fun get_user(id: i32): Result<User, RpcError> {
        let request = encode_request("get_user", [self.codec.encode(id)]);
        let reply = await self.transport.call(request);   // explicit await: indirect call
        decode_reply(reply)                               // Result<User, RpcError>
    }
}
```

- The **explicit `await`** on the Promise-returning trait method makes the stub itself
  async (the effect is inferred from the `await` node), so user code calling
  `client.get_user(42)` **auto-awaits** (a direct call). *Effect-polymorphic async*
  (auto-await through an indirect, trait-bounded call) is now **implemented** — async
  inference propagates through a generic dispatch to its candidate impls, since with no
  `dyn` every instance resolves to a statically-known impl — so `transport.call` *could*
  drop the explicit `Promise` and be an inferred-async `call(self, request: str): str`.
  We keep the explicit `Promise` as the clearer transport contract (§4), not out of
  necessity.
- **Error model: `Result<T, RpcError>` on the client.** The server `impl` returns the
  bare `T` (it's a local call there); the generated stub wraps it in `Result` because
  the round-trip can fail. `RpcError` is a derived enum:
  `Transport(str) | Decode(str) | Remote(str) | Unauthorized`. The asymmetry (impl
  returns `T`, stub returns `Result<T, _>`) is the one place the contract trait's
  signature and the generated stub's signature differ — see Q3.

## 8. The reactive north star (the capstone)

The reactive README's vision, made concrete:

```vilan
// server — holds the writable Signal; exposes a read-only view + a mutator
[service]
struct Counter {
    [rpc(Visibility.Readonly)] count: Signal<i32>,   // exposed as a remote Source
}
impl Counter { fun inc(self) { self.count.set_with(|n| n + 1); } }

// client — `count` is a remote Source<i32>, `inc` a remote method
let counter = Counter::connect(transport);
let _ = counter.count.sub(|n| print(i"count = {n}"));   // subscribes over the transport
counter.inc();                                          // round-trips; the sub fires
```

This needs three things beyond the procedure core, hence its own phase:

1. **A `Source`/`Signal` split in `std::reactive`** — extract a read-only `Source<T>`
   interface (`get`/`sub`/`map`) that `Signal<T>` implements (adding `set`/`set_with`).
   The remote handle implements `Source`, so client code can't write a server signal.
   (A reactive-lib refactor; the README already says the API is *designed* for this.)
2. **A bidirectional transport** (WebSocket) — `sub` sends a subscribe message; the
   server streams updates back; the subscription's `dispose()` (the existing
   `Disposable`/`Owner` machinery) sends an unsubscribe.
3. **Stateful-object exposure** — `[service]` on a *struct* (not just a trait), with
   `[rpc]`-attributed fields/methods, so an object — not only a stateless procedure
   set — can be a remote handle.

## 9. Where it lives

A `[library]` package, `std::rpc` (or a standalone `rpc` library). The contract trait +
payload types go in the app's own `common`-style library (imported by both sides);
`std::rpc` provides `Transport`, `Codec`, `RpcError`, the envelope types, the built-in
transports, and the `[service]` derive. Client and server packages each depend on both,
exactly like the current `common`/`client`/`server` workspace.

## 10. Prerequisites & dependencies

Small, independently-useful std extensions (Phase 0) plus larger standing dependencies:

- **`std::fetch` gains POST/body/headers** — ✅ **shipped** (commit 7340518). `post(url,
  body)` / `get(url)` builders + `.header(..)` + `.send()`; the host `fetch(url, options)`
  init object is built with `Object()` + `[extern(set,..)]` setters, headers ride as a
  `List<Header>` (no compiler change). GET `fetch(url)` unchanged.
- **`std::http` exposes the request body** — ✅ **shipped** (commit 593742a).
  `request.body(): str`; `Server::start` reads the stream eagerly (`node:stream/consumers`
  `text`) and passes it in, since the indirectly-called handler can't suspend.
- **Generation mechanism** — `[service]` must generate code from a **trait's** method
  list. Today's derives target structs/enums (`expand_derives`); a service derive needs
  either (a) extending the derive mechanism to traits, or (b) the general macro engine
  (#9). See Q1.
- **Codec derives** — Map serialization (backlog I1) and the `List<List<T>>` fix widen
  what crosses the wire; not blockers (work around as in §5).
- **`Source`/`Signal` split** + **bidirectional transport** — for Phase 4 only.
- **Effect-polymorphic async** — ✅ **shipped**: auto-await now propagates through a
  trait-bounded dispatch (§7), so an inferred-async trait method works indirectly. The
  transport keeps its explicit `Promise` by choice, not necessity.

## 11. Phased plan (XL → shippable slices)

0. **Substrate** (S) — ✅ **SHIPPED** (commits 7340518, 593742a): `fetch` POST/body/headers
   + `http` `Request::body()`. Each landed on its own, with the full round-trip verified
   (a Vilan `fetch::post` → a Vilan `http` server reading `request.body()`).
1. **Runtime, hand-written** (M) — `Transport`/`Codec`/`RpcError`, `JsonCodec`,
   `LocalTransport` + `HttpTransport`, the envelope types, and a **manually-written**
   service (dispatcher + stub) proving an end-to-end client↔server call with the
   `Result` error model and async. No codegen yet — pins the wire format and the
   runtime first (the project's "prove it before generating it").
   - **Then upgrade the example projects to use it.** Once the hand-written runtime
     works, migrate the full-stack examples (`examples/fullstack`, `examples/todo`)
     from their manual `fetch` + `request.path()` route wiring to an RPC client/stub
     over the shared `common` contract — the first real dogfooding of the system, and
     the proof it composes. **In the same pass, bring every example up to the latest
     project structure** (the platform model + library packages that have shipped
     since they were written): current `vilan.toml` conventions, the `common` shared
     `[library]`, and per-package `platform`. They're then the worked references the
     docs point at, and the regression that keeps the RPC core honest end-to-end.
2. **`[service]` generation** (L) — generate the dispatcher + stub from a `[service]`
   trait; the exposure boundary; `[rpc(auth)]`. This is the headline "no hand-written
   serializers/dispatch." Migrate the Phase 1 examples again — manual service → the
   generated `[service]` — so they always demonstrate the current best form.
3. **Bidirectional + server↔server** (L) — `SocketTransport` (WebSocket); in-process
   service composition; a server calling another server. Streaming replies.
4. **Reactive north star** (L) — `Source`/`Signal` split; stateful-object `[service]`;
   remote `Source` with `sub` over the socket transport. The capstone.

Phases 0–2 are the usable core (typed request/response RPC); 3–4 are the reactive/
streaming reach. Each phase is independently valuable and testable.

## 12. Test plan

- **Codec round-trips** — every supported payload shape (scalars, `List`, `Option`,
  nested derived structs/enums) `encode → decode` to an equal value; the §5 gaps
  asserted as *known* (so fixing them flips a test green, à la the `#[ignore]` pattern).
- **`LocalTransport` end-to-end** — a service call dispatched in-process, no network:
  request → dispatch → reply → decoded result; and the error paths (unknown method →
  `err`, malformed args → `Decode`, `[rpc(auth)]` without identity → `Unauthorized`).
  This makes the whole RPC runtime unit-testable without a socket.
- **HTTP transport** — a CLI/integration test (like `workspace.rs`) builds a tiny
  client/server workspace and exercises a real `fetch`→`http` round-trip under Node.
- **Exposure** — assert a non-`[service]` function is *not* dispatchable (no arm), and
  that `dispatch` rejects an off-contract method name.
- **Generation** (Phase 2) — golden-test the source a `[service]` derive emits, and
  compile-and-run it (mirrors the derive tests).

## 13. Settled decisions vs open questions

**Settled (recommended):** type-first (shared trait is the contract, no IDL); three
swappable layers; JSON codec default behind a `Codec` trait; pluggable `Transport`
trait with HTTP/in-process/WebSocket built-ins; opt-in exposure (`[service]` is the
whole surface); `Result<T, RpcError>` on the client; explicit `Promise`+`await` for the
transport (works today); peer-symmetric (server↔server = same mechanism); phased 0–4.

**Open questions:**

- **Q1 — generation mechanism.** `[service]` on a trait via (a) extending the derive
  mechanism to traits, (b) the macro engine (#9), or (c) a dedicated `service` keyword
  with bespoke lowering? *Lean (a)* — closest to the shipped derives — but it's new
  (derives are struct/enum-only today). Does the trait-derive extension belong to this
  proposal or to #9?
- **Q2 — codec abstraction now or later.** Ship the `Codec` trait from Phase 1 (small
  cost, enables a binary codec + a future `Map`/bytes story), or hardcode JSON and
  abstract only when a second codec appears? *Lean: trait now.*
- **Q3 — the `T` vs `Result<T, _>` asymmetry.** The contract trait's method returns
  `T`; the client stub returns `Result<T, RpcError>`. Accept the asymmetry (clean impls,
  honest client), or make the contract itself return `Result` (uniform, noisier impls)?
- **Q4 — exposure granularity & auth.** Is per-method `[rpc(auth)]` (+ `Readonly`) the
  right vocabulary, and how is a caller identity supplied to the dispatcher (a transport
  header, a connection-bound session)? Where does authorization (not just authentication)
  live — in the impl, or declaratively?
- **Q5 — addressing/config.** How does a client learn the server endpoint and a service
  learn its mount path — `vilan.toml` config, a constructor argument, both?
- **Q6 — versioning.** Client and server are built separately; a contract mismatch
  (renamed method, changed payload) should fail *clearly*. A contract hash exchanged on
  connect, or rely on `err: Decode`? (Ties to the platform model's per-package builds.)
- **Q7 — north star scope.** Confirm Phases 0–2 ship as the usable core first, with the
  reactive remote-`Source` (Phase 4) explicitly later (it pulls in the reactive split +
  WebSocket).
- **Q8 — `Map` payloads.** Is the no-`Map` codec gap acceptable to launch with (use
  structs / `List<Pair>`), or should Map serialization (backlog I1) be pulled into
  Phase 0?
