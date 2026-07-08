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

trait Source<T> {
    ..
    // Subscribe and register with the ambient owner — nothing to hold.
    fun effect(self, observer: |T| void)   // trait DEFAULT — every Source
}
```

`effect` is `self.sub(observer)` handed to `get_owner()`, and it lives on the
`Source` trait as designed — **B14 is fixed** (2026-07-07): the context pass
now adds trait-dispatch edges locally (the shared call graph stays untouched
— it is also async inference's graph), so a default body's context read is
covered when its dispatch sites are, and the hidden value threads through the
dispatch call (a candidate callee that doesn't need it ignores the trailing
argument). Fixing it exposed and repaired a LATENT MISCOMPILE beyond
contexts: `resolve_inherited_default` matched impl subjects by exact type
equality, so an inherited default on a GENERIC subject (`Signal<i32>` through
`impl Signal<type T> with Source<T>`) never matched and the call silently
bound to the trait's abstract member — now nominal matching, like
`resolve_member_on_type`, with its own pin. Extents nest — the nearest wins;
disposing an owner disposes the effects registered in its extent, stored
callbacks and post-`await` registrations included (§1).

## 4. Recorded follow-ups

- ~~**`comp`-style sugar**~~ — **SHIPPED 2026-07-07**:
  `comp<T>(body: (|| T) context owner_scope): (T, Owner)` — a fresh owner,
  the body run under it, and the product PAIRED with the disposal handle.
  Built on two same-day pieces: B15's injected closures (the body is born at
  the caller's site) and **`Context.run` made generic over its return**
  (`run<U>(self, value: T, body: || U): U` — the `batch` shape; the threading
  rewrite already evaluated to `body(value)`, so the value was free;
  `run_with_owner` yields its body's value too). Open (recorded): whether a
  `View`-producing `comp` should fold its scope INTO the view's owner — the
  `std::ui` integration question below.
- **`std::ui` integration**: `View` construction under an ambient scope
  (bindings self-registering) — deliberately out of this slice, which touches
  `std::reactive` only.
- **Error-message anchoring**: the static fence's diagnostic points at the
  `get()` inside `std::reactive` when the uncovered path starts in user code;
  anchoring it at the uncovered root's call site is a diagnostics follow-up.
- **`get_safe`** (§2.1), with the `Option`-parameter sketch.

## 5. Context-typed closure parameters — SHIPPED 2026-07-07 (backlog B15)

The user-requested route back to `run_with_owner(owner, body)` as a plain
function: a closure TYPE that carries a context requirement, so a closure can
be *injected into* an extent instead of capturing at creation —

```vilan
fun run_with_owner(owner: Owner, body: (|| void) context owner_scope) {
    owner_scope.run(owner, body);
}
```

Semantics: a closure typed `(|| void) context owner_scope` defers its binding
for that context — its reads resolve to a hidden argument supplied AT EACH
CALL, not captured at creation. Creation sites are then free; CALL sites take
on the coverage demand (calling such a closure is a context read: the caller
must be covered, or itself annotated). The clause names the CONTEXT VALUE,
not its element type — `context owner_scope`, not `context Owner` — because
two contexts can share an element type and the binding must be unambiguous;
naming a value in a signature clause has precedent (`borrows self`). `run`
accepts an annotated closure VALUE for its matching context (the literal-only
restriction lifts for exactly that case: `run` supplies the deferred
argument). Compatibility: a context-free closure passes where an annotated
one is expected (the extra hidden argument is unread); a closure literal in
an annotated position defers instead of capturing; a closure that already
captured (created inside an extent) keeps its captured value — capture wins,
the deferred argument is unread.

**Multiple contexts** (settled 2026-07-07): the single form stays bare —
`context owner_scope` — and several are a parenthesized comma list:

```vilan
fn: (|| void) context (owner_scope, request_id)
```

A bare comma list is grammatically unavailable (the clause sits inside
parameter lists and generic argument lists, which own the comma), and
parens-with-commas is the language's value-list shape (call arguments,
tuples, `[derive(A, B, C)]`) — braces would echo import name-SETS, which
these aren't. A repeated clause (`context a context b`) was considered and
rejected: no clause repeats in the grammar today, and one clause node avoids
order/duplicate normalization questions. Two rules ride the written order:
the clause's declaration order IS the hidden-argument order at call sites
(deterministic, visible, formatter-preserved), and a duplicate context in one
clause is a compile error. This also sets the shape for any future
multi-value clause (`borrows (a, b)`, if destruction work ever needs it).

**Shipped shape** (v1): the clause is legal on a PARAMETER's closure type
(other positions are a clean error); it parses as a contextual keyword (no
lexer change — `std::context` paths and `context`-named values stay legal).
The pass validates the named values are contexts, treats each call through an
annotated parameter as a read (the fence covers uncovered callers), gives an
injected LITERAL its own hidden parameter (the run-closure machinery,
reused), threads the deferred argument at every call, lets `run` accept an
annotated VALUE whose clause is exactly its context (purging the rewritten
call's stale method records — a fresh substitution would monomorphize the
parameter as a function), and restricts the value's flow to the three places
threading can follow: a call, a forward to a parameter with the SAME clause,
or `run`'s body position. `std::reactive::run_with_owner(owner, body)` now
exists — the motivating API. Fixed alongside: a context that was created but
never read or run used to emit a dangling `Context::new()` call (the
news-only early path now lowers them). Eleven pins + the extended
`reactive-owner.vl` corpus program. Deferred: clauses on `let` annotations
and return types, and forwarding to a SUPERSET clause (v1 requires exact
match).
