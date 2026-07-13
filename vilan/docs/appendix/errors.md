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

**"`std::…` is in another platform's layer and is not available when building for …"**
You imported a module the current platform doesn't have — `std::ui` in a
node build, `std::db` in a browser build. If this is shared code, it must
stay platform-neutral; if it's app code, check the package's `target`.
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

## Async

**"`…` receives an async closure, but its type awaits nothing — declare it `async || T` (or return void for spawn semantics)"**
You passed a closure that suspends into a parameter typed as a plain,
value-returning closure. Either the parameter should be `async |…| T`,
or — if fire-and-forget is fine — the parameter's return type should be
`void`.
→ [Async](../tour/async.md), [Functions & closures](../tour/functions-and-closures.md)

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

**"found '{' expected …" pointing at a struct literal in a condition or operand**
Struct literals live at the top expression level only — `Point { … } ==
q` and `if Foo { … }` are parse errors by design. Bind the literal to a
local first.
→ [spec §3.8](../spec/grammar.md)
