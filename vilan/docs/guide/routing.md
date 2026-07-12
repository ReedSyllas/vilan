# Routing

vilan's router has no pattern-string DSL: **routes are enums**, and you write
`parse`/`href` as an ordinary inverse pair of functions. The type system then
guarantees what a DSL can't — every `link` targets a route that exists, every
page receives exactly its parameters, and adding a variant makes the compiler
point at every `match` that must handle it.

`std::router` supplies the primitives: the live path signal, navigation, the
`<a>`-rendering `link`, and `segments` for parsing.

## The route model

One enum per layout level; parameters are payloads; child layouts nest:

```vilan,browser
import std::ui::{ view, View, mount_root };
import std::router::{ current_path, navigate, segments, link, Routable };
import std::reactive::Signal;
import std::option::Option::{ self, Some, None };

[derive(PartialEq)]
enum Route {
	Home,
	Workspace(i32, WorkspaceRoute),
	NotFound,
}

[derive(PartialEq)]
enum WorkspaceRoute {
	Overview,
	Task(i32),
}

// The inverse pair. `parse` consumes `segments(path)`; `href` prints the
// same shape back. Keep them adjacent — they must agree.
fun parse(path: str): Route {
	let parts = segments(path);
	if parts.len() == 0 {
		ret Route::Home;
	}
	if parts[0] == "w" && parts.len() >= 2 {
		match parts[1].parse_i32() {
			Some(let id) => {
				if parts.len() == 2 {
					ret Route::Workspace(id, WorkspaceRoute::Overview);
				}
				if parts.len() == 4 && parts[2] == "task" {
					match parts[3].parse_i32() {
						Some(let task) => ret Route::Workspace(id, WorkspaceRoute::Task(task)),
						None => {},
					}
				}
			},
			None => {},
		}
	}
	Route::NotFound
}

fun href(route: Route): str {
	match route {
		Route::Home => "/",
		Route::Workspace(let id, let inner) => match inner {
			WorkspaceRoute::Overview => i"/w/{id}",
			WorkspaceRoute::Task(let task) => i"/w/{id}/task/{task}",
		},
		Route::NotFound => "/",
	}
}

impl Route with Routable {
	fun to_path(self): str {
		href(self)
	}
}

fun main() {
	let route = current_path().map(parse);
	let _root = mount_root("app", || {
		view("div").swap(route, |current| match current {
			Route::Home => view("h1").text("Home"),
			Route::Workspace(let id, let _inner) => view("h1").text(i"Workspace {id}"),
			Route::NotFound => view("h1").text("Nothing here"),
		})
	});
}
main();
```

That's the entire pattern. Piece by piece:

## The live path → the route signal

`current_path()` is a singleton `Signal<str>` of `location.pathname`, kept
live across `navigate` and the browser's back/forward. Derive your typed
route from it once:

```vilan,fragment
let route = current_path().map(parse);
```

(`map(parse)` — a named function coerces to the closure argument; see the
[tour](../tour/functions-and-closures.md).)

## Pages swap on the route

`View.swap(route, render)` disposes the previous page's subtree and builds
the new one whenever the route **changes** — an equal route (navigating to
where you already are) re-renders nothing, which is why route enums derive
`PartialEq`. Nest the same pattern for child layouts: the workspace page can
`swap` on its own `WorkspaceRoute` while the outer swap only rebuilds when
the workspace id changes.

## Links and navigation

`link(label, route)` renders a real `<a href=…>` — middle-click, ctrl-click,
and copy-link-address all behave natively — and intercepts only a plain
left click (`prevent_default` + `navigate`). It takes any `Routable`, so a
link can only ever point at a value of your route enum:

```vilan,fragment
link("← All workspaces", Route::Home)
link(task.name, Route::Workspace(workspace_id, WorkspaceRoute::Task(task.id)))
```

Programmatic navigation (after a sign-out, a successful create):

```vilan,fragment
navigate(href(Route::Home));
```

`navigate` joins the caller's ambient turn, so a handler's writes and the
route change settle as one wave.

## Deep links and the server

A full-page load of `/w/3/task/7` asks the *server* for that path. Serve the
app shell for every non-asset path (the history-API fallback) — a catch-all
route in the http server does it:

```vilan,fragment
serve_service(4000, protocol, |request| {
	match request.path() {
		"/client.js" => …,
		"/client.css" => …,
		_ => …app shell html…,   // every route serves the shell
	}
})
```

On the client, deep-linked pages that need synced data mount under
`when(present)` so they wait for the first mirror sync — see
[Services & RPC](services.md).

## Traps

- Keep `parse` and `href` adjacent and test them as a pair — they are the
  one place the type system can't check the correspondence (a `parse` that
  drops a segment silently 404s the deep link).
- `segments` forgives trailing/duplicate slashes (they produce no segment) —
  don't special-case them in `parse`.
- Query strings and hash fragments are not modelled yet (deliberately
  deferred); `segments` sees only the pathname.
