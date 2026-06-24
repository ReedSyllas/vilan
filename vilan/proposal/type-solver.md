# Type solver — capability characterization (backlog B1)

B1 asks: stand back from the constraint machinery, characterize what the solver
*can and cannot* decide, find the cases it gets wrong, and **merge the special cases
into general code** rather than whack-a-mole each one. This is the synthesis. The
mechanism and the prior refactor live in [`analyzer-refactor.md`](analyzer-refactor.md)
(root causes; items 1–6, with 1–5 v1 shipped) and
[`constraint-queue-plan.md`](constraint-queue-plan.md) (the unified queue; v1 shipped,
v2 the dependency engine, deferred). This doc states the model, isolates the *one*
class of failure that remains, and names the cure — which turns out to be exactly the
two refactors those docs already designed and deferred.

## The model (current)

- **Types** are interned to `TypeId` (`type_id_to_type_map`); a generic parameter is
  `Type::Generic(constraint_id)` keyed by its binder; bindings are a
  `SubstitutionContext = HashMap<TypeId, TypeId>` (generic id → concrete id).
- **Inference** is a worklist fixpoint: one `Constraint` enum (12 kinds), a
  `priority()` order, and `resolve_constraints()` which **runs every constraint each
  pass and re-queues whatever defers, until a quiet pass** — there is *no* dependency
  tracking (v2). `reconcile_type` (now parameter-first) unifies + emits bindings;
  `substitute_type` applies them.
- **Generic dispatch** is recorded once into `generic_dispatch` (which member) +
  `method_call_substitution` (the bindings), keyed by call id.
- **Monomorphization** (transformer) holds `current_substitution` (the active
  bindings) and emits a concrete instance per type-arg set: free calls via
  `get_or_create_instance(generic_argument_ids)`, nested calls via
  `inherited_substitution` (the callee's generics that appear in
  `current_substitution`). Unresolved → `ensure_function_emitted` of the *generic*
  body.

## What it decides well

Direct generic calls; struct-construction inference (bug b); parameter-first
argument reconciliation (bug c); bidirectional closure-parameter inference
(`list.map(|x| x + 1)`); the 11 constraint kinds in their priority order; the
never-overflow guards. The corpus (69) and the inference suite (39) are green.

## The one class that remains: generic bindings don't flow across boundaries

Both deep-reads of the dispatch + inference engines converge on a **single failure
path**. A generic parameter's binding is lost when it must cross an inference or
monomorphization boundary, and the transformer then emits the *generic* body, inside
which dispatch resolves to the **empty abstract trait method** → `undefined` at
runtime. The binding is lost in one of two ways:

- **(A) never recorded.** The constraint that would bind the generic runs *before*
  its input type lands, commits against `Unknown`/`Unresolved`, and is never re-run —
  the fixpoint re-runs *all* constraints each pass but has no notion of "this one
  read a type that just changed." So `method_call_substitution`/`generic_argument_ids`
  is never written, and the transformer has nothing to monomorphize with.

- **(B) recorded but not composed.** The binding is keyed by the *caller's* generic
  id, but the nested callee's body references its *own* (freshly-minted) generic id.
  `inherited_substitution` matches by id, so the callee's generics aren't in
  `current_substitution`, the composition misses, and the callee is emitted generically.

### The recurring bugs are all this class

| repro | which | why |
| --- | --- | --- |
| bug **c′** — `count.derive(\|n\| format(n))` | A | `n` types late (from `derive`'s signature); `format(n)` committed against `Unknown`, not re-run. |
| RPC **#4** — `Ok(Option::from_json(json))` | A | the element type `User` arrives via the `Ok` wrapper + return type, *after* the `from_json` constraint resolved. |
| `List<List<T>>` round-trip | A/B | the inner container's element binding isn't threaded through the outer `from_json_value`. |
| RPC **#3** — object-stub `(self.t).call()` | B | the stub's `<T>` and a routed helper's `<U>` are different ids; `inherited_substitution` can't thread one through the other. |

These are not four bugs. They are one structural leak: **the substitution model is
sound for *direct* binding and leaks across *boundaries*** — late-arriving inputs
(A) and fresh ids in nested scopes (B). The targeted patches for bugs a/b/c shrank the
class; the leak itself is what B1 says to fix generally.

## The cure (already designed, deferred — now gated in)

The prior plan deferred two refactors *and named the gate*: pursue v2 "when an
ordering bug appears that targeted defers can't cover," and item 6 "once items 1–5
land." Both gates are now met — the RPC repros are exactly that ordering/identity
class. In order:

1. **Item 5 v2 — dependency-driven re-queue** (`constraint-queue-plan.md` stage 14).
   Thread the currently-resolving constraint; at the one `infer_type` **read** of an
   `Unresolved`/`Unknown` type record `(constraint ⇽ expr)`; at the one
   `resolved_types`/`expr_id_to_type_id_map` **write** re-queue the dependents; run
   the *dirty* set instead of all-each-pass, with the bounded fixpoint kept as a
   cycle backstop. **Fixes class (A)**: a binding's constraint re-runs the moment its
   late input lands, so it's recorded. This is the structural cure the doc names for
   the ordering class, and the prerequisite that makes item 6 and memoization sound.
   **Leads.**

2. **Item 6 — type interning + stable generic identity.** One stable `TypeId` per
   generic parameter (rather than fresh copies per call/impl), so a binding composes
   across scopes by id and `inherited_substitution`/`substitute_type` stop missing.
   **Fixes class (B).** High blast radius (reworks the in-place-mutation model);
   follows v2, per the existing sequence.

**Item-4 tail — ✅ resolved** (commit 6b96d3f). The duplication was in the transformer:
two near-identical instance emitters (`get_or_create_instance` for free functions, keyed
by positional type args; `emit_method_instance` for methods, keyed by a constraint→type
substitution) plus four call-emission branches that each rebuilt the same lowering. Two
emitters fed by two binding representations is the "recorded in one channel, read in
another" shape. Collapsed to one path: `emit_instance(fn, substitution)` is the single
emitter, and `call_substitution(call, target, args)` is the single place a call's binding
is *read* — positional args (free call), else the analyzer-recorded
`method_call_substitution` (method/operator), else the inherited slice. Corpus
byte-identical (a function's constraint ids are minted in parameter order, so the
sorted-by-constraint key matches the old positional key).

Note: the originally-named pair `generic_dispatch` + `method_call_substitution` is *not*
a redundant channel. `generic_dispatch` selects *which* concrete member an abstract trait
call re-dispatches to (an early-return in the transformer); `method_call_substitution`
drives monomorphization of a concrete generic callee. They are orthogonal, sequential
concerns — co-locating them removes no failure mode, so they are left separate.

## Plan + verification

- v2 is staged per `constraint-queue-plan.md` §Staged migration (scaffold the
  dep-index + the two chokepoints behind today's run-all loop first — **corpus
  byte-identical** — then switch the runner to run-dirty). Every stage gates on the
  corpus and the inference suite; after the run-dirty switch, the `sc_100..800` perf
  benchmark must stay linear (~217/398/788/1547 ms) and a dirty backstop must keep
  cycles from hanging.
- The four repros are pinned as `#[ignore]`d tests in `inference.rs` (the project's
  known-bug convention) — each flips green as the class closes, making progress
  measurable against this doc rather than anecdotal.

## Recommendation

Lead with **item 5 v2 (dependency-driven re-queue)** — the documented next step, the
structural cure for the majority of the repros (the ordering class), and the
prerequisite for item 6. Begin with the scaffolding stage (the dep index + the read /
write chokepoints, *recording* dependencies but still running all-each-pass — provably
corpus-identical), then flip to run-dirty as its own gated stage. Item 6 (stable
generic identity) follows to close class (B).

### Open questions

- **Q1 — v2 scope now, or the targeted composition fix first?** v2 is "the riskiest
  stage." A narrower alternative for class (B) alone: have the transformer *recompute*
  a nested call's substitution by reconciling the callee's parameter types against the
  resolved argument types at emit time (no item 6). Cheaper, transformer-local, but a
  point-fix — against B1's "merge into general code." Lead with v2, or de-risk with
  the point-fix first?
- **Q2 — dep granularity.** Capture deps per `(constraint, expr_id)` (precise, more
  bookkeeping) or per `(constraint, type_id)` (coarser, fewer re-queues)? The
  `constraint-queue-plan.md` chokepoint sketch implies per-expr; confirm before
  building the index.
- **Q3 — measure first?** Before the run-dirty switch, is the all-each-pass fixpoint
  actually a correctness problem (it is — class A) *and* a perf one, or only
  correctness? If only correctness, v2 can keep run-all and *just* add re-queue-on-write
  (re-run a deferred constraint when its input lands) without the full dirty-set
  scheduler — a smaller, safer v2.

## Implementation progress

**Update — class (A) is narrower than "no re-queue."** Tracing the `from_json` repro
(#4) in code refined the diagnosis. It is a **bidirectional-inference** gap, not a
late-binding/re-queue one: the binding *would* be recorded (the function-call arm
already reconciles a call's return type against its expected `constraint`), but the
expected type never reaches the call. Two leaks, both fixable directly:

1. **Constructor propagation — ✅ fixed** (commit pending). `infer_enum_constructor_arguments`
   inferred each argument against the variant's *abstract* declared payload type and
   ignored the expected enum type. It now seeds the enum's parameter bindings from the
   expected type and substitutes the payload before inferring the argument — so
   `Ok(Option::from_json(t))` in a `Result<Option<User>, str>` context types `from_json`
   against `Option<User>`. Verified: the `let`-annotated form round-trips; corpus 69/69.
2. **Return-type-driven body inference — ✅ fixed.** A function's body tail was *not*
   inferred against its declared return type, so `fun g(): Option<User> { Option::from_json(t) }`
   left the binding unrecorded (the abstract decoder). Two pieces, both clean and
   general:
   - A **`ReturnType` constraint** (priority 10, beside `Variable`/the `let`-annotation
     path it mirrors) infers the body tail against the declared return type, so a
     return-position generic call records its binding the way `let v: R = ..` does.
   - An **`expected_types` map**, seeded for the body tail during the walk and
     **propagated through `resolve_match` into each leg body**, carries that expected
     type *through* a `match` (or nested matches) between the call and the signature —
     the RPC-client shape `match .. { "ok" => Ok(Option::from_json(json)) }`. Without
     it the legs were inferred bottom-up (`resolve_match` ran at priority 5, before
     `ReturnType`, and cached the abstract decode).

   Verified: corpus 69/69 byte-identical; `from_json_indirect_element_type_runs` and
   `from_json_return_type_flows_through_match_arm` pin both halves; the RPC example now
   uses the natural `Ok(Option::from_json(json))` directly (quirk #4 retired).

So the `from_json` class is **bidirectional flow**, more contained than the full
dependency re-queue. **All of class (A) is now closed by targeted, general means:**
constructor propagation (#1), return-type body inference incl. through-match (#2), and
— already, before this work — the **late-bound closure-parameter case** (bug c′,
`count.map(|n| format(n))`), fixed by *deferring a call while an argument is an unknown
closure parameter* (the same rule the method-call resolver applies to an unknown
closure receiver; pinned by `format_in_closure_argument`). So **item 5 v2 (dependency
re-queue) no longer has a failing repro to gate it** — its targets are all closed. It
was nonetheless **shipped as a generalization** (replace the all-each-pass loop with one
principled re-queue, per B1's "merge special cases into general code"): a per-resolution
`current_waiting_on` capture, `wake_ready_constraints` re-queuing a deferred constraint
once an input lands, and a run-all backstop that keeps termination — and so the codegen
— identical to run-all by construction (resolution is monotone). Corpus byte-identical,
perf-neutral; details in `constraint-queue-plan.md` stage 14. A maintainability change,
not a bugfix.

**Class (B) / #3 — ✅ fixed, without the full item-6 rework.** Dispatch on a
generic-typed field (`(self.inner).handle(x)`, the RPC client-object form) lowered to
the abstract trait method. The diagnosis pointed at "stable generic identity" (item 6,
a high-blast-radius type-interning rework), but tracing it in the transformer localized
the divergence precisely: the struct field's `T` carried the *struct definition's*
generic id while the call binding was keyed by the *impl/receiver's* id, and
`current_substitution` missed. Two contained root-cause fixes closed it — no global
interning needed:

1. **Field access substitutes the receiver's type arguments** (`resolve_field_accessor`
   matched `Struct(id, _)`, discarding them). `self.inner` now resolves through the
   subject's actual arguments, so it carries the receiver's `T` and the dispatch binding
   composes. This is the id-divergence cure at the one place the two ids meet.
2. **A generic struct initializer doesn't leak an abstract type while deferred.** The
   object stub then exposed a second bug: `let client = Client { transport = t }` (field
   from a variable) grounded `client` as `Client<TraitBound>` because the deferred
   initializer published an unbound type (the type-arg fallback fills with the
   constraint id) that a consumer read before the resolving run. `resolve_struct_initializer`
   no longer publishes while deferred, and `infer_type` returns `Unresolved` for a
   *pending* generic initializer, so the consumer defers until the real arguments land.

Pinned by `generic_field_method_dispatch_runs` and
`generic_field_from_a_variable_dispatches`; the RPC example uses the object stub
directly. The full item-6 type interning is *not* required — the targeted fixes subsume
it. Item 6 remains available only if a future case needs genuinely stable ids that these
local substitutions can't reach.

**One class-A/B case remains open: the `List<List<T>>` round-trip** (the last row of the
bug table). A nested container's inner element binding is not threaded through the outer
`from_json_value`, so the inner decode stays abstract and `List::from_json("[[1,2]]")`
yields `[[undefined,...]]`. The single-level case works (the inherited element type
composes once); the *nesting* is what doesn't compose — the inherited substitution
reaches the outer container's element decode but not the inner one's. This was always in
the table; the earlier "all of B1 closed" note was an overstatement (the case was never
pinned as a test, so it slipped). Now pinned as the ignored
`nested_container_from_json_roundtrip_runs`. It is not in the path P6 needs (the RPC
contract types are flat structs), so it is tracked, not urgent.
