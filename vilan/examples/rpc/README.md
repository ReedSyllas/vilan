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
| **service** | `User` contract + `accounts_dispatch` (server) + `AccountsClient` object stub (client) |

The server `lookup_user` returns `Option<User>` — `None` is an *application-level*
"not found" (part of the return type), separate from an `RpcError` (an
*infrastructure* failure). The client stub returns `Result<Option<User>, RpcError>`.

## Quirks discovered

The reason this example is worth keeping. **#1, #3, and #4 were bugs — all now fixed;
#2 is intended syntax.** **Bugs #3 and #4 traced to one underlying weakness: generic
dispatch / monomorphization did not thread type arguments through indirect or nested
contexts, so a call bound to the empty *abstract* trait method.** That's the analyzer's
generic-resolution cluster (backlog B1 / `analyzer-refactor.md`), which P6 leaned on
heavily — and which is now closed. The client is written as the natural object stub.

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
`(self.transport).call(..)` method, constructed and called from `main`. The `[service]`
derive can generate this directly. (`generic_field_method_dispatch_runs` and
`generic_field_from_a_variable_dispatches` in `inference.rs` pin both halves.)

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
round-trip all work today** — Phase 1 is real. **All four quirks are resolved**: derives
in dependency modules (#1), the parenthesized field-receiver syntax (#2, intended), the
generic-field dispatch for the object stub (#3), and the indirect `from_json` inference
(#4). So the shared `common` library of derived contract types works, the client is the
natural **object stub** `AccountsClient<T: Transport>` calling `(self.transport).call(..)`,
and decoding uses the natural `Ok(Option::from_json(json))`. The generic-dispatch /
monomorphization cluster (B1) that P6 leaned on is **closed** — the `[service]` generator
can emit the ergonomic object form directly.
