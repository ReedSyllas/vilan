# `std::router` — history-API routing (backlog A10)

Status: **SETTLED 2026-07-11** (design reviewed in conversation; the decision
points below were resolved explicitly). Driven by the Kolt migration
(`kolt-migration.md` §2.3): nested layouts (`layout_main` / `layout_workspace`
are the target shapes), the `/w/ORG/WS/*` path scheme, a link component, and a
current-route signal composing with `show` / `bind_*`.

## 1. The model: routes are an enum, not pattern strings

Conventional routers (`@solidjs/router`, react-router) are built around
pattern strings — `<Route path="/w/:org/:ws">` — with params read back as
strings by name at runtime. That is the stringly-typed shape vilan exists to
avoid: a typo'd param name is a runtime `undefined`, a dead link is a 404
discovered by clicking, and the route tree is a registration DSL with its own
matching semantics.

In vilan, **the URL is just a wire format for a typed value**. An app declares
its route space as an enum (nested enums mirror nested layouts), and writes an
inverse pair of ordinary functions:

```vilan
[derive(PartialEq)]
enum Route {
    Home,
    Login,
    Workspace(str, WorkspaceRoute),   // /w/{org}/{...}
    NotFound,
}

[derive(PartialEq)]
enum WorkspaceRoute {
    Overview,          // /w/acme
    Tasks,             // /w/acme/tasks
    Task(i32),         // /w/acme/task/42
}

fun parse(path: str): Route { .. }    // segments() + match — total, returns NotFound
fun href(route: Route): str { .. }    // the inverse; links take Route VALUES
```

Everything downstream is language, not framework:

- **Nested layouts are nested functions.** `app(route)` matches the outer
  enum and hands `Workspace`'s payload to `workspace_layout(org, inner)`,
  which matches the inner enum. No route-tree registration, no outlet
  indirection, no context lookup for params — the payload IS the param,
  already typed.
- **Guards are `if`s.** An auth gate is a branch in the match, not a
  framework hook.
- **Dead links don't compile.** `link` takes a route value; the printed path
  goes through `href`, so every link is derivable from the enum.

The `parse`/`href` pair is deliberately hand-written in v1: it is small
(one arm per route), totally testable, and keeps the router out of the
business of pattern semantics. A `[derive(Route)]` generating the pair from
path attributes is recorded below as later sugar, adopted only if real apps
show the boilerplate is worth killing.

## 2. The std surface

Deliberately thin — three pieces.

### 2.1 `std::router` (browser layer)

```vilan
fun current_path(): Signal<str>    // location.pathname, live across pushState + popstate
fun navigate(path: str)            // history.pushState + signal update
fun segments(path: str): List<str> // "/w/acme/tasks" → ["w", "acme", "tasks"]

trait Routable {
    fun to_path(self): str         // implemented by the app's route enum (usually = href)
}

fun link<R: Routable>(label: str, route: R): View
```

- The path signal is a module-level singleton (the `std::reactive` pattern:
  `turn_scope`, `next_subscriber_id`), lazily wired on first use:
  initialization reads `location.pathname` and subscribes `popstate`, so
  back/forward buttons drive the same signal as `navigate`.
- Both the popstate handler and `navigate` settle like any other reactive
  boundary: popstate dispatches inside a fresh turn (the DOM-event cadence,
  exactly as `View.on` does); `navigate` joins the caller's ambient turn when
  called from a handler.
- `link(label, route)` renders a real `<a href=..>` — the href comes from
  `to_path`, so middle-click / ctrl-click / copy-link keep native anchor
  behavior — and intercepts only PLAIN left-clicks (no modifier keys, main
  button) with `prevent_default` + `navigate`. It returns a `View`, so the
  usual chaining applies (`link(..).styled(nav_item)`).
- The app derives its typed route signal itself: `current_path().map(parse)`.

### 2.2 `View.swap` (in `std::ui`) — the dynamic-subtree primitive

Routing's rendering half is a GENERAL primitive, not a router feature
(decision: general `swap`, routing is just its first customer):

```vilan
fun swap<T: PartialEq>(self, source: Signal<T>, render: (|T| View) context owner_scope): View
```

The value-generalized `when`: whenever `source`'s value CHANGES, the previous
subtree's owner is disposed and its element removed, and `render` runs under a
fresh owner (the same disposal-boundary discipline as a `bind_each` row —
proposal/ambient-owner.md §4). `T: PartialEq` makes an equal value a no-op:
navigating to the current route re-renders nothing. The route match lives in
the render closure:

```vilan
view("main").swap(route, |current| match current {
    Route::Home => home_page(),
    Route::Workspace(let org, let inner) => workspace_layout(org, inner),
    Route::NotFound => not_found_page(),
})
```

### 2.3 `Event` + `View.on_event` (in `std::dom` / `std::ui`)

`link`'s click interception needs the DOM event — machinery `std::ui` lacked
(handlers were `|| void`). Rather than a router-private helper, the general
form: `external struct Event` with `prevent_default()` and the modifier/button
getters, `Element.on_event` binding the same `addEventListener` with an
event-taking handler, and `View.on_event(name, handler)` wrapping dispatch in
a turn exactly as `View.on` does. Generally useful (keyboard handling is the
obvious next customer).

## 3. Decisions (resolved 2026-07-11)

1. **Enum routes** over pattern strings (no pattern escape hatch in v1).
2. **`link(label, route)`** takes the route value via the `Routable` bound.
3. **General `swap(signal, render)`** rather than a route-specific
   `route_view`.
4. **Query strings and hash: deferred.** `current_path()` is `pathname` only.
   When query support lands it should stay in the typed-value model (a
   `Route` payload, not a stringly side-channel).

## 4. Deferred / recorded

- **`[derive(Route)]`** — generate `parse`/`href` from per-variant path
  attributes once the hand-written pair proves annoying in practice.
- **Query strings + hash** (decision 4).
- **Scroll restoration** — browser-default for now; a `navigate` option
  later.
- **SSR / base-layer lift** — `segments` and an app's `parse`/`href` are pure
  string logic; if server-side rendering arrives, the pure parts move to a
  platform-neutral layer so the server can route too. Until then the router
  is browser-only and a `@process` import of `std::router` is the platform
  error it should be.
- **Route-change effects** (title, analytics) — plain `effect` on the route
  signal already covers this; no API needed.
