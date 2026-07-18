# std::rpc — reference

Transports, the generated service surface, errors, and connection state.
Concepts and usage: the [services guide](../guide/services.md). Most apps
touch only the **generated client**, `RpcError`, and `ConnectionState` —
everything else here is the machinery those sit on.

## The generated surface (`[service]`)

For `[service(FooClient)] struct Foo` with `[rpc]` methods and `[expose]`
signal fields, the macro generates:

```vilan,fragment
// client side
FooClient::connect(url: str, codec: Codec): Result<FooClient<SocketTransport>, RpcError>
client.some_rpc(args…): Result<T, RpcError>     // per [rpc] method; implicitly awaited
client.some_signal: Signal<T>                   // per [expose] field; a live mirror
client.transport: SocketTransport               // connection state lives here

// server side
foo.dispatcher(): Dispatcher                    // the method table
dispatcher.into_protocol(codec: Codec): RpcProtocol   // what serve_service takes
```

`connect` accepts a relative url (`"/"`) in the browser; it dials the same
host over WebSocket, waits for the server's announcement, and verifies the
**contract hash** — a drifted server fails the connect with
`RpcError::Contract`.

## Errors

```vilan,fragment
[derive(Wire, Debug)]
enum RpcError {
	Transport(str),   // couldn't reach / lost the server ("not connected", "connection lost")
	Decode(str),      // reply didn't parse
	Remote(str),      // the handler failed
	Contract(str),    // connect-time shape mismatch (old client vs new server)
	Unauthorized,
}
```

Infrastructure failures only — an *application* "not found" belongs in the
rpc's own return type (`Option<Task>`), not here.

## Connection state

```vilan,fragment
enum ConnectionState { Connected, Reconnecting, Closed }

impl SocketTransport {
	fun connection_state(self): Signal<ConnectionState>
}
```

The reconnect lifecycle (automatic): on drop → `Reconnecting`, in-flight
calls reject with `Transport("connection lost")`, new calls fail fast with
`Transport("not connected")`; dial with backoff (250 ms doubling, 4 s cap,
10 attempts); on success → contract re-check, mirrors re-attach and resync,
`Connected`. Backoff exhausted → `Closed`. Nothing is ever silently
retried — retry is the app's decision.

## Transports

```vilan,fragment
trait Transport {
	fun call(self, request: Frame): Task<Result<Frame, str>>;
}
```

| Transport | Wire | Use |
|---|---|---|
| `SocketTransport` | WebSocket (reconnecting) | what `connect` gives you — the production client transport |
| `HttpTransport` | one POST per call | stateless calls, no mirrors |
| `LocalTransport` | in-process | tests: client and service in one process |

Below `SocketTransport` sits `SocketDuplex` (the reconnect-surviving socket:
pending-call registry, inbound dispatch, `on_reconnect` hooks) and the
`DuplexTransport` machinery (`duplex_pair`, `bridge`, `connect_split` for
the SSE/split fallback). App code doesn't construct these; the generated
`connect` does.

```vilan,fragment
fun connect_socket(url: str): Result<SocketDuplex, str>   // dial + announcement (backoff)
impl SocketDuplex {
	fun transport(self): SocketTransport
}
```

## Server plumbing (`std::rpc_server`, process layer)

```vilan,fragment
fun serve_service(
	port: i32,
	protocol: RpcProtocol,
	fallback: |Request| Response,   // plain-http requests: assets + app shell
	on_ready: || void,
)
```

`serve_service` = WebSocket upgrade + per-connection session registration
(mirror attach/detach) + rpc dispatch, with `fallback` answering ordinary
http. Each handler runs in a turn (`AtEnd`). For custom per-connection
state (connection-scoped auth, an app-written attach), use
`serve_connected(port, protocol, on_connection, fallback, on_ready)` — the
same server with the session hook exposed.

## Envelope & codec layer

`Frame` is the codec-agnostic unit (`std::wire`); `encode_request` /
`open_request` / `encode_reply` read and write the rpc envelope
(`{"method": …, "args": […]}` on the json codec). `Codec` comes from
`json_codec()` (`std::json`) or `binary_codec()` (`std::binary`); both ends
must use the same one. You only meet this layer when implementing a custom
transport or protocol bridge.
