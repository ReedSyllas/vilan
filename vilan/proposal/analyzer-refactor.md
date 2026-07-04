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
- **Bug (c) — FIXED** (commit b9f61e0). Repro: `fun show<T: Display>(x: T): str
  { format(x) }; show(7)` printed `undefined`. *Initially* theorized as a
  transformer monomorphization-propagation gap; the real cause was in the
  **analyzer**: call resolution reconciled `argument`-against-`parameter`, and
  `reconcile_type`'s generic arm is left-biased, so a generic *argument* (`x: T`)
  passed to a generic *parameter* (`value: U`) bound the argument's constraint
  (`T = U`) instead of the callee's (`U = T`). `format`'s substitution keyed on
  show's `T`, never matched format's own generic, and format monomorphized to
  nothing. Fix: reconcile parameter-first (bind the callee's generics). A direct
  `x.to_string()` worked because it uses the `generic_dispatch` channel, not
  argument reconciliation.
- **Bug (c′) — OPEN** (tracked: `inference.rs::format_in_closure_argument`). The
  closure-argument variant `count.derive(|n| format(n))` still prints `undefined`:
  no substitution is recorded for `format(n)` because `n` isn't concrete when the
  closure body resolves, and the call isn't re-queued once it lands (and the
  closure itself isn't monomorphized). A *method* call on `n` (`n.to_string()`)
  works via `generic_dispatch`. This is the closure-parameter-typing +
  dependency-re-queue issue — **item 2 / item 5**, below.

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
>
> **Update (2026-07-03): this program is essentially complete.** Items 1–5 are
> resolved (1 investigated-and-rejected; 2 subsumed by the unified queue —
> [`constraint-queue-plan.md`](constraint-queue-plan.md), v1+v2 shipped; 5 with
> the item-4/5 solver pass), bug (c) and the whole A/B/C + B1 cluster are fixed
> with per-case pins ([`type-solver.md`](type-solver.md): "B1 genuinely fully
> closed"), and later solver passes extended the same channels (own-generic
> ordered values, bound-dispatch trait recording — see transport-rpc.md's
> follow-ups). Only **item 6** (type interning) remains open, as a perf/hygiene
> stabilization with no bug attached.

### 1. Inference result memoization — investigated, **NOT worth it** · medium risk
Original idea: cache `infer_type_inner(expr_id, substitution)` keyed by
`(expr_id, hash(substitution))`, cleared per pass. **Built and measured — it does
not help, and the premise was wrong:**
- **Per-call scope** (cleared at each top-level `infer_type` — the only *sound*
  scope, since `infer_type` is read-only so an entry can't go stale mid-call):
  **zero cache hits** even on a 1400-line generic-heavy program. The same
  `(expr, Unknown, substitution)` is never re-queried *within* one top-level
  inference — the recursion is a tree, not a re-converging DAG. Nothing to collapse.
- **Per-pass scope** (cleared per fixpoint iteration): ~22% hit rate and
  corpus-clean, **but theoretically unsound** — a non-`Unresolved`-but-*abstract*
  type (`Signal<Generic>`) cached early can be refined to concrete
  (`Signal<i32>`) later in the same pass (the bug-(b) refinement pattern). And it
  bought only **~3% wall-clock** (912/2793/9217 ms → 882/2589/9132 ms for
  100/200/400 functions), because the bottleneck is not the infer-call *count*.

Memoization is the wrong tool. It is also only safely cacheable once passes are
*well-ordered* (no mid-pass refinement) — i.e. after item 5. Revisit only then.

### Performance: `analyze` was quadratic — **FIXED** (commit fa55979)
Measuring synthetic programs of *independent* functions (each constructing a
generic `W<T>` and calling a few methods) found `analyze` was **~O(N²)** in
program size: 882 ms → 2589 → 9132 → ~34 000 ms for 100/200/400/800 functions
(other phases negligible — `context`/`async_infer`/`transform` each ≤ 10 ms). The
fixpoint ran a **constant 6 passes**, and every operation *count* was **linear**
(attempts, `infer_type`/`compare_type`/`reconcile_type` calls, `implementations`
= constant 14, type-id map size) — yet wall-clock was quadratic. Linear counts +
quadratic time pointed to per-call work growing with the working set.

A `callgrind` profile settled it: ~60% of time was in `core::fmt` —
`PadAdapter::write_str` (the `{:#?}` indented-Debug writer), `DebugMap`, `Id`/`Expr`
`Debug::fmt` — *not* inference. The cause: `get_entity_by_id` built its panic
message `format!("… {:#?}", id, self.expr_id_to_expr_map)` as the **argument** to
`.expect()`, so it pretty-printed the **entire expr map** (O(N)) on *every*
successful lookup. Called O(N) times ⇒ O(N²). Fixed by making the message lazy
(`unwrap_or_else(|| panic!(..))`) and dropping the map dump; same for two sibling
scope accessors. Result: **217/398/788/1547 ms** for 100/200/400/800 functions —
linear, up to **22× faster**, corpus byte-identical.

*Lesson:* `.expect(format!(..))` / `.unwrap_or(expensive())` evaluate their
argument eagerly on the success path. In a hot accessor that is a latent O(N²).
clippy's `expect_fun_call` flags these — worth clearing. (A second latent O(N²)
remains, lower-impact: a struct initializer with a placeholder id scans **all
scopes** by name — `analyzer.rs`, "Resolve struct initializer constraints".)

### 2. Closure-parameter typing + dependency re-queue — **largely DONE**
The concrete ordering bugs are fixed by two targeted patches (commits 57df341,
d851141), without the general re-queue infrastructure:
- **Defer on an unknown closure parameter** (bug c′). A call whose argument is an
  unannotated closure parameter (`count.derive(|n| format(n))`) now defers while
  that parameter is `Unknown`, instead of committing against `Unknown` and never
  revisiting — mirroring the rule the method-call resolver already applied to an
  unknown closure *receiver*. The existing defer-and-retry loop supplies the
  re-queue effect for these constraints; no dependency graph needed.
- **Bind a method's own generics from its arguments** (chained `derive`). Method
  calls bound only the impl's generics (from the receiver); a method's *own* `<U>`
  (e.g. `derive<U>`'s `U`, fixed by the closure's return type) went unbound, so
  `count.derive(|n| n * 2)` stayed `Source<U>` and a chained `.derive` saw an
  abstract element. Method calls now reconcile arguments parameter-first and bind
  their own generics, like free-function calls. Result: chained `derive`/`map`,
  i-strings, and `apply<T>(x, |x| format(x))` all work.

What's left for **item 5**: the *general* dependency re-queue — re-running an
arbitrary constraint when a type it read changes, rather than relying on each
worklist's bespoke defer condition. Worth it only if a new ordering bug appears
that the targeted defers don't cover; the structural cure still lives in item 5.

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

### 5. Unified constraint queue — **v1 DONE** (2fef2f3 … 79f6f92) · L
Replaced the per-kind `prepped_*`/`*_constraints` worklists in `build()` with one
`Constraint` enum + `resolve_constraints` runner. **v1 is complete**: all 11
fixpoint kinds — subscript, `is`, field accessor, struct initializer, match,
method call (+ slot unification + method-arg check), for-each item, variable, call
subject — now live on a single queue, resolved in one explicit priority order; the
solving loop is just `for _ { if !resolve_constraints() { break } }`. Migrated one
kind per commit, each corpus-byte-identical, id-minting kinds (method/call) in
strict priority order; compile time stayed linear. The mid-pass-spawn machinery
(the runner re-sorts each pass and keeps tasks spawned during a pass) is in place.

**v2 — still to do** (gated, separate): automatic dependency capture + dirty
re-queue (run only the tasks whose inputs changed, instead of all each pass). This
is the structural cure for the (b)/(c′) ordering class and the prerequisite that
makes memoization (item 1) safe. Pursue when an ordering bug recurs that the
targeted defers don't cover, or when memoization becomes worth having. **Full plan:
`constraint-queue-plan.md`.**

### 6. Type interning + stable generic identity — later · M · medium-high
Give `Type` `Hash + Eq` and intern `Type -> TypeId`, so a generic parameter has
one stable id rather than fresh copies per call (the `239` vs `183` confusion).
Requires reworking the in-place-mutation model (resolved types written through a
level of indirection). High blast radius — defer until items 1–5 land and the
inference paths are simpler.

## Recommended sequence

1. ~~**Item 3** (safety net)~~ — **done** (d77c8ef).
2. ~~**Item 4** (unify dispatch channels)~~ — **done** (77699dc).
3. ~~**Item 1** (memoization)~~ — **investigated, dropped** (doesn't help; see above).
4. ~~**The quadratic `analyze`**~~ — **fixed** (fa55979): an eager `{:#?}` in a hot
   accessor, found via `callgrind`. `analyze` is now linear (up to 22× faster).
5. ~~**Bug (c)**~~ — **fixed** (b9f61e0): reconcile call arguments parameter-first.
6. ~~**Item 2** (closure typing + re-queue)~~ — **largely done** (57df341, d851141):
   defer on an unknown closure parameter (bug c′) + bind a method's own generics
   from arguments (chained `derive`). Only the *general* re-queue remains, folded
   into item 5.
7. **Item 5 v1** (unified constraint queue) — **done** (2fef2f3 … 79f6f92): all 11
   fixpoint kinds on one priority-ordered queue. **v2** (dependency-driven re-queue)
   remains, gated — it subsumes item 2's remainder and makes memoization safe.
8. **Item 6** (interning) — bounds the ever-growing type-id map (a suspect in the
   former quadratic) and stabilizes generic identity (the multi-id confusion seen
   while debugging bug (c)); pursue with the now-done item 4.

Items 1–3 are the near-term, contained work that fixes the two open bugs and
makes the compiler crash-proof. Items 4–6 are the deeper stabilization that
reduces the *rate* of new bugs. Each step is gated on the corpus staying
byte-identical (`vilan/test/*.js`) plus the reactive/JSON/generic-dispatch tests.
