# The macro engine (roadmap #9)

Status: **proposal for review** (2026-07-04, not implemented). The strategic frontier:
user-land vilan code that runs *inside the compiler* and generates vilan code. Subsumes
the built-in derives and `[service]` generation — today's hand-rolled, Rust-side
special cases — and unlocks the uses they cannot serve (numeric-type families, custom
derives, embedded-DSL checking). §5 (execution model) and §6 (caching) are the two
decisions this document exists to settle; recommendations are marked.

## 1. Goals and non-goals

**Goals**

- **User-land vilan interacts with the compiler** (the defining property): a macro is
  ordinary vilan source, in the user's package or a library, that the compiler runs at
  compile time against a *reflection of the program* and whose output becomes part of
  the program.
- **Two primary uses**, and the design is shaped around exactly these:
  1. **Custom attributes** — `[my_attr(..)]` on an item transforms or augments it
     (custom derives are the flagship: `[derive(Builder)]`, `[derive(Display)]`).
  2. **Repetitious code generation** — item- and expression-position invocations that
     stamp out families of code (`macro numeric_types(i8, i16, i64)`,
     `macro lut(256, |i| ..)`).
- **Isolation**: macros live in their own scope — their code cannot touch runtime
  bindings, runtime code cannot call macros, and macro execution cannot observe
  anything but its declared inputs (no I/O, no ambient state — §4).
- **Subsumption**: the built-in derives (`PartialEq`/`Default`/`Debug`/`Json`/`Wire`)
  and the `[service(Client)]` generator become expressible as macros, and are migrated
  only behind a byte-identical corpus gate (§10).

**Non-goals (v1)**

- **Semantic queries.** A v1 macro sees *syntax* — the item's structure as data — not
  resolved types. Every shipped generator already lives within this limit (the derive
  checks are deliberately recursive-syntactic; see `analyzer-stabilization`'s record of
  why). Type-aware macros need expansion staged *after* inference, a far bigger design;
  recorded in §11.
- **Token-level `macro_rules` pattern matching.** Macros are functions over reflected
  items, not rewrite rules — one model, not two.
- **Hygiene beyond gensym.** §7 defines the v1 rule; full hygienic renaming is future.

## 2. The two uses, concretely

**A custom derive** (attribute use). Today `Debug` is ~30 lines of Rust `format!` in
`analyzer.rs`; as a macro it is vilan in userland:

```vilan
// In a library — an ordinary module marked as compile-time code (§3).
macro fun derive_display(item: Item): Source {
	let target = item.as_struct()!;
	mut arms: List<str> = [];
	for field in target.fields {
		arms.push(i"\"{field.name}=\" + format(self.{field.name})");
	}
	source(i"impl {target.name} with Display {{
		fun to_string(self): str {{ {arms.join(" + \", \" + ")} }}
	}}")
}
```
```vilan
[derive(Display)]            // dispatches to the registered macro
struct Point { x: i32, y: i32 }
```

**Repetitious generation** (invocation use). The `macro` keyword prefixes every
compile-time construct — definitions, invocations, and (future) blocks — the same way
`async`/`await` mark vilan's other evaluation-mode shifts:

```vilan
macro numeric_family(i8, i16, i64)     // item position: expands to N struct+impl sets

fun area(): i32 {
	macro unroll(4, |i| accumulate(i))  // expression position: 4 inlined calls
}
```

Item macros receive their arguments as syntax and return items; expression macros
return an expression. Attributes receive *the item they annotate* plus their arguments.
One keyword makes the compile-time boundary greppable: `macro` finds every place code
runs at expansion time.

## 3. The model: staged compilation, syntax in → source out

A macro is a `macro fun` — the `macro` modifier on an ordinary function definition
(reusing the whole `fun` grammar: parameters, generics, helpers), whose parameter and
return types come from the compiler's reflection vocabulary (`std::meta`):

```vilan
macro fun name(item: Item, arguments: Arguments): Source { .. }
```

After `macro`, the parser decides on one token: `fun` → a definition, `{` → a
compile-time block (Phase 4), an identifier → an invocation. `macro` becomes a
reserved word. Other `macro <item>` forms (`macro struct`, …) are reserved errors in
v1 ("only functions can be macros").

- **`std::meta`** (new, the compiler-interaction surface): `Item` (an enum over
  `StructItem`/`EnumItem`/`FunctionItem`/…), `Field { name: str, type_: TypeExpr }`,
  `TypeExpr` (a *syntactic* type: name + arguments, renderable), `Arguments` (the
  invocation's argument syntax), and `Source` (generated code). These are ordinary
  vilan structs — the compiler constructs them from its AST and consumes `Source` back.
- **Output is source text** (`source(str)` builds a `Source`). This is the proven
  in-house shape: every derive and the service generator emit source strings today, and
  text is what makes caching sound (§6). Quasi-quotation sugar can come later without
  changing the model; `i"…"` interpolation already carries the pattern well.
- **Staging.** Macro definitions form **stage 0**: the loader partitions each package's
  modules into macro modules (those declaring `macro fun`s — they may also hold plain
  helper functions) and program modules. Stage 0 compiles first, against the **macro
  prelude** only (§4); stage 1 (the program) then expands invocations and compiles.
  A macro module importing a program module is an error (the stage boundary); a program
  module importing a macro module gets its *macros* only. Macros generating macro
  definitions are rejected in v1 (one stage-0, no fixpoint of stages).
- **Expansion is a pre-analysis pass**, exactly where `expand_derives` sits today:
  after parse, before the walk — iterated to a fixpoint over *item* expansions (a
  macro's output may carry attribute invocations), with a depth cap (default 16) whose
  overflow is a clean "macro expansion did not settle" error naming the chain.

### Why the unit of staging is the MODULE (not the item)

The split itself is functional, not an implementation convenience — three of the
design's load-bearing properties are properties of a *compilation unit*, and vilan's
compilation unit is the module:

1. **The prelude gate is an import rule, and imports are per-module.** Isolation says
   macro code sees only the pure prelude — but an `import` binds into the *module's*
   scope; vilan has no per-item import visibility. A mixed module would need two
   simultaneous legality rules for one import list ("`std::fs` is fine for the runtime
   half, illegal for the macro half"), i.e. item-level import tracking that doesn't
   exist. Module granularity makes the gate enforceable with the shipped machinery
   (the platform-layer gating already validates per module-set).
2. **The stage boundary is a dependency-graph cut, and the loader's nodes are
   modules.** Stage 0 must compile before stage 1 exists. A mixed module would
   straddle the cut: its macro half must load in stage 0 while its runtime half may
   import stage-1 modules — resolvable only with per-item dependency staging, a far
   finer analysis than the loader performs.
3. **Helpers need an unambiguous world.** A macro's helper `fun`s run in the
   interpreter and must be pure; a runtime `fun` compiles to JS with full std. In a
   mixed module the same helper could be reachable from both worlds — Zig-style
   dual-use (`comptime`-callable functions), which is a coherent but much heavier
   model: bi-modal checking per function and an interpreter that must cover
   everything macro-reachable. The module split makes it binary: in a macro module →
   interpreted, pure; elsewhere → compiled, full.

The *granularity* is also where implementation economy and readability point the same
way: module-content hashing is what the caches already do (§6), and the reader knows
from the file which world they are editing.

**The recorded cost:** logic needed by BOTH worlds exists twice (or the macro *emits*
it — generated code freely calls runtime libraries; the macro itself cannot). This is
the deliberate trade against dual-use functions, revisitable if it bites in practice.

## 4. Isolation — the macro's own scope

The user requirement, made mechanical:

- **A separate namespace.** `macro fun`s are not values: they cannot be assigned,
  passed, or called by runtime code (`name(..)` finds no function; the error suggests
  `macro name(..)`). Runtime items are invisible to macro bodies (stage 0 compiles
  before stage 1 exists).
- **The macro prelude** is the *pure* std subset: `List`, `str`, `Option`, `Result`,
  `Map`/`Set`, `format`/`display`, `std::meta` — and nothing platform-flavored. No
  `fs`, no `http`/`fetch`, no `time`, no `random`, no `process`, no `[extern]` in macro
  modules at all. This is enforced at stage-0 load (the platform-layer machinery
  already gates module sets per target; the macro stage is one more, maximally
  restrictive, "platform").
- **Consequence: determinism by construction.** A macro's output is a pure function of
  (its own source, its inputs). No clock, no randomness, no filesystem, no environment.
  This is not just safety hygiene — it is what makes caching (§6) *correct* rather than
  heuristic, the same reasoning that bans `Date.now`/`Math.random` in workflow scripts.

## 5. Execution: interpreted vs compiled — the decision

The compiler is Rust; macros are vilan. Three ways to run them:

| | **(a) Tree-walking interpreter** (in `vilan-core`) | (b) Compile to JS, run in a node host | (c) Native plugin objects |
|---|---|---|---|
| Startup cost | ~0 | node spawn ~30–80 ms, or a persistent daemon | build step per macro crate |
| Throughput | ~10–100× slower than JS per op | full JS speed | full native speed |
| Sandboxing | **total** — the interpreter simply has no I/O ops | must sandbox node (fs/net reachable; `--experimental-permission` or a frozen realm — real attack surface) | none (arbitrary code) |
| Determinism | enforced by construction | enforced only by discipline/sandbox | no |
| LSP fit (runs per keystroke) | excellent | poor without a daemon; daemon = lifecycle complexity | poor |
| Implementation | a new, but small, eval over the existing typed AST for the prelude subset | reuses the whole backend; IPC protocol needed | contradicts "user-land vilan" |

**Recommendation: (a), an interpreter — with fuel.** The deciding arguments:

1. **The workload is small.** Macros process item syntax and build strings — hundreds
   to low-thousands of operations per item. At even 100× JS slowness that is
   microseconds-to-milliseconds per item, far inside the LSP's ~200 ms debounce budget.
   Macros are not where programs compute; a macro that *is* compute-heavy is the
   pathology fuel exists for.
2. **Sandboxing and determinism come free**, and §6's caching *depends* on determinism.
   Option (b) spends its speed winnings buying back, imperfectly, what (a) has by
   construction.
3. **Fuel bounds the failure mode**: each expansion gets an instruction budget
   (default: 1M steps; configurable per package in `vilan.toml [macros]`). Exhaustion
   is a clean spanned error naming the macro — the same pattern as the reactive flush
   budget. An infinite loop in a macro can never hang the compiler or the editor.
4. **The escape hatch is additive.** If a real macro workload outgrows the interpreter,
   a persistent compile-to-JS macro host (b, daemonized) can be added *behind the same
   `std::meta` contract* — the macro's source doesn't change, only the engine. Decide
   that from measurements, not in advance.

The interpreter's scope is the macro prelude subset only (no async, no views/arenas —
value semantics over plain data), which keeps it small and testable: its conformance
suite is "every prelude corpus program the subset admits produces the same output
interpreted as compiled" — an executable equivalence gate.

## 6. Caching — both sides of the problem, addressed

Macros run on every analysis, and the LSP analyzes on every debounced keystroke. Naive
re-expansion is O(macros × items) per keystroke; caching is mandatory. Both directions
have real problems — stated first, then the design.

**The cached-input problem.** A cache key must cover *everything the expansion read*.
If macros could read arbitrary compiler state (types, other modules, the filesystem),
the key becomes an open-ended read-log: under-key it and you serve **stale expansions —
a miscompile**, the worst outcome available; over-key it (hash the world) and nothing
ever hits. This is why §4's isolation is a caching decision as much as a safety one:
v1 shrinks the legal input surface to exactly **(macro definition, invocation input)**
— nothing else is readable, so nothing else needs keying.

**The cached-output problem.** An expansion's *analyzed* form is full of per-analysis
state: entity ids and type ids come from global counters, spans index into leaked
buffers, scopes are rebuilt each run (this is precisely the known incremental-analysis
blocker, roadmap #12). Caching analyzed output across analyses would require id/span
remapping — a project in itself. Caching *within* one analysis has a subtler trap:
expression macros run per site, and two sites with identical input may still need
distinct gensyms (§7), so even intra-run memoization must key the gensym seed.

**The design — cache text, never trees:**

1. **The unit of caching is the expansion's SOURCE TEXT** — id-free, span-free,
   analysis-independent. Key: `hash(macro definition source) × hash(invocation input
   source) × engine version`. Both hashes are cheap and already have in-house
   precedent: `load_package_module`'s content-addressed parse cache. Determinism (§4)
   is what makes this *sound*: same key ⇒ same text, always.
2. **The parse of cached text rides the existing parse cache** (content-addressed, so
   a hit costs a hash lookup), and the walk re-runs per analysis — exactly how std
   modules already work per keystroke. Fresh ids/spans every run; no remapping problem.
3. **Granularity: per-invocation** (which subsumes "module-level" — a module's
   expansions are its invocations' entries). Item attributes key on the annotated
   item's source; expression macros key on their argument source. An edit anywhere in
   a module invalidates *only* the invocations whose own input text changed — a
   keystroke inside a function body re-expands nothing item-level at all.
4. **Expression-level caching gets one extra ingredient:** the gensym counter (§7) is
   part of the *output* contract, so cached text is stored with **placeholder gensyms**
   (`__m0`, `__m1`, …) and stamped per site at splice time (a string substitution, not
   a re-run). This is the honest resolution of "cached output is problematic" at
   expression granularity: the only per-site variance is names, so names are the only
   thing re-materialized.
5. **The cache is in-memory per process** (compiler run / LSP session), bounded LRU.
   An on-disk cache is a later, optional layer with the same key — safe because the
   key already covers everything.

What this deliberately does **not** attempt: caching across *semantic* context (there
is none to read, §1 non-goals), and caching analyzed IR (blocked on the same counters
as incremental analysis; if roadmap #12 ever lands stable ids, revisit).

## 7. Hygiene and generated names

- Expanded code resolves **in the expansion site's scope** — like today's derives; a
  derive-style macro *wants* to see the item's module (imports included). The known
  sharp edge (the derive prelude's duplicate-import collision we hit in P6) becomes a
  rule: generated imports must be idempotent — the engine dedups exact-duplicate
  `import` lines in spliced output.
- **`meta::fresh(prefix: str): str`** is the gensym: names that cannot collide with
  user code or other expansions (reserved `__m` namespace, per-site stamped — §6.4).
  v1 hygiene = "use `fresh` for anything you bind"; full auto-renaming is future work.
- A macro's *helpers* (plain functions in its stage-0 module) are invisible to stage 1
  — generated code cannot call them; anything the output needs must be emitted or be
  ordinary library code the *program* imports.

## 8. Errors, spans, and the LSP

- **A macro failure is a compile error at the invocation site**: panics (converted —
  the interpreter catches its own traps), fuel exhaustion, and `Source` that fails to
  parse ("macro `X` generated invalid vilan: line N …" — the module-parse-error
  machinery, which already reports loudly with file+line, extended with the macro's
  name and the offending generated text attached).
- **Spans inside expansions** ride the existing `DERIVED_SOURCE` mechanism: entities
  from generated text are marked, editor features skip them, and diagnostics in
  generated code anchor at the invocation with the "(in generated code)" label — all
  shipped behavior (E1), inherited unchanged.
- The LSP re-expands per analysis through the §6 cache; a macro edit invalidates by
  definition-hash, so editing a macro module live-updates its expansions on the next
  debounce.

## 9. Pipeline integration

```
load (stage-0 macro modules compile first, interpreted thereafter)
  → parse (program)
  → EXPAND (fixpoint over item invocations, depth ≤ 16, per-invocation cache)
  → walk → build → contexts → async → transform     (unchanged)
```

Expression-position invocations expand during the same pass (they are syntax → syntax;
the walk never sees a `macro` invocation). The corpus stays byte-identical through the engine's landing
because nothing uses it until a program opts in — the same additive discipline as
variadic generics.

## 10. Migration — subsuming the special cases

The prize is deleting Rust: `derive_impl_source` (~500 lines) and `service_impl_source`
(~250 lines) become vilan macros in std. The gate is absolute: each built-in migrates
only when its macro produces **byte-identical generated source** for the whole corpus +
examples (the goldens are the referee), and the native path is deleted in the same
commit or not at all. Order: `Debug` (smallest) → `Default` → `PartialEq` → `Json` →
`Wire` → `[service]` (last; it is the stress test — cross-module, contract hashing,
mirror lets). `derive(..)` dispatch: built-in names resolve to std macros once
migrated; unknown names resolve to user macros in scope; a miss keeps today's
behavior (skip; the missing-impl error surfaces at the use site).

## 11. Phased plan

- **Phase 0 — `std::meta` + the interpreter core** (the long pole): the reflection
  structs, the prelude-subset interpreter with fuel, its compiled-vs-interpreted
  equivalence suite.
- **Phase 1 — attributes**: `macro fun` items, stage-0 loading, `[name(args)]` +
  `[derive(Name)]` dispatch, expansion fixpoint, the per-invocation text cache,
  `meta::fresh`, error surfacing. Exit: `derive_display` (§2) works end to end from a
  user library.
- **Phase 2 — invocations**: `macro name(args)` in item and expression position,
  expression splice + placeholder-gensym stamping. Exit: `macro numeric_family(..)`
  and a `macro unroll(..)`.
- **Phase 3 — migration** (§10), one derive per commit, goldens as referee.
- **Phase 4 (recorded, unscheduled)** — **`macro { .. }` blocks** (an anonymous,
  immediately-expanded macro: the body runs at expansion time in the macro prelude; in
  item position its emissions splice in place — comptime-style families without naming
  a macro; in expression position it yields a spliced `Source` — compile-time constant
  folding), semantic queries (post-inference expansion stage), quasi-quotation, the
  compiled macro host, on-disk caching.

## 12. Open questions for review

1. ~~The `@` sigil~~ — **resolved (review): the `macro` keyword prefixes everything**
   (definitions `macro fun`, invocations `macro name(..)`, future blocks
   `macro { .. }`; attributes stay `[..]`). Rationale: vilan marks evaluation-mode
   shifts with keywords (`async`/`await`), not sigils — invocations read as
   `await`-family; the compile-time boundary becomes greppable by one word; no
   retired sigil returns; and the block form falls out of the grammar. Cost: `macro`
   is reserved. Parse decision is one token after `macro` (§3).
2. ~~Fuel defaults~~ — **resolved (review):** 1M steps/expansion, depth 16, per-package
   configurable in `vilan.toml [macros]`.
3. **Should macro modules be *marked*** (a `[macro]` module attribute / `macro/`
   directory convention) or inferred from containing `macro fun`s? The staging
   rationale (§3) sharpens the case for **marked**: the import-legality rules apply to
   the WHOLE module, so the reader (and the LSP's platform gating) should know the
   module's world before reading any item — and under inference, adding or removing
   one `macro fun` silently flips the module's stage and its imports' legality.
4. ~~Expression macros in v1 or Phase 2?~~ — **resolved (review): Phase 2 as written.**
