# Macros: the normalized `macro_std` contract

Status: **DESIGN (2026-07-16) — awaiting sign-off, then implementation.**
Settled through discussion with the user (this document's earlier framing —
text-vs-tree interchange, expansion scope wrappers — is superseded and lives
in git history). Companion: `macro-engine.md` (the engine this evolves).

## 1. The decision

Macros interact with the compiler **indirectly, through `macro_std`** — a
small, curated, versioned API that strictly controls what a macro may see
and do. Within that:

- **Output is a normalized VALUE, not source text.** Today a macro returns
  `Source` — a string the engine re-parses and splices, with everything that
  implies: escaping and precedence traps at splice points, string-level
  gensyms, post-reparse diagnostics on synthetic spans, and module-level
  `import` lines that **leak into the deriving module** (live bug: after
  `[derive(Json)]`, `JsonValue` resolves with no import — and code can
  silently depend on it).
- **Direct graph access stays off the table as a contract.** The analysis
  graph is the compiler's most-churned surface; `macro_std` is the stable
  boundary, and the adapter from API values to compiler internals is
  private and free to change. (This also keeps worlds hermetic and the
  text-keyed expansion caches valid — what makes per-keystroke re-analysis
  affordable.)
- **The boundary inside the API: normalize ITEMS, quote EXPRESSIONS.** The
  builder vocabulary is a small closed set of declaration shapes — impls,
  functions, struct/enum items, fields, `use`s. Expression *bodies* stay
  quoted (parsed text at the leaves): rebuilding the whole expression
  grammar as builders would put every future language feature on the API's
  hook, forever. Parsing shrinks to small, cacheable leaves; the structure —
  where hygiene, scoping, and provenance live — is fully normalized. (The
  shape Rust's `quote!` and Scala's typed quotes both settled on.)

Why not speed: the re-parse cost was measured (2026-07-07) at 0.8% of a real
build and the caches erase it. The reason is the **contract** — malformed
output becomes unrepresentable-or-checked at construction (with the macro's
own spans), imports become first-class values the engine scopes, and
provenance improves: a generated impl knows which macro built it (the
missing ingredient for anchoring diagnostics at user code, backlog E8).

## 2. The contract, concretely

A macro returns an `Output` value (name open, §5):

- `output()` — an empty output; `.item(builder)` appends a normalized item.
- **Item builders** — today's `macro_std::build` vocabulary (`impl_of`,
  `fun_of`, `struct_of`, `init_of`, `match_of`, …), evolved from
  str-returning (`.render()`) to **value-returning**: builders nest as
  values, and `.render()`/string concatenation disappear from macro code.
  Wherever repetition occurs, builders take **bulk list forms** —
  `.fields(List<(str, Expr)>)`, `.methods(…)`, `.legs(…)` — alongside the
  single-entry forms, so macro code `map`s over reflection instead of
  folding a mutable builder.
- `.uses("std::default::Default")` — a first-class import, **scoped to the
  expansion by the engine**. The leak class dies here: an expansion cannot
  express a module-level import at all.
- **Semantic handles** *(user, 2026-07-16 — adopted)*: a macro resolves the
  entities it references and constructs the reference BOUND to the resolved
  identity, not to a name-in-scope:

  ```vilan
  let print_fn = resolve_module("std")?.namespace("io")?.item("print")!;
  let print_call = FunctionCall::from_fn(print_fn, [ /* arguments */ ])!;
  ```

  This is Template Haskell's resolved-`Name` / Racket's binding-identity
  move, and it is strictly better than name-based splicing wherever it
  applies: no imports, no shadowing or capture hazards (hygiene by
  identity), resolution failures are LOUD macro-time errors at the macro's
  span (note the pseudo-code already reads naturally under expression
  lifting), and arity/shape check at construction. Implementation stays
  cheap: the engine can lower a handle to a hidden, gensym-named binding in
  the expansion's scope — hygienic imports as plumbing, no new analyzer
  primitives. The churn objection does NOT apply here: modules, items,
  calls, constructions, and literals are the language's STABLE spine.
- **Quoted expressions with handle splices** — the escape hatch for
  arbitrary SHAPE (operators, matches, control flow — the grammar's churny
  tail, which is exactly what must not become builders):
  `expr(i"{lhs} + {rhs}")` where a hole accepts a handle or a built value,
  not just text. Quote the shape, splice the identities — the
  quasi-quotation shape TH and Scala both converged on. Quoted leaves parse
  (cached) inside the normalized skeleton and report errors against the
  macro's span.
- The READ side is already structured (`Item`, `Arguments`, the meta-layout
  contract pinned end-to-end) — this makes the API symmetric, and the
  future staged semantic queries (§4) return the same value vocabulary.

## 3. Before / after — the `Default` derive (real code)

Today (`std/src/default.vl` — builders exist but bottom out in strings, and
note the leaking import line):

```vilan
macro fun Default(item: Item): Source {
	import macro_std::option::Option::{ self, Some, None };
	import macro_std::build::{ impl_of, fun_of, init_of };

	if item.as_struct() is Some(let target) {
		mut literal = init_of(target.name);
		for field in target.fields {
			literal = literal.field(field.name, i"{field.type_.render()}::default()");
		}
		let constructor = fun_of("default")
			.returns(target.name)
			.expr(literal.render());
		ret source("import std::default::Default;\n"
			+ impl_of(target.name).implements("Default").method(constructor).render());
	}
	source("")
}
```

After (value-returning builders; the import is declared, scoped, unleakable;
no `.render()`, no string assembly, no re-parse of the structure):

```vilan
macro fun Default(item: Item): Output {
	import macro_std::option::Option::{ self, Some, None };
	import macro_std::build::{ output, expr, impl_of, fun_of, init_of };

	if item.as_struct() is Some(let target) {
		let defaults = target.fields
			.map(|field| (field.name, expr(i"{field.type_.render()}::default()")));
		let constructor = fun_of("default")
			.returns(target.name)
			.expr(init_of(target.name).fields(defaults));
		ret output()
			.uses("std::default::Default")
			.item(impl_of(target.name).implements("Default").method(constructor));
	}
	output()
}
```

The diff is deliberately small — the 2026-07-07 builder migration did the
hard part; this change is what the builders *return* and how the engine
*receives* it. The quoted leaf (`expr(i"…::default()")`) is the
items/expressions boundary made visible, and the bulk `.fields(…)` form
turns the old mutable-builder fold into a `map` over the reflection — the
shape macro code should read as.

## 4. Sequencing

1. **`rpc.vl` leak-dependence fix** — std code depending on a leaked name is
   a bug under every design; clean it first, independently.
2. **The contract migration** — `Output` + value-returning builders + engine
   structural splicing + expansion-scoped `uses`. The derives/`[service]`
   **byte-identical gate** protects the whole migration (same generated
   program, new plumbing). Text `Source` remains accepted during the
   transition, deprecated after std migrates.
3. **Staged semantic queries** (the capability follow-on, own design when
   demanded — the trait-based `[derive(Wire)]` check is the standing
   candidate): expansion in waves against analyzed state, read-only, cycle
   diagnostics when a macro's query depends on its own output. Recorded, not
   in this slice.

## 5. Open questions (for sign-off)

1. **Naming/shape of `Output`** — one value with `.item`/`.uses`, or a list
   of items where `use` is itself an item builder? (Draft: one value; a
   scoped `use` is semantically different from an emitted item.)
2. **Re-exports from expansions** — may an expansion `export`? (Draft: no —
   generated re-exports are spooky; generate the item itself.)
3. **`macro { .. }` blocks and expression-position invocations** — same
   `Output` channel with items restricted by position? (Draft: yes — an
   expression-position macro returns a quoted expression, item position
   returns items; the engine already distinguishes the positions.)
4. **Cache keys** — unchanged (worlds and expansions still key on source
   text; `Output` values are what the cache STORES post-execution). Confirm.
5. **The handle/quote boundary** — which constructions get first-class
   handle-based builders (calls, struct/variant construction, field access,
   local bindings as gensym handles?) vs staying quoted-with-splices. Draft:
   start with `resolve_module`/`.namespace`/`.item`, `FunctionCall::from_fn`,
   construction, and gensym locals; grow the handle layer only from real
   derive/`[service]` call sites, never speculatively — the quoted-splice
   form is always available.
6. **Handle resolution vs generated items** — a handle resolving an item
   another macro GENERATES reintroduces expansion-order sensitivity at the
   name level. Draft: v1 handles resolve only against loaded modules (std,
   real files); resolving generated items joins the staged-queries design.

## 7. Full normalization — the horizon (recorded 2026-07-16, DEFERRED)

The thought experiment (user): if API churn were solved, what would
normalizing ALL the way — expression builders, no quoted text — buy beyond
§2? Written down so the deferral is a decision, not amnesia:

- **Ergonomics.** Macro authoring becomes typed, completable vilan: the LSP
  helps write macros, malformed output is impossible at every level, no
  escaping/interpolation reasoning ever, refactors are type-guided.
- **Capabilities — the deep ones.** Values-in mean values-OUT and values
  APART: reflection could extend from items to BODIES, enabling the macro
  classes the engine structurally cannot host today — assert-diagram
  macros, instrumentation/tracing wrappers that rewrite existing bodies,
  checked DSLs, memoization attributes. With staged queries, builders could
  take TYPE handles and check assignability at construction — generated
  code correct before splicing. Every built node carries its constructing
  call site — E8 diagnostics pointing at the macro line that built the
  offending node.
- **Open questions dissolve.** Q1: `.uses` disappears entirely (references
  are handles); Q3: an expression-position macro just returns an expression
  VALUE — no position-restricted parsing; Q5: the handle/quote boundary
  stops existing. **Q6 does NOT dissolve** — order sensitivity against
  generated items is a staging problem, orthogonal to normalization.
- **Grammar decoupling cuts both ways.** Quoted text ties macros to the
  surface grammar; builders tie them to a semantic API. Neither is free —
  which is the churn problem restated.

**The churn problem, and the candidate fix (user):** generate the builder
boilerplate with Rust attributes/macros over the compiler's own enums.
Assessment: that solves the BOILERPLATE half (writing/maintaining builders)
but not the CONTRACT half — an auto-generated API mirrors internal shape,
so internal churn becomes user-visible breakage instead of maintainer toil.
The full fix likely needs both: a small versioned STABLE IR that builders
target, with the Rust-macro trick generating the mechanical parts of the
IR→internal adapter so the cost of keeping the IR stable while internals
move stays near zero. Design that when this section is picked up.

**Decision: deferred.** The §1–§6 contract (Output, value-returning item
builders with bulk forms, scoped `uses`, quoted-with-splices, semantic
handles v1 against loaded modules) is the implementation target now; this
section is its recorded horizon.
