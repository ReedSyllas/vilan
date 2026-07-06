# The macro engine (roadmap #9)

Status: **design settled; Phases 0–1 SHIPPED 2026-07-06** (every §12 question
resolved; the fueled interpreter, `macro_std`, `macro fun` items, and
`[attr]`/`[derive(X)]` expansion are in-tree — see §11 for what each phase
delivered and Phase 1's recorded v1 bounds). The strategic frontier: user-land vilan code that runs *inside the
compiler* and generates vilan code. Subsumes the built-in derives and `[service]`
generation — today's hand-rolled, Rust-side special cases — and unlocks the uses they
cannot serve (numeric-type families, custom derives, embedded-DSL checking).

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
// In any module — a macro fun's body is hermetic (§3): it sees only what it
// imports, and it imports only from `macro_std`.
macro fun derive_display(item: Item): Source {
	import macro_std::source;
	import macro_std::display::format;

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

The prefix marks **boundary crossings only** (settled in review): a `macro name(..)`
splice site sits in *program* code, so it needs the keyword. Inside the macro world a
`macro fun` calling another `macro fun` is an ordinary call — no prefix, no ambiguity
(runtime functions are invisible there, §3), and composing macros is just calling
functions and concatenating their `Source`.

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
- **The macro world is HERMETIC, per function** (settled in review — one rule solving
  the prelude gate and the helper ambiguity at once): a `macro fun`'s body sees
  **nothing** of its surrounding module — not its imports, not its functions, not its
  module-level `let`s. What a macro body can reference is exactly: its own parameters
  and locals, **other `macro fun`s** (its helpers — inside the macro world, calling one
  is an ordinary call; the `macro name(..)` syntax is for *splice sites* in program
  code), the language intrinsics (literals, `List`/`str` built-ins), and whatever it
  imports **inside its own scope** — and macro-scope imports resolve against exactly
  one package: **`macro_std`**.
- **`macro_std`** is the macro world's std — a separate, toolchain-shipped package,
  *not* a filtered view of `std`: `macro_std::meta` (the reflection types),
  `macro_std::source`, and re-exports of the pure core (`option`, `result`, `list`,
  `map`, `display`, …) so macros keep the ordinary vocabulary. There is nothing to
  subset or police: if it isn't in `macro_std`, a macro can't name it. No `fs`, no
  clock, no `[extern]` — the package simply doesn't contain them. Scoped `import` is
  **not macro-special grammar**: it is the general block-scoped-imports feature
  (**shipped 2026-07-05**; backlog H2 — imports legal in any block, binding like a
  `let`), which macro bodies consume with one restriction: their imports resolve against the
  `macro_std` universe instead of the package universe. Same grammar everywhere; the
  hermetic rule is purely a resolution restriction.
- **Two orthogonal systems, cleanly split:** macro *names* distribute through the
  ordinary module system (a module exports its `macro fun`s; `import pkg::x::my_macro`
  brings the macro into scope for `[derive(..)]`/`macro my_macro(..)` sites), while
  macro *bodies* live in the hermetic world. A macro can therefore sit in the same
  file as the runtime code it serves — there are no "macro modules", no marker, and
  no module partitioning; the `macro fun` head is the entire boundary, at exactly the
  granularity the boundary is real.
- **Staging falls out per-function**: the macro world (`macro fun`s + `macro_std`)
  is closed under its own references, so it compiles first by construction — no
  module-graph cut. Macros generating `macro fun`s are rejected in v1 (no fixpoint of
  worlds).
- **Expansion is a pre-analysis pass**, exactly where `expand_derives` sits today:
  after parse, before the walk — iterated to a fixpoint over *item* expansions (a
  macro's output may carry attribute invocations), with a depth cap (default 16) whose
  overflow is a clean "macro expansion did not settle" error naming the chain.

**The recorded cost of hermeticity:** logic needed by BOTH worlds exists twice (or the
macro *emits* it — generated code freely calls runtime libraries; the macro body
cannot). Shared constants between a macro and the runtime code beside it are likewise
duplicated. This is the deliberate trade against Zig-style dual-use functions
(bi-modal checking per function, an interpreter covering everything macro-reachable);
revisitable if it bites in practice.

## 4. Isolation — the macro's own scope

The user requirement, made mechanical:

- **A separate namespace.** `macro fun`s are not values: they cannot be assigned,
  passed, or called by runtime code (`name(..)` finds no function; the error suggests
  `macro name(..)`). Symmetrically, runtime items are invisible to macro bodies — the
  hermetic rule (§3): a macro body resolves names against its locals, other
  `macro fun`s, intrinsics, and its own `macro_std` imports, nothing else.
- **`macro_std` is the entire reachable library surface** (§3): isolation needs no
  enforcement pass, because the sandbox is the package boundary itself — `fs`, the
  clock, `random`, `process`, and `[extern]` aren't restricted, they are *absent*.
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

The interpreter's scope is `macro_std` plus the intrinsics (no async, no views/arenas —
value semantics over plain data), which keeps it small and testable: its conformance
suite is "every prelude corpus program the subset admits produces the same output
interpreted as compiled" — an executable equivalence gate.

### What the interpreter executes: the transformer's own JS AST (Phase 0 decision)

The "eval over the existing typed AST" above sharpened during implementation into
something strictly better: the interpreter evaluates **`js::Node` — the transformer's
output AST** — not the analyzed vilan IR. The macro world compiles through the
ordinary full pipeline (analyze → contexts → transform); the interpreter picks up
where the JS *formatter* otherwise would.

1. **One lowering, not two.** Generic dispatch, monomorphization, value-semantics
   copies, and match compilation live in the transformer — the exact subsystems the
   solver-stabilization arc hardened. A vilan-IR interpreter would be a second
   implementation of the hardest logic in the compiler, diverging precisely where
   bugs are subtlest. Over `js::Node`, the interpreter cannot disagree with codegen
   about what a program *means*.
2. **Equivalence by construction.** Compiled and interpreted paths share everything
   down to the last AST; the residual claim is only "this evaluator matches a JS
   engine on the emitted subset", which the conformance suite tests *directly* —
   run node, run the interpreter, diff the output.
3. **Future features are free.** Whatever the transformer learns to emit, macros can
   run — no interpreter work per language feature.
4. **The emitted subset is tiny and closed.** ~25 node kinds; values are
   undefined/null/bool/number/BigInt/string/array/`Set`/`Map`/closure plus the one
   `{ v }` cell `Shared` uses — no general objects (structs are positional arrays),
   no classes, no prototypes, no `this`. The dynamic semantics to match are JS's
   arithmetic, `===`, string `+`, UTF-16 string indexing, and insertion-ordered
   `Set`/`Map`.
5. **Runtime helpers are native.** The `__` helpers the backend injects as source
   text are implemented in Rust, mirroring their JS sources one-to-one; the impure
   ones (`__scan`/`__env`/`__args`/`__random_*`) and `[extern]` host imports are
   clean "not available at expansion time" errors — the sandbox stays a *missing
   capability*, not a check.
6. **Fuel** decrements per node evaluated; a call-depth cap bounds recursion. Both
   exhaust into clean errors naming the macro.

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
   analysis-independent. Key: `hash(the macro's REACHABLE definition set — its own
   source plus the macro funs it transitively calls) × hash(invocation input source) ×
   engine/macro_std version`. The reachable set is well-defined because the macro
   world is closed (§3); the hashes are cheap and have in-house precedent
   (`load_package_module`'s content-addressed parse cache). Determinism (§4) is what
   makes this *sound*: same key ⇒ same text, always.
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
- A macro's *helpers* (other `macro fun`s) are invisible to the program world —
  generated code cannot call them; anything the output needs must be emitted or be
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
  definition-set hash, so editing a `macro fun` live-updates its expansions on the
  next debounce. Inside a macro body, completion/hover resolve against the hermetic
  scope (`macro_std` + macro funs) — the same platform-gating shape the LSP already
  applies per target.

## 9. Pipeline integration

```
load + parse
  → macro world compiles (macro funs + macro_std; closed, so no ordering analysis)
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

- **Phase 0 — `macro_std` + the interpreter core** — **SHIPPED 2026-07-06.** The
  interpreter (`crates/vilan-core/src/interpreter.rs`) evaluates the transformer's
  `js::Node` AST (the §5 decision) behind `transform_to_ast` with fuel + a call-depth
  cap; the equivalence gate (`tests/interpreter.rs`) runs EVERY admitted corpus
  program both ways — node vs interpreter — and compares (stdout, exit code) exactly:
  70/70 equivalent, 3 exclusions (async ×2, host env ×1), ~4s. Failure modes pinned:
  fuel exhaustion, depth cap, impure capability (`Unsupported`, "not available at
  expansion time"), panic (`Thrown`). `vilan/macro_std` ships `meta` (Item/StructItem/
  EnumItem/FunctionItem/Field/TypeExpr.render/Variant/Arguments/Source) + `source()`,
  pinned end-to-end via a consumer app (`crates/vilan-cli/tests/macro_std.rs`).
  Recorded v1 bounds (each a loud error, never silent): BigInt beyond i128, async,
  the unimplemented host-method tail.
- **Phase 1 — attributes** — **SHIPPED 2026-07-06.** `macro fun` items parse
  (`macro` is reserved; only `fun` may follow in v1) and never walk in the program
  world; each file's macros compile in a per-file hermetic world — the file with
  everything outside the definitions BLANKED to whitespace (spans stay true), the
  `macro` keyword erased, analyzed against a workspace whose only dependency is
  `macro_std` (body imports checked to root there; H2's block-scoped imports carry
  the signatures too, since a `fun`'s scope is flat). `transform_functions` emits the
  world rooted at the macro funs (no `main`); `run_entry` executes one against the
  reflected `Item` (+ `Arguments` for two-parameter macros: the argument SOURCE
  TEXTS) and returns the `Source` text, which parses loudly and splices — the
  generated code walks into the invoking module's scope and may carry its own
  block-scoped imports (no synthesized prelude), with `[derive(..)]`s in output
  expanded and further attributes chased to depth 16. `[derive(Name)]` dispatches to
  the macro NAMED `Name`; built-ins keep their generators; unknown attributes error.
  Worlds cache by blanked-content hash, expansions by (world, macro, item text,
  argument texts) — both process-global. `macro_std` now re-exports the pure core
  (`option`/`result`/`display`/`debug`/`compare`/`operators`/`map`/`set` + `panic`,
  the error channel: a throw = a spanned "failed at expansion time" at the site).
  Exit criterion met: a library-defined macro drives generation in its consuming app
  (CLI-pinned), plus the §2 corpus program (`macro-derive.vl`) and 12 inference pins
  (dispatch both forms, arguments, output-derives fixpoint, hermetic violation,
  unknown name, duplicate names, panic/fuel/invalid-output/macro-generating-macro,
  body-position rejection).
  **Recorded v1 bounds:** macro names are a flat global namespace (module-scoped
  distribution = follow-up); attributes expand at file top level and `mod` bodies
  (attribute USE inside a dependency's own files is deferred — definitions there
  work); fuel is the 1M default (the `vilan.toml [macros]` knob is pending);
  `meta::fresh` waits for its first consumer. **Findings:** i-strings have no brace
  escape, so brace-heavy generation uses concatenation (ergonomic follow-up, backlog
  H3); `panic` in a match arm types as `any` (B10's recorded never-type exclusion),
  so macro guards use typed fallbacks.
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
   is reserved. Parse decision is one token after `macro` (§3). Refinement (review):
   the prefix is required only in *program* code — inside the macro world, macro funs
   call each other plainly (§2).
2. ~~Fuel defaults~~ — **resolved (review):** 1M steps/expansion, depth 16, per-package
   configurable in `vilan.toml [macros]`.
3. ~~Marked vs inferred macro modules~~ — **resolved (review): the question dissolved.**
   There are no macro modules: the macro world is hermetic PER FUNCTION (§3) — a
   `macro fun` sees nothing of its surrounding module and imports only from
   `macro_std`, inside its own scope. The `macro fun` head is the marker, at exactly
   the granularity the boundary is real; macros live beside the code they serve.
4. ~~Expression macros in v1 or Phase 2?~~ — **resolved (review): Phase 2 as written.**
