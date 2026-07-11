# Vilan Backlog — everything outstanding

A running capture of work that is *known but not done*, so nothing is lost to conversation. This
is the tactical companion to [`roadmap.md`](roadmap.md) (the ranked strategic view); items that
`roadmap.md` already tracks are cross-referenced by number rather than duplicated in full.

Per the project's engineering principles (see `CLAUDE.md`): each non-trivial item below should get
a **formal definition + unit tests + regression tests** before it is implemented, and should be
built to subsume special cases rather than patch them. Items carry a rough size (S/M/L) and known
dependencies. Unordered within a section.

Item numbers are **stable identifiers** (other documents cite them — `backlog F3`, `I2`):
completed items are removed and their numbers retired, so numbering within a section may
have gaps.

---

## A. Reactive core & UI (`std::reactive`, `std::ui`)

3. ~~**`bind_each` keyed reconciliation**~~ — **SHIPPED 2026-07-07**: rows move with their
   keys, a changed row re-renders (`T: PartialEq`), removed rows dispose + leave the DOM;
   the plan is `std::reactive::reconcile` (pure, node-tested — corpus `reactive-keyed.vl`
   + pins), applied by `bind_each` (appending a kept element MOVES it, so ordering is one
   append per row). `Owner.defer` added for non-`Disposable` teardown.

4. ~~**`flatten` reactive combinator**~~ — **SHIPPED 2026-07-07**: `outer.flatten()` on
   `Signal<Signal<U>>` (a nested-generic impl subject) follows the current inner and
   DETACHES a replaced one (corpus `reactive-flatten.vl` + pin). Internal subscriptions
   follow `map`/`combine`'s unowned precedent; the rolling inner subscription is disposed
   per switch.

5. **Ambient owner / `comp` ergonomic layer** (`proposal/ambient-owner.md`) — **COMPLETE
   2026-07-07** (basics + `comp` + B15 + the `std::ui` boundary-ownership integration:
   owner-less `View`, ambient `bind_*`, per-row owners, `when`, `mount_root`; remaining
   tails recorded with triggers: `get_safe`, fence-diagnostic anchoring). History: `owner_scope` (a `Context<Owner>`), `get_owner()`, and
   `Signal.effect` (the scope-tied `sub` — registers into the ambient owner, nothing to
   hold; misuse outside an extent is a COMPILE error via the context coverage fence). The
   substrate was proven against stored callbacks AND async first (probes: capture survives
   extent exit and `await`; interleaved extents each keep their value). Findings: a
   `run_with_owner(owner, body)` wrapper FUNCTION is impossible by the context model's own
   rules (`run` needs a closure literal; capture is at CREATION — a forwarded body is born
   outside the extent), so the extent is entered as `owner_scope.run(owner, || ..)`; macro
   sugar can restore the wrapper spelling later. The coverage check gained the DEAD-reader
   exemption (an uncalled, un-taken, non-top-level function cannot run uncovered) — without
   it every `std::reactive` importer failed. ~~`effect` on the `Source` trait~~ (shipped
   with B14's fix). ~~`comp` sugar~~ (shipped on B15 + value-returning `Context.run<U>`
   — the `batch` shape; `run_with_owner` yields too). ~~`std::ui` integration~~ (shipped:
   the boundary-ownership model — the fold-scope-into-View question resolved as
   `mount_root`/`comp` roots owning everything ambiently).

6. **Reactive turns — scoped flush + async turns** (M–L; **CORE SHIPPED
   2026-07-09** — `get_safe` + `Turn`/`turn_scope`/`flush`/`turn`/`batch` in
   `std::reactive` (injected bodies; drain-affinity stack for mid-settle
   cascades — the one runtime device; per-turn dedup + budget) + the server
   boundary (`[service]` routes wrap their bodies in per-dispatch `turn(AtEnd, ..)`;
   manual `dispatcher.on` handlers self-`batch`, as the coalescing benchmark now
   spells). The `std::ui` boundary shipped same-day
   (View.on/bind_value/mount_root wrap dispatches in turns via plain host-stored
   adapters), riding two B15 extensions: clauses on `let` annotations and clause
   ADOPTION (an unannotated closure-literal binding passed into a clause position
   adopts it — the idiomatic `let add = || ..; .on("click", add)` just works).
   Continuation settling shipped same-day too: a write landing after the turn
   settled schedules ONE microtask drain (`queue_microtask` extern), so each
   async continuation segment settles as a coalesced wave — no compiler
   insertion needed, the policies converge for async extents (a true
   held-across-await `AtEnd` = `turn_async`, recorded). `turn_async` +
   `optimistic` shipped same-day, closing the follow-ons: `turn_async(body)` =
   the TRUE transactional extent — every notification held until the body's
   whole async chain completes, one coalesced settle (spawn-then-await over
   the J2 gap); `optimistic(signal, value, commit)` = paint now, await the
   commit, reconcile to the confirmed value or roll back, returning the
   outcome. **A6 is COMPLETE**; the cadence split for directly-awaiting
   `turn` bodies is the one recorded refinement. Original
   design: **proposal: `reactive-turns.md`, 2026-07-09** — supersedes the original "auto-flush on the next
   microtask" sketch, which a review scenario killed: the scheduler's single global
   pending queue means one request's `flush` drains every interleaved request's
   notifications, and a global microtask hook makes that routine). The redesign: a
   `Turn` (queue + policy) established through `turn_scope: Context<Turn>` at
   boundaries — UI events/`mount_root` (`AtSuspension`: settle at each await +
   end, the optimistic-paint cadence), `serve_connected`/RPC dispatch (`AtEnd`:
   transactional) — with `set` routing via `get_safe` (no turn → inline, status
   quo), `flush` draining only the ambient turn, `batch` dissolving into
   join-or-create. Context capture-at-creation makes a request's turn follow its
   own awaits (the A5 probes). Prerequisite sub-slice: **`get_safe`** (the A5
   tail's first real consumer). Honest limit recorded: turns isolate NOTIFICATION
   waves, not value visibility on shared signals (eager commit; last-flush-wins).
   The optimistic-write → reconcile lifecycle remains the follow-on, riding turns.
   C3 shipped, so nothing blocks this.

7. **Server-side rendering (SSR) + hydration vs resumability** (L–XL; recorded 2026-07-08;
   proposal first) — render the initial UI as HTML on the server (first paint before any JS,
   SEO), then make it live on the client. Vilan's model is unusually well placed: the UI is
   fine-grained reactive (no VDOM — Solid's shape, where SSR is proven), the compiler already
   builds client AND server bundles from one program (the full-stack split), and value
   semantics make the state handoff mostly plain data (views are second-class and never
   stored — nothing dangling to serialize; `Shared` identity is the one careful spot).
   - **What server rendering needs regardless of strategy:** a render-only target for
     `std::ui` — `View` over an HTML string-builder (or DOM shim) instead of `document`,
     legal on `@process` where the platform gate today forbids the browser layer (a
     `_sys`-style seam: same interface, an HTML impl on the server — the platform model's
     §5 shape). Effects/subscriptions must NOT run server-side; server render is
     create-serialize-discard (A5's boundary owners just never get disposal work).
   - **Hydration** (the Solid/React lineage): the client re-runs the component tree, but a
     hydrating DOM adapter CLAIMS existing server nodes instead of creating them —
     `bind_text` adopts the server text rather than rewriting, listeners attach, signals
     re-create from serialized initial values. Needs deterministic node addressing
     (hydration markers) and a first-run-adopts discipline in the `bind_*` effects. The
     well-trodden path; maps 1:1 onto `std::ui`'s ambient bindings.
   - **Resumability** (the Qwik lineage): the server serializes enough that the client
     resumes WITHOUT re-executing components — event handlers become addressable entry
     points loaded on demand. JS frameworks contort to get this (every handler manually
     `$`-split); **vilan owns closure conversion in the compiler**, so lowering each
     handler to a top-level function + an explicit serialized environment record (Wire
     already exists) is a compiler pass, not a user convention — the language is genuinely
     better positioned than the JS ecosystem here. Still the research-grade option:
     `Shared` graph serialization, lazy chunk loading, event delegation before JS loads.
   - **Recommended shape:** v1 = server string-renderer + hydration (proven, incremental,
     every piece reusable later); resumability recorded as the ambitious follow-on riding
     the same render target and serialization format. Streaming SSR / suspense boundaries
     are beyond-v1 (interact with A6's async turns and J1). Dependencies met: platform
     model, P6 transport (data fetching), A5 boundary ownership.

8. **UI styling — typed atomic styles, compiled** (L; `proposal/ui-styling.md`,
   REVISED 2026-07-10 — **expression-flavored, rides `const` (G2); syntax settled**) —
   the last big hole in the UI model. Styles are typed values built by ordinary
   const-evaluated property functions (`let card = const display(Display::Flex) +
   padding(space(4)) + hover(background(gray(100)));`) lowered at compile time to
   deduplicated atomic CSS through the const-eval **asset channel**; merge is `+`
   (`impl Style with Add`) with per-property last-wins — record semantics, so
   specificity fights are structurally impossible; const merges fold, runtime merges
   are a map union, never string parsing. Variants are plain `match` over const
   styles (CVA dissolves), governed by the load-bearing **construct-in-const rule**:
   property functions bottom out in const-only `asset::emit`, so a runtime
   construction is a STATIC error and every variant's CSS exists at build time.
   Tokens: themeable ones (space/color/type) lower to CSS custom properties
   (re-theming and dark mode = property swaps; signal-driven values ride `var()`);
   structural ones (breakpoints) resolve to literals at const time — the first
   draft's config knob dissolved. The macro-DSL first draft is superseded (git
   history keeps it): the expression form gets hover/go-to-def/typed diagnostics
   for free and composes with functions/impls natively. Tailwind stays a documented
   SIDECAR bridge, not the foundation. Order: G2 slices 1–2, then std::ui::style
   (slices 3–5), A7-entangled tail (critical CSS, dead-style elimination) later.

9. **`vilan.toml [build] run` hooks** (S; spun off A8) — run external commands
   alongside `vilan build` / `--watch` (the Tailwind-bridge runner, asset pipelines,
   codegen sidecars). Useful independent of styling.

---

## B. Type system & the type solver

3. **Variadic-generics deferred tail** (M–L; `variadic-generics.md` §Deferred) — shipped:
   flat-tuple lowering, mapped tuple types `(U in T: F<U>)`, tuple comprehensions, `combine`.
   **Not done:** `keyof`; spread parameters (`...items: T`); elision of the flat-tuple
   construction copy; **enforcement** of arity bounds `T: (2..)` and tuple element bounds
   `(..: Display)` (parsed, not checked); trait-typed-value dispatch (B4).

4. **Trait objects / dynamic dispatch** (L; own proposal when demanded) — a value typed as a bare
   trait (`let x: Display = …`) is a clean compile error today (the silent-miscompile half was
   fixed). Making it *work* by value needs a runtime representation (a `(value, vtable)` pair /
   `Box<dyn>`-style) — a real language feature; nothing uses it today.

6. **Closure-return element inference gap** (M) — a method whose **result element** comes from a
   field-access closure return (`xs.map(|p| p.x)`) types as `List<unknown>` instead of `List<i32>`.
   Root: `map` binds its result generic `U` from `infer_type(closure return)` while the closure's
   `p.field` accessor is still in-flight, so `U` commits wrong. A general fix (in-flight reports
   `Unresolved`, dependents defer and wake) fixed the literal case but deadlocked the slot case
   (`List::new()`+`push`+`map().sum()`), so it was reverted — the clean fix needs the slot-fill and
   closure-return resolutions both observable to the wake (its own slice). Common uses (`sum`,
   `for`, arithmetic over the mapped element) work today.

8. **Trait-argument binders** (M; pin ledger) — `impl X with Trait<type S: Bound>` is an
   unsupported *feature* with a clean error, pinned `#[ignore]`d. Also notable as the alternative
   route to trait-shaped visitors (p6-followups #2/#4 record the context).

9. **Impl-binder declaration order** (S; pin ledger) — the second `#[ignore]` pin; declaration
   order affects binder resolution. Trivial workaround (reorder declarations); fix for hygiene.

11. **`!` / `?.` deferred tail** (M; `try-and-lift.md`) — the operators shipped 2026-07-04
    (both slices + the stabilization arc: bang-directed return-position generics, closure-`ret`
    participation, user-`Lift` lowering). Remaining here are the recorded deferrals only:
    closure `!` (the RPC-handler follow-up; needs the `arg → Result` linkage design), error
    conversion at the `!` boundary, expression lifting (`a? + 10`), applicatives, and
    `Signal`/`Promise` `Lift` opt-ins.

12. ~~**Missing-impl bound dispatch emits the abstract method**~~ — **FIXED 2026-07-08**:
    `check_generic_bound_satisfaction`, a post-solve pass over
    `method_call_substitution` (the one channel every instantiation shape records
    into — free functions incl. explicit `f<Cat>()` arguments, method own-generics,
    impl-subject and trait-parameter bindings): every binding of a bounded generic
    must SATISFY the bound — a concrete type through an impl of the trait or any
    SUBTRAIT of it, a generic argument through its own declared bounds
    (bound-to-bound flow; forwarding through an under-bounded wrapper is rejected
    at the inner call with "add `: Trait`" wording — bounds must be re-declared,
    which is also what closes the nested-call hole: the transformer's inherited
    substitutions never cross an unchecked edge). Spanned at the full call.
    Eleven pins (free fn, method, multi-bound naming the missing trait, static
    channel, trait-default-without-impl, subtrait satisfaction, generic impl
    subject, rebounded forward, under-bounded forward). **Conditional-impl DEPTH
    closed same day** (4 more pins): satisfaction reconciles the impl subject to
    bind its binders and recursively requires each binder bound — explicit
    (`impl Box2<type X: Greet> with Greet`) or inherited from the struct
    declaration — to hold at the argument (`Box2<Box2<Dog>>` greets,
    `Box2<Cat>` errors; depth-capped, lenient past the cap). **The family is
    CLOSED 2026-07-08** (three follow-on slices, 17 more pins): construction
    sites check DECLARED bounds (struct literals via the initializer's solved
    arguments; enum-variant calls by locally reconciling payload types against
    argument types — partial variants check exactly what they bind), and bound
    trait ARGUMENTS match (`Feed<str>` no longer satisfies `F: Feed<i32>`;
    required args ground through the call's substitution / the construction's
    own bindings / the conditional impl's binder bindings, and errors read
    "does not implement trait 'Feed<i32>'"). The unbounded-forward gap got its
    ROOT fix too (same day): the initializer's second-chance FIELD-first
    reconcile binds a declared parameter from a generic field value (the main
    loop reconciles value-first, which grounds a value's inference slots but
    never binds the struct's parameter from a generic), and the enum checker
    types identifier arguments via `infer_type` (an identifier's own expr id
    carries no type entry) — both forwards now reject, pin un-ignored, enum
    twins added. Remaining leniencies, each deliberate: an impl reached via a
    SUBTRAIT keeps trait-level argument matching; generic-value
    bound-to-bound flow stays trait-level.

14. ~~**Context threading misses trait-default dispatch edges**~~ — **FIXED 2026-07-07**:
    the context pass adds trait-dispatch edges locally (coverage, backward needs
    propagation, and argument threading through dispatch call sites; the shared call
    graph stays untouched — it is also async inference's). `effect` moved onto the
    `Source` trait as designed; pin un-ignored. The fix EXPOSED a latent miscompile:
    `resolve_inherited_default` matched impl subjects by exact type equality, so an
    inherited default on a GENERIC subject silently bound to the trait's abstract member
    (B12's shape) — now nominal matching, pinned
    (`an_inherited_default_on_a_generic_subject_dispatches`).

15. ~~**Context-typed closure parameters**~~ — **SHIPPED 2026-07-07**
    (`proposal/ambient-owner.md` §5): `body: (|| void) context owner_scope` (multi:
    `context (a, b)`), a contextual keyword on parameter closure types. Injected
    literals defer (own hidden parameter instead of creation capture); calls through
    the parameter are reads (fenced when uncovered) and thread the argument; values
    flow only where threading follows (call / same-clause forward / `run` body);
    `run` accepts a matching annotated value. `std::reactive::run_with_owner`
    shipped on it. Also fixed: unused `Context::new()` emitted a dangling call.
    Deferred: clauses on `let`/return types; superset-clause forwarding.

13. **A direct call on a closure-typed local doesn't type its unannotated parameter** (M;
    pinned `#[ignore]`d; surfaced writing macro `unroll` callbacks 2026-07-06) — `let f = |i|
    accumulate(i); f(3)` never feeds `i` from the call site (zero-param and annotated forms
    work; closures passed to methods work via reconciliation). The C′-family stabilization
    covered deferred call SUBJECTS; the binding-then-direct-call shape needs the same
    channel. Workaround: annotate (`|i: i32| ..`).

16. ~~**Methods on an ungrounded generic receiver typecheck nothing — silently**~~ —
    **SHIPPED 2026-07-10** (the full (b) fix). The class was WIDER than the item: probing
    showed even `mut a: List<i32> = List::new(); a.push("text")` passed, as did
    `Holder<i32>.replace("text")` and `Map<str, i32>.insert("k", "not an int")` — the
    method argument check (`resolve_method_arg_check`) reconciled against the RAW
    parameter type, and `Type::Generic(T)` reconciles with anything, so EVERY
    generic-typed method parameter was vacuously checked, grounded receiver or not.
    Three coordinated fixes, one mechanism each: (1) `MethodArgCheck` now carries its
    call id and applies `method_call_substitution` to parameter types before checking
    (`List<i32>.push`'s `item: T` checks as `i32`) — fixes annotated receivers, user
    generics, Map; (2) an empty `[]` literal mints a STABLE element inference slot
    (`list_element_slots` keyed by the literal's expr id, exactly `List::new()`'s
    mechanism) instead of erasing to zero-argument `List`, so pushes ground it and
    `mut a = []; a.push(10); a[0] + 1` finally works; (3) `resolve_slot_unification`
    now VERIFIES against an already-filled slot instead of no-opping (the receiver's
    `Unknown` slot records no reconcile binding, so fix (1) can't see this case) —
    first push wins, the second mismatched push errors at its argument. Subscripts on a
    still-unknown slot DEFER, and the end-of-fixpoint sweep turns a never-grounded one
    into I4's never-determined error (now also for unannotated `List::new()`, an
    improvement); `len()`-style methods on never-grounded lists stay legal (pinned), and
    typing is fixpoint-order-independent (a later push types an earlier guarded read —
    pinned). 12 pins; all 84 corpus goldens BYTE-IDENTICAL (no emission change — the
    world already type-checked cleanly under real checking); the playground repro now
    errors at `"some text"`. Recorded remainders: an unannotated `Map::new()` stays
    loose (Map is not a slot container — its K/V never ground from `insert`), and
    grounding an empty literal from a LATER annotated use (`let b: List<str> = a`)
    is not taken (pushes and annotations are the grounding channels).

---

## C. Memory model — Phase 6+ tail (deferred; see `memory-management-impl-plan.md`)

1. **`Weak<T>`** (M) — non-owning handle for breaking `Shared` cycles.

2. **Dynamic rule-4** (M; **re-scoped 2026-07-09 by `proposal/view-invalidation.md`**) — the
   STATIC half (a mutating call on the viewed root: `a.remove(i)`, `a.push(x)`,
   `free_fn(&mut a)` — constant or dynamic index alike, via `&mut` conventions) moves into
   the rule-4 scan as event **E2** of that proposal; what remains HERE is the genuinely
   dynamic remainder — writes through ALIASED paths (two `Shared` handles to one cell) —
   runtime-check territory (generation counters / poisoned views), to be sized only after
   E2/E3 have been in use.

3. ~~**No-view-across-`await`**~~ — **SHIPPED 2026-07-09** with E2, both as events of
   `view-invalidation.md`'s unified model (one lexical-liveness scan, three events: E1
   reassignment — previously shipped; E2 mutating call on the viewed root, scalar roots
   exempt; E3 `await` while ANY view is live). Includes the signature rule (a
   suspending function takes no `&`/`&mut` parameters — sync callees stay free, which
   keeps the analysis local), the async-closure capture rule, wrapped-match-leg capture
   liveness, and loop-binding origins (also fixing E1's `for e in &mut a { a = [] }`
   gap). Sub-question answered: `Shared` is NOT exempt — though `read()` returning a
   COPY means only `write()`'s view fences `await` (value semantics made reads safe by
   construction). ~25 pins. A6's ground rule is in place.

4. **Deterministic destruction** (L) — scope-end destructors / `Drop`-equivalent.

5. **Transparent-references remainder** (M; `transparent-references.md` shipped the model) —
   three sub-items:
   - **Scalar views don't auto-deref in argument position** (found 2026-07-09 probing
     `view-invalidation.md`): `print(b)` for `let b = &mut a[0]` prints the raw
     `(base, key)` pair (`[ [ 99 ], 0 ]`) instead of the element — a view passed where a
     value is expected (at least for `any`-typed parameters) leaks the representation.
   - **Inline `Option<&mut T>` transient:** `match Some(&mut a) { Some(let x) => … }` —
     constructing and matching a wrapped view *inline* is only recognized when the subject is a
     view-returning *call*; extend `compute_wrapped_view_captures` (and the escape analysis) to
     admit an immediately-matched inline constructor and a bare `&[mut]`-parameter forward.
   - **`&mut bool`:** broken for both concrete and generic — `bool` is a numeric enum, excluded
     from `is_scalar_primitive`, so it takes the aggregate view path. Fixing it means a scalar
     `(base, key)` view representation for `bool` across the view machinery (its own slice).

---

## D. Language specification & documentation

1. **Write a language specification** (L) — a single source-of-truth document for the grammar and
   semantics, so grammar changes/issues can be checked against a definition rather than inferred
   from the parser. Should cover: lexical grammar, the full expression/statement/item grammar
   (reconciled with the chumsky parser and the formatter), the type system and the memory model
   (value semantics, second-class views, `borrows`, conventions), and the evaluation/lowering
   model. Becomes the reference solver and parser work is checked against.

---

## E. LSP & tooling

2. **LSP semantic highlighting** (M; roadmap #10) — semantic tokens, precision over TextMate.

9. **Richer hover tooltips** (S–M per slice; user request 2026-07-10) — hover today is the
   bare inferred-type label (`expr_types` + `type_label`). Candidate upgrades, roughly in
   value order: (a) full declaration signatures on functions/methods — name, parameter
   names AND types, return type, `context` clauses, `async` — not just the closure type;
   (b) doc comments: surface a decl's leading `//` block in the hover (and decide whether
   to bless a `///` doc convention while at it); (c) markdown formatting — code-fence the
   signature, prose for docs (the LSP already returns markdown-capable `Hover`); (d) struct
   /enum hovers show their fields/variants; (e) constants show their value. Scope each
   slice with a look at what `Program` already carries (signatures largely reconstructable;
   doc comments need trivia access — the lexer skips them today, so (b) likely needs a
   trivia side-channel, which H6's handwritten-frontend trigger list also wants).

3. **Fix per-analysis `Box::leak` + incremental analysis** (L; roadmap #12, caching Tier 2/3) —
   the leak grows each keystroke/compile; true incremental is blocked by global
   `entity_id`/`type_id` counters. Measure first; debounce currently masks it.

4. **LSP sub-file incremental parsing** (L; roadmap #13) — tree-sitter-style reuse; chumsky is a
   batch parser, so this is the largest, lowest-priority LSP item.

5. **Migrate the codegen-snapshot corpus into `vilan test`** (S) — `vilan/test/` is a dev-time
   `.js` snapshot check, separate from the behavior runner; unify.

6. **Diagnostics remainder** (M; what E1 left open when it shipped 2026-07-04) —
   - **Buffer overlay for unsaved dependencies:** module loading is disk-backed
     (`load_package_module`), so a dependent's re-analysis sees an edited-but-unsaved import's
     *disk* content until save (`did_save` closes the loop today). A buffer overlay needs a core
     seam for the loader to consult open-document contents.
   - **Async lifecycle harness:** the publish bookkeeping (explicit empties, `published_extra`
     diffing, close-clears-extras) is exercised only structurally; the fake-`Client` +
     edit-sequence property test (*published == fresh analysis, always*) remains to build.
   - **Shared-dependency last-writer-wins:** two open docs importing the same broken module each
     publish their view of it; the merged per-URI union is not computed (harmless while both
     views agree, which re-analyze-all keeps true).

7. **Diagnostic span precision — the long-tail audit** (S–M per batch; the first pass shipped
   2026-07-04) — the harness and the top user-visible classes landed: `assert_fails_spanning`
   (exact-range span pins in the inference harness), and re-anchors for match-leg mismatches
   (→ the offending leg's body), struct-initializer field mismatches (→ that field's value)
   and unknown-struct (→ the initializer incl. its name), import root/segment errors (→ the
   segment), and `use` root/segment errors (→ the segment) — six span pins. Remaining: the
   long tail of the ~150 `diagnostics.push` sites hasn't been audited — when a coarse span
   shows up in use, re-anchor it and pin with `assert_fails_spanning`. The standard: point at
   the narrowest expression that identifies the problem (call-argument mismatches are the
   model).

---

8. **LSP + editor support for the macro engine** (M) — **core shipped 2026-07-07**: the
   TextMate grammar knows the `macro` keyword, `macro fun` definitions, `macro name(..)`
   invocations, and generic line-anchored `[name(args)]` attributes; hover on `[name]` /
   `[derive(Name)]` / `macro name(..)` shows the macro's `macro fun` signature; go-to-definition
   jumps to the defining `macro fun`, cross-file into `std` for prelude derives (derive names
   now carry per-name spans; macro names live in a separate scope namespace so trait/macro
   name sharing resolves both ways). Remaining: completion offering registered macro names at
   attribute sites, and semantic tokens classifying macro names distinctly (see #2 above).

## F. Backend & platform

2. **Numeric types `u8`…`i64`/`f32`** (S; roadmap #15) — **SHIPPED 2026-07-07**
   (`proposal/numeric-types.md`): `i8`/`u8`/`i16`/`u16`/`i64`/`u64`/`f32` as nominal
   primitives collapsing to plain JS numbers — the 64-bit lowering PROFILED
   (f64+`Math.trunc` beats BigInt 5.2–14.1× on speed, 4× on memory; `BigInt` stays the
   exact escape hatch). With it, two semantic repairs: **truncating integer division**
   (`7 / 2` is now `3` — `Math.trunc` on every integer type, generic dispatch included;
   one corpus golden regenerated run-verified) and **range-checked integer literals**
   (suffix/annotation-typed, `-128i8`-style minimums admitted at `2^(n-1)`; 64-bit bound
   = f64's ±2^53 window, error names `BigInt`). Explicit `as_*` conversions with
   Rust-`as` fold semantics; Json/Debug/operator families mirror `i32` (generated once
   by a macro, checked in — `number.vl` loads inside macro worlds, which expand with an
   empty scope, so world-loaded std files must not dispatch; the flagship
   `numeric_family` macro lives on as a pinned test). `vilan/outdated/` pruned.
   Remaining (recorded in the proposal §7): wrapping arithmetic + real widths on a
   non-JS backend, `f32` fround, Wire slots, parse family, numeric→`BigInt`.

6. ~~**Tree-shake module-level bindings**~~ — **SHIPPED 2026-07-10**: module-level
   bindings are walked per-binding (in order, names stable) and included at ASSEMBLY
   only when something emitted referenced them (one chokepoint — the `Expr::Local`
   value arm; declarations emit through a different arm, so a binding never retains
   itself). The stated semantics landed: a dropped binding's initializer does not
   run — module state exists only if something reaches it. The acceptance test:
   `number.vl` now imports `std::math::PI` for `to_radians`/`to_degrees` (workaround
   removed) and every non-math golden stays byte-identical; the reactive goldens
   dropped their vestigial `const turn_scope = null` / `owner_scope = null` (already
   rewritten away by the context pass). Known over-approximations, recorded: a
   reference made inside a DROPPED binding's initializer still retains its target,
   and a function required only by a dropped binding stays emitted. Worlds
   (macro compiles) are untouched — cached, and correctness-first. Original:
   module-level
   `let`s emit unconditionally whenever their module loads, unlike functions (which the
   transformer already emits reachability-only). Two observed consequences: `number.vl`
   cannot import `std::math::PI` — every program would gain a stray `const PI`, since
   `number.vl` is always loaded (K2 worked around it by inlining the literal, with a
   comment at the site; remove the workaround when this ships) — and a DROPPED unused
   binding with a call initializer degenerates to a bare side-effect statement
   (`Math.pow(2, 0 - 52);` appeared in every golden from `EPSILON`'s initializer — the
   same shape as the fixed dangling-`Context::new()`, which was handled for the
   news-only path specifically). Wanted: extend the existing function-reachability walk
   to module-level bindings — emit a binding only when a reachable item references it,
   and drop its initializer with it. One semantics decision to state: a truly-unreferenced
   module `let` with a SIDE-EFFECTING initializer (`Shared::new`, `Context::new`) —
   today's live ones (`scheduler`, `owner_scope`) are referenced by any program that
   loads them, so reachability keeps them; declare unused-initializer dropping as the
   defined behavior (module state exists only if something reaches it) rather than
   promising top-level side effects.

5. **Project-model deferrals from P1/P2** (M) — registry-dependency loading (only `path`
   dependencies resolve today), `[project.dependencies]` inheritance, and P1's server-side
   manifest completions. (Captured here when the shipped `project-model-p1/p2` proposals
   were pruned — their full context lives in git history.)

3. **WASM backend** (L; far future) — the second emitter on the platform model's `Backend` axis
   (`Js` is the only variant today; `platform-model.md` §7.1 reserves `Wasm`). Three parts, only
   one of which is "codegen":
   - **Emitter** — Vilan's lowered IR → WebAssembly (via a `wasm-encoder`-style crate, or emit
     WAT). Most language constructs (functions, structs, control flow) lower straightforwardly;
     closures and generics (already monomorphized) are the work.
   - **Host-import seam** (`platform-model.md` §5) — a WASM module imports host functions
     differently than JS, so an `[extern]` binding may gate on **backend**: `http_sys.wasm.vl`, or
     a layer with `backend = ["wasm"]`. The *shared interface* is unchanged — only the `_sys` impl
     differs. Needs **backend-gating on layers** (`LayerDecl` carries only `platform` today;
     `Layer.backend: Option<Backend>` per §7.1) — the one piece of platform-model scaffolding
     deferred from the stabilizing slice.
   - **Memory-model lowering** — the model is GC-free by design
     (`memory-management-rev-1.md`, goal #1): values are scope-owned copies, views are
     second-class (never outlive a frame), and `Arena` owns its slots outright with
     generational handles — none of these need collection. What a non-JS backend needs is a
     linear-memory allocator, **scope-end destruction (C4 — the linchpin**, deferred today
     precisely because the JS GC makes deferral free), and an **ARC lowering for `Shared`**
     (+ `Weak`, C1, for cycles). This is the heavy part and is **shared with F4**; do it
     once. Targets both `browser` and `@process` (WASM runs in each).

4. **Native backend — server performance** (XL; far future) — a third `Backend` emitting native
   machine code, motivated by server throughput (no V8/JS overhead). For comparison, **Rust**
   lowers `source → HIR → MIR → LLVM IR → machine code`, with **LLVM** the default backend and
   **Cranelift**/**GCC** as alternates. A Vilan native path wants the same shape — a typed
   mid-level IR to lower from — and faces two choices:
   - **Backend infra** (cheapest → fastest peak): **emit C** (portable, leans on the C compiler;
     simplest to maintain — Nim/V do this) ▸ **Cranelift** (Rust-native, fast compiles, solid
     codegen; the natural fit for a Rust project) ▸ **LLVM** (peak performance, heavy dependency,
     slow builds).
   - **Memory model** — the central challenge (bigger than codegen), but smaller than
     "build a GC": the model is deterministic by design, so the lowering is allocator +
     scope-end drops (C4) + ARC for `Shared` (+ `Weak`, C1). A bundled tracing GC would
     *contradict* rev-1's goal #1 (deterministic, GC-free memory) and is not on the table.
     Shares the F3 lowering work.
   - **Standing cost:** maintaining ≥3 backends is a real tax (each language feature must lower to
     each). Gate this behind a **stable backend abstraction + a shared lowered IR**, and prove the
     seam with a *single* non-JS backend (F3) before committing to a third. Far future — flagged
     here so the IR/abstraction work that unblocks it is designed with this in mind.

---

## G. Macros

1. **General macro engine** (L; roadmap #9; **proposal: `macro-engine.md`; Phases 0–1
   SHIPPED 2026-07-06**) — Phase 0: the interpreter over the transformer's `js::Node` AST
   (`transform_to_ast`), the 70/70 equivalence gate, `macro_std`. Phase 1: `macro fun`
   items, per-file hermetic worlds (blanked-file compile against a macro_std-only
   workspace), `[name(args)]`/`[derive(Name)]` dispatch through `run_entry`, output
   splicing with depth-16 fixpoint, world + expansion caches; library-defined macros work
   (the exit criterion). Phase 2 (also 2026-07-06):
   `macro name(..)` invocations — item + expression position, shape-checked dispatch from
   the signature, `fresh()` gensyms stamped per splice site (capture pinned as a clean
   error), output previews in errors. Phase 3 UNDERWAY (2026-07-06): the
   builtin-derive channel (`std/derives.vl`, names reserved, Rust fallback for
   unmigrated/fixture stds) with `PartialEq`/`Default`/`Debug` migrated byte-identically.
   Derives COMPLETE (2026-07-06):
   all five migrated (`Json`+`Wire` together — one Rust contract — via str-returning
   helper macro funs); `Arguments` typed accessors shipped (construction API step 1).
   `[service]` migrated same day (the
   stress test passed: `Item::Service`/`ServiceItem` reflection with compiler-gathered
   rpc surface, cache keyed on struct+methods text, in-macro djb2 via new `str.code_at`;
   byte-gated on todo/rpc bundles). Scoped names + dissolution SHIPPED
   (2026-07-06): macro names are module-scoped (leaf imports; std prelude ambient; markers
   in the analyzer; lazy per-file worlds), `derives.vl` dissolved into
   compare/default/debug/json/rpc, outputs self-carry imports. The
   **construction API** (macro-engine §3, user request 2026-07-06): ~~`Arguments`
   typed accessors~~ (step 1, shipped 2026-07-06), ~~macro_std output builders~~
   (step 2, **shipped 2026-07-07** as `macro_std::build` — `quote`/`join`/
   `indent` + `impl_of`/`fun_of`/`match_of`/`struct_of`/`init_of`; all five derives and
   `[service]` rewritten against them byte-identically; exact-bytes e2e pin);
   ~~tree interchange~~ (step 3, **measured 2026-07-07 and NOT taken**: 0.8% of the
   rpc example's build parses generated text; a 240-expansion synthetic hits 39% of a
   188ms first compile, erased by the caches on re-analysis — batching parses is the
   recorded cheap alternative if it ever matters). ~~Ambient meta vocabulary~~
   (**shipped 2026-07-07**: the meta types + `source`/`fresh` are ambient in macro
   bodies via the world prelude; explicit definitions shadow; std macros dropped the
   boilerplate imports). ~~`macro { .. }` blocks~~ (Phase 4, **shipped 2026-07-07**:
   item-position comptime families + expression-position constant folding; blocks
   survive world blanking verbatim and wrap into synthetic `__macro_block_<n>` entries
   — true spans; 9 pins + the `macro-block.vl` corpus program).
   **G1 is COMPLETE** — the engine's remaining tail is macro-engine.md §11's
   explicitly-beyond-v1 list (semantic queries, quasi-quotation, compiled host,
   on-disk caching, batched parsing), each recorded with its trigger, plus the
   derive-name registration decoupling (deferred to the first user derive needing it).

2. **`const` — compile-time evaluation** (M–L; `proposal/const-eval.md`, 2026-07-10,
   revised same day to the EXPRESSION form; the styling system A8 is the forcing use
   case, independently motivated) — `const` is a weak-precedence expression keyword
   (`let x = const 1 + 2;` — captures to the bracket/comma boundary; `let NAME =
   const expr` IS the constant declaration, so bindings stay ordinary `let`/`mut`
   with F6/clone-site machinery unchanged, and `mut cache = const initial()` works).
   Evaluates with THE macro interpreter (one evaluator, no second dialect) and
   serializes the plain-data result IN PLACE (never worse than the computation it
   replaces; sharing = bind it). Free variables must be const-known (imports,
   literals, immutable bindings whose initializers are const — chaining; `mut`
   disqualifies); runtime captures error at the reference. **No `const fn` coloring** — the
   interpreter is total over the pure language (the Zig-shaped design; Rust's
   annotation burden avoided); reaching an unavailable capability, panicking, or
   producing non-data (closure/view/Shared) is a spanned static error at the
   binding. One new capability bit: **const-only functions** (std-internal, v1),
   enforced by call-graph reachability — the first is `std::asset::emit(kind,
   line)`, the **asset channel**: compile-time-accumulated build outputs,
   line-deduplicated, deterministically ordered, written beside the `.js` (CSS for
   A8; critical CSS for A7; any codegen later). Recorded v1 bounds: no const
   generics, binding-form only, assets emitted regardless of F6 liveness
   (liveness-tied emission = dead-style elimination, recorded). General payoff:
   lookup tables, precomputed scales, wire hashes (`contract_hash` de-magicked),
   parsed static config — all zero-cost at runtime. **LSP: skips evaluation
   entirely** — sound because no downstream pass depends on const VALUES (types
   are value-independent; the asymmetry with macros, which create items). Static
   const errors (free-variable rule, const-only reachability, cycles) stay live
   in the editor; evaluation-time failures (panics, fuel) surface on
   `check`/`build` — `vilan check` DOES evaluate (check means "will it build").
   Budgeted background LSP evaluation = recorded refinement on the Tier-2 arc.

3. **Inferred `const` — automatic compile-time folding** (M; v2 of G2, recorded
   2026-07-10; design constraints in `const-eval.md` §5's recorded-v2 note) —
   `let a = 1 + 2;` folds without the keyword. No fundamental blocker. The
   soundness rules, settled up front: inference falls back SILENTLY on any
   evaluation failure, panics included (a dynamically-dead `xs[5]` must not become
   a compile error — explicit `const` remains the erroring guarantee); eligibility
   is the explicit form's (const-known free variables, the capability world,
   plain-data result); const-only functions NEVER infer (an asset-emitting style
   must not compile-or-not by optimizer mood — inference folds values, never
   creates const contexts). The v2-sized work is budgets: evaluation fuel (a
   missed fold beats a hung compiler) and serialized-size caps (don't inline a
   10 KB table nobody asked for), plus the `[build]`-preset split (debug = no
   inference for honest stack traces, release = infer).

---

## H. Parser & grammar

1. **Struct literal as an operator operand** (S) — `Point { .. } == x` fails (bind to a variable
   first); needs a `no-struct-literal` expression mode for conditions (à la Rust). Currently
   degrades to a clean parse error, documented at the parser site.

5. ~~**The `%` remainder operator**~~ — **SHIPPED 2026-07-10**: truncated remainder
   (the dividend's sign — Rust's and JS's shared semantics) at every numeric type, plus
   `%=`, binding with `*`/`/` (left-associative), overloadable through the new
   `std::operators::Rem` trait (`impl T with Rem { fun rem(..) }`). Emission is the bare
   JS `%` with NO wrap at any type — unlike `/`, an integer remainder is always
   representable (magnitude < |divisor|, sign of the dividend), so i32/u32/i64 need no
   `Math.trunc`/`>>> 0`; BigInt `%` is native (the macro interpreter mirrors with
   `checked_rem` + the division-by-zero throw). The promised cleanup landed: `f64.rem`/
   `f32.rem` bodies and `fold_unsigned` (the as_* conversion folding) now spell `%`
   directly — their "vilan has no `%` operator yet" comment removed; only the
   `math.js`/`numeric-types.js` goldens moved (one line each, parity-verified). 8 pins
   (signs, floats, i64-exact, u32, BigInt, precedence, `%=`, trait dispatch) + corpus
   `remainder.vl` + TextMate `%`/`%=`.

2. ~~Block-scoped imports~~ — **shipped 2026-07-05** (kept as the design record; macro-engine
   §3 consumes it for `macro_std` resolution). `import`/`use` are statements, legal in any
   block (function/closure/if/match-arm bodies, bare blocks, impl bodies — an impl-scope
   import serves its methods); a binding is visible throughout its enclosing block and a later
   same-name binding shadows by overwrite — both **exactly `let`'s semantics** (vilan scopes
   are flat per block; use-before-`let` already compiled, and imports have no TDZ hazard since
   they compile to nothing). Not re-exportable: `export` in a body is a spanned error. The
   compiler previously PANICKED on a body import (no `Expr` for the statement id → transformer
   `unwrap`; now `Expr::Void`), and the loader only scanned top-level nodes — `Node::for_each_child`
   (the new exhaustive structural visitor, no catch-all) drives `collect_module_refs` at every
   depth, which also carries the P3 cross-target gates, the L1 lib-surface check, the §4.2
   contract check, and the LSP platform sniffer for free. Pins: 12 in `inference.rs`, corpus
   `scoped-import.vl`, workspace body-import + §4.2-at-depth CLI tests.

---

4. ~~**Triple-quoted strings `\"\"\"text\"\"\"`**~~ — **SHIPPED 2026-07-10** (semantics
   settled by the user, = Swift's multiline rule): the whitespace before the CLOSING
   `\"\"\"` is the indentation prefix, stripped from every content line (exact-character
   match, so a tab never satisfies a space prefix; a whitespace-only line may fall short
   and becomes empty); the newlines adjoining the delimiters belong to the syntax; the
   opening `\"\"\"` takes nothing after it on its line and the closing sits alone on its —
   both compile errors with PRECISE sub-literal spans (the offending text/line, not the
   whole literal), as is insufficient indentation (named by line number). The closing
   delimiter governs — not the opening's column as this item originally sketched — because
   `let s = \"\"\"` puts the opener mid-line where its column is meaningless. The body is
   RAW: no escape processing at all (`\n` stays two characters; braces literal) — the
   paste-code-verbatim appeal; content runs to the FIRST `\"\"\"` (no way to embed one —
   recorded limitation, plain strings still have `\"`). One trim/validate helper
   (`util::trim_multiline_string`, 12 unit tests) is validated in the analyzer (a bad
   literal degrades to `\"\"` so its uses stay typed under one diagnostic) and trimmed in
   the transformer — the VALUE flows to JS emission and the macro interpreter alike, so
   macros compose (`source(\"\"\"..\"\"\"`)` pinned), patterns match, i-string holes accept
   them, and `vilan fmt` reprints them verbatim (inner whitespace is semantic). 7 pins +
   corpus `multiline-string.vl` + TextMate rule. **The one recorded follow-up:** the
   interpolated variant `i\"\"\"..\"\"\"` (the macro-authoring payoff) still needs its
   escape story — raw braces vs `{expr}` holes conflict; settle it as its own small item.

6. **Handwritten recursive-descent frontend — replace chumsky** (L; recorded 2026-07-08; take
   it when the combinator model gives trouble, not before) — after the 2026-07-08 perf arc
   (the lexer-trivia quadratic + cheap-first/rich-fallback parsing, commits 5752f76/7b026bc),
   a cold compile is *still* ~95% lex+parse (todo client: 2.43B instructions; type solver 2%,
   macro interpreter 0.09%). What remains is chumsky's **structural** overhead, not a fixable
   pathology: `choice()` finds the right branch by attempting alternatives in order where
   recursive descent switches on the lookahead token; tokens are wrapped and compared per
   attempted primitive (`to_maybe_ref` + `Token::eq` ≈ 17% of the whole build); the
   precedence tower is a `foldl` chain; recursion is boxed. A handwritten frontend is the one
   remaining big multiplier: expect 3–5× on parse — todo ~0.43s → ~0.10–0.15s release — with
   the **debug binary** gaining most (4.8s → likely under 1s; deep combinator towers are what
   unoptimized builds execute worst), and vilan-core's own rustc build gets cheaper too (the
   grammar has been instantiated twice — cheap + rich — since 7b026bc; both towers dissolve).
   - **Speed is the bonus; control is the driver.** The friction is already visible in the
     grammar: the split-shift `try_map` hack, `<`/`>` as control tokens, contextual keywords
     (`context`) via ident-guards. Diagnostics are generated expected-lists (a broken shift
     names 15 candidates) where a handwritten parser gives curated messages. And a handwritten
     parser is fast AND rich in one pass, so `parse_clean`, `CustomParseError`, and the
     cheap/rich double instantiation all dissolve. Mature frontends (rustc, TypeScript, swc)
     ended up handwritten for exactly these reasons.
   - **Do NOT do instead:** another combinator library (winnow/nom — ~2–3× at best, loses
     chumsky's recovery + rich errors, so the hard parts get hand-built anyway: worst of
     both); Tier-2 on-disk/embedded std ASTs (obsoleted — a 5× parser makes cold std parsing
     ~50ms with no owned-AST lifetime surgery and no invalidation story).
   - **Proof strategy:** the corpus byte-gate pins acceptance; scale
     `tests/parse_fast_path.rs`'s tree-equality pattern into a differential harness — both
     parsers over corpus + std + examples, identical trees required. The true cost center is
     **LSP-grade recovery** (the `nested_delimiters`-equivalent partial ASTs the language
     server depends on): hand-designed sync points typically end up *better*, but they — not
     the grammar — are the work.
   - **Triggers:** release builds creeping past ~1s on real projects; LSP latency on large
     files; the next grammar feature that fights the combinator model. Best taken after D1
     (the language spec) exists to check the new parser against, and with the grammar stable —
     a rewrite chasing a moving grammar pays twice. Unblocks E4 (sub-file incremental
     parsing), which is impractical over chumsky's batch model.

---

## I. Collections

1. **Struct keys for `Map`/`Set`** (M) — value `==` exists, but JS Map/Set key objects by
   *reference*, so by-value aggregate keys need key-serialization or a custom table.

2. **`[T; n]` — a general fixed-length array type** (M) — the codec slice shipped this item's
   immediate wants (hex literals, bitwise/shift operators, `std::bytes` over `Uint8Array` —
   `bits-and-bytes.md`); what remains is the general fixed-length / contiguous array type,
   cheaper than the heap-boxed, length-mutable `List<T>` stand-in.

3. **Validating per-type `from_json`** (M; interacts with B11 `?`/try) — the codec seam validates
   end to end (sticky deserializer errors, `RpcError::Decode`, and malformed JSON is a decode
   error rather than a thrown `JSON.parse`); the per-type `to_json`/`from_json` convenience
   surface is what remains trusting: a missing/mistyped field decodes to `undefined` and flows
   onward as garbage — the *silent* failure mode. Wanted: decode reports an error (a `Result`, or
   at minimum a `panic` naming the field) when a field is absent or the wrong shape.

4. ~~**Subscript absence semantics**~~ — **SHIPPED 2026-07-10**: panic, checked at use
   and at mint. `a[i]` — read, write, or `&mut a[i]` — requires `0 <= i < a.len()`; a
   violation panics with "index out of bounds: the length is L but the index is I".
   Writes never create slots (growth is `push`); `get(i)` stays the total,
   `Option`-returning form. Emission is three self-contained helpers
   (`__at`/`__at_put`/`__at_view` — an assignment target can't be a call, so the write
   has its own) throwing the same bare-string shape `panic` lowers to; the macro
   interpreter enforces identical bounds as `Thrown`, so a macro-time violation fails
   the expansion with the same message. An indexing expression now counts as effectful
   in itself (it can throw), so unused-binding elision can't drop a check. A deref
   through an already-minted stale view remains C2's dynamic-rule-4 remainder — the
   mint check plus E2's static fence cover the lexical cases. The circular
   empty-literal message now says what's missing ("its element type is never
   determined"). F3/F4 alignment comes free: panic is exactly what a bounds-checked
   native subscript must do. Corpus impact was 6 goldens (parity-verified); the rest
   of the corpus iterates via `for`/methods and never raw-indexes. Original design
   space, for the record: panic (taken) vs `undefined`-propagation (status quo,
   rejected) vs bare reads as a compile error in favor of `get()` (rejected — hostile
   to the common in-bounds case). Surfaced 2026-07-09 by
   `proposal/view-invalidation.md` §1's P1 case.

---

## J. Concurrency

1. **Async/await remaining phases** (L; see the `context-async-plan` memory) — `context` (scoped
   value) landed and threads as a hidden parameter; the shared call-graph (Phase 0) is in
   `call_graph.rs`. The async/await execution-model phases remain.

2. ~~**Indirect calls are not async-inferred — no implicit await through closure values**~~ —
   **SHIPPED 2026-07-10**: `async || T` closure types. The marker is written at contract
   positions and only there (parameters and `let` annotations — the same policy as types
   generally: written at signatures, inferred at literals); it composes with the B15
   clause (`(async || T) context turn_scope`). A call through an `async`-typed value is
   an await point (async inference) and emits the implicit await (`maybe_await` covers
   `async_values` — one side-channel set, the `parameter_contexts` pattern; the solver
   never sees asyncness). The divergence check kills the bug class: an async closure
   flowing into a PLAIN closure parameter with a non-void return errors, naming the fix —
   while void-returning parameters stay legal as SPAWN semantics (fire-and-forget; the
   turns machinery settles the continuations — UI handlers and turn bodies ride this,
   pinned). `turn_async` and `optimistic` dropped the spawn-then-flatten workaround for
   plain awaited calls. Six pins. REMAINING (recorded): the marker on struct fields and
   return types; async adoption for unannotated bindings (mirroring B15 adoption); flow
   tracking beyond literal-or-binding-initial arguments; and asyncness-polymorphic
   higher-order functions (monomorphize-by-asyncness — the `map` question). Original
   finding follows. — async inference infects through DIRECT calls
   (`f()` awaits when `f` is async), but a call THROUGH a closure value or parameter
   (`body()` where `body: || T`) has no static callee, so it is never inferred async: the
   call returns the host promise at runtime while typing as plain `T` — the static type
   and the runtime value diverge until something awaits. Probed: `turn(policy, || {
   status.set(..); tick(); .. })` published the pre-await write immediately because
   `run`'s rewritten `body(value)` call was not awaited. Workaround (used by `turn_async`
   and `optimistic`, documented at both): SPAWN the call then await it — `let pending =
   async body(); await pending` — the host flattens promise-of-promise, so the await
   covers the callee's whole chain, and it is harmless for sync callees. A real fix wants
   closure TYPES to carry asyncness (an `async || T` closure type, inferred at the
   literal and checked at the call) so indirect calls await implicitly like direct ones —
   which interacts with B15 clauses (a clause-typed async closure) and the async-model
   phases above.

---

## K. Std runtime

1. **`Server` streaming responses** (M) — `serve_connected` builds on `std::http`'s raw
   `node:http` bindings because an SSE stream needs partial writes and `Server`'s
   request→`Response` model is fully buffered (the seam is documented in
   `std/src/process/rpc_server.vl`'s header). Give `Server` streaming-response support and move
   `serve_connected` onto its public surface.

2. ~~**Expand the std math surface**~~ — **SHIPPED 2026-07-09**: `std::math` (constants
   `PI`/`TAU`/`E`/`EPSILON`/`INFINITY`/`NAN` — EPSILON computed, the lexer has no exponent
   literals — plus the `Ord` free functions `min`/`max`/`minmax` MOVED from `compare.vl`,
   which had zero users and were latent-broken: primitives had no `Ord`); the f64 method
   family (trig + `atan2`, `exp`/`ln`/`log2`/`log10`, `cbrt`/`hypot`, `sign`/`fract`/
   `lerp`/`to_radians`/`to_degrees` — pi inlined there, `number.vl` must not import
   `math` or its module-level constants emit into EVERY program — `is_nan`/`is_finite`/
   `is_infinite`); sized-type parity (`abs` signed-only, `pow`/`min`/`max` everywhere,
   f32 mirror incl. `sqrt`..`trunc`); truncated `rem` on every numeric type (exact for
   ints — `/` truncates; the H5 stopgap). En route, three real fixes: the comparable
   primitives gained `Eq`/`PartialOrd`/`Ord` impls (ints + `str` + `BigInt` total; floats
   `PartialOrd` ONLY — the stated NaN answer: `partial_compare` is None for unordered,
   no total-order lie; hand-written, `number.vl` is world-loaded so no macro dispatch);
   the CONFORMANCE checker now credits a supertrait member provided by a SEPARATE impl
   of the declaring trait on the same subject (`impl str with Eq {}` no longer demands
   `eq` be restated — same-named members from unrelated traits still rejected; 3 pins);
   and the macro interpreter's host table learned the `Math.*` set + `Number.isFinite`
   (the corpus-equivalence gate caught it). Corpus `math.vl` (run-verified golden) +
   7 pins; existing goldens byte-identical. Original wanted-list follows for the record —
   today `number.vl` gives
   `i32` only `abs/pow/min/max` and `f64` adds `sqrt/floor/ceil/round/trunc`; generic
   `min/max/clamp/minmax` live on `Ord` (`compare.vl`); `std::random` exists. Missing,
   roughly in demand order:
   Remaining tail (deliberately not taken): per-type `MIN`/`MAX` constants (want a
   static-member story or per-type modules — neither exists; revisit with F5/spec work).
