# Item 5: Unified constraint queue — implementation plan

A staged plan to replace the ~25 ad-hoc deferred-work lists in `analyzer.rs`
`build()` with one constraint queue. Companion to `analyzer-refactor.md` (item 5).

> **Status: v1 *and* v2 shipped.** Stages 0–13 (the unified `Constraint`/`Resolution`
> queue) landed earlier. **Stage 14 (the dependency-driven re-queue) is now done**: a
> per-resolution `current_waiting_on` set captures every expression a constraint reads
> as `Unresolved` (the one `infer_type_inner` chokepoint); `wake_ready_constraints`
> re-queues a deferred constraint once one of those inputs appears in the type maps;
> and a **run-all backstop on every quiet pass** keeps termination — and therefore the
> resolved set and the codegen — identical to the old run-all fixpoint by construction
> (resolution is monotone, so order can't change which bindings commit). Verified:
> corpus 69/69 byte-identical, full suite green, clippy ≤ baseline, and an old-vs-new
> timing on a 200–1600-function synthetic shows v2 is performance-neutral (within
> debug-build noise). `deep_dependency_chain_resolves_across_passes` pins the wake
> path. The cleanup half of stage 15 (delete the now-unused channels / merge the
> bindings recording per item 4) is still open.

## Why (and an honest scope)

The fixpoint in `build()` re-runs *every* unresolved constraint on *every* pass,
and each worklist has its own bespoke defer condition. That fragility is what bred
the ordering bugs this effort chased — bug (b), bug (c′) (a call committing
against an `Unknown` closure parameter), and the free-function/method divergence
behind the chained-`derive` bug (the two paths bound argument generics
differently). Each was fixed by a *targeted* patch.

So be honest about the goal. Item 5 is **not** fixing a current bug — the targeted
fixes already did. Its value is **structural**: make constraint ordering explicit
and testable instead of emergent, so this *class* of bug stops recurring; unify
the two call paths (item 4's deeper merge); and make inference memoization (item 1)
safe — memoization is only sound once passes are well-ordered. Treat it as a
maintainability/robustness refactor, staged to never break the corpus, not a
sprint to a feature. **Ship the structural half (v1) for most of the value; gate
the dependency engine (v2) on actual need.**

## Current shape (as of this plan)

`build()` (`analyzer.rs:5351`, called once from `analyze()` at ~8860) has three
phases:

1. **Name/type pre-passes** (5351–~5795): imports (its own retry loop), `use`
   statements, type locals, static accessors, type static accessors. Resolve
   names → entities/types.
2. **Type-inference fixpoint** (`for _ in 0..max_iterations { let mut progress =
   false; … if !progress { break } }`): **11 sections** in a fixed order — struct
   initializers, field accessors, `list[index]` subscripts, `is` tests, `match`,
   method calls, `push`/slot unifications, `for…in` element bindings, method-arg
   checks, variable constraints, call-subject constraints.
3. **Post-fixpoint** (~7110+): `for…in` protocol + deferred item commit, operator
   overloading; then end-of-fixpoint diagnostics for whatever stayed unresolved.

**Worklists** (~25 fields on `Analyzer`), by kind:
- Inference constraints: `struct_initializer_constraints`, `field_accessor_constraints`,
  `index_constraints`, `variable_constraints`, `call_subject_constraints`,
  `prepped_method_calls`, `prepped_method_arg_checks`, `prepped_is`,
  `prepped_matches`, `prepped_for_each`, `prepped_for_each_items`,
  `prepped_slot_unifications`, `prepped_binary_ops`.
- Name/type resolution: `prepped_imports`, `prepped_uses`, `prepped_locals`,
  `prepped_assignments`, `prepped_field_accessors`, `prepped_static_accessors`,
  `prepped_type_locals`, `prepped_type_static_accessors`, `prepped_trait_impls`.
- Result/dispatch maps (outputs, not worklists): `generic_dispatch`,
  `binary_op_dispatch`, `method_call_substitution`, `for_each_next`,
  `for_each_views`.

**Re-queue today:** a constraint that can't resolve stays in its list (`remaining.
push(...)` / not draining it); `progress` flips when anything resolves; the loop
stops on a quiet pass. No dependency tracking — every unresolved constraint is
retried every pass. (~38 `progress`/`remaining.push` sites.)

**Precedent to generalize:** the method-call resolver's local
`enum MethodLookup { Found, NoMethod, Defer, NotCallable }` is exactly the
per-constraint outcome shape this plan lifts to the whole `build()`.

## Design

```rust
enum Constraint<'src> {                 // one variant per current worklist
    StructInitializer(StructInitializerConstraint<'src>),
    FieldAccessor(FieldAccessorConstraint<'src>),
    MethodCall(MethodCallConstraint<'src>),
    CallSubject(CallSubjectConstraint),
    Variable(VariableConstraint),
    // … the other ~9 inference kinds; reuse the existing structs as payloads,
    //    promoting the tuple worklists (prepped_method_calls, …) to named structs.
}

enum Resolution { Resolved, Deferred, Failed }   // Failed already pushed its diagnostic

impl Analyzer {
    fn try_resolve(&mut self, c: &mut Constraint) -> Resolution { /* big match → existing bodies */ }
}
```

An enum (not `Box<dyn>`) — the variant set is closed, it avoids allocation, and it
matches the codebase's style. Each arm is the *current* section body, extracted
verbatim into a `fn resolve_<kind>(&mut self, …) -> Resolution`.

Two increments:

- **v1 — uniform abstraction, same scheduling.** One `Vec<Constraint>` (or a small
  set of priority buckets to preserve today's section order). The runner drains
  and re-queues exactly as the current fixpoint does — *run all, each pass, until a
  quiet pass*. Behavior (and corpus) identical; the win is that every constraint
  now flows through one inspectable, testable path, and the bindings channels can
  then be merged (item 4's deeper half).

- **v2 — dependency-driven re-queue.** Capture dependencies automatically: thread
  the "currently-resolving constraint" and, at the single chokepoint where a type
  is read as `Unresolved` (in `infer_type`), record `(constraint ⇽ expr)`. At the
  single chokepoint where a type is *written* (`resolved_types`/
  `expr_id_to_type_id_map` insert), re-queue the constraints that depended on it.
  Replace "run all each pass" with "run the dirty set." Keep a bounded fixpoint as
  a backstop for dependency cycles. This is where the general (c′-class) ordering
  fix lands and where memoization (item 1) becomes sound.

## Staged migration — each stage its own commit, **corpus byte-identical**

0. **Scaffolding.** Define `Constraint`/`Resolution` and an empty runner alongside
   the existing lists. Compiles, unused. No behavior change.
1. **Pilot one worklist.** Migrate a small, self-contained one — recommend
   `index_constraints` or `prepped_assignments` — into the queue, resolve it via
   the runner in its current fixpoint slot, delete the old section. Proves the
   pattern end-to-end. Validate byte-identical corpus.
2–12. **Migrate the remaining inference sections**, one per commit, preserving the
   current inter-section order via an explicit priority on each variant (the order
   is load-bearing — changing order and representation together would obscure any
   drift). Method calls and call subjects last (most entangled).
13. **Pre-passes (optional).** Fold imports/uses/type-locals/static-accessors into
   the queue. Lower value (already simple, own retry loops); do only if it
   simplifies.
14. **Dependency capture (v2).** Add automatic dep tracking + dirty re-queue; switch
   the runner from run-all to run-dirty. **Riskiest stage** — gate on corpus *and*
   perf.
15. **Cleanup.** Delete the `for _ in 0..max_iterations` loop, the `progress`
   flag, and the migrated worklist fields; merge the bindings channels
   (`method_call_substitution` + the now-uniform recording) per item 4's tail.

v1 = stages 0–13 (and 15's field cleanup for migrated kinds). v2 = stage 14.

## Risks & mitigations

- **Corpus drift = an ordering change.** The byte-identical gate per stage catches
  it; fix by adjusting the variant's priority. Never change order + representation
  in one stage.
- **Performance.** The dep index must be O(1) per re-queue; a "re-run dependents"
  that rescans all constraints would reintroduce an O(N²) like the one `callgrind`
  just found. After stage 14, re-run the `sc_100..800` benchmark — it must stay
  linear (~217/398/788/1547 ms).
- **v2 is the hard part.** If automatic dep capture proves too invasive, v1 still
  delivers the uniformity and unblocks item 4 — ship v1, defer v2. Don't bundle.
- **Cycles.** Dependencies can cycle (mutually-recursive inference); keep the
  bounded fixpoint as a backstop so v2 degrades to v1's behavior, never hangs.

## Verification (every stage)

- Corpus byte-identical (`vilan/test/*.js`) — the hard gate. Rebuild the debug
  binary first (see `golden-regen-rebuild-debug`).
- `cargo test -p vilan-core` green (incl. `tests/inference.rs`); add ordering tests
  — a constraint that reads a type which only resolves several passes later.
- clippy ≤ baseline (79).
- After v2: the perf benchmark stays linear.

## Recommendation

Do **v1 (stages 0–13)** — mechanical, low risk, high cleanup value, and it unblocks
item 4's bindings merge. Treat **v2 (stage 14)** as a separate, explicitly-gated
decision: pursue it when a fresh ordering bug appears that a targeted defer can't
cover, or when memoization (item 1) becomes worth having. The targeted fixes
already shipped mean there is no urgent bug forcing v2 today.
