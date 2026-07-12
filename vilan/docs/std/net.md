# Networking — reference

Client HTTP (`std::fetch`) and websocket framing (`std::ws`). The rpc
transports that ride these: the [rpc reference](rpc.md).

## std::fetch

A builder over the host `fetch`. `send`, `text`, and friends are async —
callers implicitly await:

```vilan,fragment
fun get(url: str): Request
fun post(url: str, body: str): Request
fun post_bytes(url: str, body: Bytes): Response   // one-shot binary POST

impl Request {
	fun header(own self, name: str, value: str): Request   // chainable
	fun send(self): Response                               // async
}

impl Response {
	fun status(self): i32
	fun text(self): str        // async
	fun bytes(self): Bytes     // async (whole body)
}
```

```vilan,norun
import std::print;
import std::fetch;

fun main() {
	let response = fetch::get("https://example.com/api/health")
		.header("Accept", "application/json")
		.send();
	print(response.status());
	print(response.text());
}
```

Streaming a body chunk by chunk: `response.body_stream().reader()` +
`read_chunk()` (`finished()`/`payload()` per chunk) — the SSE fallback
transport uses this.

Note `fetch` is base-layer: it works in the browser *and* on node. For
talking to your own vilan service, prefer the generated rpc client over
hand-rolled fetch calls.

## std::ws

Websocket **framing** — building and parsing raw frames. This is
server-side plumbing (`std::rpc_server` uses it to speak websocket on a
plain TCP socket); browser clients get sockets from the host via the rpc
transport instead.

```vilan,fragment
// Build frames
fun text_frame(text: str): Bytes
fun binary_frame(payload: Bytes): Bytes
fun pong_frame(payload: Bytes): Bytes
fun close_frame(): Bytes

// Parse a byte stream into events
enum WsEvent { Text(str), Binary(Bytes), Ping(Bytes), Close, … }
impl WsParser {
	fun new(): WsParser
	fun feed(self, chunk: Bytes): List<WsEvent>   // stateful; call per TCP chunk
}
```

`WsParser.feed` handles fragmentation and interleaved control frames; feed
it whatever the socket delivers and act on the returned events.
