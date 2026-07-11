# Router — `std::router`

A client-side routed app (proposal/router.md, backlog A10). The model in one
sentence: **routes are enums** — the URL is a wire format for a typed value,
so layouts, params, guards, and links are ordinary language (a `match`, a
payload, an `if`, a value) instead of a pattern-string DSL.

What to look at in [`app.vl`](app.vl):

- **The route space**: `Route` with a nested `ItemsRoute` — nested enums
  mirror nested layouts; `/items/{id}`'s param is an `i32` payload.
- **`parse`/`href`**: a hand-written inverse pair over
  `std::router::segments`. Total (unknown paths → `NotFound`), and testable
  like any function.
- **The shell**: `app` swaps the page on the derived route signal
  (`View.swap` — an unchanged route is a no-op; a swapped-out page's
  bindings are disposed). The nested `items_layout` repeats the same shape
  one level down.
- **Links**: `link("Items", Route::Items(ItemsRoute::List))` renders a real
  `<a href>` (middle-click and copy-link behave natively) and intercepts only
  plain left-clicks. `item_detail`'s Previous/Next links are arithmetic on
  the payload — no string splicing.
- **The live path**: `bind_text(current_path())` — the same signal
  `navigate`, link clicks, and the back button all drive.

## Build & run

```sh
vilan build .            # emits app.js beside index.html
npx serve --single .     # any static server with a history-API fallback
```

(`--single` rewrites every path to `index.html`; with a plain file server a
reload on `/items/2` would 404 — that's the serving side of history routing,
not the router's.)
