# Building UI

`std::ui` is a declarative, fine-grained reactive view layer: a `View`
describes a DOM element, methods chain to build it, and `bind_*` methods keep
individual DOM properties in sync with signals — when a signal changes, only
that property updates. There is no virtual DOM and no re-render.

Available for browser builds (`target = "browser"` in `vilan.toml`, or
`vilan build --target browser`).

```vilan,browser
import std::ui::{ view, View, mount_root };
import std::reactive::Signal;

fun main() {
	let count = Signal::new(0);
	let _root = mount_root("app", || {
		view("div")
			.child(view("p").bind_text(count.map(|n: i32| i"clicked {n} times")))
			.child(view("button").text("+1").on("click", || count.set_with(|n| n + 1)))
	});
}
```

## Views

`view(tag)` makes a fresh element; methods chain and return the view:

- **Static content**: `.text(content)`, `.class(name)`, `.attr(name, value)`,
  `.styled(style)` (see [Styling](styling.md)).
- **Structure**: `.child(view)`, `.children(views)`.
- **Events**: `.on(event, handler)`, `.on_event(event, |dispatched| …)` when
  you need the DOM `Event` (`prevent_default`, `key()`, modifier keys).
- **Reactive bindings**: `.bind_text(signal)`, `.bind_class(signal)`,
  `.bind_attr(name, signal)`, `.style_var(name, signal)`.

Every `bind_*` is an effect: it sets the property now and re-sets it on each
signal change. Nothing else on the page is touched.

## Mounting and component functions

`mount_root(id, body)` builds the body under a fresh **owner** (the root
disposal boundary) and attaches the view to the page element with that id. A
"component" is just a function returning a `View` — no registration, no
special type:

```vilan,browser
import std::ui::{ view, View, mount_root };
import std::reactive::Signal;

fun labelled_input(label: str, value: Signal<str>): View {
	view("label")
		.text(label)
		.child(view("input").bind_value(value))
}

fun main() {
	let name = Signal::new("");
	let _root = mount_root("app", || labelled_input("Name", name));
}
```

Any `effect`/binding created anywhere in the call tree registers with the
nearest enclosing boundary automatically (the ambient owner — see
[Reactive state](reactive.md)). Building UI outside every boundary is a
compile error, so subscriptions can't leak by construction.

## Events are turn boundaries

Each event dispatch runs your handler inside a fresh **turn**: all the signal
writes a click causes settle as one wave (see the
[reactive guide](reactive.md)). Handlers die with their DOM node — no
unsubscribe needed.

```vilan,fragment
.on("click", || count.set_with(|n| n + 1))
.on_event("keydown", |pressed| {
	if pressed.key() == "Enter" { submit(); }
})
```

## Inputs: `bind_value` and `bind_draft`

`bind_value(signal)` two-way binds an `<input>`: the input shows the signal,
typing writes it back. Use it for local-only state (a search box, a
new-item field).

`bind_draft(draft)` binds an input to a **local-first draft** (see
[Reactive state](reactive.md#optimistic-writes-and-local-first-drafts)): user
input pushes (locally first, then the draft's commit — typically an RPC), a
remote adoption updates the input without re-pushing, and a dirty draft
ignores adoption so an echo never moves a focused caret. Use it for fields
that edit *server* state as you type.

```vilan,browser
import std::ui::{ view, View, mount_root };
import std::reactive::{ draft, Draft, DraftState };
import std::option::Option::{ self, Some, None };

fun main() {
	let name = draft("initial", |value: str| {
		let _would_send = value; // an rpc call in a real app
		None
	});
	let _root = mount_root("app", || {
		view("div")
			.child(view("input").bind_draft(name))
			.child(view("span").bind_text(name.state.map(|state: DraftState| match state {
				DraftState::Synced => "",
				DraftState::Dirty => "saving…",
				DraftState::Failed(let reason) => i"failed: {reason}",
			})))
	});
}
```

## Lists: `bind_each`

`bind_each(source, key, render)` renders one child view per element of a
`Signal<List<T>>`, reconciled **by key** on every change:

```vilan,fragment
fun bind_each<T: PartialEq, K: PartialEq>(
	self,
	source: Signal<List<T>>,
	key: |T| K,
	render: (|T| View) context owner_scope,
): View
```

- A row whose key survives is **reused** — its element moves into the new
  order, its subscriptions stay intact.
- A surviving key whose *value* changed re-renders just that row
  (`T: PartialEq` decides).
- Each row is a disposal boundary: `render` runs under the row's own owner,
  so a row's bindings die with the row.

```vilan,browser
import std::ui::{ view, View, mount_root };
import std::reactive::Signal;

[derive(PartialEq)]
struct Todo {
	id: i32,
	title: str,
}

fun main() {
	let todos: Signal<List<Todo>> = Signal::new([
		Todo { id = 1, title = "write docs" },
	]);
	let _root = mount_root("app", || {
		view("ul").bind_each(todos, |todo| todo.id, |todo| {
			view("li").text(todo.title)
		})
	});
}
```

## Conditionals: `show`, `when`, `swap`

Three primitives, differing in what happens to the hidden content:

| | Content while off | State | Use for |
|---|---|---|---|
| `.show(condition)` | mounted, hidden | **preserved** | tabs, collapsibles — anything that should keep its input text |
| `.when(condition, body)` | unmounted, disposed | dropped | content that shouldn't exist while off (an editor for a missing record) |
| `.swap(source, render)` | previous subtree disposed on change | per-value | pages on a route signal, any value-driven subtree |

`when` and `swap` bodies build under a fresh owner per instantiation — a
disposal boundary, like a `bind_each` row. `swap` re-renders only when the
value actually *changes* (`T: PartialEq`); navigating to the current route is
a no-op.

```vilan,fragment
.show(open)                             // Signal<bool>
.when(present, || task_editor(…))       // Signal<bool> + (|| View)
.swap(route, |current| match current {  // Signal<T> + (|T| View)
	Route::Home => home_page(),
	Route::NotFound => not_found(),
})
```

## Escaping to the DOM

`View` is a thin handle over `std::dom::Element` (`view.element`). For
anything the chain doesn't cover, use `std::dom` directly:
`get_element_by_id`, `query_selector`, `element.set_attribute`, etc. — see
the [browser reference](../std/browser.md).

## Traps

- One boundary per *dynamic* subtree is the model — don't create owners per
  element or per component function; static content shares its boundary's
  lifetime.
- `show` keeps bindings live (they keep firing while hidden); if the hidden
  content is expensive, prefer `when`.
- `bind_value` fights remote updates (each keystroke overwrites) — for
  server-backed fields use `bind_draft`.
- Building views outside `mount_root`/a row/a `when`/`swap` body is the
  "context owner_scope is read here" compile error: wrap the entry point in
  `mount_root` (or `run_with_owner` in tests).
