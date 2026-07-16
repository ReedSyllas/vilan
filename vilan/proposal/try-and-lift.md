# `!` and `?` — early return and lifted chains (backlog B11)

Status: **BOTH SLICES SHIPPED 2026-07-04** — `!` and `?.` are live (`void` also became
the unit expression en route). Slice 2 landed as specified: `?` lexes as an operator,
`?.member` joins the postfix chain, and the parser groups each `?.`'s continuation (the
member plus every following plain postfix up to the next `?.`/`!`/chain end — escaping a
group is parenthesization, as in TS) over a `LiftBinder` hole; `Constraint::Lift` grounds
the binder as the subject's element (waking the continuation's deferred constraints),
picks map-vs-flatten from the continuation's type (same-container = flatten; `Result`
flatten checks the error types), and records the lowering; the transformer emits the
match-shaped inline form — bad tag short-circuits AS-IS, the element aliases into the
continuation (no closure), map rewraps via the container's good variant. A lifted chain
is rejected as an assignment target. The `Lift` marker + Option/Result impls ship in std;
user `Lift` lowering shipped in the stabilization pass: a marked container dispatches to
its own `map`/`and_then` instance (the flattening rule picks; the member's `U` binds from
the continuation), the continuation emitted as a closure whose parameter aliases the
binder — the element convention is the container's FIRST type argument (`M<T, ..>`), and
the marker stays the gate (a mappable type without `impl .. with Lift` refuses, pinned).
LSP: completion after `a?.` offers the ELEMENT's members. Ten `?.` pins + corpus
`lift-chain.vl` cover §7's rows.
The two-operator design, the four refinements in §0, and the §8 resolutions (opt-in
`Lift`; the `Try`/`Lift`/`Verdict` names; `Try` as a real trait from day one) are all
settled. Slice 1 landed as specified: postfix `!` in the member chain; `Verdict`/`Try`
+ the `Option<T> → Try<T, void>` / `Result<T, E> → Try<T, E>` impls as real std source
(`operators.vl`/`option.vl`/`result.vl`); `Constraint::TryAssert` types the good half,
checks the enclosing function (std pair by identity — Option-in-Option any element,
Result-in-Result same error; user `Try` types exact-match), and records the dispatch;
the transformer lowers std receivers to the inline tag branch (bad `Option`/`Result`
values return AS-IS — byte-identical at any success type) and user types through their
impl's emitted `verdict`/`from_bad`. Ten pins + a corpus test (`try-assert.vl`) cover
§7's `!` rows; the `assert_fails_spanning` harness pins every error at the `expr!` span.
One solver lesson en route: a new expression kind MUST have an `infer_type` arm
reporting `Unresolved` pre-resolution — without it, a `let` grounding on `expr!`
committed to void before the constraint ran.

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

1. **`!=` always lexes as not-equals.** Postfix `!` followed by an `=`-starting operator
   requires the space: `a! == b` compares an unwrapped value; `a!==b` is a lex error
   (`!=` then `=`). The formatter always emits the space; the parser's error for the
   soup case should hint at it. (`expr!` is a *value*, not a place — an assignment
   target `a! = b` is rejected in v1; place-ness of unwrapped results is a view-model
   question deferred with the rest.)
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

### The `Try` seam *(agreed — a real trait from day one)*

"Bad" is programmed by implementing the operator trait. The trait, `Verdict`, and the two
std impls are **real vilan code in std** from the first slice — not compiler-known
shortcuts (§8.3); the transformer's inline fast path (§4) is an *optimization over* those
impls, pinned semantically identical to the trait dispatch:

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

// Option's residual is the absence itself — `void`, which in vilan IS the
// unit type (an empty tuple; a prettier alias for `()`). It instantiates
// generics like any type (probed: `Result<void, str>` / `Option<void>`
// construct, match, and run), and `void` is also the unit EXPRESSION —
// the type's one value (`Verdict::Bad(void)`, `Some(void)`).
impl Option<type T> with Try<T, void> {
	fun verdict(self): Verdict<T, void> {
		match self {
			Some(let value) => Verdict::Good(value),
			None => Verdict::Bad(void),
		}
	}

	fun from_bad(bad: void): Option<T> {
		None
	}
}

impl Result<type T, type E> with Try<T, E> { .. }  // Bad = the error; from_bad = Err(e)
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

### The `Lift` seam *(agreed — opt-in)*

Opt-in (§8.1), so `?.` doesn't silently work on everything that happens to have a `map`:

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

## 4. Lowering *(agreed)* — operators, not rewrites

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

- ~~Return-position generics through `!`~~ — **fixed** (stabilization pass): annotated
  lets seed their expectation onto the value, and `resolve_try_assert` re-infers the
  receiver as `Container<expected, ..>` once the container is known — the binding rides
  the same reconcile-and-record channel as the two-step form. Pin un-ignored.

- Expression-level lifting (`a? + 10`) and the applicative form (`a? + b?`) — §0.3.
  **Proposal drafted 2026-07-16 (`expression-lifting.md`, awaiting review):** lift
  regions bounded at slot roots, applicative = left-to-right short-circuiting
  `and_then` nesting, std inline lowering, five open questions recorded.
- ~~Error conversion across types (`Option` in a `Result` fn; `From`-style `E1 → E2`)~~
  — **resolved 2026-07-15: EXPLICIT by design (§9)**. `!` stays same-type; convert at
  the value first — `.map_err(to_e2)!` for `E1 → E2`, `.ok_or(err)!` for `Option` in a
  `Result` fn. No implicit `From`/`Into` coercion (the no-silent-conversion rule).
- `!` inside closures/async blocks — kept deferred through the stabilization pass: its
  real payoff needs the `arg → Result` API redesign (the RPC-handler case), and a
  bang-in-tail closure is semantically invalid anyway (`|k| lookup(k)!` cannot rebuild
  the bad half into its own unwrapped return); the future check can say so precisely.
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

## 8. Resolved (2026-07-04)

1. **`Lift` is an opt-in marker trait** — silent lifting over any mappable type reads as
   a footgun.
2. **The names stand:** `Try`, `Lift`, `Verdict`. (A fourth name, `Absent`, was briefly
   proposed as Option's residual and dropped: `void` instantiates generics fine — probed —
   and is the canonical nothing, so `Try<T, void>` needs no new type. `Result<void, str>`
   stays exactly `Result<void, str>` everywhere.)
3. **`Try` is a real trait from day one** — the trait, `Verdict`, `Absent`, and the
   `Option`/`Result` impls ship as std source in slice 1; the compiler's inline lowering
   is an optimization over those impls, not a substitute for them (pinned equivalent: a
   user-`Try` type and `Option` must behave identically through `!` modulo the v1
   same-type restriction).

## 9. Error conversion at the `!` boundary — resolved: EXPLICIT (2026-07-15)

§6's `E1 → E2` deferral asked how `!` should cross error types. **Decision (settled
with the user): it does not — conversion is explicit, at the value, before the `!`.**
`!` stays same-type: it returns the bad half *as-is*, so the value's error type must
already be the function's. Rust folds a `From`-conversion into `?`; vilan does not,
for the same reason it forbids a silent view→value cross (transparent-references) — an
error changing type is a real operation, and the language does not perform real
operations invisibly. The `Add`/`Try` "programmable per type" rule (§0.4) governs what
*bad* means, not an automatic coercion of it.

**The explicit path — already complete, no new machinery.** The std combinators
compose with `!` today:

- **`E1 → E2` (`Result`):** `value.map_err(to_e2)!` — `Result::map_err(|E1| E2)` maps
  the error, then `!` returns the now-matching `E2`. A named fn or a closure both work
  (`query().map_err(|e| AppError { msg = e })!`).
- **`Option` in a `Result` fn:** `opt.ok_or(err)!` — `Option::ok_or(E)` turns `None`
  into `Err(err)` with a caller-supplied error, then `!` returns it. `ok_or_else(|| …)`
  for a lazy error. This is why `Option`-through-`!` requires a `Result` fn: the error
  value is *supplied here*, not fabricated.

So the only compiler work is **diagnostics**: the two mismatch errors, which read like
a missing feature ("error conversion is not supported yet"), instead point at the
explicit helper —

- `Result` `E1 != E2` → "…the error types must match; convert first with
  `.map_err(…)` before `!`".
- `Option` in a non-`Option` (`Result`) fn → "…convert to `Result` first with
  `.ok_or(err)` (or `ok_or_else`)".

**Scope.** `!` diagnostics + the analogous `?.` flatten mismatch message (§3, same
shape). No lowering change — `!` is untouched, so all existing codegen stays
byte-identical.

**Test plan (per case).** `map_err(fn)!` and `map_err(|e| …)!` run and convert
(observable via the caller's `Err`); `ok_or(e)!` converts `None`→`Err` and runs;
same-type `!` unchanged; the `E1 != E2` mismatch is rejected with the `.map_err` hint;
`Option`-in-`Result` rejected with the `.ok_or` hint; a docs example shows the pattern.
