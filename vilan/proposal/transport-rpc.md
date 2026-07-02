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

## 2. The pieces

| Piece | Role | Form |
|---|---|---|
| **Codec** | value ⇆ bytes — the *format* | a `trait` — JSON default; binary later |
| **Transport** | moves frames over the wire — a dumb pipe | a `trait` — request/response (HTTP) or **duplex** (WebSocket) |
| **Protocol** | the *semantics* over a transport + codec | **RPC** (request/response) and **Reactive** (pub/sub) — siblings |
| **Service** | the *server* surface; the client requestor is a generated projection of it (two signatures — §4) | a hand-writable foundation (`call` + `Dispatcher`), optionally sugared by a `[service(Client)]` struct (`[rpc]` methods + `[expose]` signals) |

The stack composes bottom-up: a **codec** turns values into bytes, a **transport** moves
those bytes as frames, and a **protocol** layers the *meaning* on top — request/response
for RPC, publish/subscribe for reactive. Keeping *protocol* distinct from *transport* is
what lets a plain HTTP request/response transport carry RPC with no reactive machinery
shoehorned in, and a reactive `Source` ride a duplex transport, without either concern
leaking into the other (§5, §8). Transport and codec are a protocol's two dependencies —
composed *under* it, as siblings.

Within the RPC protocol the **guide-not-generator** line is drawn precisely: the dispatch
plumbing — the server router and the client requestor — is a hand-writable foundation
(`call` + `Dispatcher`, §4.1), which the compiler can *generate* from a `[service(Client)]` struct
(§4.2) as sugar, so a remote call reads like a local one. But it generates **only
the plumbing**: the *structure* — which types cross the wire (`[derive(Wire)]`, §3) and how
a domain type projects to its wire shape (`to_wire`, §3) — stays the developer's. The
library owns the mechanical encode→route→decode that is identical every time; that is what
makes a remote call *seamless* without dictating your shape — the "C" in RPC, paid for
honestly (§7: latency and failure stay visible).

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
	fun to_wire(self): WireUser {
		WireUser {
			uuid = self.get_uuid(),     // a *computed* field — `User` has no `uuid`
			username = self.username,   // `id` and `password` simply don't cross
		}
	}
}

[derive(Wire)]
struct WireUser {
	uuid: Uuid,
	username: str,   // or could be `username: Signal<str>` — see §7
}

impl WireUser {
	// A manual subscription accessor: a plain `Signal<str>` field is the easy path,
	// but writing the `Source` by hand is sometimes what you want.
	fun get_username(self): Source<str> {
		// ...
	}
}

// A server method producing the wire shape — one `[rpc]` method of a `[service]` (§4).
// The projection is the only place the boundary is crossed, and it is explicit.
fun get_user(id: i32): Option<WireUser> {
	// ...look up the domain `User` (password and all), then project...
	Some(user.to_wire())   // `User` itself never crosses; only the wire shape does
}

// client-side — the generated `[service]` stub reads like a local call (§4, §7)
let john = accounts.get_user(1);   // -> Result<Option<WireUser>, RpcError>
```

What this buys, beyond the leak guarantee:

- **The wire shape diverges freely from the source.** `WireUser.uuid` is *computed* in
  `to_wire` and is not a field of `User` at all; `User.id` and `User.password` never
  appear. The client's view of an entity is whatever the projection chooses to expose —
  nothing more.
- **References travel as handles.** The same mechanism sends an arena `Handle` (or a
  reactive `Source`, §7) in place of an owned value — a "pointer" across the wire,
  resolved on the far side — because the projection decides what each field *means*.
- **No skip-lists, nothing to forget.** We considered per-field `[skip]` attributes and
  auto-projection; both were rejected. A skip-list is exactly the annotation a
  developer forgets. Here the boundary is a *type you write on purpose*, and the
  compiler refuses to let a non-Wire type slip across. Decode produces the Wire type
  directly (a `WireUser`), with no vestigial always-empty fields.

The cost is honest verbosity: a domain type and its wire twin, plus a `to_wire`. The
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

## 4. Exposure: the two-signature split, the foundation, then `[service]` sugar

An RPC endpoint has **two faces with different types**, and getting that right is the whole
design:

```vilan
// server — the real implementation, a clean local body
fun get_user(uuid: str): Option<WireUser> { /* look it up */ }

// client — a requestor that can fail at the wire
fun get_user(uuid: str): Result<Option<WireUser>, RpcError> { /* send, await, decode */ }
```

They differ by a `Result<_, RpcError>` layer *and* by their body. Crucially, they **cannot be
one function whose signature varies by caller**: that would require the compiler to know each
call site's "side," which is *undefined* for server↔server — a server calling another
server's endpoint is a *client* of it yet a *server* in its own right, so there is no global
side to switch on. So the two faces are **two functions in different namespaces**, not one
function the compiler bends. The **server face is the source of truth** (real logic); the
**client face is a mechanical projection** of it — wrap the return in `Result`, swap the body
for a wire call.

That reframes `[service]`/`[rpc]` as **sugar over a foundation that stands on its own**
(§4.1), not a mandatory system: both faces are ordinary Vilan, hand-writable, and read well
*without* the sugar. The sugar (§4.2) only generates the client face — and the server
routing — from the server declaration.

### 4.1 The foundation — an ergonomic hand-written API (no compiler features)

**Client:** one helper turns a typed call into a wire round-trip; the developer never touches
the envelope, the await, or the error layer:

```vilan
// Build the envelope, await the round-trip, decode the success payload as `T`
// (inferred from the call site). Infrastructure failures — transport, decode, a
// remote error — surface as `Err(RpcError)`. Generic over the codec (§6); JSON shown.
fun call<T: FromJson, Tx: Transport>(transport: Tx, method: str, args: List<str>): Result<T, RpcError> {
    let reply = await transport.call(RpcRequest { method, args }.to_json());
    match RpcReply::from_json(reply) {
        RpcReply::Success(let json) => Ok(T::from_json(json)),   // bound-derived decode (now compiles)
        RpcReply::Failure(let error) => Err(error),
    }
}

// A typed client is a thin holder over a transport; each method is one line.
struct AccountsClient<Tx: Transport> { transport: Tx }
impl AccountsClient<type Tx> {
    fun get_user(self, uuid: str): Result<Option<WireUser>, RpcError> {
        call(self.transport, "get_user", [uuid.to_json()])
    }
}
```

**Server:** a `Dispatcher` routes decoded requests to your handlers; the handlers stay plain
functions returning domain types (`Dispatcher`, `arg`, and `reply` are small `std::rpc`
primitives — the router plus typed decode/encode helpers):

```vilan
// `into_protocol()` makes the Dispatcher the `RpcProtocol` `dispatch`, so it drops
// onto any transport. `arg`/`reply` carry the codec; each handler is one line.
Dispatcher::new()
    .on("get_user", |req| reply(get_user(arg(req, 0)).map(|u| u.to_wire())))
    .on("rename",   |req| reply(rename(arg(req, 0), arg(req, 1)).to_wire()))
```

This is exactly `examples/rpc`'s hand-written dispatch/stub, **distilled into a reusable
API**: the archaic `RpcRequest { method = "get_user", args = [id.to_json()] }` + `match
RpcReply::from_json(..)` collapses into `call(..)` on the client and `.on(..)` on the server.
It is the API the developer wants *whether or not* the sugar exists — which is why it is built
first, and why the sugar is optional.

### 4.2 `[service]` / `[rpc]` / `[expose]` — sugar that generates the client from the server

The service is a **per-connection struct + impl** — the source of truth. `[service(Client)]`
on it generates a sibling client type (named by the argument — `[service]` alone defaults to
`<Struct>Client`); `[rpc]` marks a method callable over the wire; `[expose]` marks a `Signal`
field the client may observe:

```vilan
[service(Client)]
struct Session {
    [expose] status: Signal<str>,        // observable by the client (mirrored — §8)
    user_id: Shared<Option<i32>>,        // private session state — never crosses the wire
}

impl Session {
    // an async action: takes `self` (it awaits), mutating through the Signal/Shared handles
    [rpc] fun login(self, name: str, password: str): Result<void, LoginError> {
        let ok = await verify(name, password);
        if ok {
            self.user_id.write() = Some(id_of(name));
            self.status.set("online");
            Ok()
        } else {
            Err(LoginError::BadCredentials)
        }
    }
    // auth is manual (Q4): ordinary body logic over the session state `login` populated
    [rpc] fun rename(self, name: str): Result<WireUser, LoginError> {
        match self.user_id.read() {
            Some(let id) => Ok(rename_user(id, name)),
            None => Err(LoginError::NotAuthenticated),
        }
    }
}

// the server instantiates one per connection; the generated dispatcher owns it
fun on_connect(): Session {
    Session { status = Signal::new("offline"), user_id = Shared::new(None) }
}
```

- **`[service(Client)]`** names the generated client type. The struct *instance is the
  connection's session* — created on connect, owned by the generated dispatcher, so its state
  persists across that connection's calls (Q9).
- **`[rpc]`** marks a method **callable over the wire** — opt-in; the `[rpc]` methods *are* the
  surface (anything else is unreachable remotely — the attack-surface guarantee). Its signature
  must be **Wire-compatible** (every parameter and the return Wire, or `Option`/`Result`/`List`
  of Wire); a non-Wire `[rpc]` method is a clear compile error. **Auth is manual (Q4):** an
  auth `[rpc]` (`login`) populates session state and other methods check it in their body —
  no auth attribute; a declarative `[rpc(auth)]` gate is deferred sugar, reconsidered only if
  real services show the check as repeated boilerplate.
- **`[expose]`** marks a `Signal<T>` field the client may observe — private by default,
  observable only when marked, and only a `Signal` can be (exposure *is* observation; a plain
  value has nothing to subscribe to — Q9). `T` must be Wire. Any `[expose]`d field pulls in the
  reactive protocol, so the connection must be **duplex** (a pure-`[rpc]` service stays
  request/response).

From that the compiler emits the §4.1 foundation:

- a **dispatcher** that owns the per-connection `Session`, routes each `[rpc]` frame to
  `session.method(..)` (decode → call → encode), and registers each `[expose]`d signal in the
  §8 capability table; and
- a **client**, `Client::connect(transport)`, whose `[rpc]` methods are the `Result`-wrapped
  `call(..)`s (round-trip; §7) and whose `[expose]`d fields surface as read-only `Source<T>`
  mirrors (§8 `RemoteSource`).

```vilan
let client = Client::connect(socket);     // duplex — because `status` is exposed
await client.login("john", "hunter2");    // round-trip -> Result<Result<void, LoginError>, RpcError>
client.status.sub(|s| print(s));          // observe the mirrored server signal locally
```

The client is a **sibling type, not an `impl`** of anything the server wrote — its `[rpc]`
returns carry the extra `Result<_, RpcError>` layer (§7) and its `[expose]`d state is read-only
`Source<T>`, so it *cannot* share a signature with the server struct. The generated halves are
*only* this glue; the Wire types and `to_wire` projections stay yours (§2, §3).

## 5. Transport — the pipe (two shapes)

A transport is a dumb byte pipe; it moves encoded frames and knows nothing of methods or
subscriptions (that is the protocol's job, §7/§8). It comes in **two shapes**, matched to
what a protocol needs:

```vilan
// request/response — the shape the RPC protocol needs (HTTP, in-process)
trait Transport {
	// Send an encoded request frame, get the encoded reply. The explicit `Promise` marks
	// the round-trip as a place the caller `await`s deliberately (§7).
	fun call(self, request: List<u8>): Promise<List<u8>>;
}

// full-duplex — the shape the reactive protocol needs (WebSocket): either end may send a
// frame at any time, so the server can push unprompted.
trait DuplexTransport {
	fun send(self, frame: List<u8>);
	[must_use] fun on_frame(self, handler: |List<u8>| void): Subscription;
}
```

Built-ins:

- **HTTP** (`HttpTransport`) — `impl Transport`: POSTs the request to an endpoint and reads
  the reply body. The default client↔server transport, built on the shipped `std::fetch`
  POST/body support (§10). Request/response only — no reactive over plain HTTP.
- **In-process** (`LocalTransport`) — `impl Transport`: runs the server's dispatch in the
  same process. The substrate for **unit tests** (no network). (What `examples/rpc` uses.)
- **WebSocket** (`SocketTransport`) — `impl DuplexTransport`: a bidirectional frame pipe. It
  can *also* `impl Transport` by correlating a reply frame with its request, so the RPC and
  reactive protocols **multiplex over one socket**.
- **Asymmetric duplex** (`SplitDuplex`) — a `DuplexTransport` *implementation* that composes
  two directed channels internally (e.g. Server-Sent Events for server→client + HTTP POST for
  client→server, when WebSocket isn't available). The protocol still sees one
  `DuplexTransport`; the split is hidden in the transport — which is where the "duplex is two
  pipes" case belongs, not in the protocol's interface.

A custom transport (message queue, IPC pipe, WebRTC, a test double) is just an `impl` of the
shape it can provide — first-class, no registry.

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

The client requestor generated from the `[service(Client)]` struct (§4.2) *is* the seamless call —
`accounts.get_user(42)` reads like a method call. Sketched:

```vilan
// generated client requestor — a *sibling* type, not an impl of the service struct
// (its return carries the extra `Result` layer; §4.2). One method shown.
fun get_user(self, id: i32): Result<Option<WireUser>, RpcError> {
	let request = encode_request(self.codec, "get_user", [self.codec.encode(id)]);
	let reply = await (self.transport).call(request);     // round-trip
	decode_reply(self.codec, reply)                       // Result<Option<WireUser>, RpcError>
}
```

- **Async is seamless and honest.** The stub `await`s the transport, so it is async and a
  caller auto-awaits it — including when the transport is reached through a trait bound,
  since effect-polymorphic async now propagates through an indirect dispatch (no `dyn`, so
  every instance resolves to a statically-known impl; ✅ shipped). Latency stays *visible* as
  an `await`: the stub reads like a method call, not like a free local one — the RPC fallacy
  avoided.
- **The `T` → `Result<T, _>` shift is the contract's, and the generator owns it (Q3,
  settled).** The `[service]` method declares the *logical* signature — `get_user(id):
  Option<WireUser>` — and the server `impl` returns exactly that, a clean local body. The
  round-trip can fail, so the **generated client stub wraps the return in
  `Result<_, RpcError>`** — the developer never writes the wrapping. `RpcError` is a derived
  enum: `Transport(str) | Decode(str) | Remote(str) | Unauthorized`. The two sides differ by
  exactly one `Result` layer, applied by codegen, not by hand: the honest client without the
  noisy server.

## 8. The reactive north star — a second protocol (the capstone)

A `Signal`/`Source` is **not data** — it is a *capability*: a live reference to server state
plus an ongoing event stream. So it does not ride the Wire/codec model as a value. It is the
concern of a **second protocol**, sibling to RPC, that shares the same pure codec but requires
a **duplex** transport (§5):

```vilan
struct ReactiveProtocol<Tx: DuplexTransport, Cx: Codec> {
	transport: Tx,   // moves frames both ways (a WebSocket, or a `SplitDuplex`)
	codec: Cx,       // the *same* pure Wire codec RPC uses
	// the capability table: exported/imported `Source`s by channel id, and live subscriptions
}

// client code only ever sees a `Source<T>`; the protocol makes a *remote* one behave locally
let reactive = ReactiveProtocol { transport = socket, codec = codec };
let count: Source<i32> = reactive.source(handle);   // `handle` arrived over the wire (below)
let _ = count.sub(|n| print(i"count = {n}"));        // subscribes over the socket
```

**How a capability crosses — the Cap'n Proto capability-table pattern.** A `Source<T>` never
serializes as a value. Where a reply (or a `to_wire` projection) contains one, the reactive
protocol *exports* it into a per-connection table and puts a plain-Wire **`ChannelId`** on the
wire in its place; the receiving side *imports* that id into a `RemoteSource<T>` bound to its
protocol. So the three worries dissolve, each landing in the right layer:

- the **handle** is a `ChannelId` — a Wire id in the capability table, nothing more, so the
  codec only ever sees an integer;
- the **update payloads** are plain Wire `T` values — the codec encodes/decodes those exactly
  like any other value;
- **subscribe / update / unsubscribe** are frames the *protocol* sends over the duplex
  transport: `sub` sends a subscribe frame for the id, the server forwards its signal's updates
  as encoded-`T` frames, and `dispose()` (the existing `Disposable`/`Owner` machinery) sends an
  unsubscribe.

None of that touches the codec (pure) or the transport (a dumb pipe): the signal semantics live
in exactly one place, `ReactiveProtocol`. And because it is bound `Tx: DuplexTransport`, a
reactive protocol over a plain `HttpTransport` is a **compile error** — you cannot claim a
subscription works where the transport can't push. (A `Source` is therefore "Wire" only
*through* a reactive protocol that supplies the table, so a payload carrying one must ride the
reactive protocol, never plain RPC — the honest constraint.)

The same export/import-by-id pattern is how *any* live reference would cross — a remote object,
an arena `Handle`, a callback — so the capability table is worth designing generically even if
`Source` is the first, and at first only, capability.

The pieces this needs, all in the reactive phase:

1. **A `Source`/`Signal` split in `std::reactive`** — a read-only `Source<T>` (`get`/`sub`/`map`)
   that `Signal<T>` implements (adding `set`/`set_with`), so the remote handle implements
   `Source` and client code can't write a server signal. (The reactive README designs the API
   for this; it also intersects the signal-batching revision drafted separately.)
2. **A `DuplexTransport`** (WebSocket, §5) — plus its `SplitDuplex` fallback (SSE + POST) for
   WebSocket-less environments.
3. **The `ReactiveProtocol` + capability table** — export/import of `Source`s by id, the
   subscribe/update/unsubscribe frame protocol, and the connection-scoped lifecycle: exported
   sources reclaimed when the connection drops or the client `Owner` disposes — a natural fit
   for the existing `Owner` scopes.

## 9. Where it lives

A `[library]` package, `std::rpc` (or a standalone `rpc` library), providing the stable
core: the `Transport` and `DuplexTransport` shapes + built-in transports, the `Codec` trait
+ `JsonCodec`, `RpcError`, the envelope types, and — in the reactive phase — the
`ReactiveProtocol` and its capability table. The `[derive(Wire)]` derive, the
`[service]`/`[rpc]` generation (dispatcher + stub), and the `[trait_only]`/`[doc(hidden)]`
attributes are **compiler** features, not library code (§10). The application's own domain types, their
Wire twins, the `to_wire` projections, and the `[service]` contract live in the app —
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
  `[service(Client)]` struct's `[rpc]` methods + `[expose]` fields (§4.2). This is generation *over a struct+impl*, beyond today's
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
- **The reactive protocol** — the `Source`/`Signal` split, a `DuplexTransport` (+ its
  `SplitDuplex` fallback), and `ReactiveProtocol` with its capability table (§8) — for the
  reactive phase only.

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
   `[derive(Json)]` to `[derive(Wire)]` with explicit `to_wire` projections — the first
   dogfood. **In the same pass, bring every example up to the latest project structure**
   (platform model + library packages): current `vilan.toml` conventions, the shared
   `common` `[library]`, per-package `platform`.
3. **`[service]` generation — seamless remote functions** (L) — generate the server
   dispatcher and the client stub from a `[service(Client)]` struct (§4.2, §7), with the `Result`
   wrapping applied by codegen (auth stays manual body logic — Q4) and the **contract hash**
   (Q6 v2: the generator hashes the service surface; a mismatch on connect is a clean
   `RpcError` instead of silent decode garbage). This is the headline "C in RPC"
   and resolves Q1. Migrate `examples/rpc` from the hand-written dispatch/stub to the
   generated `[service(Client)]` struct, so the example always shows the current best form.
4. **`DuplexTransport` + server↔server** (L) — the WebSocket `SocketTransport` as a
   `DuplexTransport` (also `impl Transport` by correlation, so RPC and reactive multiplex over
   one socket), plus the `SplitDuplex` fallback; in-process service composition; a server
   calling another server. The duplex substrate the reactive protocol builds on.
5. **Reactive north star — `ReactiveProtocol`** (L) — the `Source`/`Signal` split, the
   capability table (export/import `Source`s by id), and the subscribe/update/unsubscribe frame
   protocol over the duplex transport (§8). The capstone.

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
  `err`, malformed args → `Decode`, a manual auth check without identity → its app error).
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
- **Reactive protocol** (Phase 5) — a `Source` exported to a `ChannelId` round-trips to a
  working `RemoteSource` over an in-memory `DuplexTransport` pair; `sub` receives the server
  signal's updates and `dispose()` unsubscribes; and a `ReactiveProtocol` over a
  request/response `Transport` is a clean compile *error* (the `DuplexTransport` bound).

## 13. Settled decisions vs open questions

**Settled:** the library is a *guide* for structure and a *generator* for plumbing —
Transport + Codec are the stable core; the dispatch plumbing is a **hand-writable
foundation** (`call` on the client, a `Dispatcher` on the server; §4.1) that a `[service(Client)]`
struct can *sugar* by generating it (§4.2), never a mandatory system. An endpoint has **two
signatures** — the server face returns `T`, the client face `Result<T, RpcError>` — so they
are **two functions**, not one the compiler bends by caller side (undefined for
server↔server); the server face is the source of truth and the client a generated *sibling*
projection (only the glue — the Wire types and `to_wire` projections stay the developer's). `[derive(Wire)]` is the data boundary with
the all-fields-Wire rule (sensitivity is a type property; no skip-lists); explicit
`to_wire` projections (the wire shape diverges freely from the domain type); `[rpc]`
marks the exposed surface with a Wire-compatibility signature check; `[expose]` publishes a
`Signal` field to the client as a mirrored `Source` (§8); `[trait_only]` keeps
derived methods off the concrete type (default for derives) and `[doc(hidden)]` keeps them
out of completion. The codec is the *format* (bytes, not `str`), chosen as a runtime value
so JSON↔binary is a programmatic / env switch; JSON is the default and only codec at first.
**Transport and codec compose *under* a protocol, not each other:** RPC (request/response) and
Reactive (pub/sub) are sibling protocols over a transport + codec, so plain HTTP RPC carries no
reactive machinery. The transport is a dumb pipe in two shapes — request/response (`Transport`;
HTTP/in-process) and full-duplex (`DuplexTransport`; WebSocket, or a `SplitDuplex` of SSE+POST);
the reactive protocol requires the duplex shape (a compile error otherwise). A `Signal`/`Source`
is a *capability*, exported as a `ChannelId` into a per-connection table (Cap'n Proto style) so
the codec stays pure. `Result<T, RpcError>` on the client, applied by codegen;
effect-polymorphic async (auto-await through the indirect transport call); peer-symmetric.
**Addressing is programmatic** (a transport is constructed with its endpoint; how the string is
loaded is the developer's business — no library config surface). **Versioning:** v1 relies on
single-workspace builds + clean runtime errors; v2 adds a generated contract hash with
`[service]` generation (Q6).

**Open questions** (Q1–Q9 settled; Q10 parked on a general `?`/try operator; kept numbered so
cross-references hold):

- **Q1 — client invocation form. ✅ Settled (refined):** the seamless call is **sugar over a
  hand-writable foundation** (§4.1) — `call<T>` on the client, a `Dispatcher` on the server —
  not a mandatory system. A `[service(Client)]` struct (§4.2) generates that foundation; the client is
  a generated *sibling*, not an `impl` of the trait (the two-signature split). The compiler
  generates only the glue, never the structure.
- **Q2 — codec abstraction. ✅ Settled:** ship the `Codec` trait now, with the *format*
  behind it — bytes output and a `Serializer` visitor so a binary codec is zero-overhead
  (§6). JSON is the default and only codec at first.
- **Q3 — the `T` vs `Result<T, _>` asymmetry. ✅ Settled:** the `[service]` method declares
  `T`, the server `impl` returns `T`, and the generated client stub wraps it in
  `Result<T, RpcError>` — codegen owns the one-layer difference, not the developer (§7).
- **Q4 — auth. ✅ Settled: manual (for now).** Identity lives in the **per-connection session
  struct**, populated on connect or by an auth `[rpc]` (`login`); authorization is ordinary
  body logic reading that state — §4.2's `rename` shows the pattern
  (`match self.user_id.read() { None => Err(NotAuthenticated), .. }`). No `[rpc(auth)]`
  attribute: a declarative gate is deferred sugar, revisited only if real services show the
  check as repeated boilerplate (it would then need a predicate convention, e.g.
  `fun authorized(self): bool`).
- **Q5 — addressing/config. ✅ Settled: programmatic — the transport owns its address.** A
  transport is constructed with its endpoint (`HttpTransport::new("https://api.example.com/rpc")`;
  a port + mount path on the server side); the client type stays address-agnostic (it just holds
  a transport), and *where* the string comes from — hardcoded, env var, config file, CLI flag —
  is the developer's choice, not a library config surface. One endpoint serves the whole service
  (the envelope carries the method name), so there are no per-method routes to configure. A
  browser transport may later default to same-origin (a transport nicety). The one residual —
  multi-service on one server (a mount path per service vs a service field in the envelope) — is
  decided with `[service(Client)]` generation.
- **Q6 — versioning. ✅ Settled: runtime errors for v1; a contract hash in v2 (rides with
  `[service]` generation).** v1: both sides build from one workspace, so the compiler guarantees
  the contract at build time and drift is deploy hygiene. The shipped failure modes: a renamed or
  removed method → a clean `RpcError::Remote("unknown method: …")`; a changed Wire *shape* →
  silent garbage (`from_json` doesn't validate — missing fields decode to `undefined`), the mode
  v2 exists to close. v2, with `[service(Client)]` generation (which holds the whole surface):
  emit a **contract hash** (method names + Wire shapes, normalized), sent on connect (WS) or as a
  header (HTTP); a mismatch is a clean `RpcError` *before* any decode — and can drive a "new
  version, please refresh" UX for the stale-browser-tab case. Separately backlogged (I3):
  **validating `from_json`** — decode errors instead of `undefined`, codec hardening that closes
  silent garbage for *all* malformed input, beyond version skew.
- **Q7 — projection sugar. ✅ Deferred by decision.** `to_wire` stays explicit — it *is* the
  paradigm (the wire shape diverges freely from the domain type, §3). A scaffolding derive is
  additive and waits until the explicit form has proven itself; out of scope for the initial
  build.
- **Q8 — `Map` payloads. ✅ Launch without.** Structs / `List<Pair>` cover the initial
  payloads; Map serialization (backlog I1) is pulled in when a real payload needs it
  (prove-first), not up front.
- **Q9 — service-declaration form. ✅ Settled — the canonical §4.2 form.**
  The form is `[service(Client)] struct Session { .. } impl Session { .. }`, generating
  a sibling `Client` requestor — *not* a `[service]` trait or a `mod` of free functions. The
  decisive advantage is **per-connection state**: the struct *instance* is the connection's
  session (created on connect, owned by the generated dispatcher so state persists
  across a connection's calls), which a trait/module has nowhere to hold. It subsumes the
  stateless case (a fieldless struct) and converges with the connection/turn layer
  (`reactive-batching.md`) — one object carries session state, the method surface, and the
  flush turn. The generated client stays a *sibling type* (§4.2). Three sub-questions, now resolved:
  - **Reader methods. ✅ Round-trip.** Every client method is a wire round-trip (`async` +
    `Result`) — simplest, uniform. The reactive-mirror path (a `Signal` field mirrored via §8,
    read cheaply and locally — the RPC+reactive+batching north star) is **deferred**; the escape
    hatch is that a client can read the mirrored signal directly, or hand-add a method to the
    generated `Client`.
  - **Error layering. ✅ Keep the uniform wrap — nested `Result` and all.** The client wraps the
    server's *exact* return `T` in `Result<T, RpcError>`, always — so a server method returning
    `Result<void, LoginError>` yields `Result<Result<void, LoginError>, RpcError>` on the client.
    Clunky to match, but `RpcError` stays the *uniform outer error* across every method, which is
    what lets generic client code (retry wrappers, error boundaries) hold; a merged
    `CallError<App>` would vary the error type per-method and break those consumers. No merging.
  - **Field exposure. ✅ Private by default; `[expose]` a `Signal` field.** Service-struct fields
    are server-private session state; a field is client-visible only via an explicit `[expose]`,
    and only if it is a `Signal<T>` (Source) — exposure *means* the client observes it, and only
    something observable can be mirrored (a plain value has nothing to subscribe to; a one-time
    read is what a method is for). The generated `Client` then carries a `Source<T>` for it (a §8
    `RemoteSource`), so `client.x` is a local, always-current mirror — the cheap read the
    round-trip default deferred, recovered per-field. The element `T` must be Wire; and reactive
    push needs a duplex transport (§8), so exposing any field constrains the connection to duplex
    (a pure-RPC service with no exposed fields stays request/response). Net split at the service
    surface: **methods = RPC actions (round-trip); `[expose]`d Signals = observable state.**
  - **Mutable session state. ✅ By nature — `&mut self`+plain for sync, `Signal`/`Shared` for
    async/exposed.** `&mut self` is the idiomatic in-place receiver (as `Arena`/`List`/`Map` use),
    so the connection *owns* the session and re-borrows `&mut self` per call with no `Shared` —
    ideal for *synchronous* state transitions with plain fields. But a view can't be held across an
    `await` (no-view-across-await, an intended-but-deferred rule), so an async method takes `self`
    by value (as every transport's `async fun call(self, ..)` already does); persisting a mutation
    through a by-value `self` then requires a `Shared<T>`/`Signal<T>` field (`self.x.write() = ..`).
    So: exposed or async-touched state → `Signal`/`Shared`; sync-only private state → plain field +
    `&mut self`. Default lean: `Signal`/`Shared` (await-safe, matches the reactive code), plain
    `&mut self` as the sync optimization — a `&mut self` method is itself a promise that it does
    not await. No auto-wrapping magic; the field type is the developer's and signals the method's
    nature.
- **Q10 — server-handler decode ergonomics.** `arg(req, i)` reads clean on the happy path; a
  malformed argument wants `arg -> Result<T, RpcError>` + a `?`/try to stay terse (else a
  handler regrows a per-argument match). This is really a **general error-handling dependency**
  (a `?`/try operator), not an RPC-specific decision — the foundation works today with the
  happy path plus an explicit decode-failure reply. Track as a prerequisite; revisit when
  `?`/try lands.
