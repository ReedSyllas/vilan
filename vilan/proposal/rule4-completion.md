# Rule-4 completion — the `borrows` root-set and the `bumps` effect (C10 + C6)

> **Status: COMPLETE 2026-07-19 — all four slices shipped same day.** 144d44e (S1
> root-set + a chain-miscompile fix) → d595ed2 (S2 `bumps`) → 9b5e3cf (S3 anchoring +
> the E2 swap; the pass-ordering bug found by the sweep; ZERO fallout incl. kolt) →
> the S4 commit (iterator-chain pins proving the ForEach arm composes; spec §6.4 +
> errors-appendix language matched to enforcement — the appendix had never had the
> rule-4 entries at all). Residues live in the slice notes below; backlog items 6/10
> are closed. Original plan follows. One arc, two backlog
> items that are one machine: **C10** (call-returned views and wrapped-view captures are
> invisible to rule 4 — `compute_view_origins` anchors only direct `&place` bindings,
> and `Function.borrows` is a bare bool recording no projected root) and **C6** (E2
> fires on any `&mut`-convention call — the "may bump" proxy — producing the recorded
> conservatisms). They land together because anchoring without precision over-rejects:
> the moment `list.at(0)` / `arena.get(h)` / option-views join the anchored set, E2's
> coarseness multiplies across std. C6 is the valve that keeps legitimate code
> compiling through the tightening. **Sequencing decision (user, 2026-07-19): this arc
> precedes A13** — it is breaking-flavored (rejects programs that compile today), so it
> is cheapest now, before more code is written against the blind checker. Classification
> under `claims-and-epochs.md` §4: checker-precision refinement, both directions —
> tightening (C10 anchors what the law always covered) and loosening (C6 stops flagging
> geometry-stable calls). The law is unchanged.

## 1. The design

**The `borrows` root-set.** `Function.borrows: bool` becomes a set of projected
*parameter positions* (receiver = position 0). Inference extends the existing
machinery: where `infer_borrows` / `derives_from_view_param` /
`function_returns_wrapped_view` today answer "does a returned view project *a*
parameter", they record *which*. Chains compose by fixpoint over the call graph (the
sixth verse of the inferred-effect worklist): a leaf that is itself a borrows-call maps
the callee's root-set through that call's arguments into the caller's parameters.
Explicit `borrows self` annotations keep working (they name a position); no new surface
syntax — the set is inferred, shown in hover like the other effects.

**Origin seeding.** `compute_view_origins` gains two sources beside `&place` bindings:

- a binding (or expression use) of a **borrows-call result**: origins = the call's
  argument places at the callee's projected positions, resolved to their roots;
- a **wrapped-view `match` capture** (`Some(let v)` over a wrapped-view call or inline
  transient): the same mapping, through `compute_wrapped_view_captures`.

Origins compose transitively exactly as view→view copies already do. The scalar-root
exemption keys off the *root's* shape as today (§6.0's per-owner event table).

**The `bumps` effect (C6).** Per function, per `&mut`-convention parameter (receiver
included): **content-stable** (field/element writes only — the epoch does not advance)
or **bumping** (may resize / reassign / remove / drop through it). Base facts: a curated
table for the native container surface (`push`/`pop`/`insert`/`remove`/`clear` bump;
element `set` and field writes are stable; `Map`/`Set` insert/remove bump). User
functions infer by fixpoint over bodies: whole-reassignment of (a place rooted at) the
parameter, a bumping call on it, or passing it onward to a bumping position ⇒ bumping;
otherwise stable. Unknowable callees — externs off the table, dispatched calls —
default to **bumping** (the safe direction). Inferred-only in v1; an explicit `bumps`
annotation is recorded future surface (the `borrows` keyword precedent exists if
wanted).

**Enforcement swap.** E1 unchanged (reassignment of a viewed root). E2 fires when a
call passes a viewed root (or a place rooted at it) to a **bumping** position — no
longer on every `&mut` convention. E3 unchanged in rule (every live view fences
`await`) but now sees the newly anchored views — this is where honest fallout lives:
code holding a call-returned view across `await` compiles today and will stop. Of the
two recorded E2 conservatisms, the arc cleared ONE *(corrected 2026-07-19 by the
fresh-eyes review — the shipped claim overstated)*: the scalar-field-view-under-
`&mut s` case clears via the callee's stable verdict (pinned). The generic-typed-ROOT
conservatism REMAINS — the root-side scalar exemption keys on `is_scalar_view_pointee`,
false for a still-generic root, so a bumping call on one is still flagged even when the
instantiation is scalar. Over-rejection (safe direction); clears when `bumps` gains
per-instantiation verdicts (the recorded S2 residue). Second reviewer note, same
family: `call_bumps_positions`' dispatched default gates onward-bumps on
`index == 0 || &mut-reference`, so a BARE-forwarded view parameter at position > 0 to a
dispatched callee is missed — a distinct path of the recorded view-binding-argument
under-approximation. And the review BLOCKED the first S4 state on a real one:
`scan_bumps` omitted `Expr::TupleComprehension` (the parity sweep's line window cut off
one arm short of `scan_move`'s list), silently reading comprehension-body bumps as
content-stable — a demonstrated-on-JS soundness regression, fixed + pinned
(`a_bump_inside_a_tuple_comprehension_is_rejected`).

## 2. Slices (each suite-gated, docs same commit, per-case pins)

1. **S1 — the root-set, inference only** (no enforcement change; validation-only,
   corpus byte-identical): `borrows` becomes the position-set through
   `infer_borrows`/annotation parsing/hover; wrapped-view recording carries positions;
   the call-graph fixpoint. Pins: direct projection, chained projection (a borrows fn
   whose leaf is another borrows call), multi-parameter projection (a view of either
   argument by branch → both positions), wrapped-view position, explicit-annotation
   agreement, hover shows the set.
2. **S2 — `bumps` inference, effect only** (no enforcement change): the native base
   table + the body fixpoint + per-instantiation verdicts for generic callees (the R11
   instance machinery). Pins: each native table row, a user stable fn (field/element
   writes), a user bumping fn (push-through, reassign-through, onward-pass), the
   extern-default, the dispatched-default, a generic callee verdict at two
   instantiations. *(S2 resolution, 2026-07-19: generic callees take the
   instantiation-INDEPENDENT conservative verdict — a dispatched/T-typed call defaults
   to bumping — because with no enforcement consumer the per-instantiation distinction
   has no S2 observable; S3's per-instantiation E2 pins are where precision lands, on
   the R11 instance machinery. Second recorded residue: a view-binding argument
   (`let v = &mut xs; unknown(v)`) roots at the binding, under-approximating — S3's
   origin mapping resolves it.)*
3. **S3 — anchoring + the E2 swap, together** (the breaking slice): *(Shipped
   findings, 2026-07-19: the sweep's first run failed seven E2 pins — ALL from one
   bug, S2's table+fixpoint parked at the END of `analyze()` while
   `check_invalidation` runs earlier, so E2 consumed unseeded (∅ = universally
   stable) verdicts; the block now runs beside `infer_borrows`, before every
   consumer — the pass-ordering lesson: an effect landing "inference-only" must
   still be positioned for its future consumers. After the fix: ZERO fallout —
   std, corpus, docs, examples, and kolt all compile under the tightening; the
   arena rule-4 pin un-ignored with `insert` as its invalidator since `set` is now
   correctly stable — its old body pinned the coarse behavior.)* origin seeding
   from call results + captures; E2 keys off `bumps`; E3 sees the anchored set;
   un-ignore `arena_mutation_under_a_live_get_view_is_rejected`. **The fallout sweep
   is the acceptance**: std, corpus, docs, examples, and kolt (read-only check) all
   compile — any hit is per the E2/E3 precedent a std-pattern redesign or a
   proposal-level stop, never a checker special-case. Pins: the C10 shapes
   (`list.at(0)`+push, `arena.get(h)`+insert, wrapped capture + mutation, across-await
   with a call-returned view), the C6 relaxations (stable `&mut` call under a live
   view now accepted; the two recorded conservatisms cleared), E1 still fires, sibling
   roots unaffected, scalar-root exemption preserved.
4. **S4 — the iterator chain + residues + docs**: `for e in &mut user_container`
   origins compose through `next_mut` (two hops: binding → iterator → container);
   spec §6.4/§6.5 language updated to match enforcement (they already state the
   intent), errors appendix entries, tour Traps updates; residues recorded honestly
   (whatever S3's sweep defers); backlog C10/C6 closed; `Weak.get`'s C1 note updated
   (it inherits the fix).

## 3. Risks, recorded

- **The E3 fallout is the unknown.** Newly anchored views across `await` in
  std::reactive/ui (option-views, `Shared.write()` results are already fenced — but
  wrapped shapes may lurk). S3's sweep decides; a legitimate pattern with no clean
  re-acquire form is a stop condition.
- **Over-anchoring through copies**: a borrows-call result stored then re-derived must
  not double-count roots (dedupe as view→view copies already do).
- **Bodiless trait signatures under-mark `borrows`** (found by S2's dispatched-default
  bug — a bodiless method's never-inferred empty set must not read as a known
  verdict): a dispatched view-returning trait method leaves its call results
  unanchored. S3 must decide the dispatched-borrows default (honor an explicit
  `borrows` clause on the trait signature; without one, a view-returning dispatched
  call conservatively projects every `&`/`&mut` argument).
- **The native table is a judgment surface** — keep it small, comment every row, and
  bias to bumping when unsure; a wrong "stable" is a soundness lie, a wrong "bumping"
  is a false rejection someone reports.
- **Cost**: two more fixpoints per analysis; both are call-graph-shaped like the four
  that exist. Measure if LSP latency moves (E3's per-analysis budget note).
