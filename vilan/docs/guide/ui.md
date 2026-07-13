# Building UI

`std::ui` is a declarative view layer with no virtual DOM. A `View`
describes a DOM element. Methods chain to build it. Where React re-runs
components and diffs the result, vilan binds individual DOM properties to
signals — when a signal changes, exactly that text node or attribute
updates and nothing else runs.

Available in browser builds (`target = "browser"` in `vilan.toml`, or
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

Read that top to bottom: make a `div`, give it a paragraph whose text
follows the counter, give it a button that bumps the counter. That's the
whole mental model.

## Views

`view(tag)` makes a fresh element. Methods chain, and each returns the
view so you can keep going:

- **Static content**: `.text(content)`, `.class(name)`,
  `.attr(name, value)`, `.styled(style)` (see [Styling](styling.md)).
- **Structure**: `.child(view)`, `.children(views)`.
- **Events**: `.on(event, handler)`, or `.on_event(event, |e| …)` when
  you need the DOM event itself (`prevent_default`, `key()`, modifiers).
- **Reactive bindings**: `.bind_text(signal)`, `.bind_class(signal)`,
  `.bind_attr(name, signal)`, `.style_var(name, signal)`.

Every `bind_*` sets the property now and re-sets it whenever the signal
changes. There is no render loop to trigger.

## Components are just functions

A "component" is a function that returns a `View`. No registration, no
special types, no props system — parameters are the props:

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

`mount_root(id, body)` builds the body and attaches it to the page
element with that id. It also establishes the root **owner**, which is
why you never think about cleanup: every binding you create, at any
depth of function calls, registers with the nearest owner automatically
(the [reactive guide](reactive.md) explains owners).

If you build UI outside any root you'll get a compile error mentioning
`owner_scope`. It means "wrap this in `mount_root`" (or
`run_with_owner` in a test).

## Events run in turns

Each event dispatch runs your handler inside a fresh **turn**: all the
signal writes one click causes are batched, and watchers see the final
state once. Handlers die with their DOM node, so there is nothing to
unsubscribe.

```vilan,fragment
.on("click", || count.set_with(|n| n + 1))
.on_event("keydown", |pressed| {
	if pressed.key() == "Enter" { submit(); }
})
```

## Inputs

Two ways to wire an `<input>`, for two different situations:

**`bind_value(signal)`** is the simple two-way bind: the input shows the
signal, typing writes it back. Use it for local state — a search box, a
"new item" field.

**`bind_draft(draft)`** binds the input to a local-first
[draft](reactive.md#optimistic-writes-and-local-first-drafts) whose
commit is typically an rpc. Typing updates the input instantly and
commits in the background. A remote update folds in without re-sending.
An echo of your own edit never moves the caret. Use it for fields that
edit *server* state as you type:

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

`bind_each(source, key, render)` renders one row per element of a
`Signal<List<T>>`. Rows are **keyed**, like React's `key` prop, and the
key does real work here:

- A row whose key survives a change is **reused**. Its element moves to
  the new position with its state and subscriptions intact.
- A row whose key survives but whose *value* changed re-renders just
  that row (that's why `T: PartialEq`).
- Removed rows are disposed properly — each row is its own owner, so a
  row's bindings die with the row.

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

```vilan,fragment
fun bind_each<T: PartialEq, K: PartialEq>(
	self,
	source: Signal<List<T>>,
	key: |T| K,
	render: (|T| View) context owner_scope,
): View
```

## Conditionals: `show`, `when`, `swap`

Three primitives. Pick by what should happen to the content while it's
not visible:

| | Content while off | State | Use for |
|---|---|---|---|
| `.show(condition)` | mounted, hidden | **preserved** | tabs, collapsibles — anything that should keep its input text |
| `.when(condition, body)` | unmounted, disposed | dropped | content that shouldn't exist while off (an editor for a missing record) |
| `.swap(source, render)` | previous subtree disposed on change | per-value | pages on a route signal, any value-driven subtree |

```vilan,fragment
.show(open)                             // Signal<bool>
.when(present, || task_editor(…))       // Signal<bool> + (|| View)
.swap(route, |current| match current {  // Signal<T> + (|T| View)
	Route::Home => home_page(),
	Route::NotFound => not_found(),
})
```

`when` and `swap` build their content under a fresh owner each time, so
everything inside cleans up when the content goes away. `swap` re-renders
only when the value actually *changes* (`T: PartialEq`), so navigating
to the page you're already on does nothing.

## The ownership picture

Here is the whole cleanup model in one picture. Owners exist at the
places marked `◆` — the boundaries where a subtree can die. Every
binding registers with the *nearest* boundary above it, no matter how
many plain function calls sit in between:

```text
◆ mount_root("app", …)                the root owner — lives forever
│
├── view("header")                     static: no boundary of its own
│     └─ .bind_text(title)             → registers with the ROOT
│
├── ◆ .swap(route, |page| …)           one owner PER PAGE shown
│     └─ home_page()
│           └─ .bind_text(…)           → registers with the PAGE
│
└── ◆ .bind_each(todos, key, |t| …)    one owner PER ROW
      ├─ row(id = 1)
      │     └─ .bind_class(…)          → registers with ROW 1
      └─ row(id = 2)
            └─ .on("click", …)         → dies with ROW 2's DOM node
```

Navigate away, and the page's owner is disposed — every binding the page
created dies with it. Delete row 2, and only row 2's bindings die. This
is why there is no unsubscribe code anywhere in a vilan app: the tree of
boundaries *is* the cleanup logic, and the framework already placed
them where subtrees end.

## Escaping to the DOM

`View` is a thin wrapper over `std::dom::Element` (it's right there as
`view.element`). For anything the chain doesn't cover, use `std::dom`
directly: `get_element_by_id`, `query_selector`,
`element.set_attribute`, and so on. See the
[browser reference](../std/browser.md).

## Traps

- `show` keeps bindings live while hidden — they keep firing. If the
  hidden content is expensive, use `when`.
- `bind_value` fights remote updates (every keystroke overwrites). For
  server-backed fields, use `bind_draft`.
- The `owner_scope` compile error means you built UI outside every
  boundary. Wrap the entry point in `mount_root`.
- Don't create owners per element or per component function. Boundaries
  belong where subtrees can *die*: roots, rows, conditionals. The
  framework already puts them there.
