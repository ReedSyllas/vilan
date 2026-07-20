# `lazy` — defer to first demand

> **Status: DRAFT 2026-07-20 — for review.** Origin: the lazy-database question
> (2026-07-20) — the accessor sketch (`fun db(): &Database` over a `mut
> Option<Database>` global) fights three model walls at once (module writes, view
> returns, payload locking) because it relocates ownership plumbing into user code.
> The resolution moves only the *time* of one event. The user's reframing is the
> design: `lazy` must not be a one-case keyword — it is **one semantic in several
> positions**: *defer evaluation to first demand, evaluate at most once, memoize.* A
> lazy parameter defers an argument; a lazy binding defers an initializer. One
> lowering (a memo thunk) serves both.

## 1. Lazy parameters

```vilan
impl Option<type T> {
    fun expect(self, lazy message: str): T { .. }     // new surface
    fun unwrap_or(self, lazy fallback: T): T { .. }   // retrofit (§6b)
}

option.expect(i"no row for {key} in {table.describe()}");
```

- **Call site**: the argument expression is not evaluated; the compiler packages it
  as a thunk closing over its free variables. Every existing capture rule applies to
  that closure unchanged — a view in the expression is a view capture (rejected), a
  resource local is an R9 capture (rejected; module-level resources pass, per the
  exemption), so nothing new leaks.
- **Callee side**: the parameter reads as a plain `T` — fully transparent. The thunk
  forces on the parameter's **first read** and memoizes; later reads reuse the value
  (call-by-need, not call-by-name — an argument never runs twice, so a side effect
  can at most happen once, late). A parameter never read never runs — that is the
  point (`expect`'s happy path).
- **Forwarding**: passing a lazy parameter onward to another lazy position forwards
  the thunk (one memo, however deep the chain); passing it to an eager position
  forces it there.
- **v1 restrictions**, each the conservative direction:
  - **Data only.** A lazy *resource* argument would make the thunk own a resource —
    a closure owning a resource is exactly what R9 forbids. Resources stay eager.
  - **Sync only.** An awaiting argument would smuggle asyncness into the callee at
    an invisible forcing point; deferred async already has a spelling (`async
    expr` → pass the `Task`).
  - **Context-free.** The thunk forces inside the callee, where the call site's
    ambient contexts may be gone — the same self-containment rule `drop` bodies and
    (§2) lazy initializers obey. A context-reading expression in lazy position is
    rejected with the steer to pass a closure explicitly.
- **Traits**: a `lazy` parameter is part of the signature; impls must match. (B29 —
  name-only conformance — means the mismatch is not caught today; the `lazy` check
  should not wait for B29: laziness changes call-site *codegen*, so the S1 slice
  carries a targeted arity/laziness agreement check for impls of lazy-parameter
  signatures.)

## 2. Lazy module bindings

```vilan
lazy let database: Database = Database::open("kolt.db");
```

The initializer runs at the binding's **first use** instead of module load, then
memoizes. *Everything else is identical to today's eager module global*: the module
owns the value, process lifetime, resources are loan-only and write-frozen (the
2026-07-20 rule), closures may reference it (the R9 exemption), platform coloring
flows from the initializer exactly as global init colors today, and it never drops.
The lazy-database answer is this one line — no accessor, no `Option`, no view
returns.

- **The initializer is sync and context-free** — first touch can happen anywhere, so
  the deferred code must be self-contained. Pleasing symmetry: birth and death
  (`drop`) obey the same law, for the same reason (their call sites don't thread
  contexts).
- **Reentrancy**: an initializer that (transitively) touches its own binding traps
  with a clear message ("lazy initialization cycle: `database`") via an
  in-progress flag — not a silent hang.
- **A failed initializer poisons the binding** (recommendation, §6a): the panic
  propagates at the touching site, and later touches re-panic with the poisoned
  message. Retry-on-next-touch would re-run side effects and turn "at most once"
  into "at least once per attempt".
- **Turns**: initialization is atomic within a turn by construction (the
  initializer is sync; nothing interleaves mid-init on one turn).

## 3. What v1 excludes, and why

- **Lazy local `let`** for resources: an end-of-scope drop would need a runtime
  was-it-initialized flag — drop flags are ratified out (C4 (c)). Lazy *data*
  locals are harmless but weakly motivated; excluded for symmetry, recorded as the
  relaxation if demand appears.
- **Lazy resource parameters, async/context-carrying thunks**: above.
- **`lazy` fields**: a struct field's forcing point is any read anywhere — that is
  interior mutability through a copyable value, a different feature (`Shared`'s
  territory). Out.

## 4. Classification under the closure rule

`claims-and-epochs.md` §4: **surface, not a model change.** No new alias kind — a
lazy binding is loaned exactly like an eager one, a lazy parameter is a compiler-
managed closure. No new epoch event — the initialization event exists either way;
only its time moves, and no claim can precede the birth it waits on (a loan of a
lazy binding forces it first). The law is untouched.

## 5. Lowering

One shape serves both positions: a memo cell `{ state, value, thunk }` with a
`__force` helper (state: pending → running → done | poisoned; `running` is the cycle
trap). Parameters pass the cell; bindings store it module-level. **The helper needs
its interpreter arm in the same commit** (the equivalence gate). Hover renders
`lazy` in signatures like the other effect surface. `lazy` is a hard keyword
(grep-verified free of identifier uses) — and per the AGENTS.md invariant it lands
in **three homes**: lexer, TextMate grammar, book theme, same commit.

## 6. Open questions — calls wanted before S1

- **(a) Poison vs retry** on a panicking initializer/argument. Recommendation:
  poison (at-most-once stays true; retry re-runs side effects).
- **(b) Retrofitting `Option.unwrap_or` / `Result.unwrap_or`** to `lazy fallback`.
  This *changes observable behavior* of existing call sites whose fallback has side
  effects (they stop running eagerly on the `Some` path). Recommendation: retrofit —
  deferred-fallback is what callers nearly always mean, `unwrap_or_else` remains the
  explicit form, and the corpus/std/docs sweep is the gate (any observable
  difference is a stop-and-decide, per the E2/E3 precedent). New `expect(self, lazy
  message: str)` lands alongside either way.
- **(c) Keyword spelling**: `lazy` (recommendation; Kotlin/Swift precedent) vs
  `defer` (collides with Go/Zig's unrelated meaning) vs annotation-style.

## 7. Slices (suite-gated, docs same commit, per-case pins)

1. **S1 — lazy parameters** (the pure-sugar half; derisks the memo lowering):
   keyword through lexer/parser/formatter (+ the three grammar homes), thunking at
   call sites, forcing on first read, forwarding, the v1 rejections (resource /
   async / context arguments, each with its steer), the impl-signature agreement
   check, `__force` + interpreter arm. Pins: forced-once (memo observable via a
   counting side effect), never-forced-never-runs, forwarding chains, each
   rejection, trait mismatch, hover.
2. **S2 — lazy module bindings**: the memo cell at module scope, first-use init
   order (observable via prints), cycle trap, poison, loan-only/write-frozen
   inheritance for resources (pins with a real `Database`), platform coloring of
   the initializer, docs (spec §6.x sentence for bindings + the std pages for
   `expect`/`unwrap_or`, tour "lazy resources" section replacing nothing — the
   eager idiom stays primary).
3. **S3 — std adoption + the retrofit sweep** per (b).
