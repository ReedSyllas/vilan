# The ambient owner (backlog A5)

Status: **v1 settled 2026-07-07; basics land in this slice.** The ergonomic
layer over the explicit `Owner`/`Disposable` primitives
(the pruned `reactive-ownership.md` shipped those; git keeps its context):
reactive registrations that tie themselves to the enclosing scope without the
owner being passed by hand.

## 1. The substrate is proven

The design rides `std::context` — compile-time dynamic scoping via hidden
parameters — and every property A5 needs was probed against the live compiler:

- **Stored callbacks**: a closure created inside a `run`, stored, and fired
  after the extent exits still reads the captured value (capture-at-creation).
- **Async**: `get()` before and after an `await` in one async function; an
  async block spawned inside a `run` resuming after the `run`'s body exited;
  two interleaved extents each seeing their own value. Hidden parameters are
  ordinary parameters — the continuation closes over them, so there is no
  restore-on-resume problem *by construction* (unlike an
  `AsyncLocalStorage`-style runtime global).
- **The static fence**: `get()` is statically total — code reachable without
  an enclosing `run` is a compile error ("context is read here, but this code
  can be reached without an enclosing `run`"), a whole-call-graph coverage
  check. Misuse of the ambient API is therefore a compile error, not a
  runtime absence.

## 2. The decisions (from review, 2026-07-07)

1. **Strict-only — no absence semantics.** The ambient reader *requires* an
   enclosing scope, enforced by the existing static check. Nothing ever
   observes "no owner", so nothing needs `Option` semantics. This falls out
   of decision 2: with separate ambient and explicit methods, the explicit
   one needs no context at all and the ambient one may demand it.
   - **`get_safe` recorded, not taken.** A general "read it if established"
     accessor is coherent — the hidden parameter's honest type for
     *possibly*-established context is `Option<T>`: strict-only regions keep
     the bare `T` flavor and the static check; `get_safe`-reachable regions
     carry `Option<T>`; covered→safe boundaries `Some`-wrap; safe-only roots
     synthesize `None`. Two parameter flavors, same weaving. Take it when a
     real consumer appears (tracing is the classic one) — A5 does not need it.
2. **Two methods, not one dual-mode method.** `sub` stays exactly as shipped:
   explicit, `[must_use]`, returns the `Subscription`. The ambient variant is
   a *separate* method with nothing to hold — which dissolves the
   `[must_use]` interplay too.
3. **Threading breadth accepted.** Reading the ambient owner inside
   `std::reactive` weaves the hidden parameter through everything that
   transitively reaches it — intrinsic to compile-time threading, per-context
   (unwoven code pays nothing), and exactly what makes the async story
   correct for free. It is also why this is compiler work: the weaving is
   what makes the pattern miserable to hand-write.

## 3. v1 surface (this slice — `std::reactive` only)

```vilan
// Establish `owner` as the ambient owner for the dynamic extent of `body`.
fun run_with_owner(owner: Owner, body: || void)

// The ambient owner established by the nearest enclosing `run_with_owner`.
// Reaching this without one is a compile error (the §1 static fence).
fun get_owner(): Owner

impl Signal<type T> {
    ..
    // Subscribe and register with the ambient owner — nothing to hold.
    fun effect(self, observer: |T| void)
}
```

`effect` is `self.sub(observer)` handed to `get_owner()`. It was DESIGNED as
a defaulted `Source` trait method, but probing moved it: **the context call
graph does not wire trait-dispatch edges to trait DEFAULT bodies** — a
default body reading a context is flagged "reachable without an enclosing
`run`" even when its only call site is covered (conservative, not a
miscompile; pinned `#[ignore]`, backlog B14). Until that edge lands, `effect`
is an inherent method on `Signal` — every real `Source` today. `run_with_owner`
nests like `run`: the nearest extent wins; disposing an owner disposes the
effects registered in its extent, stored callbacks and post-`await`
registrations included (§1).

## 4. Recorded follow-ups

- **`comp`-style sugar**: `comp(|| view(..))` creating the owner, running the
  body under it, and returning owner+result together — after the v1 surface
  proves the shape in the examples.
- **`std::ui` integration**: `View` construction under an ambient scope
  (bindings self-registering) — deliberately out of this slice, which touches
  `std::reactive` only.
- **Error-message anchoring**: the static fence's diagnostic points at the
  `get()` inside `std::reactive` when the uncovered path starts in user code;
  anchoring it at the uncovered root's call site is a diagnostics follow-up.
- **`get_safe`** (§2.1), with the `Option`-parameter sketch.
- **`effect` on the `Source` trait** — once the context call graph learns
  trait-default dispatch edges (backlog B14), move/extend `effect` to the
  trait so remote sources get it too.
