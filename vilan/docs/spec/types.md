# Spec §5 — The type system

## 5.1 Types

The type forms (grammar §3.9) denote:

- **Nominal types** — structs and enums, possibly generic
  (`Task`, `Option<i32>`, `Map<str, List<i32>>`). Two nominal types are
  equal iff they name the same declaration and their arguments are equal
  — there is no structural typing of nominals.
- **Primitives** — `bool`, `str`, `i8 i16 i32 i53 u8 u16 u32 u53`,
  `f32 f64`, `BigInt`. Declared in std as external structs; nominally
  distinct (no implicit numeric conversions, §5.8).
- **Tuples** — `(T, U, …)`; structural: equal iff element-wise equal.
  `()` and one-element tuples do not exist as distinct types (`(T)` is
  `T`; the unit is `void`).
- **Closure types** — `|T, U| R`, `|| R`, `|| void`; structural in their
  parameter and return types. An `async` closure type (§7.4) and a
  `context`-claused type (§8, Phase B) are distinct from their plain
  counterparts.
- **View types** — `&T`, `&mut T` (§6). Views are second-class: these
  types appear in parameter and return positions and in short-lived
  locals only.
- **`void`** — the unit: one value, also written `void`.
- **`any`** — the dynamic top type, produced at host boundaries; it
  unifies with every type (absorbing).
- **`Never`** — the type of diverging expressions (`panic(..)`, `ret ..`,
  `jump break`/`continue`). Never unifies by *yielding*: a diverging
  match leg or if branch doesn't constrain the construct's type, and a
  `Never` value satisfies any expected type. Internal — not written in
  source.
- **Generics** — a bound type parameter in scope (`T`) is a type; it is
  abstract within its binder's body.

## 5.2 `null`

`null` is not a member of ordinary types. It exists for host
interoperability (an extern that may return JS null); std APIs flatten it
at the boundary (`Option`, or a documented sentinel like `storage::get`'s
`""`). A conforming program cannot assign `null` to a non-host type.

## 5.3 Declarations

A `struct` introduces a nominal product type; field types are mandatory
in non-external structs. An `enum` introduces a nominal sum type; each
variant is a constructor (with payload types) and a static member of the
enum. An `external struct` declares a host type: no fields, its surface
defined entirely by externs in impls.

## 5.4 Impls

`impl Subject { … }` adds **inherent** members to `Subject`;
`impl Subject with Trait { … }` provides `Trait` for `Subject`. The
subject is a type pattern whose `type X: Bounds` binders declare the
impl's generics:

```vilan,fragment
impl List<type T: PartialEq> { … }      // for every List<T> where T: PartialEq
impl Signal<Signal<type U>> { … }       // only for nested signals
impl type T: Display { … }              // blanket: every T that is Display
```

An impl applies to a concrete type when the pattern matches it and every
binder's bounds hold. Members may be functions (with or without `self`)
— a function without `self` is a **static**, reached as
`Subject::name(…)`.

*Implementation note (soundness gap, tracked): a conditional impl's
bounds are not yet re-checked when the impl is selected through a
GENERIC bound — `List<B>` can satisfy a `Marker` bound via
`impl List<type T: Marker>` even when `B` lacks `Marker`. Derive-based
checks (`Wire`, `Json`) therefore verify field trees syntactically rather
than through trait bounds.*

## 5.5 Traits

A trait declares required methods (signature-only) and defaults (with
bodies). `trait X with Y` makes `Y` a supertrait: implementing `X`
requires `Y`. A trait's generic parameters may carry defaults
(`trait PartialEq<B = Self>`); `Self` in a trait body denotes the
implementing type. Traits are used as **bounds**; a trait is not a type —
`let x: Display` is a compile error (no trait objects).

Trait members resolve on a value when exactly one visible impl provides
the name; a trait default is inherited by impls that don't override it.

## 5.6 Generic binding and inference

Type checking is **expectation-directed**: every expression is checked
against an expected type (possibly unknown), and expectations flow inward
(a `let` annotation to its initializer, a parameter type to its argument,
a field's declared type to its initializer value).

For a call `f(a₁ … aₙ)` where `f` has generic parameters:

1. Each parameter type is unified with its argument's type; unification
   of a generic parameter with a concrete (or caller-generic) type
   **binds** it. Bindings are per-call.
2. A generic mentioned only in the return type is bound by unifying the
   declared return type against the call's expected type
   (`let c: Cell<i32> = Cell::fresh()` binds `T := i32`). Only the
   **callee's own** binders participate — a caller-side generic
   introduced by substitution is never re-bound against the expectation.
3. After binding, every bound's satisfaction is checked; an unsatisfied
   bound is an error naming the parameter and bound.
4. A call whose generics cannot all be grounded (no argument or
   expectation determines them) is an error at the call.

Method calls additionally bind the receiver's impl binders from the
receiver's type before the parameters are considered. Closure arguments
participate: a closure's parameter types take the callee's expectations,
and its return type may ground the callee's generics; resolution defers
until the closure's body has typed.

Generic code is **monomorphized**: each distinct binding vector of a
generic function/impl produces its own specialization; dispatch is
static. A program that would require an unbounded set of specializations
(polymorphic recursion) is not required to compile.

## 5.7 Operator and method dispatch

The operators dispatch through lang-item traits (appendix §A.4):
`+ - * / %` and the bit/shift operators through `Add`/`Sub`/…;
`== !=` through `PartialEq`; `< <= > >=` through `PartialOrd`. The
left operand's type selects the impl; the trait's `B` parameter types the
right operand (default `Self`); the result type is the impl's (for the
arithmetic traits, `Self`). Compound assignment `x op= e` is exactly
`x = x op e` with `x`'s place evaluated once.

`is` (§3.7 level 10) tests a value against a match pattern and yields
`bool`; bindings inside an `is` pattern are scoped to nothing (use
`match` to bind).

## 5.8 Conversions and coercions

There are **no implicit conversions** between numeric types; use the
`as_*` methods (value-converting, Rust-`as` semantics). There is **one
implicit coercion**: a reference to a plain named function coerces to a
matching closure type —

```vilan,fragment
let transform: |str| i32 = measure;    // fun measure(text: str): i32
words.map(measure);
```

Eligibility: a non-generic, non-method, non-`async`, non-`external`
`fun` whose signature equals the target closure type. Everything else
(generics, methods, async functions, externs) requires an explicit
wrapping closure.

`any` unifies with every type in both directions (it is produced by
`panic` and host boundaries; it absorbs rather than converts).

## 5.9 Variadic tuples

A generic parameter with a **tuple bound** ranges over tuples:
`T: (2..)` (arity ≥ 2), `T: (..: Display)` (every element `Display`).
A **mapped type** `(U in T: F<U>)` denotes the tuple obtained by mapping
each element `U` of `T` to `F<U>` — `combine`'s signature is the
canonical use:

```vilan,fragment
fun combine<T: (2..)>(sources: (U in T: Signal<U>)): Signal<T>
```

A **tuple comprehension** `(x in xs => e)` is the value-level mapping
form. *Implementation note: arity and per-element bounds are parsed but
not yet enforced (tracked); `keyof` and spread parameters are recorded
future work.*

**Positional access** `t.0`, `t.1` (chaining as `t.0.1`) types as that
element and, through a `mut` binding, assigns it. Tuples store flat: a
tuple-typed element occupies its elements' slots, so accessing one
yields its region as a value (destructuring reads the same layout).

## 5.10 `!` and `?.`

Both dispatch through lang-item traits and desugar per expression:

- `e!` — **try-assert**. With `v = e` of a type implementing
  `Try<T, B>`: if `v.verdict()` is `Good(t)`, the value is `t`;
  if `Bad(b)`, the enclosing function returns
  `R::from_bad(b)` where `R` is its declared return type (which must
  implement `Try<_, B>` compatibly). `Option` and `Result` implement
  `Try` in std.
- `e?.m…` — **lift**. With `v = e` of a container type implementing
  `Lift` + `Try`: if `v` is good with value `t`, the continuation
  (`.m` and the following plain postfixes, §3.6) applies to `t`; the
  result re-wraps in the container — unless the continuation itself
  yields the container type, in which case it is returned as-is
  (flattening). If `v` is bad, the container passes through unchanged.

```vilan
import std::print;
import std::option::Option::{ self, Some, None };

fun main() {
	let word: Option<str> = Some("dune");
	print((word?.len()).unwrap_or(0));       // 4  — lift + rewrap
	let missing: Option<str> = None;
	print((missing?.len()).unwrap_or(0));    // 0  — bad passes through
}
```

## 5.11 Type errors of note

Normative rejection cases (each is a compile error):

- Using a trait as a type (`let x: Display = …`).
- An unsatisfied bound at a call (`generic parameter 'T' is missing the
  bound …`).
- A `match` whose VALUE legs' types don't unify. Diverging legs
  (`ret`, `panic`, `jump`) are `Never` and don't participate (§5.1).
- An `i53`/`i32` operand mix (no implicit widening — suffix the
  literal).

*Implementation note (tracked gaps): a closure bound to a local and
called directly does not infer its parameter types from the call;
`effect`'s unannotated closure parameter can type against the impl's
abstract `T` (B23); chained element access on a call result can lose the
element type. Each has a pinned test; the workaround is an annotation or
a binding.*
