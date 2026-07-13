# Persistence and the server

This chapter covers the server half of a full-stack app: SQLite via
`std::db`, http serving via `std::http`, files via `std::fs`, and the
process itself via `std::process`. These modules live in the process
layer, so they're available in node/deno/bun builds. The rpc layer that
sits on top is [Services & RPC](services.md).

## SQLite: `std::db`

vilan ships with an embedded SQLite binding (node's built-in SQLite
underneath). There is no ORM and no query builder. You write SQL, with
`?` placeholders for values:

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

The whole surface fits in a few lines:

- `Database::open(path)` opens or creates the file. `":memory:"` gives
  you a throwaway in-memory database, handy in tests.
- `db.exec(sql)` runs DDL and one-off statements.
- `db.prepare(sql)` gives a `Statement`. Then `.run(params)` executes
  and returns the last insert id, `.first(params)` fetches an
  `Option<Row>`, and `.all(params)` fetches a `List<Row>`.
- Rows read by column name: `text`, `integer` (i32), `big_integer`
  (i53 — use it for epoch-millis timestamps, which outgrow i32),
  `real` (f64), and `is_null`.

Two habits to keep:

- Values always go through `?` placeholders. Never interpolate them into
  the SQL string.
- Don't name a column with an SQL keyword. `desc` is the one that bites
  in practice — spell it `description`.

The API is synchronous, which fits rpc handlers (the dispatch path is
synchronous too), and there is no connection pool to manage.

## Serving http: `std::http`

For an rpc app, `serve_service` (from the [services guide](services.md))
owns the port, and you only supply the http **fallback** for plain
requests. The plain server underneath is usable on its own too:

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

`Request` gives you `path()`, `method()`, and `body()`. Responses come
from a builder: `.code(i32)` (200 by default),
`.set_header(name, value)`, `.body(str)`, `.build()`.

Here is the standard full-stack fallback. It serves the client bundle
and answers *every other path* with the HTML shell, so deep links load
(see [Routing](routing.md)):

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

The typical server reads the client bundle and shell into memory once at
boot, then serves from the strings.

## The process: `std::process`

```vilan,fragment
fun args(): List<str>          // CLI arguments (vilan run app.vl -- …)
fun env(key: str): Option<str> // an environment variable
fun exit(code: i32)            // end the process
fun scan(): str                // a line from stdin
```

One behavior to plan around: **the process exits when `main` finishes.**
A server stays alive because `start()` holds the event loop open. A
long-lived *client* process (a probe holding a socket, say) has to keep
`main` open itself — await something that ends with the app, or
`sleep_for` a long duration.

## Putting it together

The boot sequence of a kolt-shaped server, in order:

1. `Database::open`, then `exec` the schema
   (`CREATE TABLE IF NOT EXISTS …`).
2. Load the mirrored state from SQLite into the service's signals.
3. Wire the service's handlers to statements. Write SQL first, then
   update the signal — the mirror broadcasts the signal.
4. `fs::read_file_to_str` the client bundle and shell.
5. `serve_service(port, protocol, fallback, on_ready)`.

The ordering inside step 3 matters. Persist first, then update the
signal. That way a crash between the two can never broadcast state that
was never stored.
