# RPC example — the hand-written runtime (roadmap P6)

A working, end-to-end RPC round-trip written out **by hand** — no `[service]`/`[rpc]`
codegen sugar — so the whole system is visible. It's the concrete form of Phase 1 of
[`proposal/transport-rpc.md`](../../proposal/transport-rpc.md), and it exists to
**surface compiler quirks** that the eventual `[service]` generation will have to deal
with. The reusable runtime is in [`src/rpc.vl`](src/rpc.vl); the application — the
shared contract, the server dispatcher, the client stub — in [`src/main.vl`](src/main.vl).

```sh
vilan run vilan/examples/rpc
```
```
ok: found Ada
ok: no such user
raw error: {"Remote":"unknown method: delete_everything"}
```

## What it is

In-process (a `LocalTransport` that runs the dispatcher in the same process), so it
builds and runs today with **no network** — none of the Phase-0 `fetch`-POST / `http`
body work is needed. The four layers of the proposal are all spelled out:

| Proposal layer | Here |
| --- | --- |
| **transport** | `trait Transport` + `LocalTransport` (a handler wrapped in a Promise) |
| **codec** | the `Json`/`FromJson` derives, used directly (no `Codec` trait yet) |
| **wire** | `RpcRequest { method, args }` / `RpcReply { Success \| Failure }` / `RpcError` |
| **service** | `User` contract + `accounts_dispatch` (server) + `get_user` (client stub) |

The server `lookup_user` returns `Option<User>` — `None` is an *application-level*
"not found" (part of the return type), separate from an `RpcError` (an
*infrastructure* failure). The client stub returns `Result<Option<User>, RpcError>`.

## Quirks discovered

The reason this example is worth keeping. **#1 and #4 were bugs — now fixed; #3 is a
bug; #2 is intended syntax.** **Bug #3 (and the now-fixed #4) trace to one underlying
weakness: generic dispatch / monomorphization did not thread type arguments through
indirect or nested contexts, so a call bound to the empty *abstract* trait method.**
That's the analyzer's generic-resolution cluster (backlog B1 / `analyzer-refactor.md`),
and P6 leans on it heavily.

### 1. `[derive(..)]` only expanded in the entry file — ✅ FIXED

Originally, putting the runtime in a separate `src/rpc.vl` and importing it gave
`cannot find 'from_json' in RpcRequest` for every imported derived type, while a
`[derive(Json)]` struct *in* `main.vl` worked — `expand_derives` ran on the **entry
program only**.

**Fixed** (commit 3592343): derive expansion now runs in *every* module — each loaded
module and each dependency `lib.vl` — so a derived type's `to_json`/`from_json`/… work
wherever it's defined. This example now demonstrates it directly: the runtime and its
derived envelope types live in `rpc.vl`, imported by `main.vl`. The proposal's shared
`common` library of derived contract types is unblocked.

### 2. Calling a method on a generic field needs parens + a struct-level bound (intended syntax — *not* a bug)

The natural client stub is an object holding the transport, and the first instinct
errors:

```vilan
struct AccountsClient<T> { transport: T }                       // bound only on the impl
impl AccountsClient<type T: Transport> {
    fun get_user(self, id) { ... self.transport.call(..) ... }  // ✗ cannot call method 'call' on T
}
```

Two things are wrong, both **intended language rules**: a method call on a
field-*projection* receiver must **parenthesize the receiver**, and the trait **bound
must be on the struct definition** (so the field's type carries it). With both, it
type-checks:

```vilan
struct AccountsClient<T: Transport> { transport: T }            // bound on the struct
impl AccountsClient<type T: Transport> {
    fun get_user(self, id) { ... (self.transport).call(..) ... }  // ✓ type-checks
}
```

(`(self.transport).call(..)` is the same disambiguation that makes a *closure* field
call `(self.handler)(request)` — which the runtime above uses.)

### 3. …but that object stub then *miscompiles* (runtime bug)

The form from #2 type-checks, then prints `undefined`: the generic-field dispatch
lowers to the empty abstract trait method.

```vilan
// (self.transport).call(r)  with  struct AccountsClient<T: Transport>  emits:
function paren(self, r) { return call(self[0], r); }   // `call` = function call(self, request) {}  ← abstract, empty
```

So the correctly-written object stub still doesn't *run*. (Routing through a generic
helper hits the same wall — the generic call, invoked from an already-generic context,
never monomorphizes to `LocalTransport::call`.)

- **Workaround here:** the client stub is a **free generic function taking the
  transport as a direct parameter**, called from `main` with a concrete transport —
  `get_user<T: Transport>(transport: T, id)`. Monomorphized at a top-level concrete
  call site, the dispatch lowers correctly.
- **What the plan needs:** the object-stub form (and the proposal's
  `Accounts::connect(transport)`) needs this nested/field generic dispatch fixed in
  codegen. Until then the `[service]` derive should emit **free functions**.

### 4. `from_json` element-type inference through an indirect path — ✅ FIXED

```vilan
RpcReply::Success(let json) => Ok(Option::from_json(json)),   // ✓ now binds User
```

Here `Option::from_json` must infer its element type `User` through the `Ok(..)`
wrapper and the function's return type. That indirect path *used to* lower the inner
decode to the empty abstract `from_json_value`, yielding `Some(undefined)` — so the
stub pinned the type with a local `let user: Option<User> = ..`.

**Fixed** (the return-type-driven body inference, B1): a function's body is now
inferred *against* its declared return type, and `resolve_match` propagates that
expected type into each leg, so the type flows `Result<Option<User>, _>` → the `Ok`
arm → the `Ok(..)` wrapper → `Option::from_json`'s element. The stub above uses the
natural indirect form directly — no pinning needed. (`enum_constructor_..` and
`from_json_return_type_flows_through_match_arm` in `inference.rs` pin both halves.)

- **What the plan needs:** the `[service]` generator can emit the natural form; it no
  longer has to pin every `from_json` to dodge this edge.

## What this validates for the plan

The **wire model, codec, dispatcher, `Result`/`Option` error layering, and async
round-trip all work today** — Phase 1 is real. **Derives-in-dependency-modules (#1)
and the indirect `from_json` inference (#4) are now fixed**, so the shared `common`
library of derived contract types works (this example imports its derived runtime from
`rpc.vl`) and the client stub decodes with the natural `Ok(Option::from_json(json))`.
The remaining gate on the ergonomic surface (a pluggable generic *client object*) is
the **generic-field dispatch bug (#3)** — the last of the generic-dispatch /
monomorphization cluster (B1, class B / stable generic identity). Until that lands, the
`[service]` generator must stay within the forms that work: free generic functions.
