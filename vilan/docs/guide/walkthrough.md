# A full-stack walkthrough

Every guide so far taught one layer. This chapter builds a whole app, so
you can see the layers meet: **Notes** — sign in, a note list that syncs
live between browser windows, and an editor that saves as you type.

The finished app lives in the repo at
[`vilan/examples/walkthrough/`](../../examples/walkthrough/), about 500
lines across three packages. Every snippet below is quoted from those
files, and the test suite builds the app on every run, so this chapter
can't quietly rot. To run it:

```sh
cd vilan/examples/walkthrough
vilan build .
node dist/server.js     # → http://localhost:4600
```

Open two browser windows side by side. Sign in, add a note in one window,
and watch it appear in the other. Open a note and type — the other window
follows keystroke by keystroke.

## The shape

```
walkthrough/
  vilan.toml            [project] packages = ["common", "client", "server"]
  common/               [library] — the service + the types that cross the wire
  client/               [package] target = "browser"
  server/               [package] (node)
```

One workspace, three packages ([Hello vilan](../tour/hello-vilan.md)
introduced this layout). `common` is imported by both sides, so it may
only use platform-neutral std. The compiler enforces that.

The data flows in one loop, and the whole app hangs off it:

```
you type → rpc → server writes SQL → server writes its signal
        → the mirror updates every client → the UI re-renders one binding
```

Your own edit comes back to you the same way everyone else's does. There
is no "local state vs server state" bookkeeping — the mirror *is* the
state, and drafts smooth over the last inch (the input you're typing in).

## `common`: the contract

The shared package declares the payload type and the service. This is
the whole client/server contract — there is no schema file, no endpoint
list, no client SDK to regenerate:

```vilan,fragment
[derive(Wire, PartialEq, Debug)]
struct Note {
	id: i32,
	title: str,
	body: str,
}

[service(NotesClient)]
struct NotesStore {
	[expose] notes: Signal<List<Note>>,
	sign_in_hook: Shared<|str, str| AuthOutcome>,
	add_hook: Shared<|str, str| i32>,
	retitle_hook: Shared<|str, i32, str| i32>,
	rewrite_hook: Shared<|str, i32, str| i32>,
	delete_hook: Shared<|str, i32| i32>,
}
```

Two design choices worth pausing on:

- **The hooks pattern.** The rpc methods just call closures the server
  installs at boot. That keeps `common` free of SQL and platform code —
  it describes the surface, and the server supplies the behavior.
- **Title and body commit separately** (`retitle_note` and
  `rewrite_note`). The editor uses one draft per field, and per-field
  rpcs mean one field's edit never re-sends the other's text.

Each `[rpc]` method is small and uniform:

```vilan,fragment
[rpc]
fun retitle_note(self, token: str, note_id: i32, title: str): i32 {
	let hook = self.retitle_hook.read();
	hook(token, note_id, title)
}
```

(The bind-then-call shape is a current parser limitation — see
[gotchas](../appendix/gotchas.md).)

## The server: SQL first, then the signal

The server ([`server/src/main.vl`](../../examples/walkthrough/server/src/main.vl))
opens SQLite, loads the mirror once, and wires each hook. Every write
hook has the same rhythm — check the session, write SQL, then update the
signal:

```vilan,fragment
let retitle = |token: str, note_id: i32, title: str| {
	match session_user(db, token) {
		Some(let _user) => {
			db.prepare("UPDATE note SET title = ? WHERE id = ?").run([title, note_id]);
			notes.set_with(|list| list.map(|note| {
				if note.id == note_id {
					Note { id = note.id, title = title, body = note.body }
				} else {
					note
				}
			}));
			note_id
		},
		None => 0 - 1,
	}
};
```

The order matters: persist first, then update the signal. The signal
write is what broadcasts to every client, so a crash between the two can
never announce state that was never stored
([Persistence](persistence.md) covers this).

Auth is register-or-login in one rpc: an unknown username creates the
account (pbkdf2-hashed password), a known one checks it, and either path
opens a session row whose token identifies later calls
([Services & RPC](services.md#authentication)).

Boot is five lines at the end: read the client bundle and shell from
disk, then

```vilan,fragment
serve_service(4600, store.dispatcher().into_protocol(json_codec()), |request| {
	match request.path() {
		"/client.js" => Response::builder().set_header("Content-Type", "text/javascript").body(client_js).build(),
		"/client.css" => Response::builder().set_header("Content-Type", "text/css").body(client_css).build(),
		_ => Response::builder().set_header("Content-Type", "text/html").body(app_html).build(),
	}
}, || print("notes server listening on http://localhost:4600"));
```

The catch-all serves the shell for every unknown path. That's what makes
deep links like `/note/7` load ([Routing](routing.md#deep-links-and-the-server)).

## The client entry: four signals and a mount

[`client/src/main.vl`](../../examples/walkthrough/client/src/main.vl) is
the whole wiring diagram:

```vilan,fragment
async fun main() {
	let notes: Signal<List<Note>> = Signal::new([]);
	let token = Signal::new(storage::get("notes-token"));
	let route = current_path().map(parse);

	match NotesClient::connect("/", json_codec()) {
		Ok(let client) => {
			let _sync = client.notes.sub(|list| notes.set(list));
			let _root = mount_root("app", || screen(client, notes, token, route));
		},
		Err(let error) => print(i"connect failed: {error.debug()}"),
	}
}
```

Read it as: mirror in, token from `localStorage` (a reload stays signed
in), the typed route derived from the URL, connect, mount. Everything
after this line is just views reading those signals.

## Routes

[`client/src/routes.vl`](../../examples/walkthrough/client/src/routes.vl)
is the enum-router pattern from [Routing](routing.md), at its smallest:

```vilan,fragment
[derive(PartialEq)]
enum Route {
	Home,
	Note(i32),
	NotFound,
}
```

plus `parse` and `href` as the inverse pair, and pages that `swap` on
`route`.

## The views

[`client/src/views.vl`](../../examples/walkthrough/client/src/views.vl)
has three layers, each one guide's idea:

**The gate.** The sign-in panel `show`s while the token is empty; the
routed app `show`s once it isn't. Signing in stores the token; signing
out removes it and navigates home.

**The list page.** An add form bound to a local signal, and the list
itself — one keyed `bind_each` over the mirror:

```vilan,fragment
.child(view("ul").bind_each(notes, |note| note.id, |note| note_row(client, note, token)))
```

That single line is the live sync. When any client adds or deletes a
note, the mirror updates and the keyed rows reconcile
([Building UI](ui.md#lists-bind_each)).

**The editor.** The note page finds its note in the mirror, waits for it
under `when(present)` (so a deep link shows "loading…" until the first
sync), and then the editor binds one draft per field:

```vilan,fragment
let title = draft(seed_title, |value: str| commit_outcome(client.retitle_note(token.get(), note_id, value), note_id));
let body = draft(seed_body, |value: str| commit_outcome(client.rewrite_note(token.get(), note_id, value), note_id));

// Remote edits (another session's typing — or our own echo) fold in.
entry.effect(|current: Option<Note>| {
	match current {
		Some(let note) => {
			title.adopt(note.title);
			body.adopt(note.body);
		},
		None => {},
	}
});

view("div")
	.child(view("input").styled(field).attr("placeholder", "Title…").bind_draft(title))
	.child(view("span").styled(muted).bind_text(title.state.map(state_text)))
	…
```

This is the local-first loop from [Reactive state](reactive.md) closed
end to end: typing updates the input instantly, each keystroke commits
through its rpc, the server broadcasts, and the `adopt` in the effect
folds remote changes in. Your own echo changes nothing. Another
session's edit updates your field — unless you're mid-edit, in which
case your text wins until it commits. There is no Save button because
there is nothing left for one to do.

## Things to try

- **Two windows.** Type a title in one and watch the other follow. Then
  type in *both* fields at once, one per window.
- **Kill the server** (Ctrl-C) with the app open. The "reconnecting…"
  banner appears (one `show` on the transport's state signal). Restart
  the server: the banner clears and the mirror resyncs by itself.
- **Restart the server** and reload: the notes are still there. SQLite
  did that, not the mirror.
- **Deep-link** to a note (`/note/1`) in a fresh window: "loading…"
  flashes until the first sync, then the editor seeds.

## Where each idea came from

| In this app | Taught in |
|---|---|
| the workspace, `vilan.toml` | [Hello vilan](../tour/hello-vilan.md) |
| `Note`, derives, the enums | [Data & traits](../tour/data-and-traits.md) |
| signals, effects, drafts | [Reactive state](reactive.md) |
| views, `bind_each`, `when`, `show` | [Building UI](ui.md) |
| the `const` styles | [Styling](styling.md) |
| the route enum, `swap`, `link` | [Routing](routing.md) |
| `[service]`, mirrors, reconnect | [Services & RPC](services.md) |
| SQLite, the fallback, boot order | [Persistence](persistence.md) |

From here, the honest next step is to change something: add a
`created_at: Instant` to `Note` (the compiler will walk you through
every place it matters), or add a second entity. The shape you'd follow
is exactly the one above.
