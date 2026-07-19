# Process modules — reference

The process layer (node/deno/bun builds): `std::db`, `std::http`,
`std::fs`, `std::process`, `std::rpc_server`. Task-oriented usage:
[Persistence and the server](../guide/persistence.md).

## std::db — SQLite

```vilan,fragment
resource external struct Database;       // a resource: moves, closes on drop

impl Database {
	fun open(path: str): Database        // ":memory:" for an in-memory db
	fun exec(self, sql: str)             // DDL / one-off statements
	fun prepare(self, sql: str): Statement
}
impl Statement {
	fun run(self, parameters: List<any>): i32          // → last insert id
	fun all(self, parameters: List<any>): List<Row>
	fun first(self, parameters: List<any>): Option<Row>
}
impl Row {
	fun text(self, name: str): str
	fun integer(self, name: str): i32
	fun big_integer(self, name: str): i53   // i53-wide INTEGER (epoch millis)
	fun real(self, name: str): f64
	fun is_null(self, name: str): bool
}
```

Parameters are `?` placeholders. Synchronous by design (fits the rpc
dispatch path). `desc` and other SQL keywords fail as column names.

`Database` is a **`resource`**: it has a single owner and *moves* rather than
copies, and it closes its `node:sqlite` handle when its owner's scope ends — a
`let db =
Database::open(..)` local closes on the function's return, with no `close()`
method to remember. `drop(db)` closes it early (the move spends the binding).
A **module-level** `Database` is the serve-forever idiom: it has process
lifetime, never drops, and is reachable only by loan (method calls,
`&`-passing) — moving or `drop`ing a module-level database is a compile error.
Being a resource, a `Database` cannot go into a `List` (use `Option` or a
struct field), cross the wire (`[derive(Wire)]` rejects it), or be a field of a
`[service]` struct (the generated dispatcher would capture the store — keep the
database at module scope instead, next to the service).

## std::http — the server

```vilan,fragment
impl Server { fun builder(): ServerBuilder }
impl ServerBuilder {
	fun port(own self, port: i32): ServerBuilder
	fun on_request(own self, handler: async |Request| Response): ServerBuilder
	fun on_upgrade(own self, handler: |NodeRequest, NodeSocket, Bytes| void): ServerBuilder
	fun on_start(own self, callback: |Server| void): ServerBuilder
	fun on_stop(own self, callback: |Server| void): ServerBuilder
	fun build(self): Server
}
impl Server {
	fun start(self)        // begin listening; holds the event loop
	fun url(self): str
}

impl Request {
	fun path(self): str
	fun method(self): str
	fun body(self): str      // the body as text
	fun bytes(self): Bytes   // the same body raw (binary POSTs)
}
impl Response {
	fun builder(): ResponseBuilder
}
impl ResponseBuilder {
	fun code(own self, code: i32): ResponseBuilder          // default 200
	fun set_header(own self, name: str, value: str): ResponseBuilder   // repeatable
	fun body(own self, body: str): ResponseBuilder
	fun body_bytes(own self, body: Bytes): ResponseBuilder  // binary body
	fun streaming(own self, on_open: |ResponseStream| void): ResponseBuilder
	fun build(self): Response
}
impl ResponseStream {
	fun send(self, chunk: str)          // write without ending
	fun close(self)                     // end the response
	fun on_close(self, handler: || void)   // the client went away
}
```

A **streaming** response holds the connection open: once the status and
headers are written, `on_open` receives the live `ResponseStream` and
writes chunks over time (SSE's shape — a suspending `on_open` runs as
spawned work). `on_upgrade` mounts a WebSocket-style handshake handler
over the raw bindings (`NodeRequest`/`NodeSocket`). For an rpc-serving
app you won't touch any of this directly — `serve_service` wraps it
(below), and `serve_connected` itself now rides this surface.

## std::rpc_server

```vilan,fragment
fun serve_service(
	port: i32,
	protocol: RpcProtocol,             // service.dispatcher().into_protocol(codec)
	fallback: |Request| Response,      // plain-http requests
	on_ready: || void,
)

fun serve_connected(port, protocol, on_connection, fallback, on_ready)
	// the same server with the per-connection hook exposed (custom attach/auth)
```

Websocket upgrade + session registry (mirror attach/detach) + rpc dispatch;
each handler runs in a turn (`AtEnd`). Details and the client side:
[Services & RPC](../guide/services.md) and the [rpc reference](rpc.md).

## std::fs

```vilan,fragment
fun exists(path: str): bool                // sync
fun read_file_to_str(path: str): str       // async, UTF-8
fun write_file(path: str, contents: str)   // async
```

## std::process

```vilan,fragment
fun args(): List<str>            // CLI arguments
fun env(key: str): Option<str>   // environment variable
fun exit(code: i32)
fun scan(): str                  // read a line from stdin
```

A completed `main` ends the process — long-lived programs must hold it open
(a listening server does; a socket-holding client needs an explicit wait).
