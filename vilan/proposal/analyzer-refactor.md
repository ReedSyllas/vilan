# Analyzer stabilization refactor

The analyzer (`crates/vilan-core/src/analyzer.rs`, ~9000 lines) has grown
feature-by-feature without a structural refactor, and a recurring **class** of
bugs has emerged around generic type inference:

- (a) super-linear / stack-overflowing recursion when a generic method calls
  another generic method on the same receiver;
- (b) a closure passed to a generic method types its parameter as an abstract /
  fresh generic instead of the concrete receiver binding, so a generic call in
  the body (`format(n)`, `n.to_string()`) fails to dispatch;
- (c) generic dispatch resolving to dangling/empty functions (fixed: 1e166ce);
- (d) monomorphization gaps — return-type-only generics (fixed: dfc0..dfb),
  nested generic-container calls (`List<List>`).

These aren't independent bugs; they're symptoms of a few structural weaknesses.
This document is the plan to address the **root causes**, so the class shrinks
instead of being whack-a-moled.

## The motivating bugs

- **Bug (a) — FIXED** (commit d77c8ef). Repro:
  `impl Cell<T> { fun update(self, f) { self.set(f(self.get())) } }` — two method
  calls on `self` overflowed even a 256 MB stack. The recursion could **not** be
  located with depth probes on any analyzer function (none reach depth 30 before
  overflow) because it wasn't in the analyzer at all — it was in the
  **transformer's** `resolve_type_id`, which followed `Generic` through a
  `T -> Generic(T)` self-mapping substitution with no guard and looped forever.
  Fixed with a self-map guard there (plus a `RecursionGuard` safety net). Lesson:
  `vilan check` runs `transform` too — bracket the whole pipeline, not just one
  pass.
- **Bug (b) — FIXED** (commit 1144633), and **misdiagnosed here**. Repro:
  `count.derive(|n| n.to_string())` (`count = Signal::new(0)`) — `n` typed as a
  fresh `Generic`, not `i32`. The theory below blamed closure-parameter typing /
  fixpoint ordering / generic identity (items 2/5/6). The **actual** cause was
  much earlier and simpler: **generic struct construction dropped its inferred
  type arguments.** `Signal::new(0)` produced `Signal<T>` (abstract), not
  `Signal<i32>`, so *every* method on `count` saw an abstract element — the
  closure parameter was just the visible symptom. Two one-spot fixes (fill the
  struct-initializer's type args from the bindings it already computes; record a
  call's substitution off the inferred bindings, not the function's own generic
  list, so a static constructor binds the impl's `T`). Lesson: trace the value's
  type to its origin before theorizing about the use site.
- **Bug (c) — OPEN** (tracked: `inference.rs::format_through_nested_generic`).
  Repro: `fun show<T: Display>(x: T): str { format(x) }; show(7)` prints
  `undefined`. A generic function whose body only **forwards** its type parameter
  to another generic call (`format(x)`, not a direct `x.to_string()`) is **not
  monomorphized** by the transformer, so the nested call's trait dispatch
  (`value.to_string()`) resolves to the abstract, empty `Display::to_string`. A
  *direct* dispatch (`x.to_string()`) monomorphizes fine. This is a **transformer
  monomorphization-propagation** gap — demand for monomorphization must propagate
  through call chains — not one of the analyzer items below. Surfaces as
  `count.derive(|n| format(n))` printing `undefined`.

## Root causes (verified)

1. **No memoization of inference.** `infer_type`/`infer_type_inner`
   (analyzer.rs:~4356) re-infers an expression's type from scratch on every
   fixpoint pass and every parent that references it. `exprs_seen` is a *path*
   cycle-guard (added on entry, removed on return), **not** a result cache, and
   it's recreated fresh per top-level `infer_type` call. Nested method calls
   re-infer shared sub-expressions combinatorially → bug class (a).
2. **A fragile, order-dependent fixpoint.** ~30 ad-hoc deferred-work mechanisms
   on the `Analyzer` struct — ~17 `prepped_*` vectors, 5 `*_constraints`, ~8
   dispatch/substitution maps — each with its own defer/re-queue logic, drained
   by `std::mem::take` each pass. There's no dependency tracking: when a
   constraint resolves and a type lands, the constraints that depended on it are
   **not** re-queued; they only re-run if they happen to still be in their list.
   A closure-body call resolved *before* its parameter's type lands is never
   re-resolved → bug class (b).
3. **Several parallel "what is this generic bound to?" channels** —
   `SubstitutionContext`, `method_call_substitution`, `generic_argument_ids`,
   `generic_static_accessors`, `generic_method_dispatch`. A binding recorded in
   one and missed in another silently produces wrong/dangling codegen → bug
   classes (c)/(d).
4. **Unguarded recursive type operations.** `reconcile_type` (analyzer.rs:~4910)
   and `substitute_type` (~5176) recurse with no depth cap; `substitute_type` had
   a `T -> T` self-mapping guard but `reconcile_type` did **not** (now fixed), and
   `substitute_type` didn't handle `Type::Closure` at all (now fixed). A compiler
   must never SIGSEGV on user input.
5. **Free fresh-generic minting + non-interned types.** `type_id_for_type`
   (~1046) mints a fresh `TypeId` per call and types are mutated in place (so
   interning is deferred). Per-call/per-impl generic instantiation creates fresh
   `Generic` ids (the `Generic(239)` vs impl `T` `183` in bug (b)) that the
   substitution doesn't always cover → bug class (b).

## Prioritized refactor items

Each lists the bug classes it shrinks, scope (S/M/L), risk, and dependencies.

> **Status.** Bugs (a) and (b) are fixed by small, targeted patches (see above) —
> *not* by the items below. **Item 3** (safety net: `RecursionGuard`) and **item
> 4** (unify dispatch channels into `generic_dispatch`) are **done** (commits
> d77c8ef, 77699dc). The remaining items (1, 2, 5, 6) are now **stabilization**:
> they make these bug *classes* structurally hard to reintroduce and simplify the
> inference paths, rather than fixing a specific open bug. The one open bug, (c),
> lives in the transformer's monomorphization, which none of these items address.

### 1. Inference result memoization — re-inference class · M · medium risk
Cache `infer_type_inner(expr_id, substitution)` results within a single fixpoint
pass, keyed by `(expr_id, hash(substitution))`; clear the cache at the start of
each pass (types mutate between passes, not within a well-ordered one). Collapses
the combinatorial re-inference into linear, which is the direct cure for the
deep/exponential recursion. *Risk:* a stale entry within a pass if a type lands
mid-pass — mitigated by clearing per pass and not caching `Unresolved`.

### 2. Closure-parameter typing + dependency re-queue — ordering class · M · medium
Two halves: (i) once a closure parameter's type is inferred from the expected
closure type, **persist** it on the parameter (and don't overwrite a concrete
type with an abstract one); (ii) when a constraint resolves and changes a type,
**re-queue the constraints that read it** — minimally, the calls inside a closure
body must re-resolve after the parameter type lands. This is the smallest version
of item 5 and fixes the ordering bug directly. (The `substitute_type` `Closure`
arm and the `infer_closure_args_against_params` substitution, both already
landed, are prerequisites that weren't sufficient alone.)

### 3. Never-overflow safety net — independent · S · low risk — **DONE** (d77c8ef)
A global recursion budget via `util::RecursionGuard` (a thread-local depth Cell,
cap 2048, RAII decrement) entered at the recursive type operations; when exceeded
it bails to the current type rather than recursing. The compiler degrades instead
of a SIGSEGV. Pairs with the `reconcile_type`/`substitute_type` self-map guards.

### 4. Unify the dispatch channels — **DONE** (77699dc) · M · medium
Collapsed `generic_static_accessors` / `generic_method_dispatch` /
`trait_method_dispatch` into one `generic_dispatch: HashMap<Id, GenericDispatch>`
(`OnConstraint` / `OnType`), recorded once and consumed uniformly by analyzer,
transformer, and call-graph (corpus byte-identical). `method_call_substitution`
stays separate — it carries the *bindings*, a distinct concern from *which member
to dispatch to*. The deeper merge of bindings channels waits on item 5.

### 5. Unified constraint queue — foundational · L · medium
Replace the ~30 `prepped_*`/`*_constraints` lists with one
`Vec<Constraint>` where each variant implements
`try_resolve(&mut Analyzer) -> Resolved | Deferred(depends_on)`. The fixpoint
becomes: resolve in order, and when a constraint resolves, re-queue the ones that
declared a dependency on what it produced. Makes ordering **explicit and
testable** instead of emergent, which is the structural cure for the whole
fragility (bug class (b) and future ones). Large, so stage it: introduce the enum
alongside the existing lists, migrate worklists one at a time (each migration
corpus-byte-identical), then delete the old fields.

### 6. Type interning + stable generic identity — later · M · medium-high
Give `Type` `Hash + Eq` and intern `Type -> TypeId`, so a generic parameter has
one stable id rather than fresh copies per call (the `239` vs `183` confusion).
Requires reworking the in-place-mutation model (resolved types written through a
level of indirection). High blast radius — defer until items 1–5 land and the
inference paths are simpler.

## Recommended sequence

1. ~~**Item 3** (safety net)~~ — **done** (d77c8ef).
2. ~~**Item 4** (unify dispatch channels)~~ — **done** (77699dc).
3. **Item 1** (memoization) — speeds the fixpoint and collapses the combinatorial
   re-inference that made bug (a) un-debuggable; highest remaining value.
4. **Item 5** (unified constraint queue), staged — removes the ordering fragility
   that breeds (b)-like bugs; subsumes item 2 and enables a deeper merge of the
   *bindings* channels.
5. **Item 2** (closure typing + re-queue) — only if item 5 is deferred and a
   concrete ordering bug recurs; otherwise folds into item 5.
6. **Item 6** (interning) — only if residual generic-identity bugs justify it.

**Separately, bug (c)** is a transformer concern, independent of the above:
monomorphization demand must propagate through generic call chains so a generic
function that forwards its type parameter to a nested generic call (`format(x)`)
is itself monomorphized. Today only a *direct* trait dispatch (`x.to_string()`)
triggers it. Investigate `transformer.rs` monomorphization queueing
(`monomorphized`, `current_substitution`, the generic-call-nested-in-body path
~line 792).

Items 1–3 are the near-term, contained work that fixes the two open bugs and
makes the compiler crash-proof. Items 4–6 are the deeper stabilization that
reduces the *rate* of new bugs. Each step is gated on the corpus staying
byte-identical (`vilan/test/*.js`) plus the reactive/JSON/generic-dispatch tests.
