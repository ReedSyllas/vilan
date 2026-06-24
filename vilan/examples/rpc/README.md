# RPC example — the hand-written runtime (roadmap P6)

A working, end-to-end RPC round-trip written out **by hand** — no `[service]`/`[rpc]`
codegen sugar — so the whole system is visible in one file. It's the concrete form of
Phase 1 of [`proposal/transport-rpc.md`](../../proposal/transport-rpc.md), and it
exists to **surface compiler quirks** that the eventual `[service]` generation will
have to deal with. Everything is in [`src/main.vl`](src/main.vl).

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

The reason this example is worth keeping. Every workaround below is a thing the
`[service]` generator (or the compiler) must handle for the real library. **Quirks
2–4 are one underlying weakness: generic dispatch / monomorphization does not thread
type arguments through indirect or nested contexts, so a call binds to the empty
*abstract* trait method.** That's the analyzer's generic-resolution cluster (backlog
B1 / `analyzer-refactor.md`), and P6 leans on it heavily.

### 1. `[derive(..)]` only expands in the entry file

Putting the runtime in a separate `src/rpc.vl` and importing it gave
`cannot find 'from_json' in RpcRequest` and `RpcRequest has no method 'to_json'` for
every imported derived type — while a `[derive(Json)]` struct *in* `main.vl` worked
fine. `expand_derives` runs on the **entry program only**, not on imported/dependency
modules.

- **Workaround here:** everything is in one file (`main.vl`).
- **What the plan needs:** the proposal puts the shared contract types in a `common`
  **library**, imported by both sides — so **derives must expand in dependency
  modules** before that works. A hard prerequisite, surfaced immediately.

### 2. Generic dispatch fails on a generic-typed *field projection* (type error)

The natural client stub is an object holding the transport:

```vilan
struct AccountsClient<T> { transport: T }
impl AccountsClient<type T: Transport> {
    fun get_user(self, id: i32): ... { ... self.transport.call(..) ... }   // ✗
}
```

`self.transport.call(..)` → **`cannot call method 'call' on T`**. A trait method
dispatches fine on a *direct generic parameter* (`fun f<T: Transport>(t: T) { t.call(..) }`)
or a *concrete-typed* field, but not on a **generic-typed field projection**.
Binding it to a local first (`let t = self.transport; t.call(..)`) does **not** help.

### 3. A generic helper called from a generic context *miscompiles* (runtime)

Routing quirk 2 through a helper type-checks but is worse — it lowers to the empty
abstract method:

```vilan
fun transport_call<T: Transport>(t: T, r: str): Promise<str> { t.call(r) }   // type-checks
// ... but the emitted helper is:  function $p(t, r) { return call(t, r); }
//     where `function call(self, request) {}`  ← the empty *abstract* Transport::call
```

So `await` got `undefined`. The generic call inside the helper, invoked from the
already-generic stub method, never monomorphized to `LocalTransport::call`.

- **Workaround here:** the client stub is a **free generic function taking the
  transport as a direct parameter**, called from `main` with a concrete transport —
  `get_user<T: Transport>(transport: T, id)`. Monomorphized at a top-level concrete
  call site, the dispatch lowers correctly.
- **What the plan needs:** the object-stub form (and the proposal's
  `Accounts::connect(transport)`) needs nested-generic dispatch fixed. Until then the
  `[service]` derive should emit **free functions**, or the compiler grows this.

### 4. `from_json` element-type inference through an indirect path → abstract method

```vilan
RpcReply::Success(let json) => Ok(Option::from_json(json)),   // ✗ Some(undefined)
```

Here `Option::from_json` must infer its element type `User` through the `Ok(..)`
wrapper and the function's return type. That indirect path lowered the inner decode
to the empty abstract `from_json_value`, yielding `Some(undefined)`. Pinning the type
**directly** fixes it:

```vilan
let user: Option<User> = Option::from_json(json);   // ✓
Ok(user)
```

- **What the plan needs:** the `[service]` generator already knows every concrete
  type, so it can always emit the pinned form — but this is a sharp edge a
  hand-writer hits, and a sign the return-type-driven `from_json` inference is
  fragile across wrappers.

## What this validates for the plan

The **wire model, codec, dispatcher, `Result`/`Option` error layering, and async
round-trip all work today** — Phase 1 is real. But the ergonomic, generic surface the
proposal wants (a `common` library of derived contract types, a pluggable generic
client object) is **gated on the generic-dispatch / monomorphization cluster and on
derives-in-dependencies**. Phase 0/1 should pull those in (or the generator must stay
within the forms that work: one module, free generic functions, pinned `from_json`).
