# Services & RPC

This is the chapter where vilan's full-stack story comes together. The
short version: you write one ordinary struct on the server, mark a few
things on it, and you get a typed client, live data sync, and reconnect
handling without writing any protocol code.

A **service** is that struct. Three attributes do the work:

- `[service(ClientName)]` on the struct names the generated client type.
- `[rpc]` on a method makes it callable from the client.
- `[expose]` on a `Signal<T>` field **mirrors** it: every connected
  client gets a live copy that updates when the server writes it.

No REST endpoints, no fetch calls, no JSON shapes to keep in sync by
hand. The compiler knows both sides.

Here's a complete little server:

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

And a client. `NotesClient::connect` gives you an object whose exposed
fields are live local signals and whose rpc methods are ordinary calls
that return `Result`:

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

In a real app the service definition lives in a shared `[library]`
package (usually called `common`) that both the client and server
import. [Hello vilan](../tour/hello-vilan.md) shows that workspace
layout.

## What can cross the wire: `Wire`

Everything that travels — rpc parameters, return types, mirrored
payloads — must be serializable, which vilan calls **Wire**. The scalars
are Wire (`bool`, the integers including `i53`, floats, `str`). `List`
and `Option` of Wire types are Wire. And your own types opt in with a
derive:

```vilan,fragment
[derive(Wire, PartialEq, Debug)]
struct Note { id: i32, text: str }
```

That triple is the standard shape for payload types: `Wire` to travel,
`PartialEq` because mirrors and UI reconciliation compare values, and
`Debug` for error paths.

`derive(Wire)` checks every field recursively. A closure or a `Signal`
hiding inside a payload type is a compile error at the derive, which is
exactly where you want to find out.

The codec is chosen at connect time: `json_codec()` for a readable wire,
`binary_codec()` for a compact one. Client and server must use the same
one.

## What rpc calls do

- On the client they return `Result<T, RpcError>` and are implicitly
  awaited, like any async call.
- `RpcError` tells you what went wrong: `Transport` (couldn't reach the
  server), `Decode`, or `Remote` (the handler failed). Errors are
  values. Look at them and decide.
- At connect time, both sides compare a hash of the service's shape. If
  a stale client meets a redeployed server, the connect fails cleanly
  instead of calls corrupting halfway. This is the **contract check**.
- On the server, each handler runs inside a turn, so all the signal
  writes one rpc makes are broadcast as a single consistent update.
- Handler bodies can await — call another service, `sleep_for`, wait on
  I/O. The reply is sent when the body finishes, and the turn holds
  across the awaits: writes before and after a suspension still
  coalesce into that same single update.

## Mirrors

The `[expose]` mirror is the piece that replaces most "fetch on mount,
refetch on focus, invalidate on mutation" client code. The server writes
its signal whenever and however it likes. Every connected client's copy
updates. That's it.

Three patterns follow from it:

- **Derive views locally.** Expose one `tasks` list and let each page
  `map` it down (filter by workspace, sort by date). Don't add an rpc
  per view.
- **Mutate via rpc, observe via mirror.** Your create/delete handlers
  write the server signal. The confirmation the user sees is their own
  change arriving back through the mirror.
- **Edit through drafts.** Bind text inputs to
  [drafts](reactive.md#optimistic-writes-and-local-first-drafts) whose
  commit is the rpc, and `adopt` mirror updates into them. Typing stays
  instant, remote edits fold in, and your own echoes are no-ops.

## Connection state and reconnection

Connections drop. The transport handles it: it reconnects with backoff,
re-verifies the contract, and re-attaches every mirror, so state resyncs
on its own. Your code sees two things.

First, a signal you can bind a banner to:

```vilan,fragment
let state = client.transport.connection_state();
view("p").text("reconnecting…")
	.show(state.map(|current| current == ConnectionState::Reconnecting))
```

Second, honest call failures. A call in flight when the connection drops
rejects with "connection lost". A call made while down fails immediately
with "not connected". Nothing is silently retried, because an rpc might
not be safe to repeat. Retrying is the app's decision — a draft's next
push, or the user pressing the button again.

> **Going deeper.** The backoff dials at 250 ms doubling to a 4 s cap,
> ten attempts before giving up (`Closed`). Mirrors rebind by
> re-running the contract check and re-attaching each subscription; you
> never re-subscribe manually. The full state machine is in the
> [rpc reference](../std/rpc.md).

## Authentication

The straightforward shape, proven in the kolt pilot: a `login` rpc
returns a token, later rpcs take the token as their first parameter, and
the server validates it per call.

```vilan,fragment
[rpc]
fun login(self, username: str, password: str): AuthOutcome { … }

[rpc]
fun create_task(self, token: str, workspace_id: i32, name: str): i32 { … }
```

When token-per-call gets noisy, the recorded refinement is
connection-scoped identity via `std::context`. It isn't built into the
generated dispatch yet.

## Where the service lives

The service lives **next to the resources its methods use** — a
database handle, the filesystem, other services. In a single-package
app (one `[package]` with an `[entry.client]` and an `[entry.server]`
— see [Platforms](../tour/platforms.md)), that's just a module both
entries can see:

```vilan,fragment
// src/store.vl — bodies use server std directly
[service(TodoClient)]
struct TodoStore { … }

// src/client.vl
import pkg::store::TodoClient;
```

In a multi-package workspace the same idea reads: the service sits in
the server package, and the client package depends on it, importing
just the generated client (`import server::store::TodoClient;`).

Either way, the browser build takes only the stub and the contract hash
from that module; the method bodies and the dispatcher are
server-colored and out of its reach. A shared `common` library is still
a fine home for the payload types both sides speak — it's just no
longer the only legal home for anything.

## The server side

```vilan,fragment
serve_service(
	port,
	service.dispatcher().into_protocol(json_codec()),
	|request| …,       // http fallback: serve assets + the app shell
	|| …,              // on_ready
)
```

`dispatcher()` is generated by `[service]`. The fallback answers every
plain http request — serve `client.js` and `client.css`, and return the
app shell for anything else so deep links work (see
[Routing](routing.md)). For custom per-connection state, drop down to
`serve_connected` — see the [rpc reference](../std/rpc.md).

## Traps

- Mysterious contract-mismatch failures while developing usually mean an
  *old server process* is still holding the port. Check with
  `ss -tlnp | grep <port>` and kill it by pid.
- The wire is value-semantic. A mirrored list is a fresh copy per
  update. Mutate through rpcs, never by writing the client's mirror
  signal.
- An rpc handler's reply is its return value, so the handler runs to
  completion before the client hears back. Long work belongs in spawned
  tasks that write signals when done.
