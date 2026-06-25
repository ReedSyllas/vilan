# Transport / RPC library (roadmap P6)

Two Vilan processes communicate and move data across a wire — client↔server and
server↔server. The largest remaining *Next up* item (XL). This proposal settles the
**model and philosophy** before any build.

**The shift in this revision.** An earlier draft made the library a *generator*: a
`[service]` trait that emitted a server dispatcher and a client stub, with
`[derive(Json)]` serializing whole structs. We've since concluded that an RPC library
can only do so much before it begins encroaching on application logic or collapsing
under its own configuration surface. So the library's job is narrower and more
durable: **be a guide, not the structure.** It provides a few sharp primitives and an
established paradigm — it *nudges* the developer toward the correct shape rather than
generating it. The systems help build the right structure; they are not themselves
that structure. The core we already have (a `Transport` seam, a codec) is usable
today; what's left is to settle *how* one is meant to use it.

## 1. Requirements (from the roadmap)

- **Data crosses without hand-written codecs** — a derive handles encode/decode; the
  developer never writes a serializer by hand.
- **Pluggable transports** — HTTP / WebSocket / in-process as built-ins, *custom
  transports first-class* (not privileged over built-ins).
- **An explicit, narrow exposure surface** — what's remotely callable is opt-in and
  small; nothing is reachable by default.
- **The reactive north star** — a remote handle: the server holds a writable `Signal`,
  the client sees a read-only `Source` whose `.sub(..)` subscribes over the transport.

## 2. The four pieces

| Piece | Role | Form |
|---|---|---|
| **Transport** | sends an encoded message over the wire, gets the reply | a `trait` — HTTP / WebSocket / in-process / custom |
| **Codec** | encodes & decodes messages | a `trait` — JSON default; binary later |
| **Dispatcher** *(a maybe)* | helps build an over-the-wire invocation | a helper, not a generator — possibly just a pattern |
| **Service** | the remotely-callable surface | a *paradigm*, emergent from `[rpc]`-marked methods |

The first two are concrete library types and are the stable core. The **Dispatcher is
deliberately a "maybe"** — a convenience for assembling an invocation (client side) and
routing a decoded one to the right method (server side). It may earn a place as a thin
library helper, or it may stay a documented pattern; either way it is glue, never a
code generator. **Service is intentionally *not* a library feature**: there is no
`[service]` keyword or trait. A service is what *emerges* when a developer marks some
methods `[rpc]` (§4) and points a transport at them. The library supplies the parts;
the paradigm supplies the assembly. This is the whole philosophy in one row of the
table: the pieces that *are* infrastructure are types you instantiate; the piece that
*is* your application's shape is a pattern you follow.

It is **peer-symmetric**: "client" and "server" are just *who hosts the methods* vs
*who calls them*. Server↔server is the same mechanism with an HTTP/WS transport between
two Node processes; client↔server is the same with the browser calling over HTTP.

## 3. The data boundary: `[derive(Wire)]`

This is the heart of the new model. Data crosses the wire **only** as a *Wire type* — a
struct or enum that opts in with `[derive(Wire)]`. One rule governs it, and the rule is
the entire safety story:

> **Every field of a `[derive(Wire)]` type must itself be Wire.** A non-Wire field is a
> *compile error*, not a silently-omitted field.

This inverts the usual "remember to strip the sensitive field before sending" chore —
the thing a developer means to do later and forgets, leaking a password hash — into a
property the type system enforces *by construction*. Sensitivity becomes a property of
a **type**, declared once, not a checklist re-applied at every call site:

```vilan
// server-side

[derive(Wire)]
struct Uuid {
	// ...
}

// NOT `[derive(Wire)]` — a password hash must never reach the wire, so the type that
// holds it is simply not Wire. Nothing containing one can be Wire either.
struct Password {
	hash: str,
}

impl Password {
	fun set(self, plaintext_password: str) {
		self.hash = bcrypt::hash(plaintext_password, bcrypt::gen_salt());
	}
}

impl Password with PartialEq<str> {
	fun eq(self, plaintext_password: str): bool {
		bcrypt::compare(self.hash, plaintext_password)
	}
}

// The rich domain type. It holds a `Password`, so it *cannot* derive `Wire` — and the
// compiler says so. There is no way to "accidentally" send a `User`.
struct User {
	id: u32,
	username: str,
	password: Password,
}

impl User {
	// The explicit projection from the domain type to its wire shape. Developer-
	// written, so it can diverge from the source arbitrarily.
	fun to_wired(self): WiredUser {
		WiredUser {
			uuid = self.get_uuid(),     // a *computed* field — `User` has no `uuid`
			username = self.username,   // `id` and `password` simply don't cross
		}
	}
}

[derive(Wire)]
struct WiredUser {
	uuid: Uuid,
	username: str,   // or could be `username: Signal<str>` — see §7
}

impl WiredUser {
	// A manual subscription accessor: a plain `Signal<str>` field is the easy path,
	// but writing the `Source` by hand is sometimes what you want.
	fun get_username(self): Source<str> {
		// ...
	}
}

// A "service" is just a struct whose `[rpc]` methods form the callable surface (§4).
struct Server { /* server state: db handles, etc. */ }

impl Server {
	[rpc]   // exposed over the wire; the signature must be Wire-compatible
	fun get_user_by_username(username: str): Option<WiredUser> {
		// ...
		Some(user.to_wired())
	}
}

// client-side — reads like a local call
let john = Server::get_user_by_username("john");
let john_username_source = john.get_username();
```

What this buys, beyond the leak guarantee:

- **The wire shape diverges freely from the source.** `WiredUser.uuid` is *computed* in
  `to_wired` and is not a field of `User` at all; `User.id` and `User.password` never
  appear. The client's view of an entity is whatever the projection chooses to expose —
  nothing more.
- **References travel as handles.** The same mechanism sends an arena `Handle` (or a
  reactive `Source`, §7) in place of an owned value — a "pointer" across the wire,
  resolved on the far side — because the projection decides what each field *means*.
- **No skip-lists, nothing to forget.** We considered per-field `[skip]` attributes and
  auto-projection; both were rejected. A skip-list is exactly the annotation a
  developer forgets. Here the boundary is a *type you write on purpose*, and the
  compiler refuses to let a non-Wire type slip across. Decode produces the Wire type
  directly (a `WiredUser`), with no vestigial always-empty fields.

The cost is honest verbosity: a domain type and its wire twin, plus a `to_wired`. The
paradigm accepts that — the explicitness *is* the feature — but it is the first place
**syntactic sugar** would earn its keep (a derive that scaffolds a projection for the
encodable fields, which the developer then edits), and that sugar is a deliberately
later, additive step, never the default.

### 3.1 What is Wire

Wire-by-default: scalars, `str`, `bool`, `List<T: Wire>`, `Option<T: Wire>`, and
`[derive(Wire)]` structs/enums (nested). Mechanically this reuses the existing
`Json`/`FromJson` round-trip (`std::json`); `Wire` is the *capability marker* that says
"this is intended for, and permitted on, the wire" — distinct from `Json`, which is
general-purpose serialization with no exposure semantics. The current codec gaps carry
over and are *codec* limits, not RPC limits (they lift as the derives improve):

- ⛔ **`Map<K, V>`** — no JSON impl yet; use a derived struct or `List<Pair>` until Map
  serialization lands (backlog I1).
- ⛔ **`List<List<T>>`** — a collection directly nested in a collection doesn't
  round-trip yet (the dispatch-time monomorphization gap); wrap the inner list in a
  one-field Wire struct for now.

## 4. Exposure: `[rpc]` methods, and Service as a paradigm

`[rpc]` on a method is the entire exposure mechanism:

- It marks the method **callable over the wire**. The set of `[rpc]` methods on a type
  *is* the remotely-reachable surface — opt-in, nothing else reachable. A function
  without `[rpc]` cannot be invoked remotely; that *is* the attack-surface guarantee.
- **The signature must be Wire-compatible** — every parameter type and the return type
  must be Wire (or `Option`/`Result`/`List` of Wire). The compiler checks this and
  rejects, with a clear diagnostic, an `[rpc]` method that takes or returns a non-Wire
  type (e.g. a raw `User`). This is what makes the exposure *typed*: you cannot expose a
  method whose data can't legally cross.
- **Gating** rides as an attribute argument: `[rpc(auth)]` requires an authenticated
  caller, resolved from the transport (a header, a connection-bound session) and
  rejected with `err: unauthorized` *before* the body runs. (Whether finer
  authorization lives in the attribute or the body is Q4.)

**"Service" is then just a name for the pattern**: a struct (`Server` above) whose
`[rpc]` methods form a callable namespace. Peer-symmetric — the server type carries the
real method bodies; the client side presents the same signatures as stubs that
round-trip (§6). There is no `[service]` trait to satisfy, no generated dispatcher
class to subclass; there is a struct, some `[rpc]` marks, and a transport. The library
never *owns* the service abstraction, so it can never get the service abstraction
wrong for your application.

## 5. Transport — the sender

A request/response transport is one method; making it a trait is what lets HTTP,
WebSocket, in-process, and *custom* transports all satisfy the same contract:

```vilan
trait Transport {
	// Send an encoded request, get the encoded reply. `Promise<str>` is explicit by
	// choice: a round-trip is where the caller should `await` deliberately (§6).
	// (Auto-await now works through a trait-bounded call, so an inferred-async
	// `call(self, request: str): str` would also type-check — the explicit `Promise`
	// is the clearer transport contract.)
	fun call(self, request: str): Promise<str>;
}
```

Built-ins:

- **HTTP** (`HttpTransport`) — the default client↔server transport. `call` POSTs the
  request to an endpoint and reads the reply body. Built on the now-shipped `std::fetch`
  POST/body support (§10).
- **In-process** (`LocalTransport`) — `call` runs the server's dispatch in the same
  process. The substrate for **unit tests** (no network) and for composing services
  within one server. (This is the transport the current `examples/rpc` uses.)
- **WebSocket** (`SocketTransport`) — a *bidirectional* transport (Phase 3), the
  substrate for subscriptions/streaming and the reactive north star. It extends the
  base with a server→client message channel.

A custom transport (message queue, IPC pipe, WebRTC, a test double) is just an
`impl Transport` — first-class, no registry.

## 6. Codec — the encoder, and the wire envelope

The codec turns Wire values into bytes and back, abstracted so a compact binary format
can replace JSON later, but **JSON is the default and only codec at first**:

```vilan
trait Codec {
	fun encode<T: Wire>(self, value: T): str;
	fun decode<T: Wire>(self, bytes: str): Result<T, RpcError>;
}
```

`JsonCodec` delegates to the `Wire` round-trip. The codec also encodes the **invocation
envelope**, which is itself a Wire type so it is handled uniformly:

```jsonc
// request envelope                          // reply — success / failure
{ "method": "get_user_by_username",          { "ok": { "uuid": "…", "username": "Ada" } }
  "args": ["john"] }                          { "err": { "kind": "unauthorized", "message": "…" } }
```

- **Procedure name** is a string (debuggable, JSON-native; a numeric id is a later
  compaction). `args` is positional — the dispatch side knows each method's parameter
  order, so it decodes argument *i* at the *i*-th parameter's type.
- The **codec rejects malformed input** (decode → `Result`), so a hostile or stale
  payload is a clean `err`, never a panic or a type-confusion.

## 7. Client invocation, async, and errors

On the client, an `[rpc]` call reads like a local one — `Server::get_user_by_username("john")`
— and underneath: encode the args, build the envelope, `await transport.call`, decode
the reply. Whether that body is **hand-written, assembled from a `Dispatcher` helper, or
lightly generated from the `[rpc]` signature is the central open question** (Q1) — the
philosophy leans toward the least generation that stays ergonomic.

```vilan
// A client-side stub, sketched (hand-written or Dispatcher-assisted)
fun get_user_by_username(self, username: str): Result<Option<WiredUser>, RpcError> {
	let request = self.dispatcher.invocation("get_user_by_username", [self.codec.encode(username)]);
	let reply = await self.transport.call(request);   // round-trip
	self.dispatcher.decode_reply(reply)               // Result<Option<WiredUser>, RpcError>
}
```

- **Async now works through the indirect call.** Effect-polymorphic async is
  implemented: async inference propagates through a trait-bounded dispatch to its
  candidate impls (no `dyn`, so every instance resolves to a statically-known impl), so
  a caller `await`s correctly whether the transport call is direct or indirect. The
  transport keeps its explicit `Promise` as the clearer contract, not out of necessity.
- **Error model: `Result<T, RpcError>` on the client.** The server method returns the
  bare `T` (a local call there); the client side wraps it in `Result` because the
  round-trip itself can fail. `RpcError` is a derived enum:
  `Transport(str) | Decode(str) | Remote(str) | Unauthorized`. This `T`-vs-`Result<T,_>`
  asymmetry between the two sides of the same method is intrinsic to a fallible
  round-trip and is accepted, not papered over (Q3).

## 8. The reactive north star (the capstone)

A Wire type can carry **reactive handles**, which is how a remote subscription falls out
of the same model. `WiredUser` above can expose its username either as a `Signal<str>`
field or via a hand-written `get_username(self): Source<str>` accessor; on the client,
that `Source` is *remote* — its `.sub(..)` flows over a bidirectional transport:

```vilan
let john = Server::get_user_by_username("john");
let source = john.get_username();                       // a remote Source<str>
let _ = source.sub(|name| print(i"username = {name}")); // subscribes over the transport
```

This needs three things beyond the request/response core, hence its own phase:

1. **A `Source`/`Signal` split in `std::reactive`** — a read-only `Source<T>`
   (`get`/`sub`/`map`) that `Signal<T>` implements (adding `set`/`set_with`). The remote
   handle implements `Source`, so client code can't write a server signal. (The
   reactive README already designs the API for this; it also intersects the batching
   revision being drafted separately.)
2. **A bidirectional transport** (WebSocket) — `sub` sends a subscribe message; the
   server streams updates; the subscription's `dispose()` (the existing
   `Disposable`/`Owner` machinery) sends an unsubscribe.
3. **A reactive Wire convention** — how a `Signal<T>`/`Source<T>` field encodes (as a
   subscription channel id, not a value snapshot) so the projection in `to_wired` can
   hand the client a live handle. The manual `get_username` accessor is the escape hatch
   when the convention doesn't fit.

## 9. Where it lives

A `[library]` package, `std::rpc` (or a standalone `rpc` library), providing the stable
core: the `Transport` trait + built-in transports, the `Codec` trait + `JsonCodec`,
`RpcError`, the envelope types, and (if it earns its place) a `Dispatcher` helper. The
`[derive(Wire)]` derive and the `[rpc]` attribute + its signature check are **compiler**
features, not library code (§10). The application's own domain types, their Wire twins,
and the `[rpc]` methods live in the app — typically a shared `common`-style `[library]`
for the Wire types that both sides import, with the server and client packages
depending on both, exactly like the current `common`/`client`/`server` workspace.

## 10. Prerequisites & dependencies

Small, independently-useful std extensions (Phase 0) plus the compiler features the
paradigm needs:

- **`std::fetch` gains POST/body/headers** — ✅ **shipped** (commit 7340518). `post(url,
  body)` / `get(url)` builders + `.header(..)` + `.send()`.
- **`std::http` exposes the request body** — ✅ **shipped** (commit 593742a).
  `request.body(): str`; `Server::start` reads the stream eagerly and passes it in,
  since the indirectly-called handler can't suspend.
- **Effect-polymorphic async** — ✅ **shipped**: auto-await propagates through a
  trait-bounded dispatch (§7), so an indirect transport call awaits correctly.
- **`[derive(Wire)]`** — a new derive: the all-fields-Wire check (the §3 rule, the
  safety boundary) plus the encode/decode glue (reusing the `Json` round-trip
  underneath). New compiler work, but a *derive over a struct/enum* — squarely in the
  shape `expand_derives` already handles, unlike the abandoned `[service]`-over-a-trait
  generation.
- **`[rpc]` attribute + signature check** — mark a method exposed and verify its
  parameters/return are Wire-compatible. A focused analyzer check, not codegen.
- **Codec derives** — Map serialization (backlog I1) and the `List<List<T>>` fix widen
  what crosses; not blockers (work around as in §3.1).
- **`Source`/`Signal` split** + **bidirectional transport** — for the reactive phase
  only.

## 11. Phased plan (XL → shippable slices)

0. **Substrate** (S) — ✅ **SHIPPED** (commits 7340518, 593742a): `fetch` POST/body/headers
   + `http` `Request::body()`, with the full round-trip verified end-to-end.
1. **Runtime, hand-written** (M) — ✅ **largely done** in `examples/rpc`:
   `Transport`/`Codec`/`RpcError`, `JsonCodec`, `LocalTransport` + `HttpTransport`, the
   envelope types, and a **manually-written** dispatcher + stub proving an end-to-end
   client↔server call with the `Result` error model and async. Pins the wire format and
   the runtime first (the project's "prove it before generating it"). *Remaining:* an
   `HttpTransport` example over a real socket (the in-process `LocalTransport` is
   proven).
2. **`[derive(Wire)]` + `[rpc]`** (L) — the data boundary and the exposure check: the
   all-fields-Wire rule and its diagnostics, the `[rpc]` signature check, and the
   `Wire` round-trip. This is the headline "data crosses without hand-written codecs,
   and can't carry what it shouldn't." Convert the `examples/rpc` payloads from
   `[derive(Json)]` to `[derive(Wire)]` with explicit `to_wired` projections — the first
   dogfood of the paradigm. **In the same pass, bring every example up to the latest
   project structure** (platform model + library packages): current `vilan.toml`
   conventions, the shared `common` `[library]`, per-package `platform`.
3. **Dispatcher helper *(maybe)*** (M) — *if* it earns its place: a thin helper that
   builds invocations (client) and routes a decoded envelope to the right `[rpc]` method
   (server), driven by the `[rpc]` marks. Decide here whether it's a library type or
   stays a documented pattern. Migrate the examples to whichever wins.
4. **Bidirectional + server↔server** (L) — `SocketTransport` (WebSocket); in-process
   service composition; a server calling another server; streaming replies.
5. **Reactive north star** (L) — `Source`/`Signal` split; the reactive Wire convention;
   remote `Source` with `sub` over the socket transport. The capstone.

Phases 0–2 are the usable core (typed request/response with the Wire boundary); 3 is
ergonomics; 4–5 are the reactive/streaming reach. Each is independently valuable and
testable.

## 12. Test plan

- **Wire round-trips** — every supported payload shape (scalars, `List`, `Option`,
  nested derived Wire structs/enums) `encode → decode` to an equal value; the §3.1 gaps
  asserted as *known* (so fixing them flips a test green, à la the `#[ignore]` pattern).
- **The Wire rule** — a `[derive(Wire)]` on a struct with a non-Wire field is a clean
  compile *error* (pinned like the analyzer's other diagnostics); a Wire twin of the
  same data compiles. This is the safety property, so it gets a first-class test.
- **The `[rpc]` signature check** — an `[rpc]` method taking/returning a non-Wire type
  fails to compile; a Wire-compatible one passes.
- **`LocalTransport` end-to-end** — an invocation dispatched in-process, no network:
  request → dispatch → reply → decoded result; plus the error paths (unknown method →
  `err`, malformed args → `Decode`, `[rpc(auth)]` without identity → `Unauthorized`).
- **HTTP transport** — a CLI/integration test (like `workspace.rs`) builds a tiny
  client/server workspace and exercises a real `fetch`→`http` round-trip under Node.
- **Exposure** — a non-`[rpc]` method is *not* dispatchable; an off-surface method name
  is rejected.

## 13. Settled decisions vs open questions

**Settled:** the library is a *guide*, not a generator — Transport + Codec are the
stable core, Service is an emergent paradigm; `[derive(Wire)]` is the data boundary with
the all-fields-Wire rule (sensitivity is a type property; no skip-lists); explicit
`to_wired` projections (the wire shape diverges freely from the domain type); `[rpc]`
marks the exposed surface with a Wire-compatibility signature check; JSON codec default
behind a `Codec` trait; pluggable `Transport` with HTTP/in-process/WebSocket built-ins;
`Result<T, RpcError>` on the client; explicit `Promise`+`await` for the transport
(auto-await now works regardless); peer-symmetric.

**Open questions:**

- **Q1 — client invocation form.** Is the client-side stub hand-written, assembled from
  a `Dispatcher` helper, or lightly generated from the `[rpc]` signature? The philosophy
  favors the least generation that stays ergonomic — but `Server::method("john")` reading
  cleanly on the client implies *something* bridges the signature to the transport.
  Resolving this also resolves whether the **Dispatcher** is a library type or a pattern.
- **Q2 — codec abstraction now or later.** Ship the `Codec` trait from Phase 1 (small
  cost, enables a binary codec + a future `Map`/bytes story), or hardcode JSON until a
  second codec appears? *Lean: trait now.*
- **Q3 — the `T` vs `Result<T, _>` asymmetry.** The server method returns `T`; the
  client side returns `Result<T, RpcError>`. Accept the asymmetry (clean server bodies,
  honest client), or unify on `Result` everywhere (uniform, noisier server)?
- **Q4 — exposure granularity & auth.** Is `[rpc(auth)]` the right vocabulary, and how is
  a caller identity supplied (a transport header, a connection-bound session)? Where does
  *authorization* (not just authentication) live — in the attribute, or the body?
- **Q5 — addressing/config.** How does a client learn the server endpoint and a method
  learn its mount path — `vilan.toml` config, a constructor argument, both?
- **Q6 — versioning.** Client and server are built separately; a contract mismatch
  (renamed method, changed Wire shape) should fail *clearly*. A contract hash exchanged
  on connect, or rely on `err: Decode`? (Ties to the platform model's per-package builds.)
- **Q7 — projection sugar.** When and how to add the scaffolding derive for `to_wired`
  (§3) — additive, and only once the explicit form has proven the paradigm.
- **Q8 — `Map` payloads.** Is the no-`Map` codec gap acceptable to launch with (use
  structs / `List<Pair>`), or should Map serialization (backlog I1) be pulled into
  Phase 0?
