# Reactive ownership & disposal (backlog A2)

**Status:** proposal (not implemented). Fixes the subscription leak in `std::reactive`/`std::ui`
and makes a leaked subscription impossible to create silently.

## Motivation

`sub()` returns a `Subscription` that every caller drops, and nothing disposes it — the observer
stays registered in the signal's subscriber list forever. For app-lifetime signals that is benign,
but `bind_each` makes it a real bug: every list change `clear()`s the DOM and rebuilds rows, each row
re-`sub()`-ing; the old rows' subscriptions stay live, firing on every change and mutating detached
nodes, growing without bound.

The fix is an **owner scope**: subscriptions register with an ambient owner that disposes them as a
group when its scope ends (a component unmounts, `bind_each` reconciles a row away).

## Decisions (locked)

- **Strategic owners, library-only — not a `comp` grammar.** Owners are created at the *dynamic*
  boundaries where reactive content is created then destroyed (`bind_each`, conditional mount,
  `mount`), not per component. Static composition doesn't leak, so it needs no owner — which is why
  no component syntax is required. A `comp` ergonomic layer, if ever wanted, is a macro (#9), never
  core grammar.
- **`sub()` requires an owner (loud).** A subscription cannot be created without something
  responsible for disposing it; `sub()` with no ambient owner fails loudly (see Mechanism).

## Mechanism: a module-level owner stack (not the `context` intrinsic)

The natural design — carry the owner in a `Context<Owner>` — does **not** work library-only. The
ambient owner must wrap the *component call* (a component subscribes while building its `View`,
*before* `mount` receives it), so `mount` must take a thunk and run it under the owner. But
`Context.run` requires a **closure-literal body** ("`run` must be called on a named context with a
closure literal body") — it cannot run a passed-in thunk. So `mount(|| view)` / `Owner::run(thunk)`
can't be expressed over `Context`.

Instead, carry the current owner in a **module-level stack** (how Solid carries its owner):

```vilan
let STACK: Shared<List<Owner>> = Shared::new([]);
```

`run` pushes/pops around a plain thunk (no `Context` restriction); `sub()` reads the top. Trade-offs
vs. the `Context` route, accepted for this cut:

- **Runtime** require-owner (a dev-mode throw), not compile-time. Loud enough — you hit it the first
  time the path runs — and within the locked "loud" decision.
- **Synchronous rendering only** (a global stack interleaves under concurrent async renders).
  Reactive rendering is synchronous, so this is moot in practice.

**Deferred upgrade:** route the owner through `Context<Owner>` for a *compile-time* require-owner
guarantee (every path, not just executed ones) and async-safety. It needs a context-pass extension —
run a thunk parameter under a context and trace it inter-procedurally so a thunk passed to `mount`
that reads the owner is admitted. Tracked as a follow-up; not in this cut.

## API — `Owner`

```vilan
struct Owner {
    // Cleanup thunks: a subscription's `dispose`, a child owner's `dispose`, a
    // `sub`-body cleanup. Run (in reverse) on `dispose`.
    cleanups: Shared<List<|| void>>,
}

impl Owner {
    // Run `body` with a fresh root owner current; returns the owner so the caller
    // can `dispose()` it later (the app, a test). `mount` uses this.
    fun root(body: || void): Owner { … push, body(), pop … }

    // A child of the current owner: disposed when the parent is, or earlier by
    // the caller (e.g. `bind_each` before a rebuild).
    fun child(): Owner { … register `|| owner.dispose()` with the current owner … }

    // Make `self` current for the dynamic extent of `body`.
    fun run(self, body: || void) { … push self, body(), pop … }

    // Register a cleanup with `self`.
    fun on_cleanup(self, cleanup: || void) { self.cleanups.write().push(cleanup); }

    // Run every cleanup once (idempotent: clears the list).
    fun dispose(self) { … run cleanups, clear … }
}

// The current owner, or a loud failure if there is none.
fun current_owner(): Owner { … top of STACK, else panic("…needs an owner; wrap in `mount`/`Owner::root`") … }
```

## Changes to `std::reactive`

- **`sub` registers with the current owner and returns `void`.** It still produces a `Subscription`
  internally, but registers `|| subscription.dispose()` with `current_owner()` rather than handing it
  back. `current_owner()` is the loud require-owner point. (Callers that dropped the `Subscription` —
  every binding, `combine` — are unaffected; manual early disposal is done with a child owner.)
- **Per-run cleanup (Solid-style `onCleanup`):** a `sub`/`map` body may register a cleanup that runs
  before the next apply and on dispose. *(Optional in v1; include if cheap.)*

## Changes to `std::ui`

- **`mount(id: str, build: || View): Owner`** — runs `build` under a fresh root owner so the
  component's subscriptions are owned, attaches the result, and returns the root owner (so the app
  can later dispose it). **Signature change** from `mount(id, view: View)`; migrate `app.vl`
  (`mount("counter", || counter())`).
- **`bind_each`** — keep a child owner *per render*: dispose the previous child, create a new one,
  and build the rows under it (`child.run(|| { for item in list { append(render(item).element) } })`)
  so each row's subscriptions register with the child and are disposed on the next reconcile. This is
  the leak fix.
- **`show`** — *no change.* As written it toggles `hidden` and never destroys its subtree, so it does
  not leak and needs no owner. A *destroy-on-hide* conditional (Solid's `<Show>`, which unmounts
  children when false) is a **separate, new combinator** (`mount_when`?) and a separate decision —
  do not conflate it with `show`.

## How it lowers

Pure `std` — a `Shared<List<Owner>>` stack, `Shared<List<|| void>>` cleanup lists, and closures. No
compiler work. (The deferred `Context` upgrade is the only part that would touch the compiler.)

## Test plan

- **Leak fix:** mount a `bind_each` over a signal; change the list N times; assert the source's
  subscriber count does not grow (old rows' subs disposed) and a disposed row's observer no longer
  fires.
- **Disposal propagates:** `mount` → `dispose()` the returned owner → all subscriptions gone; a
  subsequent `set` fires nothing.
- **Require-owner is loud:** `count.sub(..)` with no ambient owner throws (dev) with a message
  pointing at `mount` / `Owner::root`; the same under an `Owner::root(|| …)` compiles and runs.
- **Per-render scope:** a row's `sub`-body cleanup runs when that row is reconciled away.

## Open questions

- **`sub` return type:** `void` (owner-managed; this proposal) vs. keep returning `Subscription` for
  manual control. Leaning `void` + child owners for manual scopes.
- **`show` vs. a destroy-on-hide `mount_when`:** confirm `show` stays hide-only; scope `mount_when`
  separately (it interacts with keyed reconcile / A3).
- **Compile-time upgrade:** when to do the `Context<Owner>` + context-pass extension (compile-time
  require-owner, async-safety).
