# Browser modules — reference

The browser layer of std: `std::dom`, `std::ui`, `std::router`,
`std::storage`. Available only for browser builds. Concepts:
[Building UI](../guide/ui.md), [Routing](../guide/routing.md).

## std::dom

Opaque handles over real DOM objects.

```vilan,fragment
external struct Element;
fun get_element_by_id(id: str): Element
fun create_element(tag: str): Element
fun query_selector(selector: str): Element
fun query_selector_all(selector: str): List<Element>

impl Element {
	fun set_text(self, text: str)                      // textContent =
	fun set_class(self, name: str)                     // className =
	fun set_attribute(self, name: str, value: str)
	fun set_style_property(self, name: str, value: str) // style.setProperty (CSS custom props)
	fun append(self, child: Element)
	fun remove(self)                                   // detach from the document
	fun clear(self)                                    // remove every child
	fun set_hidden(self, hidden: bool)
	fun value(self): str                               // an input's current text
	fun set_value(self, value: str)
	fun on(self, event: str, handler: || void)
	fun on_event(self, event: str, handler: |Event| void)
}

external struct Event;
impl Event {
	fun prevent_default(self)
	fun button(self): i32        // 0 = main button
	fun meta_key(self): bool
	fun ctrl_key(self): bool
	fun shift_key(self): bool
	fun alt_key(self): bool
	fun key(self): str           // "Enter", "Escape", "a", …
}
```

Raw `element.on` handlers do **not** establish a turn — that's `View.on`'s
job. Prefer the `View` layer; drop to `dom` for what it doesn't cover.

## std::ui

```vilan,fragment
struct View { element: Element }
fun view(tag: str): View
fun mount(id: str, view: View)                                   // attach only
fun mount_root(id: str, body: (|| View) context owner_scope): Owner
```

`mount_root` = fresh owner + turn boundary + attach; it returns the root
owner (most apps let it live forever). `mount` is the attach half alone —
use only when you already hold a boundary.

### View methods

| Method | Signature (self elided) | Notes |
|---|---|---|
| `text` | `(content: str): View` | static text |
| `class` | `(name: str): View` | static class |
| `styled` | `(style: Style): View` | classes from a compiled style |
| `attr` | `(name: str, value: str): View` | static attribute |
| `style_var` | `(name: str, source: Signal<str>): View` | reactive CSS custom property |
| `on` | `(event: str, handler: (\|\| void) context turn_scope): View` | handler runs in a fresh turn |
| `on_event` | `(event: str, handler: (\|Event\| void) context turn_scope): View` | same, with the DOM event |
| `child` | `(child: View): View` | append one |
| `children` | `(items: List<View>): View` | append several |
| `bind_text` | `(source: Signal<str>): View` | reactive text |
| `bind_class` | `(source: Signal<str>): View` | reactive class |
| `bind_attr` | `(name: str, source: Signal<str>): View` | reactive attribute |
| `bind_value` | `(signal: Signal<str>): View` | two-way input bind |
| `bind_draft` | `(draft: Draft<str>): View` | local-first input bind ([drafts](reactive.md#draft--local-first-cells)) |
| `bind_each` | `(source: Signal<List<T>>, key: \|T\| K, render: (\|T\| View) context owner_scope): View` — `T: PartialEq, K: PartialEq` | keyed rows; each row is a disposal boundary |
| `when` | `(condition: Signal<bool>, body: (\|\| View) context owner_scope): View` | state-DROPPING conditional |
| `swap` | `(source: Signal<T>, render: (\|T\| View) context owner_scope): View` — `T: PartialEq` | dispose + rebuild per changed value |
| `show` | `(condition: Signal<bool>): View` | state-PRESERVING visibility toggle |

Semantics, choosing between `show`/`when`/`swap`, and examples: the
[UI guide](../guide/ui.md).

## std::router

```vilan,fragment
fun current_path(): Signal<str>       // location.pathname, live (navigate + back/forward)
fun navigate(path: str)               // pushState + update current_path
fun segments(path: str): List<str>    // "/w/3/task/7" → ["w", "3", "task", "7"]

trait Routable { fun to_path(self): str }
fun link<R: Routable>(label: str, route: R): View   // a real <a>; intercepts plain left-clicks
```

`current_path()` is a singleton signal — every caller gets the same one, and
the `popstate` listener is wired on first use. `link` renders a real anchor
(middle-click, ctrl-click, and copy-link keep native behavior) and intercepts
only a plain left click, calling `prevent_default` + `navigate`. Route
modelling (`parse`/`href` over enums): the [routing guide](../guide/routing.md).

## std::storage

`localStorage` / `sessionStorage`, string-keyed strings. A missing key reads
as `""`.

```vilan,fragment
fun get(key: str): str
fun set(key: str, value: str)
fun remove(key: str)
fun session_get(key: str): str
fun session_set(key: str, value: str)
fun session_remove(key: str)
```

```vilan,browser
import std::storage;

fun main() {
	storage::set("token", "abc");
	let token = storage::get("token");
	if token != "" {
		storage::remove("token");
	}
}
```
