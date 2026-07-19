# Error index

You saw an error; this page says what it means and where to go. Messages
are quoted the way the compiler prints them, with `…` standing in for the
parts that vary. Find yours with a page search.

(Organized companion: the [gotchas checklist](gotchas.md) covers traps by
topic rather than by message.)

## Names and imports

**"cannot find '…' in this scope"** · **"cannot find type '…'"**
The name isn't visible here. Usually a missing `import` — remember that
everything, even `print`, is imported explicitly. If you did import it,
check for a typo or a shadowing local.
→ [Hello vilan](../tour/hello-vilan.md), [spec §4](../spec/names.md)

**"`std` is a namespace, not a value — import the module first …"**
You wrote a qualified path like `std::math::min(1, 2)` inline. That
spelling isn't supported. Import the module, then qualify through its
name: `import std::math;` and `math::min(1, 2)`.
→ [Hello vilan](../tour/hello-vilan.md)

**"`…` requires the `…` layer of `std` and cannot run on `…`"**
Code reachable from this build's entry calls into a module the platform
doesn't have — `std::fs` from a browser build, `std::dom` from a node
build. The error lists the call chain from `main` to the crossing.
Importing the module is not the problem (imports are free); reaching it
is. Move the call behind the right entry, or check the package's
`target`.
→ [Platforms](../tour/platforms.md)

**"`…` requires … and cannot run on `…` / reachable from `…`, fenced `[platform(…)]`"**
A function declared a platform fence and something it (transitively)
reaches requires a layer one of the fenced platforms doesn't serve. The
chain shows the path from the fence. Fences check on every compile —
narrowing the fence, or moving the colored call out from behind it, are
the two fixes.
→ [Platforms](../tour/platforms.md)

**"cannot find module '…' to import"**
The path names a module file that doesn't exist. `pkg::routes` means
"`routes.vl` in this package's source root" — check the file name and
the package you're in.
→ [Hello vilan](../tour/hello-vilan.md)

## Types and generics

**"Expected …, but got … instead."**
The general type mismatch. One special case surprises people: an `i53`
mixed with a bare integer literal — the literal is `i32`, and there are
no implicit conversions. Suffix it (`stamp + 1000i53`).
→ [Values and types](../tour/values-and-types.md)

**"generic parameter '…' is missing the bound ': …' required by this call"**
You called something that needs a capability (say `PartialEq`) with a
generic parameter that doesn't declare it. Add the bound to *your*
signature: `fun caller<U: PartialEq>(…)`.
→ [Data and traits](../tour/data-and-traits.md)

**"cannot call method '…' on …"**
The value's type doesn't have that method. If the type is a generic
parameter, you probably need a bound. If it says something like
`|i32| i32`, you're calling a method on a closure — often a sign a
different value was passed than you think.
→ [Data and traits](../tour/data-and-traits.md)

**"'…' does not implement trait '…': missing '…'"**
An `impl … with Trait` doesn't provide every required method, or a bound
demands a trait the type never implemented.
→ [Data and traits](../tour/data-and-traits.md)

**"match is not exhaustive: missing …"** · **"match is not exhaustive: add a catch-all `_` leg"**
Some variants have no arm. Handle them or add `_ => …`. This error is
the feature: it's what fires everywhere when you add a variant.
→ [Control flow](../tour/control-flow.md)

**"struct '…' has no field '…'"** · **"variant '…' does not belong to the matched enum"**
A field or variant name is off. For the variant case inside `match`,
remember patterns bind with `let` — a bare misspelled variant is an
error here, never a silent catch-all.
→ [Control flow](../tour/control-flow.md)

**"`…` compares two values of the same type, but the operands are `…` and `…`"**
Comparisons follow the trait model (`==` is `PartialEq`, `<` is
`PartialOrd`): the right operand must be the left's type, and there are
no implicit conversions. An unsuffixed literal adapts to its peer
(`stamp < 3` is fine for an `i53` stamp); two differently-typed
*variables* need a suffix or an `as_*` conversion. Related:
**"`bool` has no ordering"** (compare with `==`/`!=`) and
**"`&&` takes `bool` operands"** (vilan has no truthiness).
→ [Values and types](../tour/values-and-types.md)

**"type '…' does not implement the `…` operator; add `impl … with …` providing `…`"**
An operator was used on a type without the matching trait impl — `+`
needs `Add`, `==` needs `PartialEq`, `<`/`<=`/`>`/`>=` need
`PartialOrd` (implement `partial_compare` once; the operators dispatch
through it, and `lt`/`le`/`gt`/`ge` come free as defaults).
→ [Data and traits](../tour/data-and-traits.md)

**"the literal `…` is out of range for `…` (…)"**
The number doesn't fit the type. For `i53`/`u53` the range is ±2^53 —
JavaScript's exact-integer window. Bigger integers take `BigInt` (`7n`).
→ [Values and types](../tour/values-and-types.md)

**"unknown numeric suffix `…`"**
The letters after the number aren't a type. If it says `i64` or `u64`:
those were renamed to `i53`/`u53`.
→ [Values and types](../tour/values-and-types.md)

**"type of … could not be resolved"**
Inference gave up somewhere upstream — this error is usually the *echo*
of another one, so fix the first error in the list. When it appears
alone, an annotation at the binding usually grounds it.
→ [gotchas](gotchas.md)

## Memory and mutation

**"cannot mutate immutable '…'"**
The binding was declared with `let`. Declare it `mut` — or, if you're
inside a method, take `&mut self`.
→ [The memory model](../tour/memory-model.md)

**"a view cannot escape its scope: it may not be returned, stored in a field, placed in a collection, or carried in an enum payload. …"**
Views (`&x`, `&mut x`) are short-lived by design: lend, use, done. To
keep a reference around, store a plain value, a `Handle` into an
`Arena`, or a `Shared` cell.
→ [The memory model](../tour/memory-model.md)

**"cannot hold a view across 'await': '…' is still live here. …"**
Your function suspends while a view is live, and whatever it points into
could change during the pause. Re-derive the view after the await
(`rows[i].field` again) instead of keeping it.
→ [The memory model](../tour/memory-model.md), [Async](../tour/async.md)

**"view binding '…' cannot be `mut`: a view cannot be rebound. …"**
`mut v = &mut x` doesn't mean what it would in Rust. Declare the view
with `let`; assigning through it (`v = …`) already writes the target.
→ [The memory model](../tour/memory-model.md)

## Resources

A `resource` type has a single owner and moves rather than copies; a
struct, enum, or tuple holding one is a resource too, inferred by
containment (`Option<Database>` is a resource, `Option<i32>` is not). A
resource *moves* on binding (`let b = a`), on `own`-passing, on return, and
into a constructor; it is *loaned* — no ownership change — through `self`,
`&`, and `&mut`. The `Drop` destructor trait and its restrictions are below;
the compiler-inserted teardown that runs `drop` at each scope end lands in
later work.

**"use of `…` after it was moved — a resource has a single owner"**
The binding was moved (bound to another name, passed to an `own`
parameter, returned, or matched by value) and then used again. The note
points at the move. Loan it instead (`&x` / `&mut x`, or a method call),
or, if you really need two owners, restructure with `Option` + `take`.
→ [The memory model](../tour/memory-model.md)

**"cannot move a resource field out of a live aggregate — … no partial moves …"**
`let x = s.db`, or passing / returning `s.db` by value, would move a
resource out of a struct that is still alive — v1 has no partial moves.
Loan the field (`&s.db`, `&mut s.db`, `s.db.method(…)`), or make the field
an `Option<…>` and `take()` it out.
→ [The memory model](../tour/memory-model.md)

**"`…` is moved on one path through this branch but not another — …"**
An `if`/`match` moves the binding on some paths and not others, so its
end-of-scope ownership isn't static (v1 has no runtime drop flags). Move it
on *every* path, or on none — or hold it in an `Option` and `take()` on the
path that consumes it. A diverging leg (one that `ret`s or `jump`s out) is
exempt: it never reaches the merge.
→ [The memory model](../tour/memory-model.md)

**"`…` is declared outside this loop and moved inside it — …"**
Moving a binding from a loop body would move it again on the next
iteration. Move a value declared *inside* the loop, or loan the outer one
(`&x` / `&mut x`).
→ [The memory model](../tour/memory-model.md)

**"a closure cannot capture the resource `…` — …"**
A closure or `async`/spawn body referenced a resource from an enclosing
scope; capturing it would give the closure a second owner. Pass a loan into
the call, or give ownership to the struct that owns the closure's lifetime.
(A closure's own *parameter* is per-call, not a capture — injected
`context`-clause bodies are unaffected.)
→ [The memory model](../tour/memory-model.md)

**"`…` is not move-clean when instantiated with a resource — …"**
A generic function or method was called with a resource type argument
(`Option<Database>`, `wrap(db)`), and its body — checked with that type
parameter treated as a resource — breaks the affine rules: it uses a
value of the parameter's type more than once, moves it on some paths but
not all, or captures it in a closure. The error is spanned at the **call**
(the instantiation), and a note points into the generic's body at the
offending use — the body is fine for data, but a resource has a single
owner. A generic that means to accept resources must move each such value
at most once (as `Option::unwrap(self): T` does), never copying or
capturing it. Instantiating the same generic at a data type is unaffected.
→ [The memory model](../tour/memory-model.md)

**"the resource `…` cannot be used where `any` is expected — …"**
`any` is a data sink, and a resource must keep its single owner: passing
one to `print`, binding it to `let x: any`, or returning it as `any`
would launder the discipline away. Debug-print the resource's fields
instead.
→ [The memory model](../tour/memory-model.md)

**"`…` cannot hold the resource `…` — a native container's internals are host code …"**
`List`, `Map`, `Set`, and the external generics (`Shared`, `Task`,
`Promise`, `Context`) can't hold a resource in v1 — the move checker
can't see inside host storage. `Option` is the sanctioned resource
container; or keep the resource in a struct field.
→ [The memory model](../tour/memory-model.md)

**"field `…` of `[derive(Wire)]` / `[derive(Hashable)]` / `[derive(PartialEq)]` type `…` is the resource `…` — …"**
A resource is not plain data: it cannot be sent over the wire, hashed by
value, or compared by copy. Drop it from the derived type, or carry a
plain-data handle (an id, a key) in its place.
→ [The memory model](../tour/memory-model.md)

**"`…` implements `Drop` but is not a resource — … declare it a `resource` …"**
`Drop` — the destruction hook — may be implemented only for a `resource`
type. A destructor without move discipline is exactly the double-close bug:
copy the value and each copy would run `drop`. Declare the type `resource`
so it moves instead of being copied. (Plain-data, framework-driven teardown
uses the cooperative `Disposable` protocol, not `Drop`.)
→ [The memory model](../tour/memory-model.md)

**"`drop` for `…` is async — teardown must be synchronous …"**
A `drop` body may not be `async`, nor await (call an async function): a
destructor runs synchronously in v1. Cancel owned tasks through an
`OwnedNursery` — whose own `drop` cancels them — rather than awaiting them.
Awaited teardown is a future design.
→ [The memory model](../tour/memory-model.md)

## Async

**"`…` receives an async closure, but its type awaits nothing — declare it `async || T` (or return void for spawn semantics)"**
A closure that suspends was stored into a struct field typed as a
plain, value-returning closure (at the literal or a later assignment).
Either the field should be `async |…| T`, or — if fire-and-forget is
fine — its return type should be `void`. (A plain *parameter* no longer
produces this error: it adapts — the callee instantiates an async copy
that awaits the callback.)
→ [Async](../tour/async.md), [Functions & closures](../tour/functions-and-closures.md)

**"`…` requires a synchronous closure (`sync`): its completion is part of the declaring function's synchronous protocol …"**
The parameter is a `sync` contract position — `Signal::map`,
`set_with`, `turn`/`batch` bodies, the UI render callbacks — where the
callback must finish inside a synchronous protocol, so it cannot adapt.
Move the async work outside the callback: an explicit `turn(…)` whose
awaiting body holds one turn across its awaits, `Draft`/`optimistic` for local-first commits, or a
spawned `async { … }` block. The transitive form ("this call passes an
async closure that reaches `…`") points at the call that made the
closure async and notes where it was forwarded.
→ [Async](../tour/async.md), [Reactivity](../guide/reactive.md)

**"`…` is a host (`external`) function — it cannot await a vilan closure …"**
Host code can't await your callback, so an `external` function's
value-returning closure parameters only accept synchronous closures
(void-returning ones spawn, as everywhere). A parameter *declared*
`async |…| T` is exempt — that is the host's explicit contract to await
the closure itself.
→ [Async](../tour/async.md)

**"an async closure cannot adapt a trait/generic-dispatched call …"**
Adaptation instantiates a statically-known callee, and a
trait/generic-dispatched call doesn't have one — the concrete method
varies per instantiation. Bind the receiver concretely before the call,
or declare the trait method's parameter `async || T` so every impl
takes the typed channel.
→ [Async](../tour/async.md)

**"`…` returns an async closure, but its declared return type awaits nothing — declare it `async || T` (or return void for spawn semantics)"**
The function's declared return type is a plain, value-returning closure,
but a `ret` (or the tail) hands back a closure that suspends. Mark the
return type `async || T` so calls through the returned value await —
`make()()` and `let go = make(); go()` both do — or return a
`void`-returning closure for spawn semantics.
→ [Async](../tour/async.md)

**"the initializer of `…` calls `…`, which is async — a module-level binding cannot await"**
A top-level `let` runs when the module loads, and module initialization
is synchronous — there is no enclosing function to become async, so the
value would be a live promise wearing the wrong type. Wrap the work in
a function and call it from `main`. The variant "the initializer of
`…` runs a closure that awaits" is the same rule when the awaiting
thing has no name — an adopted async closure applied directly, a
`run(value, body)` whose body suspends, or a `nursery` at top level.
(Creating an async closure at top level is fine; it awaits nothing
until called.)
→ [Async](../tour/async.md)

**"`!` requires the nearest enclosing function to declare an `Option`/`Result`-compatible return type …"**
`!` propagates the failure by *returning* it, so the surrounding
function must return an `Option`/`Result` that can carry it. Inside a
closure or a UI handler, `match` instead.
→ [Control flow](../tour/control-flow.md)

## Contexts and UI

**"context `owner_scope` is read here, but this code can be reached without an enclosing `run`"**
The most common first UI error: you built reactive state (an `effect`, a
binding) outside every ownership boundary. Wrap the entry point in
`mount_root` — or `run_with_owner` in a test.
→ [Building UI](../guide/ui.md), [Reactive state](../guide/reactive.md)

**"`…` reads context `…`, so it can't be used as a value"**
A function that reads an ambient context (like the current owner) can't
be passed around as a plain closure — the context channel would be
severed. Wrap it in a closure literal at the use site instead.
→ [Functions & closures](../tour/functions-and-closures.md)

**"an injected (`context`-typed) closure can only be called, forwarded …, or passed to `run`"**
Injected closures (the ones with `context` clauses in their type) are
deliberately restricted so the ambient value can always be threaded to
them. Don't store them; call or forward them.
→ [Functions & closures](../tour/functions-and-closures.md)

**"unused result of a `[must_use]` call: bind it (e.g. `owner.take(…)`), or `let _ = …` to discard."**
The call returns something that stops working if you drop it (a
`Subscription`, typically). Keep it, hand it to an owner, or discard it
on purpose with `let _ = …`.
→ [Reactive state](../guide/reactive.md)

## Wire and rpc

**"field `…` of `[derive(Wire)]` type `…` is `…`, which is not Wire — …"**
Something unserializable (a closure, a `Signal`) is inside a payload
type. Wire types carry data only: scalars, `str`, `bool`,
`List`/`Option` of Wire, and other Wire types.
→ [Services & RPC](../guide/services.md)

**`RpcError::Contract` at connect time**
Client and server were built from different versions of the service.
Rebuild both. During development, a *leaked old server* still holding
the port is the usual culprit — `ss -tlnp | grep <port>` and kill it.
→ [Services & RPC](../guide/services.md), [gotchas](gotchas.md)

**`RpcError::Transport("not connected")` / `("connection lost")`**
The connection is down (fail-fast) or dropped mid-call (in-flight
rejection). Nothing is retried automatically, because your rpc might
not be safe to repeat. Retry at the app level if that's correct — a
draft's next push already does.
→ [Services & RPC](../guide/services.md)

## Compile-time evaluation

**"`asset::emit` outside a `const` expression"**
Styles (and other build assets) are constructed at compile time. Build
the `Style` in a `const` (`let card = const style()…`); select and merge
already-built styles at runtime.
→ [Styling](../guide/styling.md), [Macros & const](../tour/macros-and-const.md)

**"a `const` result must be plain data; this evaluates to …"**
The `const` expression produced something that can't be baked into the
output (a closure, a host object). Fold values, not behavior.
→ [Macros & const](../tour/macros-and-const.md)

## Syntax

**A struct literal in a condition parses as the block**
Struct literals are ordinary operator operands (`Point { … } == q`
compares), but condition positions exclude them — after `if Foo` or a
`match` subject, the `{` is the block/arms, by design. Written without
parentheses, `if p == Point { … } { … }` leaves a bare `Point` as the
condition's operand, which reports **"`Point` is a type, not a value"**.
Parenthesize the literal: `if p == (Point { x = 1 }) { … }`.
→ [spec §3.8](../spec/grammar.md)

**"`Name` is a type, not a value"** (also *"a trait / a type parameter /
a module, not a value"*)
A type, trait, type parameter, or module name was used where a value is
expected (`let q = Point;`). A type names a kind, not a runtime value —
construct it (`Point { … }`), name a variant (`Color::Red`), or call a
static (`Point::new(…)`).
→ [spec §4.2](../spec/names.md)
