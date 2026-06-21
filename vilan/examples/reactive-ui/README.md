# Reactive UI — `std::ui`

> **Implementation status — increment 1 is built.** The reactive core
> (`std::reactive`: `Signal`/`Source` with `derive`, `sub`, `set`/`set_with`) and
> the counter-level UI (`std::ui`: `view` + `text`/`class`/`attr`/`on`/`child`/
> `children` + `bind_text`/`bind_class`/`bind_attr`/`bind_value` + `mount`) work.
> The counter ([`counter.vl`](counter.vl)) builds and runs:
> `vilan build --target browser app.vl` (emits `app.js`; serve `index.html`).
>
> **Built:** `combine` (variadic over its inputs' distinct types — a mapped tuple
> parameter; a `Signal` is passed as `.source()`), `bind_each` (a reactive list —
> clear-and-rerender on change), and `show` (toggle a node on a `Source<bool>`). So
> [`todos.vl`](todos.vl) now compiles and is mounted by [`app.vl`](app.vl).
> **Not yet built:** `flatten`. `bind_each` re-renders the whole list on any change
> (correct, but not yet keyed-reconciled — the `key` argument is reserved for that).
>
> **Known limitation:** calling a *generic* function/method (e.g. `format(n)` or
> `n.to_string()`) on a closure parameter inside a binding doesn't infer the
> param's concrete type yet, so it needs a type annotation (`|n: i32| …`). For
> rendering a number, an i-string (`i"Count: {n}"`) sidesteps it — see
> `counter.vl`. The inference fix is the top follow-up.

## The model: explicit dependencies, known eagerly

Solid-style auto-tracking discovers a computation's dependencies by *running its
body* and recording which signals it reads. That makes dependencies **lazy** (not
fully known until the body runs) and it breaks across an `await` — reads after
the await aren't tracked. Making tracking survive the await still discovers the
dependency *late*, which defeats eager scheduling and a graph you can reason
about or move.

So this design makes dependencies **explicit and structural** — known the moment
a reactive value is constructed, never discovered by running it:

- A reactive value is a `Source<T>`.
- You build new ones with **combinators** whose *shape is the dependency graph*:
  `a.derive(f)` depends on `a`; `combine(a, b)` depends on `a` and `b`. There is
  no body to run to "find" dependencies — the graph is the combinator tree, as
  data.
- A `derive`/`sub` body is a **pure function of its inputs**, passed as
  parameters. It can `await` freely; its dependencies were fixed structurally
  before it ran.

Reading a source with `.get()` *inside* a body is therefore a deliberate
**non-reactive sample**, not a dependency. Tracking is opt-*in* (combine it in),
not opt-out — so unlike Solid there's no `untrack` escape hatch to remember.

> **This is where Solid itself is heading.** Solid 2.0 splits effects into a
> *compute phase (the explicit dependency declaration)* and an *apply phase (side
> effects)* — exactly our `source` (deps) + `.sub(apply)`. It enforces read/write
> separation ("no public API exposes a single writable reference"), which is our
> `Source`/`Signal` split. We're aligned with the state of the art, not off in
> the weeds.

A **component** is a function returning a `View`; an **app** is composition plus
`mount`.

## A first look

```vilan
fun counter(): View {
    let count = Signal::new(0);

    view("section")
        .child(view("p").bind_text(count.derive(|n| "Count: " + format(n))))
        .child(view("button").text("+").on("click", || count.set_with(|n| n + 1)))
}
```

[`counter.vl`](counter.vl) is the minimal component; [`todos.vl`](todos.vl) shows
`combine`, two-way input, a reactive list, and conditional rendering;
[`app.vl`](app.vl) mounts both.

## API — `std::reactive` (the core)

The read/write split is the spine: **`Source<T>` is the read interface** every
reactive value implements; **`Signal<T>` is a writable root** that extends it.
`derive`/`combine`/`flatten` always yield read-only `Source`s — only a root
`Signal` can be written. The combinators are monadic: `derive` is map, `combine`
is the product, `flatten` is join, `sub` is the consumer.

### `Source<T>` — any reactive value

| Item | Meaning |
| --- | --- |
| `source.get(): T` | Sample the current value — **non-reactive**, creates no dependency. |
| `source.derive(\|T\| U): Source<U>` | A derived source; depends structurally on `source`. |
| `source.sub(\|T\| void): Subscription` | Run now and on every change; owned by the active scope (below). |
| `source.flatten(): Source<U>` *(when `T` is `Source<U>`)* | Follow whichever source `source` currently holds — a **dynamic** dependency. |
| `combine(a, b, c, …): Source<(A, B, C, …)>` | **Variadic** product; a source of the tuple, depending on all inputs. |

`combine` yields a plain source rather than taking the mapping function, so it
composes both ways:

```vilan
combine(a, b, c).sub(|(a, b, c)| print(i"a={a} b={b} c={c}"));   // observe together
let sum = combine(a, b, c).derive(|(a, b, c)| a + b + c);        // or derive a value
```

### `Signal<T>` — a writable root (`Signal<T>: Source<T>`)

| Item | Meaning |
| --- | --- |
| `Signal::new(value): Signal<T>` | A writable root signal. *(landed)* |
| `signal.set(value)` | Replace the value. *(landed)* |
| `signal.set_with(\|T\| T)` | Read-modify-write in one step. |
| *(inherits every `Source` method)* | `get` / `derive` / `sub` / `flatten`. |

### Dynamic dependencies — `flatten`

A dependency *chosen at runtime* is where auto-tracking and compile-time analysis
both struggle; the explicit graph handles it with `flatten` (the monadic join):

```vilan
// `selected` is a source whose value is itself a source (the selected row).
let selected: Source<Source<Todo>> = …;
let label = selected.flatten().derive(|todo| todo.label);   // tracks the *current* selection
```

## API — `std::ui` (views)

`view(tag: str): View` creates an element node; `View` is a handle to a DOM
element (like `std::dom::Element`). Methods chain. Reactive setters **take a
`Source`** (read); `bind_value` takes a writable `Signal` (it writes back).

| Method | Effect |
| --- | --- |
| `.text(str)` / `.bind_text(Source<str>)` | Text content, static / reactive. |
| `.class(str)` / `.bind_class(Source<str>)` | CSS class. |
| `.attr(name, str)` / `.bind_attr(name, Source<str>)` | An attribute. |
| `.bind_value(Signal<str>)` | **Two-way** `<input>` value ↔ signal. |
| `.show(Source<bool>)` | Reactively include/remove the node. |
| `.on(event: str, \|\| void)` | An event listener. |
| `.child(View)` / `.children(List<View>)` | Append children. |
| `.bind_each(Source<List<T>>, key: \|T\| K, render: \|T\| View)` | Reactive keyed list. |
| `mount(id: str, View)` | Attach a view to the page element with that id. |

`bind_each` renders one view per element and, as the list changes, reconciles by
`key`: rows reorder with their items, a removed key's row is disposed, and a row
whose value changed is re-rendered — `render` receives the item **value** (a
snapshot). This mirrors Solid 2.0's `<For keyed={…}>`. A later **by-index**
variant (render receives a `Source<T>` so a stable-position row updates in place
instead of re-rendering) is the counterpart to Solid's `<For keyed={false}>`.

## How it lowers (so it's grounded)

- A derived `Source` (`derive`/`combine`) is a graph node caching its value,
  recomputing when a dependency fires. Because the graph is eager it's scheduled
  topologically — each node recomputes once per change, glitch-free.
- `sub(f)` is the leaf consumer: subscribe, run `f` now and on change.
- A binding *is* a `sub`: `.bind_text(src)` → `src.sub(|s| element.set_text(s))`;
  `bind_class`/`bind_attr`/`show` are the same shape.
- `.bind_value(sig)` → `sig.sub(|s| input.set_value(s))` **and**
  `input.on("input", || sig.set(input.value()))`.
- `.bind_each(src, key, render)` → `src.sub(|list| reconcile(list, key, render))`.
- `mount(id, view)` → `dom::get_element_by_id(id).append(view.element())`.

## Lifecycle: an owner scope via `context`

`sub` returns a `Subscription`; dropping it must not silently unsubscribe (then
`count.sub(..)` as a statement would never fire). So subscriptions register with
an ambient **owner scope** that disposes them as a group — when a component
unmounts, or when `bind_each` tears down a removed row. That ambient owner is
exactly what the **`context` API** is for (the *owner*, not a dependency
tracker). Following Solid 2.0, a `sub` body may also return a per-run cleanup:

```vilan
source.sub(|value| {
    let timer = start(value);
    || stop(timer)   // cleanup: runs before the next apply, and on dispose
});
```

## North star: the same API across the server/client boundary

Because dependencies are explicit values, a `Source` can be a *remote* handle and
the combinators become transport-aware — `sub` sends a "subscribe" message,
`derive` maps locally. The `Source`/`Signal` split is the seam: the remote read
handle implements `Source`; only the server holds the writable `Signal`.

```vilan
// server
struct Client {
    [rpc(Visibility.Readonly)]
    count: Signal<i32>,
}
impl Client {
    fun inc(self) { self.count.set_with(|x| x + 1); }
}

// client — `count` is a remote `Source<i32>`
let count = client().count;
let _ = count.sub(|count| print(i"Count is {count}"));   // subscribes over the transport
let double = count.derive(|count| count * 2);
client().inc();                                          // round-trips; `count`/`double` update
```

This needs a whole RPC/transport layer — a much larger system than the UI. Treat
it as the **constraint that shapes today's API** (keep the graph explicit, keep
`Source` an interface), not part of the first build.

## Decisions (locked) and what's left to sequence

Resolved from the last round:

1. ✅ **Read/write split** — `Source<T>` (read interface) + `Signal<T>: Source<T>`
   (writable root). Derived values are read-only `Source`s.
2. ✅ **`bind_each`** — keyed value-list (`render` gets the item value); a by-index
   variant comes later.
3. ✅ **`combine` is variadic** — heterogeneous inputs → a tuple source.
4. ✅ **Ownership via `context`** — an ambient owner scope disposes subscriptions;
   per-run cleanup returns from the `sub` body.
5. ✅ **Incremental build** — `Signal`/`Source` core first, then the UI bindings.

The one real **prerequisite/sequencing** question: variadic `combine` over
heterogeneous sources needs **variadic generics (parameter packs)** — a language
feature that doesn't exist yet. Options for ordering:

- Build the core **without `combine`** first (`Source`/`Signal`/`derive`/`sub`/
  `flatten` need no new language feature), land variadics, then add `combine`.
- Ship `combine` at **fixed arities** (`combine2`/`combine3`/…) as a stopgap, then
  collapse them into one variadic `combine` once parameter packs land.

Everything else in increment 1 (the `Source`/`Signal` split, `derive`, `sub`, and
the counter-level UI: `view`/`text`/`class`/`on`/`child`/`bind_text`/`mount`) is
buildable today on the existing reactive primitives.
