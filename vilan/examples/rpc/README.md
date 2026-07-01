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
```

In-process, so it builds and runs today with **no network** — none of the Phase-0
`fetch`-POST / `http` body work is needed.

## The data boundary (proposal §3)

The headline of the paradigm: **data crosses the wire only as an explicit *wire type*, and
sensitive data is simply a type that cannot cross.**

- `Password` has **no codec** — no `[derive(Json)]`. A value of it cannot be encoded, so
  `[derive(Json)] struct User { password: Password, .. }` *will not compile*
  (`Password has no method 'to_json'`). The boundary is enforced by the type system, not by
  a per-field reminder you might forget.
- `User` is the rich, server-side domain type; it holds a `Password`, so it never crosses.
- `WireUser` is the **explicit projection** (`User::to_wire`), a `[derive(Json)]` DTO of only
  encodable fields. It drops `password` and *adds* a computed `handle` the domain type has no
  field for — the wire shape diverges freely from the source. The client only ever sees
  `WireUser`; it has no `password` field to leak.

`[derive(Json)]` stands in for the proposed `[derive(Wire)]`; the all-fields-encodable
property already holds (a struct with a non-encodable field can't derive), and `[derive(Wire)]`
will formalize it with a friendlier diagnostic and an exposure marker.

## The layered runtime

The pieces of the proposal, bottom-up — a codec, transports, and protocols over them:

| Proposal piece | Here |
| --- | --- |
| **codec** (§6) | the `Json`/`FromJson` derives, used directly (frames are JSON `str`) |
| **transport** (§5) | `trait Transport` (request/response) + `LocalTransport`; `trait DuplexTransport` + `DuplexEnd` / `duplex_pair` (full-duplex, in-process) |
| **protocol** (§2) | `trait Protocol { receive }` — `RpcProtocol` (request/response) and `ReactiveServer`/`ReactiveClient` (pub/sub) all implement it |
| **service** (§4, a *paradigm*) | the would-be `[service]`/`[rpc]` surface — here `accounts_dispatch` routed through an `RpcProtocol`, plus the `AccountsClient` stub |

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

## Quirks discovered

Part of why this example is worth keeping: it surfaces compiler quirks the eventual generation
will lean on. **#1, #3, #4 were bugs — all fixed; #2 is intended syntax; #5 is a new limitation
the reactive runtime surfaced, still open.** Bugs #3–#4 traced to one weakness — generic
dispatch / monomorphization not threading type arguments through indirect or nested contexts —
the analyzer's B1 cluster, now closed for the direct and nested cases (#5 is the remaining
*closure-capture* case).

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
call `(self.handler)(request)` — which the runtime above uses.)

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

So the **object stub is the form used here** — `AccountsClient<T: Transport>` with a
`(self.transport).call(..)` method, constructed and called from `main`. (The
`generic_field_method_dispatch_runs` and `generic_field_from_a_variable_dispatches`
tests in `inference.rs` pin both halves.)

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

### 5. Generic-through-closure monomorphization — the reactive runtime stays `str` (⚠️ open)

Building the reactive protocol surfaced two limits that shaped its shape. Neither is fixed;
both are what the proposal's generic `ReactiveProtocol<Tx, Cx>` is waiting on.

- **`param: SomeTrait` is not a generic bound.** `fun expose(source: Source<T>)` is rejected —
  *"a trait is not a value type (vilan has no trait objects)"*; you must write the explicit
  `fun expose<T, S: Source<T>>(source: S)`. (So the proposal's `fun stringify(value: ToJson)`
  sketch is aspirational, not current syntax.)
- **A generic *element* serialized inside a closure doesn't monomorphize.** With
  `S: Source<T>`, `T: Json`, `source.sub(|value| value.to_json())` fails to compile
  (`cannot call method 'to_json' on T`); routing through a free `encode<T: Json>` helper
  compiles but emits `undefined` — the same abstract-dispatch failure as #3/#4, but reached
  through a *closure capture* of a generic, which the B1 fixes didn't cover.

So the runtime is **monomorphic over `str`**: the capability table stores `Signal<str>`, and
the application erases `T` at the concrete boundary — `counter.map(|n| n.to_json())` on the
server, `i32::from_json(json)` in the client's observer — where `T` is concrete and
monomorphizes fine. When generic-through-closure monomorphization lands, those `str`s become a
generic `T` and the `.map`s vanish, yielding the proposal's `ReactiveProtocol<Cx: Codec>`
directly.

## What this validates for the plan

The **data boundary, both transports, the codec, both protocols (RPC and reactive), the
capability table, over-the-wire subscription, and the `Result`/`Option` error layering all work
today** — the hand-written core is real, and the *paradigm* (a domain type, an explicit
`to_wire` projection, a sensitive type that can't cross, a signal observed remotely) holds with
today's features. The generic-dispatch cluster (B1) that P6 leaned on is closed for the direct
and nested cases; the remaining closure-capture case (#5) is the concrete compiler work the
*generic* `ReactiveProtocol<Cx: Codec>` needs.

What's left is additive and stays in the spirit of "guide, not generator": `[derive(Wire)]`
(the boundary, friendlier diagnostics), `[service]`/`[rpc]` (generate the dispatcher + stub),
and closing #5 (to make the protocols generic over the codec) — none of them *replace* the
paradigm this example works.
