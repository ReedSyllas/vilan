# RPC example — the hand-written paradigm (roadmap P6)

A working, end-to-end **RPC + reactive** runtime written out **by hand**. The library is a
*guide*, not a generator (see [`proposal/transport-rpc.md`](../../proposal/transport-rpc.md)):
it provides a codec, transports, and two sibling **protocols** over them — request/response
(`RpcProtocol`) and publish/subscribe (`ReactiveProtocol`) — and a *paradigm* for using them.
This example works that paradigm so the whole system is visible. The reusable runtime is in
[`src/rpc.vl`](src/rpc.vl); the application in [`src/main.vl`](src/main.vl).

```sh
vilan run vilan/examples/rpc
```
```
ok: found ada (@ada)
ok: no such user
raw error: {"Remote":"unknown method: delete_everything"}
--- reactive: a remote Source<i32> ---
count = 0
count = 1
count = 2
count = 10
count = 16
rpc add -> 16
--- session: the [service] paradigm, by hand ---
status = offline
whoami -> not logged in
login -> false
status = online
login -> true
whoami -> ada (@ada)
```

In-process, so it builds and runs today with **no network** — none of the Phase-0
`fetch`-POST / `http` body work is needed.

## The data boundary (proposal §3)

The headline of the paradigm: **data crosses the wire only as an explicit *wire type*, and
sensitive data is simply a type that cannot cross.**

- `Password` is **not Wire** — no `[derive(Wire)]`. So `[derive(Wire)] struct User { password:
  Password, .. }` *will not compile*: the field `password` of type `Password` is not Wire, a
  clear compile error. The boundary is enforced by the type system, not by a per-field reminder
  you might forget.
- `User` is the rich, server-side domain type; it holds a `Password`, so it never crosses.
- `WireUser` is the **explicit projection** (`User::to_wire`), a `[derive(Wire)]` DTO of only
  Wire fields. It drops `password` and *adds* a computed `handle` the domain type has no field
  for — the wire shape diverges freely from the source. The client only ever sees `WireUser`;
  it has no `password` field to leak.

`[derive(Wire)]` enforces the rule directly (proposal §3): **every field of a Wire type must
itself be Wire** — a scalar, `str`, `bool`, `List`/`Option` of Wire, or another `[derive(Wire)]`
type; anything else is a compile error. It reuses the `Json` round-trip for encode/decode, so a
Wire type serializes like a `[derive(Json)]` one — the difference is the boundary check.

## The layered runtime

The pieces of the proposal, bottom-up — a codec, transports, and protocols over them:

| Proposal piece | Here |
| --- | --- |
| **codec** (§6) | the `Json`/`FromJson` derives, used directly (frames are JSON `str`) |
| **transport** (§5) | `trait Transport` (request/response) + `LocalTransport`; `trait DuplexTransport` + `DuplexEnd` / `duplex_pair` (full-duplex, in-process) |
| **protocol** (§2) | `trait Protocol { receive }` — `RpcProtocol` (request/response) and `ReactiveServer`/`ReactiveClient` (pub/sub) all implement it |
| **service** (§4.1 foundation + §4.2 by hand) | the ergonomic hand-written API the `[service]` sugar would generate: a `Dispatcher` of `[rpc]` handlers over the `call` helper (`accounts_dispatcher()` + `AccountsClient`), and the stateful pair — a per-connection `Session` and its sibling `Client` |
| **the turn** (`reactive-batching.md`) | every inbound frame is handled in a `batch` (`local_rpc`, both duplex `on_frame`s) — a handler's signal writes coalesce into one `Update` per source, delivered with the reply |

The client and server now go through the §4.1 **foundation** — `call<T>` collapses a client
round-trip (build envelope → `await` → decode) into one line, and `Dispatcher` + `arg`/`reply`
replace the hand-rolled envelope/`match`. It is plain Vilan (no compiler feature); the eventual
`[service(Client)]` sugar just generates it, which is why it's built and proven first.

The server `lookup_user` returns `Option<User>` — `None` is an *application-level* "not found"
(part of the return type), separate from an `RpcError` (an *infrastructure* failure). The
dispatcher **projects** the domain `User` to a `WireUser` before encoding; the client stub
returns `Result<Option<WireUser>, RpcError>`.

## The reactive protocol (proposal §8)

A `Signal`/`Source` is **not data** — it is a *capability* (a live reference plus an event
stream), so it never rides the codec as a value. `ReactiveProtocol` is the second protocol, a
sibling to RPC over a **duplex** transport:

- The server `ReactiveServer` holds a per-connection **capability table**: `expose(source)`
  registers a source under a fresh **channel id** — the id is what crosses the wire in place of
  the signal. On a `Subscribe(id)` frame it forwards that source's values as `Update(id, json)`
  frames.
- The client `RemoteSource` implements **`Source<str>`** (the read-only half of the reactive
  split — client code can't write a server signal). Its `sub` opens the channel and observes a
  local mirror that inbound `Update` frames keep in sync; `count = 0` is the current value,
  delivered on subscribe, then `1` and `2` as the server `set`s it.

The `Source` trait itself is a small, additive `std::reactive` change: `Signal`'s read-only
`get`/`sub` moved into `trait Source<T>`, which both `Signal` and `RemoteSource` implement (the
corpus stays byte-identical).

## The wire turn (reactive batching)

The scenario that motivated `std::reactive`'s batching (`proposal/reactive-batching.md`): an
**RPC call mutates a signal the client is subscribed to**. Without a boundary, every `set`
inside the handler pushes its own `Update` frame, mid-handler, before the reply even exists.
So the runtime handles **every inbound frame in a `batch`** — the *turn* (`local_rpc` and both
duplex `on_frame`s). The demo shows all three behaviours:

- A lone `set` outside any batch stays **eager** — one write, one `Update` (`count = 1`, `2`),
  byte-for-byte the pre-batching behaviour.
- An explicit `batch(|| { counter.set(5); counter.set(10); })` **coalesces**: the mirror
  recomputes once, so ONE frame crosses (`count = 10` — the intermediate 5 is never observed).
- The `add` RPC method writes the counter **twice** in its handler; `local_rpc`'s turn defers
  both, so a single `Update` (`count = 16`) is delivered in the same turn as the reply
  (`rpc add -> 16`). Values commit eagerly — the second write reads the first's result — only
  the *notification* defers.

In-process the update lands just before the reply (delivery is synchronous); a buffering
transport (WebSocket, plan phase 4) would flush the coalesced frames and the reply together at
the turn's end via `transport.flush()`.

## The session service (proposal §4.2, by hand)

`[service(Client)]` isn't implemented yet, so the demo writes out **exactly what it will
generate**: a per-connection `Session` struct (the source of truth) and its *sibling* `Client`
(the two-signature split made visible — `Session::login(..): bool` vs
`Client::login(..): Result<bool, RpcError>`).

- **Per-connection state (Q9).** One `Session` is created "on connect"; the dispatcher's
  handlers capture it, so state persists across the connection's calls (`login` then `whoami`).
  Mutable state lives in `Signal`/`Shared` handles — closures capture a *copy* of the struct
  (value semantics), so the shared cells are what make the state one.
- **Manual auth (Q4).** `whoami` is ordinary body logic over the state `login` populated —
  unauthenticated is an application-level `None` (`whoami -> not logged in`), no `[rpc(auth)]`
  attribute. The `Password` check happens entirely server-side (`matches` is the only operation
  the type exposes; the hash never leaves).
- **An exposed field (`[expose]`, §8).** `Session.status` is exported under a channel id; the
  `Client` carries a `RemoteSource` mirror for it. A successful login flips it — and the wire
  turn delivers `status = online` in the same turn as `login -> true` (the failed login changes
  nothing). Observers decode the JSON-encoded value at the concrete site; the typed wrapper is
  the sugar's job.

## Quirks discovered

Part of why this example is worth keeping: it surfaces compiler quirks the eventual generation
will lean on. **#1, #3, #4, #5 were bugs — all fixed; #2 is intended syntax.** Bugs #3–#5 traced
to one weakness — generic dispatch / monomorphization not threading type arguments through
indirect, nested, or closure-capture contexts — the analyzer's B1 cluster, now closed across all
three (#5 being the *closure-capture* case).

### 1. `[derive(..)]` only expanded in the entry file — ✅ FIXED

Originally, putting the runtime in a separate `src/rpc.vl` and importing it gave
`cannot find 'from_json' in RpcRequest` for every imported derived type, while a
`[derive(Json)]` struct *in* `main.vl` worked — `expand_derives` ran on the **entry
program only**.

**Fixed** (commit 3592343): derive expansion now runs in *every* module — each loaded
module and each dependency `lib.vl` — so a derived type's `to_json`/`from_json`/… work
wherever it's defined. This example demonstrates it directly: the runtime and its
derived envelope types live in `rpc.vl`, imported by `main.vl`. The proposal's shared
`common` library of wire types is unblocked.

### 2. Calling a method on a generic field needs parens + a struct-level bound (intended syntax — *not* a bug)

The natural client stub is an object holding the transport, and the first instinct
errors:

```vilan
struct AccountsClient<T> { transport: T }                       // no bound anywhere
impl AccountsClient<type T> {
    fun get_user(self, id) { ... self.transport.call(..) ... }  // ✗ cannot call method 'call' on T
}
```

Two things are wrong, both **intended language rules**: a method call on a
field-*projection* receiver must **parenthesize the receiver**, and the trait **bound
must be declared on the struct definition** (so the field's type carries it). With
both, it type-checks:

```vilan
struct AccountsClient<T: Transport> { transport: T }            // bound on the struct
impl AccountsClient<type T> {                                   // impl infers it
    fun get_user(self, id) { ... (self.transport).call(..) ... }  // ✓ type-checks
}
```

The impl does **not** restate the bound: an `impl AccountsClient<type T>` can only
apply to an `AccountsClient`, whose existence already requires `T: Transport`, so the
binder inherits that bound. (Restating it, `impl AccountsClient<type T: Transport>`,
is still accepted and means the same thing.)

(`(self.transport).call(..)` is the same disambiguation that makes a *closure* field
call `(self.handler)(request)` — which the runtime uses throughout, e.g. `Dispatcher`'s
`(route.handler)(request)` and `ReactiveServer`'s `(self.transport).send(..)`. The
`AccountsClient` stub above no longer needs it: `get_user` now passes the transport to the
free `call` helper (§4.1), so the method-on-a-field form lives in `rpc.vl`, not `main.vl`.)

### 3. …and that object stub used to *miscompile* — ✅ FIXED

The form from #2 type-checked, then printed `undefined`: the generic-field dispatch
`(self.transport).call(..)` lowered to the empty abstract trait method, because the
struct field's `T` carried the struct definition's generic id while the call's binding
was keyed by the impl/receiver's id — `current_substitution` missed and the abstract
`call` was emitted.

**Fixed** by two root-cause changes (backlog B1, class B):

1. **Field access substitutes the receiver's type arguments** (`resolve_field_accessor`):
   `self.transport` on `AccountsClient<LocalTransport>` (or, inside the impl, on the
   impl's own `T`) now resolves to the concrete/impl-bound type instead of the struct's
   abstract parameter — so the dispatch binding composes.
2. **A generic struct initializer no longer leaks an abstract type while deferred.**
   `let client = AccountsClient { transport = transport }` (field from a *variable*)
   used to ground `client` as `AccountsClient<Transport>` (the trait bound) because the
   initializer published an unbound type before the field value resolved. It now defers
   cleanly, so `client` grounds to `AccountsClient<LocalTransport>`.

So the **object stub is the form used here** — `AccountsClient<T: Transport>`, constructed
and called from `main`. Its `get_user` now passes the `T`-typed field to the generic `call`
helper (§4.1) rather than dispatching on it directly; the same field-substitution fix makes
that generic-through-generic path monomorphize (pinned by `generic_field_method_dispatch_runs`,
`generic_field_from_a_variable_dispatches`, and `generic_call_over_a_bounded_transport_decodes`
in `inference.rs`).

### 4. `from_json` element-type inference through an indirect path — ✅ FIXED

```vilan
RpcReply::Success(let json) => Ok(Option::from_json(json)),   // ✓ now binds WireUser
```

Here `Option::from_json` must infer its element type `WireUser` through the `Ok(..)`
wrapper and the function's return type. That indirect path *used to* lower the inner
decode to the empty abstract `from_json_value`, yielding `Some(undefined)` — so the
stub pinned the type with a local `let user: Option<WireUser> = ..`.

**Fixed** (the return-type-driven body inference, B1): a function's body is now
inferred *against* its declared return type, and `resolve_match` propagates that
expected type into each leg, so the type flows `Result<Option<WireUser>, _>` → the `Ok`
arm → the `Ok(..)` wrapper → `Option::from_json`'s element. The stub above uses the
natural indirect form directly — no pinning needed. (`enum_constructor_..` and
`from_json_return_type_flows_through_match_arm` in `inference.rs` pin both halves.)

### 5. Generic element serialized inside a closure — ✅ FIXED

Building the reactive protocol surfaced a monomorphization gap that shaped `expose`. With
`S: Source<T>`, `T: Json`, `source.sub(|value| value.to_json())` used to fail *two* ways:
the closure parameter `value` lost its `T: Json` bound (a compile error, `cannot call method
'to_json' on T`), and — since `T` appears only in the bound `F: Source<T>`, not a direct
parameter — `T` was never derived from the concrete `Signal<i32>: Source<i32>` at the call
site, so `to_json` monomorphized to the empty abstract method (`undefined`). The same
abstract-dispatch failure as #3/#4, but reached through a *closure capture* of a generic,
which the earlier B1 fixes didn't cover.

**Fixed** by three analyzer changes: the `Type::Generic` method-resolution arm now substitutes
a parameterized bound's arguments (so the closure parameter keeps its `T: Json`);
`resolve_call_subject` / `bind_method_own_generics` derive a bound-only generic from the
concrete argument's impl (`derive_generics_from_bounds`); and `resolve_method_call` defers a
call whose method still has an unbound own-generic while an argument is unresolved, so an
*inferred* source (`let s = Signal::new(7)`, no annotation) re-derives once its type lands —
the method path now matches the free-function path. So `expose<T: Json, S: Source<T>>(source: S)`
monomorphizes — the JSON erasure moved *inside* the runtime (a `Signal<str>` mirror per
channel), off the application, which now just calls `server.expose(counter)`.

One adjacent item remains, noted for honesty: **`param: SomeTrait` is not a generic bound**
(you write the explicit `<S: Source<T>>`, so the proposal's `fun stringify(value: ToJson)`
sketch is aspirational syntax). The capability table also still stores `str`, since it holds
heterogeneous sources and vilan has no trait objects.

## What this validates for the plan

The **data boundary, both transports, the codec, both protocols (RPC and reactive), the
capability table, over-the-wire subscription, the wire turn (reactive batching — an RPC
handler's writes coalescing with its reply), the per-connection session (state + manual auth +
an exposed, client-mirrored signal), and the `Result`/`Option` error layering all work
today** — the hand-written core is real, and the *paradigm* (a domain type, an explicit
`to_wire` projection, a sensitive type that can't cross, a signal observed remotely) holds with
today's features. The generic-dispatch cluster (B1) that P6 leaned on is now closed through the
closure-capture case too (#5), so `expose` is generic over any `Source<T>`.

What's left is purely mechanical, in the spirit of "guide, not generator": the
`[service(Client)]`/`[rpc]`/`[expose]` sugar generates exactly the `Session` → `Client` +
dispatcher pair this example now writes by hand — it replaces none of the paradigm.
(`[derive(Wire)]`, the `call`/`Dispatcher` foundation, and the wire turn are already real.)
