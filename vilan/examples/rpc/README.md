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

The reason this example is worth keeping. **#1, #3, and #4 are bugs; #2 is intended
syntax** (so it's documented as a gotcha, not a defect). **Bugs #3 and #4 are one
underlying weakness: generic dispatch / monomorphization does not thread type
arguments through indirect or nested contexts, so a call binds to the empty
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
