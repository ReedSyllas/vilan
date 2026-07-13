# Routing

Routers you've used probably match URL pattern strings: `"/w/:id/task/:tid"`.
vilan's router doesn't. **Routes are enums.** You describe your app's pages
as an enum, write one function that parses a path into it and one that
prints it back, and the type system takes it from there. Every link targets
a page that exists. Every page receives exactly the parameters it declares.
When you add a page, the compiler points at every `match` that now needs to
handle it. A pattern-string router can't promise any of that.

`std::router` supplies the primitives: the live path signal, `navigate`,
the `link` helper, and `segments` for parsing.

## The route model

Here's a small two-level app: a home page, and workspace pages that have
their own sub-pages.

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
```

That's the whole pattern. Yes, `parse` is more code than a pattern
string. In exchange it's ordinary code: type-checked, debuggable, and
free to do things pattern strings can't (validation, aliases, redirects).
Now the pieces one at a time.

## The live path becomes a route signal

`current_path()` is a `Signal<str>` of `location.pathname`. It stays
current across `navigate` calls and the browser's back/forward buttons.
Derive your typed route from it once:

```vilan,fragment
let route = current_path().map(parse);
```

(Passing `parse` by name instead of `|p| parse(p)` is the named-function
coercion from [the tour](../tour/functions-and-closures.md).)

## Pages swap on the route

`View.swap(route, render)` is the page container. When the route
changes, it tears down the old page (disposing all its bindings) and
builds the new one. When the route *doesn't* change — say the user
clicks a link to the page they're on — nothing happens at all. That's
why route enums derive `PartialEq`.

Nesting works the way you'd hope: the workspace page can `swap` on its
own `WorkspaceRoute` while the outer swap only rebuilds when the
workspace id changes.

## Links and navigation

`link(label, route)` renders a real `<a href=…>`. Middle-click,
ctrl-click, and copy-link-address all behave like a normal link. Only a
plain left click is intercepted and turned into an in-app navigation.
And because it takes your route enum rather than a string, a dead link
is a compile error:

```vilan,fragment
link("← All workspaces", Route::Home)
link(task.name, Route::Workspace(workspace_id, WorkspaceRoute::Task(task.id)))
```

For programmatic navigation (after a sign-out, after creating a thing):

```vilan,fragment
navigate(href(Route::Home));
```

`navigate` joins the caller's current turn, so a handler's state changes
and the page change land together as one update.

## Deep links and the server

When someone loads `/w/3/task/7` fresh, the request goes to your
*server*, which has to answer with the app shell no matter the path.
That's the standard history-API fallback, and the catch-all in your http
handler does it:

```vilan,fragment
serve_service(4000, protocol, |request| {
	match request.path() {
		"/client.js" => …,
		"/client.css" => …,
		_ => …app shell html…,   // every route serves the shell
	}
})
```

On the client side, a deep-linked page usually needs data that hasn't
synced yet. Mount it under `when(present)` so it appears when the data
does — see [Services & RPC](services.md).

## Traps

- Keep `parse` and `href` next to each other and test them as a pair.
  Their agreement is the one thing the type system can't check for you.
  A `parse` that drops a segment silently turns a working deep link into
  a NotFound.
- `segments` already forgives trailing and duplicate slashes (they
  produce no segment). Don't special-case them in `parse`.
- Query strings and hash fragments aren't modelled yet (a deliberate
  deferral). `segments` sees only the pathname.
