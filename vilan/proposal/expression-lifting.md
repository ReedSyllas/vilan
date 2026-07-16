# Expression lifting — `a? + 10` and `a? + b?` (B11 deferred tail)

Status: **ACCEPTED 2026-07-16** — the five review questions resolved with the
user (§6). The §0.3 deferral of `try-and-lift.md`, designed. Everything here
layers on the shipped `?.`/`!` machinery; nothing changes for existing
programs (bare `?` is a parse error today, so the new form occupies empty
grammar space).

## 1. What it looks like

```vilan
// One `?` — map. The rest of the expression is the continuation.
let doubled = count? * 2;                 // Option<i32> — Some(n * 2) or None
let banner  = user?.name + "!";           // Option<str> — chain and operators in one region
let overdue = deadline? < now();          // Option<bool>

// The `?` may mark EITHER operand — the region rule is position-independent
// (the body is the whole region with the element at the hole):
let halved  = 2 * count?;                 // Option<i32> — count.map(|x| 2 * x)
let expired = now() >= deadline?;         // Option<bool> — note: now() still runs
                                          //   when deadline is None (§2 — operands
                                          //   LEFT of a `?` evaluate unconditionally)

// Two `?` — applicative. Good only if BOTH are good, left-to-right.
let total = price? + tax?;                // Option<i32>
let area  = width? * height?;             // Option<i32>

// Result: the FIRST bad half short-circuits out, carrying its error.
let sum = parse(a)? + parse(b)?;          // Result<i32, ParseError>

// Calls and members participate like any operator — but an ARGUMENT is its
// own slot, so `?` does not lift the call it is an argument of (§3):
let label = status?.describe() + suffix;     // Option<str> — lifted
let label = describe(status?) + suffix;      // ERROR: `?` lifts nothing here
                                             // (write `status?.describe()`
                                             //  or `status.map(describe)`)
```

And what it replaces:

```vilan
// today                                        // proposed
let total = price.and_then(|p| {                let total = price? + tax?;
    tax.map(|t| p + t)
});

match deadline {                                let overdue = deadline? < now();
    Some(let d) => Some(d < now()),
    None => None,
}
```

## 2. Semantics — the lift region

Postfix `?` on `expr: M` (where `M: Lift`, unchanged) lifts **the rest of the
enclosing expression, up to its slot root**, as the continuation:

- **Slot root** — the nearest enclosing *syntactic slot*: a `let`/`mut`
  initializer, a function/method **argument**, a `ret` value, a field value in
  a struct literal, a list/array/tuple element, an index expression, a
  condition, a match subject, or a block tail. The lifted result (`M<U>`)
  is what the slot receives.
- **One region, many `?`s.** Every `?` under the same slot root joins one
  region. The region evaluates left to right; at each `?` the container
  splits: good feeds the rest, **bad short-circuits the whole region with the
  bad half as-is** — receivers to the right are **not evaluated** (the `&&`
  / Rust-`?` precedent: `base()? + surcharge()?` never calls `surcharge`
  when `base()` is bad). This is `and_then` nesting ending in a `map`,
  spelled as one expression.
- **The `?` may sit on any operand — binary operators stay symmetrical.**
  `2 * count?` and `now() >= deadline?` lift exactly like their mirrored
  forms: the hole's position in the body is wherever the `?` is; nothing
  about the rule privileges the left operand. The one direction-sensitive
  fact is *evaluation order*, which is source order: everything **left** of
  a `?` has already evaluated by the time that receiver splits, so in
  `now() >= deadline?` the `now()` call runs even when `deadline` is bad —
  short-circuiting skips only what lies to the **right** of a bad `?`.
- **Typing.** With every `?`-receiver `M<T₁> … M<Tₙ>` and the body typing as
  `U` (each hole at its element type): the region is `M<U>` — unless `U` is
  the receivers' own container `M<V>`, which **flattens** to `M<V>` (the
  chain rule of `try-and-lift.md` §3, inherited unchanged).
- **All receivers must be the same named container**, and for `Result` the
  same `E` (reconciled, so unsuffixed literals and generics behave as
  everywhere else). Mixing `Option` and `Result` in one region is an error
  that points at §9's explicit converters (`.ok_or(err)`, `.map_err(…)`) —
  conversion stays visible, per the no-silent-conversion rule.

### What delimits a region (v1)

The region is a **flat expression**: operators, calls, member/index access,
literals, struct literals, list/tuple elements — but it never crosses:

1. **A slot boundary.** An argument is a slot, so `describe(status?)` does
   *not* lift `describe` — the region is just `status?`, which is the
   degenerate identity lift and therefore an **error** ("`?` lifts nothing
   here — the region is the whole argument; write `status?.describe()` or
   `status.map(describe)`"). This is deliberate (§3).
2. **Control flow.** `if`/`match` branches, loop bodies, and closure bodies
   are their own slots; a `?` inside one lifts within that branch only.
   A `?` in a *condition* or *match subject* lifts that subexpression — and
   then the condition is `M<bool>`, not `bool`, so the ordinary type error
   fires (with a hint: "the `?` lifted this condition to `Option<bool>`;
   match on it instead").
3. **`!`.** As in chains, `!` ends the region: `(a? + b?)`-then-`!` in
   `a? + b? !` — spelled `(a? + b?)!` for sanity — asserts on the lifted
   result. A `!` *between* two `?`s of one region (`a? + b!`) is rejected in
   v1: the continuation may not early-return from inside a lift (the same
   closure problem `!`-in-closures defers).

**Parentheses delimit the region** *(resolved in review — §6.2)*:
`(a?.b) + 1` means `(a.map(|x| x.b)) + 1` — the lift does not reach outside
the parens (and that particular expression is then the ordinary
`Option + i32` type error). This matches the chain-internal grouping rule
("escaping a group is parenthesization") and TS intuition. Implementation
consequence: the parser must *record* parenthesization (a lightweight
`Node::Paren` wrapper, or equivalent) so the walk-time region builder can
see the boundary — today redundant parens dissolve at parse. The wrapper is
region-delimiting and otherwise fully transparent (typing, lowering,
formatter idempotence all unchanged).

## 3. Why the region stops at slots (the rejected alternative)

The alternative — lift to the *statement* root, so `describe(status?)`
becomes `status.map(describe)` — was rejected for consistency: the shipped
chain form already does **not** lift through calls (`f(a?.b)` passes
`Option` to `f` today, and programs depend on it). Making bare-`?` lift
through the same position that `?.` doesn't would fork the mental model of
one operator, and silently rewriting a call an author wrote as
`describe(status?)` into `status.map(describe)` is exactly the "real
operation performed invisibly" the language refuses elsewhere. `?` rewrites
only the expression it is *part of*, never its callers.

## 4. Lowering — the shipped machinery, generalized

No new runtime, and for std containers no closures:

- **std fast path** (`Option`/`Result`): the same match-shaped inline form
  `?.` emits today — operands evaluate left-to-right into temps; each `?` is
  a tag branch; a bad tag makes the whole region's value the bad container
  **as-is** (`None` is `None` at any element; `Err(e)` rewraps at the region's
  success type exactly as `!`'s lowering does); the body computes on the
  aliased elements. `a? + b?` costs two branches — cheaper than the
  `and_then`/`map` closures it replaces.
- **trait path** (user `Lift` types): nested `and_then` calls ending in
  `map`, each continuation an IR-level closure over the remaining region —
  the user-`Lift` chain lowering, nested. Left-to-right, so effects order as
  written.
- **Analyzer shape**: parse bare postfix `?` as an ordinary postfix node
  (today's "must be followed by `.`" check removed); the *walk* groups every
  `?` under a slot root into one region with binder holes — the existing
  `Constraint::Lift`/`LiftBinder` machinery, extended from "postfix
  continuation" to "expression continuation". Chain form `a?.b.c` becomes
  the special case it always was: a region whose body is a member chain.

## 5. Interactions

- **`?.` chains**: unchanged meaning, one generalization — `a?.b + 1`
  (today a type error: `Option + i32`) becomes legal, the region absorbing
  the operator tail: `a.map(|x| x.b + 1)`. Existing well-typed programs are
  untouched (anything that compiles today keeps its meaning; the new
  meanings occupy what were errors).
- **`!`**: region delimiter (§2); composition `(region)!` works as today's
  `a?.parse()!` does.
- **Evaluation twice?** `size? * size?` evaluates `size` twice (two temps,
  two branches) — legal, value semantics make it a copy; the docs note it
  and recommend binding once when the receiver is a call.
- **LSP**: inside a region, the element (not the container) is the hovered/
  completed type at each hole — the existing `?.` behavior at more
  positions.
- **Formatter**: `?` prints tight to its operand, as `?.` does.

## 6. Resolved in review (2026-07-16, with the user)

1. **Slot inventory — argument-is-a-slot stands.** `?` stays local; lifting
   the enclosing call was judged too confusing. `describe(status?)` errors.
2. **Parens delimit the region.** `(a?.b) + 1` = `(a.map(|x| x.b)) + 1` —
   the lift never reaches outside parens. Requires recording parens in the
   AST (§2's implementation note).
3. **Identity lift is a hard error.** `let x = a?;` / `f(a?)` — "`?` lifts
   nothing here", with the `?.`/`map` steer.
4. **Lazy right operands confirmed.** Receivers right of a bad `?` do not
   evaluate; receivers/operands left of it already did (source order).
5. **`Result` regions preserve `E`.** `result? + 1` on `Result<i32, E>` is
   `Result<i32, E>` — the bad half rides through unchanged. Corollary: all
   `Result` receivers in one region must share that `E` (one result type),
   so `E1`/`E2` mixing is the `.map_err(…)` -hinted error, exactly as at `!`.

## 7. Test plan (per case, as always)

Map region (`count? * 2` — Some and None, runtime-pinned); the mirrored
forms (`2 * count?`, `now() >= deadline?`) with an **effect-order pin** that
a call LEFT of a bad `?` still runs; applicative both
good / left bad / right bad, with **effect-order pins** (right receiver not
called on left-bad); `Result` first-error-wins + same-`E` mismatch (with the
`.map_err` hint); mixed `Option`/`Result` region rejected (`.ok_or` hint);
flatten (body yields the container — pinned by type, `M<V>` not `M<M<V>>`);
chain-tail generalization `a?.b + 1`; slot boundaries (`describe(status?)`
identity error; `?` in a condition errors with the hint; branch-local
region); **paren delimiting** (`(a?.b) + 1` is the Option+i32 type error;
`(a? + b?)` groups as one region; formatter idempotence over recorded
parens); `!` between `?`s rejected; `(region)!` composes; twice-evaluated
receiver; user-`Lift` type through the trait path (effects ordered);
corpus byte-identical (nothing uses bare `?` today); docs page + gotcha
entry; interpreter equivalence for every runnable pin.
