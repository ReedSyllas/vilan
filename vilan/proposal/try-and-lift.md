# `!` and `?` — early return and lifted chains (backlog B11)

Status: **proposal** (2026-07-04, not implemented). The two-operator design and the four
refinements below are **agreed**; the mechanism sections marked *(recommendation)* are the
implementation shape proposed for review. Ships in two slices: `!` first (the ergonomics
workhorse), `?.` second.

## 0. The split, and the settled decisions

Rust folds two different jobs into one `?`: *bail out early* and *keep working inside the
container*. Vilan splits them:

- **`expr!`** — *assert the value is good, secured by a return*: evaluate `expr`; if it is
  good, the expression is the unwrapped value; if bad, **return the bad half from the
  nearest enclosing callable**. Rust's `?` semantics under a more assertive glyph.
- **`a?.b.c(d)`** — *lifted member chains*: apply the rest of the chain to the value
  *inside* the container, staying inside it. TypeScript's `?.` shape with honest monadic
  semantics — and, like every mainstream `?.`, **flattening**.

Settled up front (from review):

1. **`!=` always lexes as not-equals.** Postfix `!` followed by `=` requires the space:
   `a! = b` assigns an unwrapped value; `a!=b` is a comparison. `a! == b` needs its space
   too (`a!==y` is a lex error — `!=` then `=`). The formatter always emits the space; the
   parser's error for the `!==`-soup case should hint at it.
2. **`?` flattens.** When the chain's continuation produces the receiver's own container
   type, the result is one level, not nested (`a?.get(1)` on `a: Option<List<T>>` is
   `Option<T>`, not `Option<Option<T>>`). Semantically `map` + `flatten`, i.e. `and_then`.
3. **Expression-level lifting is deferred.** `a? + 10` (reinterpreting an enclosing
   arbitrary expression as the closure body) and the applicative form (`a? + b?`) are
   *not* in scope; `a.map(|x| x + 10)` stays the spelling. `?` is valid only as `?.` — a
   link in a member/call chain.
4. **Both operators are *operators*, not source-text macros.** They dispatch through
   declared operator implementations (the `Add`/`PartialEq` model), so `Signal`, `Promise`,
   or a user type can implement them; the compiler lowers the std cases directly. `!`'s
   *meaning* is fixed — return-when-bad — but *what "bad" is* is programmable per type.

## 1. Motivation

P6 made `Result` the dominant type at every user-facing seam: every generated stub call,
every `decode`, every `connect`. The examples grew `report(...)`-style helpers purely to
hide match boilerplate:

```vilan
// today                                          // with !
match client.add(label) {                         let id = client.add(label)!;
	Ok(let id) => use(id),
	Err(let error) => {                           // with ?.
		print(error.debug());                     let name = user?.profile.name;
		ret;                                      // today: user.map(|u| u.profile.name)
	},
}
```

`!` also unblocks I3's remaining half (validating per-type decode wants `Result`-returning
`from_json` that call sites can propagate tersely) and would simplify *generated* dispatcher
and stub code as much as hand-written code.

## 2. `expr!` — assert-or-return

### Semantics

`expr!` where `expr: M` and `M` implements `Try`:

1. Evaluate `expr` once.
2. Split it by the type's `Try` implementation into **good** (`T`) or **bad** (`B`).
3. Good: the whole `expr!` has type `T`, value = the good half.
4. Bad: **return from the nearest enclosing callable** (the B10 rule — the same boundary
   `ret` uses) with the bad half rewrapped in the callable's return type.

### The `Try` seam *(recommendation)*

"Bad" is programmed by implementing the operator trait:

```vilan
enum Verdict<T, B> {
	Good(T),
	Bad(B),
}

trait Try<T, B> {
	// Split: is this value good (yielding T) or bad (yielding the residual B)?
	fun verdict(self): Verdict<T, B>;
	// Rebuild a value of Self from a residual — how a bad half returns.
	fun from_bad(bad: B): Self;
}

impl Option<type T> with Try<T, void-ish> { .. }   // Bad = the absence itself
impl Result<type T, type E> with Try<T, E> { .. }  // Bad = the error
```

- **v1 compatibility rule:** the nearest callable's declared return type must be the
  **same named type** as the receiver — `Option<_>` inside an `Option`-returning function
  (any element: the bad half is `None`, which fits every `Option<U>`), `Result<_, E>` with
  the **same `E`** inside a `Result`-returning function (`Err(e)` re-wraps at any success
  type). No `Option` inside `Result`, no error conversion — a `From`-style conversion layer
  is the recorded follow-up, not v1.
- **Why `from_bad` isn't enough generally:** `from_bad(bad): Self` returns the *receiver's*
  `Self` (`Option<i32>`), while the enclosing function may return `Option<str>`. Vilan has
  no higher-kinded types to say "same constructor, other element", so for the std pair the
  compiler rebuilds directly (`None` / `Err(e)` at the enclosing type's arguments), and for
  **user `Try` types v1 requires the enclosing return type to equal the receiver type
  exactly**. Stated limitation, loosened if associated-type machinery ever lands.
- **Where `!` is legal (v1):** inside a *function* whose declared return type satisfies the
  rule. Inside a closure or `async` block: a clean compile error for now — closures' return
  types are inferred, and B10 deliberately left `ret`-in-closures unchecked. **First
  follow-up** (not v1): allow `!` where the closure's return type is contextually known —
  the motivating case is RPC handler closures (`|request| { ... }` returning `RpcOutcome`,
  which would carry its own `Try` impl so a handler can write `let n: i32 = arg(request)!`).
  B10's return-position checking is what makes every one of these cases *diagnosable*.

### Grammar & lexing

- Postfix, binds tighter than prefix `!` (logical not) and all binary operators;
  chains left-to-right: `a!.b!` unwraps twice, `config().port!` applies to the call result.
- The `!=` rule from §0. The only reserved pair: `!=` wins; everything else about postfix
  `!` is whitespace-insensitive.
- The glyph deliberately diverges from Swift/Kotlin (`!` = trap there). Vilan's postfix `!`
  **never panics** — trapping stays spelled `.unwrap()`. The docs own this loudly.

## 3. `a?.b` — lifted member chains

### Semantics

`?` appears only as `?.` — a link in a member/call chain. The segments **from one `?` to
the next `?` (or the chain's end)** form one continuation:

```vilan
a?.b.c(d)          // chain(a,  |x| x.b.c(d))
a?.b.c(d)?.e       // chain(chain(a, |x| x.b.c(d)), |y| y.e)
```

Each `chain(recv, k)` is typed by the continuation's result:

- `k: |T| U` where `U` is **not** the receiver's container → **map**: result `M<U>`.
- `k: |T| M<V>` (the receiver's own named type) → **map + flatten**: result `M<V>`.

This is the flattening every mainstream `?.` has (settled, §0.2): `a?.get(1)` on an
`Option<List<T>>` is `Option<T>`. "The receiver's own container" = the same struct/enum id
— the analyzer's ordinary nominal check, no higher-kinded reasoning needed.

- **Not an assignment target:** `a?.b = x` is a parse error (v1; matches TS).
- **Bare `a?`** (no following `.`) is a parse error — it would be `map(identity)`.
- Mixing is natural and ordered postfix-left-to-right: `a?.parse()!` lifts, then
  asserts-or-returns on the lifted result.

### The `Lift` seam *(recommendation)*

Opt-in, so `?.` doesn't silently work on everything that happens to have a `map`:

```vilan
trait Lift {}                      // the marker: this type supports `?.`
impl Option<type T> with Lift {}
impl Result<type T, type E> with Lift {}
```

The operator then resolves the receiver's **`map`** and **`and_then`** methods by the
ordinary method machinery (the `for … in` / `next()` duck-typed-protocol precedent) and
picks per the flattening rule. A type opting in supplies those two methods with the usual
shapes; `Signal` (derived signals: `signal?.field` — its `and_then` is exactly the A4
`flatten` combinator) and `Promise` are the recorded candidates, **not v1** — each is its
own decision because the reading of `?.` silently changes domain (reactive/async) with the
receiver.

## 4. Lowering *(recommendation)* — operators, not rewrites

Per §0.4, neither operator is a source-text expansion. The house pattern is the binary
operators (`Add`/`PartialEq`: trait-declared, analyzer-recorded in `binary_op_dispatch`,
transformer-emitted):

- The analyzer records a `try_dispatch` / `lift_dispatch` entry per operator site (receiver
  type, continuation ids, chosen map-vs-chain), monomorphizing the continuation as an
  IR-level closure — never pasted source.
- The transformer emits:
  - **std fast path** — `Option`/`Result` lower to inline tag checks (`Option` is a tagged
    array at runtime): `a?.b.c` becomes a branch, no closure allocation; `expr!` becomes a
    branch + `return` — *cheaper* than the `.map(..)` the sugar replaces.
  - **trait path** — any other `Lift`/`Try` type dispatches to its impl's methods, exactly
    like a user `Add`.

## 5. Interactions with what already shipped

- **B10:** `!`'s "nearest enclosing callable" is `ret`'s rule; the return-position checker
  is what turns every misuse (wrong enclosing type, `!` in a bare-void function) into a
  clean spanned error instead of a miscompile.
- **E7:** both operators anchor their diagnostics at the operator token / the offending
  chain link; every error case in the test plan carries an `assert_fails_spanning` pin.
- **LSP:** completion after `a?.` must offer the **inner** `T`'s members (not `Option`'s) —
  the receiver for member resolution is the lifted value. Hover on `!` shows the
  unwrapped type.
- **Formatter:** `a! = b` prints with the space (§0.1); `?.` prints tight.

## 6. Deferred (recorded, not drifted into)

- Expression-level lifting (`a? + 10`) and the applicative form (`a? + b?`) — §0.3.
- Error conversion across types (`Option` in a `Result` fn; `From`-style `E1 → E2`).
- `!` inside closures/async blocks (first follow-up; wants contextually-known closure
  return types — the RPC-handler case).
- `Signal`/`Promise` opting into `Lift` (each its own review).
- User-`Try` types returning a *different* instantiation than the receiver (needs
  associated-type machinery).

## 7. Test plan (per case, as always)

- **`!`:** `Ok`→value / `Err`→returned (observable via caller); `None`→returned; wrong
  enclosing return type (span pin at the `!`); mismatched `E`; bare-void function; `!` in
  a closure (v1 error); `a!.b!` chains; `a! = b` spacing (lex pin both ways: `a!=b` is
  comparison); formatter idempotence; goldens for the inline lowering.
- **`?.`:** map case (plain member) and flatten case (Option-returning member) both
  pinned by *type* (`Option<T>`, not `Option<Option<T>>`); segment grouping
  (`a?.b.c` short-circuits `.c` when `a` is `None` — runtime pin); multi-link chains;
  `?.method(args)`; `?.` on a non-`Lift` type (span pin); `?` not followed by `.` (parse
  pin); `a?.b = x` rejected; `?.` + `!` composition; corpus byte-identical throughout
  (nothing uses the operators yet).

## 8. Open questions

1. `Lift` as an opt-in **marker trait** vs pure duck-typing on `map`/`and_then` — marker
   recommended above (silent lifting over any mappable type reads as a footgun).
2. `Verdict`/`Try`/`Lift` naming.
3. Whether slice 1 (`!`) should land with `Try` as a *real trait* from day one, or with the
   std pair compiler-known and the trait added when the first non-std type wants it (the
   derive machinery took the second path and it aged well).
