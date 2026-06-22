# Reactive ownership & disposal (backlog A2)

**Status:** **implemented** (2026-06-22). **Explicit** owners (no ambient/automatic tracking) plus a
`[must_use]` `sub`, so a dropped subscription is loud without any magic. Shipped: the `Disposable`
trait + `Owner` (`new`/`take`/`dispose`) in `std::reactive`, `View` self-collecting its bindings'
subscriptions in `std::ui`, `bind_each`'s per-render child owner (the leak fix), and the `[must_use]`
attribute (its own general feature — backlog B7) on the new `[name(..)]` attribute syntax (H2). The
deferred ergonomic layer (ambient owner / `comp` macro) remains future sugar.

## Motivation

`sub()` returns a `Subscription` every caller drops, and nothing disposes it — the observer stays
registered in the signal's subscriber list forever. For app-lifetime signals that is benign, but
`bind_each` makes it a real bug: every list change `clear()`s the DOM and rebuilds rows, each row
re-`sub()`-ing; the old rows' subscriptions stay live, firing on every change and mutating detached
nodes, growing without bound.

## Decision: explicit ownership, no magic (for now)

Defer *ambient* owner tracking (a `Context<Owner>` or a module-level owner stack) until we have an
ergonomic API proven against `async`, callbacks, and indirection. Every mechanism we sketched for it
carried a tax — `Context.run` needs a closure-literal body so it can't run a thunk param; a global
stack is sync-only and not async-safe; the compile-time guarantee needs a context-pass extension.
Rather than pick a magic now, **ownership is an explicit value you pass and dispose.** Whatever
ergonomic layer we eventually want (ambient owner, a `comp` macro) is just sugar that desugars to
these primitives — so this is the right foundation, not a throwaway.

It also rolls back the two compromises the ambient design forced:

- `mount(id, view)` stays as-is (no thunk) — the owner is external, so mount wraps nothing.
- `sub()` keeps returning `Subscription` (no `void` change).

## API — `Owner` + `Disposable`

Verified working (generic-bound dispatch through a captured closure compiles and runs):

```vilan
trait Disposable {
    fun dispose(self);
}

struct Owner {
    cleanups: Shared<List<|| void>>,
}

impl Owner {
    fun new(): Owner {
        Owner { cleanups = Shared::new([]) }
    }
    // Take ownership of any disposable — a Subscription, a View (its whole
    // subtree), or a child Owner. The type is erased into a `|| void` so one list
    // holds them all.
    fun take<T: Disposable>(self, item: T) {
        self.cleanups.write().push(|| { item.dispose(); });
    }
}

impl Owner with Disposable {
    fun dispose(self) {
        for cleanup in self.cleanups.read() {
            cleanup();
        }
        self.cleanups.write() = [];
    }
}
```

`Subscription`, `View`, and `Owner` all implement `Disposable`, so owners nest and views are owned
uniformly. `dispose` is idempotent (clears the list).

## `sub` is `[must_use]`

`sub(self, observer): Subscription` keeps its signature but gains `[must_use]`. Dropping its result —
a bare statement that discards a `Subscription` — is a loud diagnostic:

> unused `Subscription`: `take()` it into an `Owner`, or `dispose()` it (`let _ = …` to discard).

That restores the no-silent-leak property **without ambient tracking** — the loudness is a local
"unused value" check, not a lifetime analysis. Intentional fire-and-forget is `let _ = count.sub(..)`.
(`[must_use]` is a general attribute — see the `must_use` and attribute-syntax backlog items.)

## Views collect their own subscriptions

A `View` owns the subscriptions its bindings create, so handing a `View` to an `Owner` (or disposing
it) tears down the whole subtree — no need to surface each binding's `Subscription`:

- `view(tag)` — a `View` with an empty `cleanups` list (same shape as `Owner`).
- each `bind_*` subscribes and registers `|| subscription.dispose()` on the `View`.
- `.child(c)` nests: registers `|| c.dispose()` on `self` (so a tree's cleanups roll up to the root).
- `View with Disposable` — `dispose` runs the cleanups.

```vilan
fun counter(owner: Owner): View {
    let count = Signal::new(0);
    owner.take(view("p").bind_text(count.map(format)))   // take(View) owns the subtree
}

fun app() {
    let owner = Owner::new();
    mount("counter", counter(owner));
    // owner.dispose() when the app / route tears down
}
```

## `bind_each` — an internal child owner (the leak fix)

No ambient owner; `bind_each` manages a child `Owner` for the rows itself:

```vilan
fun bind_each<T, K>(self, source: Signal<List<T>>, key: |T| K, render: |T| View): View {
    let element = self.element;
    let rows = Owner::new();
    self.cleanups.write().push(|| { rows.dispose(); });   // unmounting the list disposes the rows
    self.take(source.sub(|list| {                         // reconcile sub is must_use -> take it
        rows.dispose();                                   // drop the previous rows' subs
        element.clear();
        for item in list {
            let row = render(item);
            rows.take(row);                               // own the new row's subtree
            element.append(row.element);
        }
    }));
    self
}
```

The same `rows` owner is reused — `dispose()` clears it, then it refills — so the subscriber list
stays bounded across re-renders.

## `show` — unchanged

`show` toggles `hidden`; it never destroys its subtree, so it doesn't leak and needs no owner. A
*destroy-on-hide* conditional (Solid's `<Show>`, which unmounts children when false) is a separate,
new combinator (`mount_when`?) and a separate decision — do not fold it into `show`. It interacts
with keyed reconcile (A3).

## `[must_use]` — the general feature

`[must_use]` on a function marks its result as must-be-consumed. A call whose result is **dropped**
(a non-tail statement expression whose value is discarded, not bound, not an argument, not assigned)
gets a diagnostic. Escapes: bind it (`let s = …`, `owner.take(…)`), make it the block tail, or
explicitly discard with `let _ = …`.

- **Detection:** scan block statement lists for a call expression to a `must_use` callee whose value
  is dropped. (The transformer already distinguishes statement vs. tail and tracks side effects.)
- **Severity:** a **warning** is the right fit (Rust-style) — which means adding a `Warning` severity
  to diagnostics (today they are all errors; the LSP already filters by severity). Recommended. The
  fallback, if we don't add severities now, is an **error** with `let _ =` as the escape — loudest,
  but it forces every drop site to be handled (which is the A2 migration anyway).
- **Syntax:** written `[must_use]`, per the new attribute syntax (`@name` → `[name]`) — backlog item.

## How it lowers

Pure `std` — `Shared` lists, closures, and the `Disposable` trait (no runtime owner stack, no context
threading). The only compiler touch is `[must_use]` (parse the attribute + the unused-result
diagnostic + possibly a `Warning` severity).

## Migration

- `bind_*` register their subs on the `View`; `mount` stays `(id, view)`.
- `combine` creates subscriptions internally (one per input) that live for the derived signal's life;
  it should `take` them into an owner tied to the derived signal, or be exempt — **decide** (open
  question).
- `reactive.vl` / examples: own top-level subs in an `Owner` (or `let _ =` for genuinely
  app-lifetime ones) once `sub` is `[must_use]`.

## Test plan

- **Leak fix:** mount a `bind_each` over a signal; change the list N times; assert the source's
  subscriber count stays bounded and a reconciled-away row's observer no longer fires.
- **Teardown:** `owner.dispose()` on a mounted tree → a subsequent `set` fires nothing.
- **`[must_use]`:** a dropped `sub()` is a diagnostic; `take` / `let _ =` / a tail position silence it.
- **Nesting:** disposing a parent owner disposes child owners and views.

## Open questions

- **`must_use` severity:** warning (add a `Warning` severity) vs. error (+ `let _ =` escape).
- **`combine`'s internal subscriptions:** who owns them.
- **`show` vs. a destroy-on-hide `mount_when`:** scope `mount_when` separately (ties to A3 keyed
  reconcile).
