# Claims and epochs — one law for the memory model, and its closure

> **Status: RATIFIED 2026-07-18 (drafted and ratified the same day) — descriptive +
> classificatory; changes no semantics by itself.** Companion to `destruction.md` (C4). Three jobs: **(1)** state the single law
> every shipped memory-model mechanism already enforces, so views / handles / `Shared` /
> `Weak` / resources read as two projections of one relation instead of five vocabularies;
> **(2)** show the resulting table is *complete*, and define "refinement" vs "major
> change" precisely — recording the standing decision (2026-07-18) that **C4 is the last
> major change to the memory model**; **(3)** settle the asymmetries the table exposed:
> two Tier-2 clarifications folded into `destruction.md` §10 (`Weak.get`, the trap law),
> two refinements queued (backlog **C6** geometry effects, **C7** wire-blessed handles).
> **Ratified 2026-07-18** — §8's three decisions accepted per recommendation: the closure
> rule stands, C7 is blessed under §6's conditions, and the frame graduates into spec §6
> at C4's S5 (build sequence: `destruction-impl-plan.md`). **Graduated 2026-07-19:**
> spec `memory.md` now opens with the law as §6.0 and carries resources as §6.8 — this
> document remains the design record and the closure rule's statement; the spec is the
> normative form.

## 1. The law

Vocabulary:

- An **owner** is any value with independent existence: a binding, a container, an arena
  slot, a counted cell, a resource.
- Every owner has an **epoch** — an abstract counter that advances on a fixed set of
  **events**, determined by the owner's shape:

  | owner shape | epoch events |
  |---|---|
  | scalar cell (boxed local) | rebinding, death |
  | aggregate (struct root) | + reassignment of an aggregate field out from under an interior view |
  | container (`List`, arena slots) | + geometry: resize, insert, remove, reallocation |
  | counted cell (`Shared`, C4 Tier 2) | + the strong count reaching zero |
  | resource (C4) | + the move (the source binding's epoch ends), + `drop` (the final event) |

- A **claim** is any alias to (a place inside) an owner: a `&`/`&mut` view, an
  `Arena` `Handle`, a `Weak`, a loan of a resource. Semantically a claim is
  *(owner identity, path, epoch at capture)*.
- **The law: a claim is valid while its owner's epoch is unchanged.** Nothing else is
  forbidden — aliasing is fine, aliased writes are fine (rule 4's recorded trade against
  Rust); only using a claim whose epoch has passed is not.
- A **suspension point** (`await`) is the degenerate event: while suspended, other turns
  run, so *every* owner's epoch must be assumed to advance. `await` bumps the world.

That is the whole model. Everything below is enforcement.

## 2. Two regimes, one relation

A claim's validity can be established at exactly two times:

- **Static discharge — views.** The compiler proves no event occurs between capture and
  last use. The proof needs a *surveyable interval*, which is why views are second-class:
  lexical liveness (declaration to block end) is the interval the analyzer can audit.
  Because the proof is total, access is infallible and free — a view compiles to a bare
  `(base, key)` and never checks anything at runtime.
- **Dynamic carry — handles.** The claim outlives every surveyable interval (stored in a
  field, kept across `await`, held between turns), so it carries its epoch as data —
  `Handle.generation` — and every access re-establishes validity by comparison:
  `get → Option<&T>`. Because the check runs at use time, access is failable, and the
  failability is *in the type*. **A handle is a view that survived by carrying its proof
  obligation with it.**

Rule 3's ban sites (views in fields, collections, returns-without-provenance, across
`await`) are not restrictions bolted onto views — they are the frontier of static
provability. Crossing the frontier does not make the claim illegal; it changes its
enforcement regime. Today the language expresses that crossing by switching data
structures (drop the view, mint a handle); the table in §3 is why that is the same
operation everywhere it appears.

Two properties fall out and are hereby design invariants:

- **Same law, both regimes.** The dynamic checker must enforce the same event set as the
  static checker — no stricter (that fossilizes a false rule into a runtime trap), no
  looser (that reopens the hole). §5(b) fixes the one place the documents currently
  disagree.
- **Failability is honest.** Where Swift hides the regime (its runtime exclusivity
  checks are traps discovered in production), vilan's dynamic tier answers in `Option`.
  The type tells you which column of §3 you are standing in.

## 3. The table

| concern | static (proof) | dynamic (check) |
|---|---|---|
| interior access | `&`/`&mut` views; `borrows` provenance | `Arena.get(h) → Option<&T> borrows self`; `Weak.get` (§5a) |
| invalidation | rule 4 E1 (reassign) + E2 (mutating call) | generation mismatch → `None`; C2 runtime generations (→ F4, debug) |
| suspension | E3: no view across `await` | handles cross freely; the next `get` re-validates |
| death | resources: use-after-move error; drops placed statically (R7) | stale handle → `None`; `Weak.upgrade()`/`get()` → `None` |
| exclusivity | declined — aliased views + content writes are legal | `Shared` traps on *invalidating* overlap only (§5b) |

Reading down the columns: the left column is what the compiler proves when liveness is
lexical; the right column is the identical relation carried as data when the claim must
outlive the compiler's sight.

Every shipped rule maps into a cell — the checklist that nothing lives outside the frame:

- **Rules 1–2 (copies + elision).** Copies have no claims to police. Elision's "last use
  *including through views*" is the law applied to the compiler itself: reusing storage
  is a death event, illegal under a live claim. The optimizer is just another
  claim-holder.
- **Rule 3 (second-class).** The static column's interval requirement (§2).
- **Rule 4, E1/E2/E3.** The static invalidation and suspension cells. The E2 scalar-root
  exemption is §1's owner-shape table read off directly: a scalar cell has no geometry
  events, so no `&mut` call can bump it — not a special case, the first row. (E2's two
  *recorded conservatisms* are a proxy problem — §5c.)
- **`borrows` provenance.** Claim origins made explicit at the return boundary —
  provenance without duration, which is why it never becomes lifetimes.
- **`Arena`/`Handle`.** The dynamic column, verbatim: `generation` *is* the epoch, `get`
  *is* the validity comparison, `remove`'s bump *is* the death event. (One shipped-vs-
  specified gap: std's `arena.vl` still returns `Option<T>` *by copy* from `get` — the
  interim API recorded when the arena landed before wrapped views existed. The rev-1
  view form became expressible with the Phase-5 wrapped-view work; **C8** migrates std
  to it.)
- **`Shared` (today, on JS).** A copyable owner-*carrying* alias; `read()` returns a copy
  (no claim), `write()`'s view is second-class and E3-fenced. Tier 2 gives the cell real
  epochs (count-to-zero); §5(b) states its trap law.
- **`Weak` (C4 §10).** A handle whose arena is the counted heap: `upgrade`/`get` is the
  validity check, the zero-crossing is the epoch event. Not a sixth mechanism — the
  dynamic death cell.
- **Resources (C4 R1–R12).** The static death row: use-after-move is the compile-time
  twin of the stale handle's `None`; R7's move-on-every-path-or-none rule is what keeps
  the final event statically placeable; `drop` is the final bump. R9/R10 (no captures,
  no container elements) are the same frontier as rule 3's: places static discharge
  cannot survey.

## 4. Completeness — the closure argument

**The concern axis is exhaustive** because it enumerates what can befall an owner: its
contents change (never invalidating — that is what writes *are*), its shape changes
(geometry), its binding changes (rebind / move), it dies (drop, zero, remove), or an
interval becomes unsurveyable (suspension). There is no sixth fate. **The regime axis is
exhaustive** because a proof happens at compile time or at use time; there is no third
time. A new memory-model ask therefore lands in an existing cell — and a proposal that
cannot name its cell is describing a different language.

**Classification.** A **major change** is anything that adds an alias kind, adds an epoch
event kind, or weakens the law. A **refinement** is anything that (a) improves a
checker's *precision* within the law (accepts more programs, changes no runtime
behavior), (b) adds std surface over existing mechanisms, (c) implements
already-specified semantics on a new backend, or (d) relaxes a recorded v1 conservatism
along its recorded path.

**The decision (2026-07-18): C4 is the last major change.** With C4 ratified, both cells
of the death row exist and the table is fully populated — Tier 2 is *specified inside
C4* (§10), so building it later is implementation, not change. Everything on the board
classifies as refinement:

| open item | classification |
|---|---|
| C4 Tier 2 build (counting, `Weak`, counted closure environments) | specified in C4 §10; native-arc implementation |
| C2 dynamic remainder (runtime generations, poisoned views) | the dynamic invalidation cell; F4, debug-mode |
| **C6** geometry effects (`bumps`) — §5c | precision refinement of E2 + the shared event source for the dynamic checkers |
| **C7** wire-blessed handles — §6 | std surface + docs idiom over an existing mechanism |
| **C8** `Arena.get` view-form migration — §5a | std catching up to rev-1's specified surface; the compiler machinery already shipped |
| computed projections (rev-1: the `_modify`-style coroutine lowering) | a new *lowering* for the same `: &T borrows self` surface; claim semantics unchanged |
| `Store<T>` trait (rev-1) | surface; its trigger arrives with Tier 2 (§5a) |
| `List<R>` move-in/view-out (C4 R10's v1.5) | recorded relaxation within the affine rules |
| R7 drop flags (C4 open (c)) | recorded relaxation |
| F3/F4 lowering (allocator, scope-end drops, ARC) | backend implementation of specified semantics |
| Part C workers (J1) | inherits §6's answer: copies and names cross; addresses and owners don't |

**The one watched item: async drop.** C4 §5 rejects awaiting destructors for v1
("teardown must be synchronous"). If awaited teardown is ever truly needed, the closure
stance is: meet it with *surface* — an explicit `await conn.close()` before scope end, an
owner whose `drop` cancels through `OwnedNursery` — never by making `drop` itself a
suspension point. An async `drop` would fire a new event kind inside the suspension row's
blind spot (drops suspending mid-unwind, across turns), and it is the one direction this
document marks **out**, not deferred.

**The honest limits, restated** — the law does not promise these, and no future mechanism
may be admitted on the argument that it would: leak-freedom under an unbridged,
never-settling `await` (C4 §5's recorded limit; structured concurrency's job, not the
memory model's), and cross-handle aliased-write detection on JS (semantically empty
there; C2/F4's debug machinery on native).

## 5. What the frame exposed — settled where

- **(a) `Weak.get` — folded into `destruction.md` §10.** `upgrade(): Option<Shared<T>>`
  is the ownership-extending form; the access row demanded the scoped twin:
  `get(&self): Option<&T> borrows self`, mirroring `Arena.get`'s *specified* form
  (rev-1; shipped std still carries the interim copy-returning `get` — **C8** migrates
  it). Every dynamic alias then answers the same verb with the same `Option<&T>` shape. This also delivers
  the *second store* rev-1's deferred `Store<T>` trait was waiting for (`Shared`/`Weak`
  is a one-slot counted arena: `clone` = retain, `Weak` = the handle, zero = the bump) —
  extract the trait when Tier 2 builds, not before.
- **(b) The trap law — folded into `destruction.md` §10.** Rev-1 specified "a `write()`
  while any other view is live traps" and called it the runtime form of rule 4 — but it
  is *stricter* than the static rule, which deliberately permits aliased views and
  content writes and forbids only invalidation. Mismatched twins are exactly how a model
  splits. The reconciled law: the dynamic check enforces the same event set as the static
  check — trap on reassignment / geometry / death under a live claim, never on
  overlapping content writes. Spec §6.4's implementation note and §6.7's "exclusive"
  parenthetical currently state the stricter form; C4's S5 re-words them (recorded in
  `destruction.md` §12).
- **(c) E2's conservatism is a proxy problem — queued as backlog C6.** E2 keys off
  "`&mut` convention" as an approximation of "may bump the root's epoch"; its two
  recorded conservatisms (a scalar-field view under a `&mut s` call; generic-typed
  roots) are precisely where the proxy is coarse. The root fix: infer a **geometry
  effect** per function — for each `&mut` parameter, receiver included,
  *content-stable* (field and element writes only) or *bumping* (may resize / reassign /
  drop through it) — surfaced, like `borrows`, as an inferred `bumps` clause.
  `set_all(&mut self)` stops invalidating; `push` keeps invalidating; nothing is ever
  annotated. Fifth verse of the inferred-effect worklist (async, platform, contexts,
  `borrows`), and the shared event classifier for Tier 2's trap law and C2's
  generations: one law, one classifier, two checkers.
- **(d) Wire-blessed handles — queued as backlog C7; decision wanted (§8b).**

## 6. Names cross, addresses don't — the distribution alignment

A view is an **address**: present tense, this frame, free. A handle is a **name**:
durable identity plus the epoch to re-validate against, and no access of its own. This is
why the same alias that survives `await` is the one that can cross the wire — a name is
pure data (`Handle` is a pair of plain integers; `Wire` derives mechanically), while an address cannot
leave its frame and an owner-carrying alias cannot leave its machine:

| | view | handle | `Shared` |
|---|---|---|---|
| carries | an address | a name + epoch | the owner itself |
| crosses a block boundary | no (second-class) | yes | yes |
| crosses `await` | no (E3) | yes | the cell yes; claims through it no |
| crosses the wire | no | **decidable — recommend yes** | no (identity is machine-local) |

Taken seriously, this reframes `Arena` from "the escape hatch for graphs" into **the
naming layer of the full-stack story**: a server-side arena whose handles flow to clients
as stable entity references is the shape `Draft` commit targets, router entities, and
"update node X" RPCs already want. Stale-handle → `None` becomes the *distributed*
staleness story for free — a client acting on an entity another client deleted gets the
same clean `None` as local code holding a stale handle. No phantom writes, one rule from
a local `List` to an RPC boundary. No language on the §7 board can tell this story; it is
what makes the advanced corner earn its complexity twice. Part C's workers inherit the
same answer unchanged: sendability is `Wire`, so copies and names cross, addresses and
owners don't.

**The capability note (C7 must honor it):** a wire-crossing handle is a capability, and
`(index, generation)` is guessable. The blessed idiom is per-session / per-scope arenas —
a handle from one session names nothing in another; anything cross-tenant adds a
per-arena random **brand** checked alongside the generation. Blessing without this note
would trade a memory-safety non-bug for an authorization bug.

## 7. The design space — this point, on purpose

The graded middle is flanked by two coherent corners, both declined with reasons — the
justification record for the model's complexity:

- **All-dynamic (Vale).** Generational references everywhere: no rule 4, no E3, no
  `borrows` — and every dereference pays a check and answers in `Option` (or traps).
  Forfeits "read the cost off the source" and the single-conformance property (a
  JS-compiling program has no native aliasing UB). Vale ran this experiment so vilan
  doesn't have to.
- **All-static (regions / brands).** Handles statically tied to their arena by a branded
  type parameter (Cyclone regions, GhostCell) skip the runtime check — by threading
  invariant lifetime-like parameters through types: the exact machinery rule 3 exists to
  delete. The on-ramp back to lifetimes.

What the comparison languages settle, one line each (the evidence base):

- **Rust** — every static discipline grows a dynamic twin (`RefCell`/`Rc`); making the
  twins asymmetric in API and failure mode is what makes a dynamic tier feel bolted on.
  Its ecosystem fled first-class borrows for index handles at scale (rustc itself, every
  ECS, the GUI crates) — vilan blesses in std what Rust cannot retrofit. `Pin` is the
  standing proof of E3. Polonius's origins-as-loan-sets is `borrows` provenance under
  another name.
- **Swift** — the nearest cousin (value semantics, CoW, `~Copyable` ≈ C4's class,
  `_modify` ≈ computed projections); proves the static-where-provable /
  dynamic-otherwise pairing at mainstream scale, and warns: hiding which regime the code
  is in (runtime exclusivity traps) is the mistake. Vilan's `Option`-shaped dynamic tier
  is the correction.
- **C#** — the strongest external validation of the exact pair: `Span<T>`/`ref struct`
  is a second-class view (no fields, no lambda capture ≈ R9, no crossing `await` ≈ E3 —
  shipped for years, in a GC language); `Memory<T>` is the storable name you *present*
  (`.Span`) to touch. Same rhythm, grown ad hoc, never unified in the teaching — the gap
  this document closes for vilan.
- **Pony** — capabilities-as-deny is the clean theory; six of them is the usability
  cliff (never surface a lattice). Its `tag` capability (identity-only: holdable,
  comparable, sendable, unreadable) is the handle, independently derived.
- **Zig** — allocator-as-parameter proves the "present the owner to use the name"
  ergonomics; arena bulk-free is the composition argument C4 makes against per-object
  `defer`.
- **Go** — the control group: no model. Its costs (map races, function-scoped `defer`,
  hand-threaded cancellation) price what rules 1–4 buy; its virtue (most code needs no
  vocabulary at all) is already captured by rules 1–2 being the whole story for ordinary
  code.
- **Hylo** — mutable value semantics and subscripts-as-projections: the academic
  backbone of rules 1–3 (rev-1 already cites it).
- **Vale** — the all-dynamic corner, occupied; also "higher RAII" as the contrast to
  C4's silent scope-end drops.

## 8. Decisions wanted

> **All three ratified 2026-07-18** per recommendation (see the status block). Kept
> below for the record.

- **(a) Ratify the closure rule** — §4's major/refinement definition, the standing
  decision that post-C4 memory work must name its cell, and async drop marked out-not-
  deferred. Recommendation: ratify.
- **(b) C7 — wire handles**: bless `[derive(Wire)]` on `Handle<T>` plus the documented
  naming-layer idiom (recommendation: bless, with §6's capability note as a hard
  condition), or ban handle serialization and keep names process-local.
- **(c) Where this frame lives**: stays a proposal-side companion, or graduates into the
  spec as the introduction of the memory chapter (§6) when C4's S5 lands.
  Recommendation: graduate — the spec should teach the law once and derive the
  mechanisms, escape-ladder style, rather than teach five vocabularies.
