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

## The two open bugs as motivation

- **Bug (a)** repro: `impl Cell<T> { fun update(self, f) { self.set(f(self.get())) } }`
  — two method calls on `self` overflow even a 256 MB stack. Splitting the inner
  call into a field read (`let x = self.value.read(); self.set(f(x))`) avoids it.
  The recursion could **not** be located with depth probes on `infer_type_inner`,
  `reconcile_type`, `substitute_type`, `method_member_impl_subject`, or
  `reconcile_argument_types` (none reach depth 30 before overflow) — i.e. it's
  spread across many interprocedural frames with no single guard. That
  un-debuggability is the core problem.
- **Bug (b)** repro: `count.derive(|n| format(n))` (count: `Signal<i32>`) — `n` is
  typed as a fresh `Generic`, not `i32`. Annotating (`|n: i32|`) or an i-string
  (`i"{n}"`) works around it.

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

### 1. Inference result memoization — **fixes (a)** · M · medium risk
Cache `infer_type_inner(expr_id, substitution)` results within a single fixpoint
pass, keyed by `(expr_id, hash(substitution))`; clear the cache at the start of
each pass (types mutate between passes, not within a well-ordered one). Collapses
the combinatorial re-inference into linear, which is the direct cure for the
deep/exponential recursion. *Risk:* a stale entry within a pass if a type lands
mid-pass — mitigated by clearing per pass and not caching `Unresolved`.

### 2. Closure-parameter typing + dependency re-queue — **fixes (b)** · M · medium
Two halves: (i) once a closure parameter's type is inferred from the expected
closure type, **persist** it on the parameter (and don't overwrite a concrete
type with an abstract one); (ii) when a constraint resolves and changes a type,
**re-queue the constraints that read it** — minimally, the calls inside a closure
body must re-resolve after the parameter type lands. This is the smallest version
of item 5 and fixes the ordering bug directly. (The `substitute_type` `Closure`
arm and the `infer_closure_args_against_params` substitution, both already
landed, are prerequisites that weren't sufficient alone.)

### 3. Never-overflow safety net — independent · S · low risk
A global inference recursion budget (a depth counter threaded through, or checked
at `infer_type_inner` entry) that, when exceeded, records a diagnostic
("type of this expression is too complex to infer") and returns `Unresolved`
instead of recursing. Even with item 1 outstanding, the compiler degrades to an
error rather than a crash. Pairs with the `reconcile_type`/`substitute_type`
guards already added. *Do this first* — it's cheap insurance.

### 4. Unify the substitution channels — shrinks (c)/(d) · M · medium
Collapse `generic_argument_ids` / `generic_static_accessors` /
`generic_method_dispatch` / `method_call_substitution` into one representation of
"this call's generic bindings," recorded once and consumed uniformly by both the
analyzer and the transformer. Removes the "recorded here, missed there" class.
Best done after item 5 gives a cleaner call-resolution path.

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

1. **Item 3** (safety net) — immediate; the compiler stops crashing on (a)/(b).
2. **Item 1** (memoization) — fixes (a); also speeds the fixpoint.
3. **Item 2** (closure typing + re-queue) — fixes (b).
4. **Item 5** (unified constraint queue), staged — removes the ordering fragility
   that breeds new (b)-like bugs; unlocks item 4.
5. **Item 4** (unify substitution channels) — shrinks (c)/(d).
6. **Item 6** (interning) — only if the residual generic-identity bugs justify it.

Items 1–3 are the near-term, contained work that fixes the two open bugs and
makes the compiler crash-proof. Items 4–6 are the deeper stabilization that
reduces the *rate* of new bugs. Each step is gated on the corpus staying
byte-identical (`vilan/test/*.js`) plus the reactive/JSON/generic-dispatch tests.
