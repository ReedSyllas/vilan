# Services & RPC

A **service** is a struct on the server whose methods clients call over a
WebSocket and whose exposed signals clients **mirror** live. You write the
struct; the `[service]` macro generates the typed client, the dispatch table,
and the sync plumbing. One definition, three attributes:

- `[service(ClientName)]` on the struct — names the generated client type.
- `[rpc]` on a method — callable remotely.
- `[expose]` on a `Signal<T>` field — mirrored to every connected client.

```vilan,norun
import std::print;
import std::reactive::Signal;
import std::json::json_codec;
import std::http::Response;
import std::rpc_server::serve_service;
import std::shared::Shared;

[derive(Wire, PartialEq, Debug)]
struct Note {
	id: i32,
	text: str,
}

[service(NotesClient)]
struct Notes {
	[expose] entries: Signal<List<Note>>,
	next_id: Shared<i32>,
}

impl Notes {
	[rpc]
	fun add(self, text: str): i32 {
		let id = self.next_id.read();
		self.next_id.write() = id + 1;
		self.entries.set_with(|list| {
			mut updated = list;
			updated.push(Note { id = id, text = text });
			updated
		});
		id
	}
}

fun main() {
	let notes = Notes {
		entries = Signal::new([]),
		next_id = Shared::new(1),
	};
	serve_service(4000, notes.dispatcher().into_protocol(json_codec()), |request| {
		Response::builder().body("app shell here").build()
	}, || print("listening on :4000"));
}
```

And the client — the generated `NotesClient::connect` returns a connected
client whose exposed fields are live local signals and whose rpc methods are
async, `Result`-returning calls:

```vilan,browser
import std::print;
import std::reactive::Signal;
import std::json::json_codec;
import std::result::Result::{ self, Ok, Err };
import std::shared::Shared;

[derive(Wire, PartialEq, Debug)]
struct Note {
	id: i32,
	text: str,
}

[service(NotesClient)]
struct Notes {
	[expose] entries: Signal<List<Note>>,
	next_id: Shared<i32>,
}

impl Notes {
	[rpc]
	fun add(self, text: str): i32 {
		let id = self.next_id.read();
		self.next_id.write() = id + 1;
		id
	}
}

async fun main() {
	match NotesClient::connect("/", json_codec()) {
		Ok(let client) => {
			// The mirror: fires on every server-side change, on every client.
			let _sync = client.entries.sub(|list: List<Note>| print(list.len()));
			// An rpc call: implicitly awaited, Result-typed.
			match client.add("hello") {
				Ok(let id) => print(id),
				Err(let error) => print(i"rpc failed: {error.debug()}"),
			}
		},
		Err(let error) => print(i"connect failed: {error.debug()}"),
	}
}
```

In a real app the service definition lives in a shared `[library]` package
(`common`) imported by both the server and the client — see the layout in
[the full-stack walkthrough] *(Phase 3)*.

## What can cross the wire: `Wire`

Every rpc parameter, return type, and exposed-signal payload must be
**Wire** — serializable. Scalars (`bool`, the sized ints including `i64`,
floats, `str`), `List`/`Option` of Wire types, and structs/enums that derive
it:

```vilan,fragment
[derive(Wire, PartialEq, Debug)]
struct Note { id: i32, text: str }
```

`derive(Wire)` requires every field to be Wire, recursively — a closure or a
`Signal` field is a compile error at the derive site. `PartialEq` is wanted
by mirrors and UI reconciliation; `Debug` by error paths — deriving all
three is the standard shape for payload types.

Codecs: `json_codec()` (`std::json`) for a readable wire, `binary_codec()`
(`std::binary`) for a compact one. Client and server must agree.

## RPC semantics

- Client rpc methods return `Result<T, RpcError>` and are implicitly awaited
  (call them from an async context).
- `RpcError` distinguishes `Transport` (couldn't reach the server),
  `Decode`, and remote failure — handle or surface, they're values.
- **Contract check**: at connect, client and server compare a hash of the
  service's shape; drift (an old client against a redeployed server) fails
  the connect rather than corrupting calls.
- Handlers run inside a **turn** (`AtEnd`): all signal writes an rpc handler
  makes settle as one wave — mirrors broadcast a consistent snapshot.

## Mirrors

An `[expose] field: Signal<List<T>>` on the service becomes a live
`Signal<List<T>>` on every connected client. The server writes it like any
signal (from rpc handlers, timers, anywhere); every client's copy updates.

Patterns that follow:

- **Derive client views locally.** One `tasks` mirror; each page maps it
  (`tasks.map(|list| …filter…)`) — don't add per-view rpcs.
- **Mutate via rpc, observe via mirror.** A create/update/delete rpc writes
  the server signal; the confirmation you see is your own change arriving
  through the mirror (the echo).
- **Local-first editing**: bind inputs to [drafts](reactive.md) whose commit
  is the rpc; `adopt` mirror updates into them (see the reactive guide's
  draft rules — echoes are no-ops, dirty fields win).

## Connection state and reconnection

The transport survives drops: on disconnect it reconnects with backoff
(250 ms doubling, 4 s cap, 10 attempts), re-verifies the contract, and
re-attaches every mirror — state resyncs by itself.

What your code sees:

- `client.transport.connection_state(): Signal<ConnectionState>` —
  `Connected` / `Reconnecting` / `Closed`. Bind a banner to it:

```vilan,fragment
let state = client.transport.connection_state();
view("p").text("reconnecting…")
	.show(state.map(|current| current == ConnectionState::Reconnecting))
```

- Calls **in flight** when the connection drops reject with a transport
  error ("connection lost"); calls made **while down** fail fast ("not
  connected"). Nothing is silently retried — an rpc might not be idempotent,
  so retrying is the app's decision (a draft's next push, a user's retry).
- Mirrors need nothing: they rebind on reconnect and deliver the fresh
  state.

## Authentication

The standard shape (the kolt pilot): an `[rpc] login(…): AuthOutcome` issues
a token; subsequent rpcs take the token as their first parameter and the
server validates per call. Session identity via `std::context` (an ambient
value the dispatch layer establishes per connection) is the recorded
refinement for when token-per-call gets noisy.

## The server side

```vilan,fragment
serve_service(
	port,
	service.dispatcher().into_protocol(json_codec()),
	|request| …,       // http fallback: serve assets + the app shell (history-API fallback)
	|| …,              // on_ready
)
```

`dispatcher()` is generated by `[service]`. The fallback handles every plain
http request — serve the client bundle and return the app shell for unknown
paths so deep links load (see [Routing](routing.md)). For custom
per-connection state (an app-written attach, connection-scoped auth), drop
down to `serve_connected` — see the [rpc reference](../std/rpc.md).

## Traps

- Changing a service's shape while an old server process is still holding
  the port looks like mysterious contract-mismatch failures — check for a
  leaked server first (`ss -tlnp | grep <port>`).
- The wire is value-semantic: a mirrored list is a *copy* per update; mutate
  through rpcs, never by writing the client's mirror signal.
- rpc handlers are synchronous with respect to the dispatch (the reply is
  the return value) — long work belongs in spawned tasks that write signals
  when done.
