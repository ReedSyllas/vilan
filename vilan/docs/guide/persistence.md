# Persistence and the server

The process layer (node/deno/bun builds): SQLite via `std::db`, http serving
via `std::http`, files via `std::fs`, and the process itself via
`std::process`. This chapter is the server half of a full-stack app — the
rpc layer on top of it is [Services & RPC](services.md).

## SQLite: `std::db`

A synchronous embedded database (node's built-in SQLite). Open, `exec` DDL,
`prepare` + run statements with `?` placeholders:

```vilan,norun
import std::print;
import std::db::{ Database, Statement, Row };
import std::option::Option::{ self, Some, None };

fun main() {
	let db = Database::open("app.db");
	db.exec("CREATE TABLE IF NOT EXISTS task (
		id INTEGER PRIMARY KEY AUTOINCREMENT,
		name TEXT NOT NULL,
		created_at INTEGER NOT NULL
	)");

	let id = db.prepare("INSERT INTO task (name, created_at) VALUES (?, ?)")
		.run(["write docs", 1720656000000i53]);
	print(id);

	match db.prepare("SELECT * FROM task WHERE id = ?").first([id]) {
		Some(let row) => print(row.text("name")),
		None => print("missing"),
	}

	for row in db.prepare("SELECT * FROM task").all([]) {
		let row_id = row.integer("id");
		let name = row.text("name");
		print(i"{row_id}: {name}");
	}
}
```

The surface:

- `Database::open(path)`, `db.exec(sql)` (DDL / one-off statements),
  `db.prepare(sql): Statement`.
- `statement.run(params): i32` — executes; returns the last insert id (for
  an INSERT) / change info.
- `statement.all(params): List<Row>`, `statement.first(params): Option<Row>`.
- Row accessors by column name: `text`, `integer` (i32), `big_integer`
  (i53 — use for epoch-millis timestamps; they outgrow i32), `real` (f64),
  `is_null`.
- `:memory:` as the path gives an in-memory database — handy in tests.

Patterns and traps:

- Parameters are always `?` placeholders — never interpolate values into
  SQL text.
- `desc` (and any SQL keyword) fails as a column name — spell it out
  (`description`).
- The API is synchronous, which fits rpc handlers (the dispatch path is
  sync); there is no connection pool to manage.

## Serving http: `std::http`

`serve_service` (from the [services guide](services.md)) is the usual entry
point — it owns the port and takes an http **fallback** for plain requests.
Underneath sits a plain builder you can use directly for an rpc-less
server:

```vilan,norun
import std::print;
import std::http::{ Server, Request, Response };

fun main() {
	Server::builder()
		.port(8080)
		.on_request(|request| {
			match request.path() {
				"/health" => Response::builder().body("ok").build(),
				_ => Response::builder()
					.code(404)
					.set_header("Content-Type", "text/plain")
					.body("not found")
					.build(),
			}
		})
		.on_start(|server| print(i"listening at {server.url()}"))
		.build()
		.start();
}
```

- `Request`: `path()`, `method()`, `body()`.
- `Response::builder()`: `.code(i32)` (default 200), `.set_header(name,
  value)`, `.body(str)`, `.build()`.

The standard full-stack fallback serves the client bundle and answers every
other path with the HTML shell (the history-API fallback for deep links):

```vilan,fragment
|request| match request.path() {
	"/client.js" => Response::builder().set_header("Content-Type", "text/javascript").body(client_js).build(),
	"/client.css" => Response::builder().set_header("Content-Type", "text/css").body(client_css).build(),
	_ => Response::builder().set_header("Content-Type", "text/html").body(app_html).build(),
}
```

## Files: `std::fs`

```vilan,fragment
fun exists(path: str): bool               // sync — boot code can branch on it
fun read_file_to_str(path: str): str      // async (implicitly awaited), UTF-8
fun write_file(path: str, contents: str)  // async
```

The typical server boot: read the client bundle and shell into memory once,
then serve from the strings (the fallback example above).

## The process: `std::process`

```vilan,fragment
fun args(): List<str>          // CLI arguments (vilan run app.vl -- …)
fun env(key: str): Option<str> // an environment variable
fun exit(code: i32)            // end the process
fun scan(): str                // a line from stdin
```

One behavior to plan around: **a completed `main` ends the process.** A
server stays alive because `start()` holds the event loop; a long-lived
*client* process (a probe holding a socket) must keep `main` open —
`sleep_for` a long duration or await something that ends with the app.

## Putting it together

The kolt-shaped server boot sequence:

1. `Database::open` + `exec` the schema (`CREATE TABLE IF NOT EXISTS …`).
2. Load mirrored state from SQLite into the service's signals.
3. Wire the service's hooks to statements (write SQL, then update the
   signal — the mirror broadcasts).
4. `fs::read_file_to_str` the client bundle + shell.
5. `serve_service(port, protocol, fallback, on_ready)`.

Step 3's ordering matters: persist first, then update the signal, so a
crash between the two can't broadcast state that was never stored.
