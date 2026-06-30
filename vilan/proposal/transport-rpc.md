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
| **Codec** | turns Wire values into bytes and back (the *format*) | a `trait` — JSON default; binary later |
| **Service** | the shared contract — the remotely-callable surface | a `[service]` trait of `[rpc]` methods |
| **Dispatcher + stub** | route a decoded call to a method (server) / send one (client) | **generated** from the service trait — the plumbing |

The first two are concrete library types and the stable core. The other two are where
the **guide-not-generator** line is drawn precisely. The compiler *generates* the
dispatch plumbing — the server router and the client stub — from a `[service]` trait
(§4), so a remote call reads like a local one. But it generates **only the plumbing**:
the *structure* — which types cross the wire (`[derive(Wire)]`, §3) and how a domain
type projects to its wire shape (`to_wired`, §3) — stays the developer's, hand-written.
The library never owns your data model or what a service *means*; it owns the
mechanical encode→route→decode that is identical every time. Generating that boring
half is exactly what makes a remote call *seamless* without the library dictating your
shape — the "C" in RPC, paid for honestly (§7: latency and failure stay visible).

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

// A server method producing the wire shape — one `[rpc]` method of a `[service]` (§4).
// The projection is the only place the boundary is crossed, and it is explicit.
fun get_user(id: i32): Option<WiredUser> {
	// ...look up the domain `User` (password and all), then project...
	Some(user.to_wired())   // `User` itself never crosses; only the wire shape does
}

// client-side — the generated `[service]` stub reads like a local call (§4, §7)
let john = accounts.get_user(1);   // -> Result<Option<WiredUser>, RpcError>
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

### 3.2 Keeping ubiquitous derives out of the way: `[trait_only]`

The Wire boundary is most useful when `[derive(Wire)]` is cheap to put on *everything* —
but a `Wire` derive on every struct (alongside `Debug`, `Json`, …) would bury each type's
real API under generated methods (`encode`, `decode`, `to_json`, …) and invite **name
collisions** with a type's own `id`/`name`/`encode`. Two attributes keep the namespace
clean. Both are *general language features*, not RPC-specific, so they likely warrant
their own small proposal that this one depends on; they are recorded here because they
are what makes ubiquitous `Wire` livable.

- **`[trait_only]`** — a trait method so marked is reachable *only through the trait*,
  never promoted onto a concrete type's method surface. Vilan has no `dyn`, so "through
  the trait" means *through a trait bound* (`fun f(x: ToJson)` is sugar for
  `f<T: ToJson>`): the method resolves on a trait-bounded receiver but not on the bare
  concrete type.

  ```vilan
  trait ToJson {
      [trait_only]
      fun to_json(self): str;
  }
  impl Point with ToJson { fun to_json(self): str { i"{'x':{self.x},'y':{self.y}}" } }

  point.to_json()        // ✗ error: no method `to_json` on struct `Point`
  stringify(point)       // ✓
  fun stringify(value: ToJson): str { value.to_json() }   // ✓ — via the bound
  ```

  This is stronger than Rust's "the trait must be in scope to call its method": it forbids
  the direct call *even with the trait in scope*. That extra restriction is the point — it
  buys **collision-safety**: a type's own `id`/`encode`/`to_json` is never shadowed by, nor
  shadows, a blanket-derived one; clutter alone would only need `[doc(hidden)]` below. The
  cost is that the convenient `point.to_json()` is gone — you go through the trait
  deliberately.

  **Derived trait methods are `[trait_only]` by default.** A `[derive(Wire)]` /
  `[derive(Json)]` / `[derive(Debug)]` generates `[trait_only]` methods, so "derive on
  everything, clutter nothing" is the default rather than a per-method chore; a trait opts a
  method back *out* when the concrete-type call is genuinely wanted. Mechanically it is a
  small hook on resolution paths the analyzer already has — a `[trait_only]` method stays in
  the trait-bound (`OnConstraint`) dispatch and is excluded from concrete-type member
  resolution; no new subsystem.

- **`[doc(hidden)]`** — Rust-style: the method stays fully callable, but the language server
  omits it from completion. A *tooling* concern only, with no resolution change, for methods
  you want reachable-if-typed but not in the `.` menu. Where `[trait_only]` changes *what
  resolves*, `[doc(hidden)]` changes only *what is suggested*.

## 4. Exposure: `[rpc]` methods, the `[service]` trait, and generated stubs

A **`[service]` trait** is the shared contract — the whole remotely-callable surface, and
nothing else is reachable. It lives in `common`, imported by both sides; its methods are
marked `[rpc]`:

```vilan
// common/src/lib.vl — the contract, imported by BOTH sides
[service]
trait Accounts {
    [rpc] fun get_user(id: i32): Option<WiredUser>;
    [rpc(auth)] fun rename(id: i32, name: str): WiredUser;   // gated — needs auth
}
```

- `[rpc]` marks a method **callable over the wire**. The `[rpc]` methods of a `[service]`
  trait *are* the surface — opt-in, nothing else reachable; a method without `[rpc]`, or any
  function outside the trait, cannot be invoked remotely. That *is* the attack-surface
  guarantee.
- **The signature must be Wire-compatible** — every parameter and the return type must be
  Wire (or `Option`/`Result`/`List` of Wire). The compiler checks it and rejects, with a
  clear diagnostic, an `[rpc]` method that takes or returns a non-Wire type (e.g. a raw
  `User`). This is what makes the exposure *typed*: you cannot expose a method whose data
  can't legally cross.
- **Gating** rides as an attribute argument: `[rpc(auth)]` requires an authenticated caller,
  resolved from the transport (a header, a connection-bound session) and rejected with
  `err: unauthorized` *before* the body runs. (Whether finer authorization lives in the
  attribute or the body is Q4.)

From that one trait the compiler generates **two implementations of it** — the plumbing:

- a **server dispatcher** that decodes the envelope, routes on the method name, decodes each
  argument at its parameter type, calls *your* `impl Accounts for ServerState`, and encodes
  the reply; and
- a **client stub**, `Accounts::connect(transport, codec) -> impl Accounts`, whose every
  method encodes its args, builds the envelope, `await`s `transport.call`, and decodes the
  reply.

Because both sides *implement the same trait*, the contract is one type-checked thing with
no drift, and `accounts.get_user(42)` on the client reads exactly like the local call on the
server — the seamless "C" in RPC. This is the hand-written `accounts_dispatch` of
`examples/rpc`, mechanized: the example proves the runtime first, before any generation (the
project's "prove it before generating it"). The generated halves are *only* this glue — the
Wire types and the `to_wired` projections they carry stay yours (§2, §3).

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

## 6. Codec — the format (data ⇆ bytes)

`[derive(Wire)]` settles *what* crosses and its *structure*; the **codec** settles the
*format* — the actual bytes. Keeping the two apart is what lets the same Wire types ride
JSON (readable, for development) or a compact binary format (fast, for production) with no
change to the types:

```vilan
trait Codec {
	fun encode<T: Wire>(self, value: T): List<u8>;
	fun decode<T: Wire>(self, bytes: List<u8>): Result<T, RpcError>;
}
```

- **Bytes, not `str`.** A binary format is not text, so the codec produces `List<u8>` (a
  stand-in until a real byte-array type lands — §10) and the transport moves bytes; JSON is
  just UTF-8 bytes. (The hand-written `examples/rpc` uses `str` because it is JSON-only; this
  generalizes that to bytes.)
- **Wire describes, the codec formats.** For "any serializer" to be real — not JSON with
  extra steps — `[derive(Wire)]` targets a `Serializer`/`Deserializer` visitor: the derived
  code *describes* a value's fields to a serializer, and `JsonSerializer` / `BinarySerializer`
  decide the bytes, so a binary codec carries no intermediate allocation. (A simpler first
  cut is a format-neutral `WireValue` tree each codec converts to/from — one allocation, but
  easy to ship. JSON ships first either way.)
- **The codec is a value, chosen at wiring time** — so the choice is *programmatic*, not a
  build flag baked into the derive. Switch by environment by constructing it at startup:
  `let codec = if Env::is_prod() { BinaryCodec::new() } else { JsonCodec::new() };` then
  `Accounts::connect(transport, codec)`. A `vilan.toml`/env setting is just one way to pick
  that value.
- **Both sides must agree on the format**, or negotiate it (a content-type announced on
  connect). Switching codecs is a deployment-wide decision across the client and server
  packages — the same drift concern as Q6. A self-describing binary format (MessagePack /
  CBOR-like) needs no shared schema; a compact one (protobuf-like) leans on the shared `Wire`
  type for field order.
- The **codec rejects malformed input** (decode → `Result`), so a hostile or stale payload is
  a clean `err`, never a panic or a type-confusion.

The codec also encodes the **invocation envelope** — an invocation is `(method name,
arguments)`, a reply is a result or an error — itself a Wire type, handled uniformly. In
JSON:

```jsonc
// request envelope                  // reply — success / failure
{ "method": "get_user",              { "ok": { "id": 42, "username": "ada", "handle": "@ada" } }
  "args": [42] }                      { "err": { "kind": "unauthorized", "message": "…" } }
```

The method name is a string (debuggable; a numeric id is a later compaction); `args` is
positional — the dispatcher knows each method's parameter order, so it decodes argument *i*
at the *i*-th parameter's type.

## 7. The generated stub: async and errors

The client stub generated from the `[service]` trait (§4) *is* the seamless call —
`accounts.get_user(42)` reads like a method call. Sketched:

```vilan
// generated `impl Accounts for AccountsClient` (one method shown)
fun get_user(self, id: i32): Result<Option<WiredUser>, RpcError> {
	let request = encode_request(self.codec, "get_user", [self.codec.encode(id)]);
	let reply = await (self.transport).call(request);     // round-trip
	decode_reply(self.codec, reply)                       // Result<Option<WiredUser>, RpcError>
}
```

- **Async is seamless and honest.** The stub `await`s the transport, so it is async and a
  caller auto-awaits it — including when the transport is reached through a trait bound,
  since effect-polymorphic async now propagates through an indirect dispatch (no `dyn`, so
  every instance resolves to a statically-known impl; ✅ shipped). Latency stays *visible* as
  an `await`: the stub reads like a method call, not like a free local one — the RPC fallacy
  avoided.
- **The `T` → `Result<T, _>` shift is the contract's, and the generator owns it (Q3,
  settled).** The `[service]` trait declares the *logical* signature — `get_user(id):
  Option<WiredUser>` — and the server `impl` returns exactly that, a clean local body. The
  round-trip can fail, so the **generated client stub wraps the return in
  `Result<_, RpcError>`** — the developer never writes the wrapping. `RpcError` is a derived
  enum: `Transport(str) | Decode(str) | Remote(str) | Unauthorized`. The two sides differ by
  exactly one `Result` layer, applied by codegen, not by hand: the honest client without the
  noisy server.

## 8. The reactive north star (the capstone)

A Wire type can carry **reactive handles**, which is how a remote subscription falls out
of the same model. `WiredUser` above can expose its username either as a `Signal<str>`
field or via a hand-written `get_username(self): Source<str>` accessor; on the client,
that `Source` is *remote* — its `.sub(..)` flows over a bidirectional transport:

```vilan
let accounts = Accounts::connect(transport, codec);
let john = accounts.get_user(1);                        // the wire user (sketch elides Result/Option)
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
`RpcError`, and the envelope types. The `[derive(Wire)]` derive, the `[service]`/`[rpc]`
generation (dispatcher + stub), and the `[trait_only]`/`[doc(hidden)]` attributes are
**compiler** features, not library code (§10). The application's own domain types, their
Wire twins, the `to_wired` projections, and the `[service]` contract live in the app —
typically a shared `common`-style `[library]` for the contract + Wire types both sides
import, with the server and client packages depending on both, exactly like the current
`common`/`client`/`server` workspace.

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
- **`[derive(Wire)]`** — a new derive: the all-fields-Wire check (the §3 rule, the safety
  boundary) plus the encode/decode glue against the `Serializer` visitor (§6). A *derive over
  a struct/enum* — squarely in the shape `expand_derives` already handles.
- **`[rpc]` attribute + signature check** — mark a method exposed and verify its
  parameters/return are Wire-compatible. A focused analyzer check.
- **`[service]` generation** — generate the server dispatcher + client stub from a
  `[service]` trait's `[rpc]` methods (§4). This is generation *over a trait*, beyond today's
  struct/enum derives — the one genuinely new piece of codegen — and resolves Q1. It is the
  headline "seamless remote functions"; the hand-written `examples/rpc` is its proof.
- **`[trait_only]` + `[doc(hidden)]`** — the namespace-hygiene attributes (§3.2): a
  resolution hook excluding `[trait_only]` methods from concrete-type member lookup, and an
  LSP filter for `[doc(hidden)]` in completion. General language features (their own small
  proposal) that make ubiquitous `Wire`/`Debug` derives livable.
- **A byte-array type for binary codecs** — a binary `Codec` produces bytes, not text (§6).
  `List<u8>` is the stand-in for now (probably easiest); a proper fixed `[u8]`/`Bytes` array
  type is the real want (added to the backlog). JSON-only needs nothing here (UTF-8 `str`).
- **Codec derives** — Map serialization (backlog I1) and the `List<List<T>>` fix widen what
  crosses; not blockers (work around as in §3.1).
- **`Source`/`Signal` split** + **bidirectional transport** — for the reactive phase only.

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
2. **`[derive(Wire)]`, `[rpc]`, and `[trait_only]`** (L) — the data boundary and the
   exposure check: the all-fields-Wire rule and its diagnostics, the `[rpc]` signature
   check, the `Wire` round-trip against the `Serializer` visitor, and the
   `[trait_only]`/`[doc(hidden)]` attributes so derived methods stay out of the way (§3.2,
   derived methods `[trait_only]` by default). Convert the `examples/rpc` payloads from
   `[derive(Json)]` to `[derive(Wire)]` with explicit `to_wired` projections — the first
   dogfood. **In the same pass, bring every example up to the latest project structure**
   (platform model + library packages): current `vilan.toml` conventions, the shared
   `common` `[library]`, per-package `platform`.
3. **`[service]` generation — seamless remote functions** (L) — generate the server
   dispatcher and the client stub from a `[service]` trait (§4, §7), with the `Result`
   wrapping applied by codegen and `[rpc(auth)]` gating. This is the headline "C in RPC"
   and resolves Q1. Migrate `examples/rpc` from the hand-written dispatch/stub to the
   generated `[service]`, so the example always shows the current best form.
4. **Bidirectional + server↔server** (L) — `SocketTransport` (WebSocket); in-process
   service composition; a server calling another server; streaming replies.
5. **Reactive north star** (L) — `Source`/`Signal` split; the reactive Wire convention;
   remote `Source` with `sub` over the socket transport. The capstone.

A **binary codec** (and the byte-array type it needs, §10) is an additive slice that can
land any time after Phase 2 — the `Codec`/`Serializer` seam is designed for it; JSON is the
default throughout. Phases 0–2 are the usable core (typed request/response with the Wire
boundary); 3 makes the calls seamless (generated stubs); 4–5 are the reactive/streaming
reach. Each is independently valuable and testable.

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
- **`[service]` generation** — golden-test the dispatcher + stub the `[service]` derive
  emits, then compile-and-run a full client↔server round-trip through the generated pair
  (mirrors the derive tests); confirm the generated client returns `Result<T, RpcError>`
  while the trait/impl is `T`.
- **`[trait_only]` / `[doc(hidden)]`** — a `[trait_only]` method is callable through a
  trait bound but a clean compile *error* on the bare concrete type; a derived trait's
  methods are `[trait_only]` without annotation; a `[doc(hidden)]` method stays callable
  but is absent from the language server's completion list.

## 13. Settled decisions vs open questions

**Settled:** the library is a *guide* for structure and a *generator* for plumbing —
Transport + Codec are the stable core; a `[service]` trait is the contract, and the
compiler generates its dispatcher + client stub (only the glue — the Wire types and
`to_wired` projections stay the developer's). `[derive(Wire)]` is the data boundary with
the all-fields-Wire rule (sensitivity is a type property; no skip-lists); explicit
`to_wired` projections (the wire shape diverges freely from the domain type); `[rpc]`
marks the exposed surface with a Wire-compatibility signature check; `[trait_only]` keeps
derived methods off the concrete type (default for derives) and `[doc(hidden)]` keeps them
out of completion. The codec is the *format* (bytes, not `str`), chosen as a runtime value
so JSON↔binary is a programmatic / env switch; JSON is the default and only codec at first.
Pluggable `Transport` with HTTP/in-process/WebSocket built-ins; `Result<T, RpcError>` on
the client, applied by codegen; effect-polymorphic async (auto-await through the indirect
transport call); peer-symmetric.

**Open questions** (Q1–Q3 settled by the latest round; kept numbered so cross-references hold):

- **Q1 — client invocation form. ✅ Settled:** generate the dispatcher + client stub from a
  `[service]` trait (§4). The seamless call is the point of RPC; the compiler generates only
  the glue, never the structure.
- **Q2 — codec abstraction. ✅ Settled:** ship the `Codec` trait now, with the *format*
  behind it — bytes output and a `Serializer` visitor so a binary codec is zero-overhead
  (§6). JSON is the default and only codec at first.
- **Q3 — the `T` vs `Result<T, _>` asymmetry. ✅ Settled:** the `[service]` trait declares
  `T`, the server `impl` returns `T`, and the generated client stub wraps it in
  `Result<T, RpcError>` — codegen owns the one-layer difference, not the developer (§7).
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
