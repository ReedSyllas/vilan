# todos — the realtime full-stack example

The transport/RPC library's milestone app (`proposal/transport-rpc.md`, phase 6):
a to-do list whose data lives on the **server**, edited from the **browser**, with
every connected tab kept in sync in realtime. Open the page twice; add a todo in
one tab and watch it appear in the other. Restart the server; the list is still
there.

```
vilan run vilan/examples/todo     # build both bundles + start the server
# → http://localhost:59386/  (open it in two tabs)
```

## The shape

```
common/   the shared vocabulary — compiled into BOTH bundles
  Todo                 [derive(Wire)]: the codec + the proof it's wire-safe
  TodoStore            [service(TodoClient)]: state + [rpc] methods
  TodoClient           GENERATED: the typed stub the browser calls
server/   node — owns the one TodoStore, persists it, serves everything
client/   browser — renders signals; never touches the data directly
```

One struct is the whole contract. `[service(TodoClient)]` generates the server's
`dispatcher()` and the client's `TodoClient<T: Transport>` sibling (each call
`Result`-wrapped — the two-signature split), and a `contract_hash()` shared by
both, so a stale bundle is detectable, not mysterious.

## How the data flows

The server holds the list in an `[expose]`d `Signal<List<Todo>>` and mounts the
generated dispatcher with `std::rpc_server::serve_service` — one port serving
the page, the WebSocket upgrade, and the RPC/SSE routes. Each tab makes ONE
generated call:

1. `TodoClient::connect("/", json_codec())` — opens the WebSocket, **verifies
   the contract hash** (a stale bundle is a clean `Err(Contract)`, not decode
   garbage), attaches this connection, and wires the `todos` mirror;
2. `client.todos.sub(…)` — the remote handle: the current list arrives at
   once, then every change as it lands. RPC calls ride the same socket
   (multiplexed), so there are no further HTTP requests at all.

Mutations go the other way: the checkbox calls `client.toggle(id)`, the server
mutates its signal, and the change fans out to **every** subscribed session —
including the tab that made it. The client never edits its local list; there is
no refetch, no cache invalidation, one code path whether the edit was yours or
another tab's. Each inbound frame is handled in a reactive `batch` (the wire
turn), so a handler's writes coalesce into one `Update` per source.

Persistence is the same mechanism pointed at disk: the server `sub`scribes to
its own signal and writes `todos.json` on every change. The wire, the file, and
the UI are all just observers of one `Signal`.

What stays client-side is the *view* state: the draft input and the active
filter are local signals, and `remaining`/`visible` are derived (`map`,
`combine`) — the shared list and this tab's own state compose in one dependency
graph.

## Where the seams are (deliberately)

- The connect handshake, the per-connection sessions, and the mirror wiring are
  all generated/runtime now (§4.2's `Client::connect` + the session registry) —
  this example used to hand-write an `attach` method and a sessions map; apps
  needing custom per-connection state (auth identity, say) still can, via
  `serve_connected` and their own registry.
- The mirror still delivers JSON strings the client decodes at the site
  (`List::from_json`) — typed mirrors land with the reactive-on-codec
  follow-up.
- Sessions end: when a tab closes (socket or SSE), the runtime disposes that
  connection's session, so a long-running server doesn't accumulate dead wires.
