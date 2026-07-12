# The language specification (D1b)

## 1. Problem

D1a (the user-facing docs, `proposal/documentation.md`) answers "how do I
use it". Nothing yet answers "what exactly is the language": the grammar
lives only in the chumsky parser, the type rules only in the analyzer, the
async/memory/context models only in proposals that record *design
deliberation* (with superseded revisions) rather than settled definition.
That has costs: subtle behavior is settled by experiment ("what does the
compiler do?"), regressions in undocumented corners aren't recognizable as
regressions, and any future second implementation (a native backend's
constant folder, external tooling, a formatter) has no reference.

D1b is the **single source-of-truth specification**: the backlog §D.1 item.

## 2. Goals and non-goals

Goals:

- Complete: every construct the parser accepts and the analyzer types has a
  section — nothing is defined only by the implementation.
- Precise: a reader can predict the compiler's verdict on a program without
  running it — including the rejection cases (this is where the docs book
  deliberately stays vague).
- Testable where possible: every example in the spec compiles (or is a
  labelled counter-example) under the existing docs gate.
- Honest: where the implementation is known to diverge from intent (the
  `#[ignore]`d pin set, backlog gaps), the spec states the INTENT and links
  the divergence — the spec leads, the compiler follows.

Non-goals:

- Machine-checked formalism (judgment rules, mechanized proofs). The
  audience is users and implementers; the register is precise English +
  EBNF. A grammar-conformance generator is recorded as a possible follow-up,
  not built.
- Specifying std's API surface (that's the D1a reference). Std enters only
  where the language depends on it (lang items: `Option` for `?.`,
  `Try`/`Lift` for `!`, operator traits, `Context`).
- A stability promise. vilan is pre-1.0; the spec versions with the
  compiler, and changing it is a normal reviewed commit.

## 3. Venue and form

`vilan/docs/spec/` — part of the docs book, so the compile gate covers spec
examples exactly as it covers the tour, and tour chapters can link to their
normative counterparts ("the full rules: spec §…").

Conventions inside the spec:

- **Normative text** is unmarked prose. EBNF blocks define the grammar
  (notation defined in §1 of the spec).
- ` ```vilan ` examples must compile (gate-enforced); rejection examples are
  ` ```vilan,fragment ` and always say what error class they produce.
- *Implementation notes* — italic blocks — record divergences (linking
  `backlog.md` items / `#[ignore]` pins) and deliberate
  implementation-defined latitude (e.g. diagnostic wording, JS output
  shape).

## 4. Chapters

Grounded in the actual surface (32 keywords; the `Node` AST in
`crates/vilan-core/src/node.rs`; the pipeline lexer → parser → analyze →
thread_contexts → async_infer → transform):

1. **Introduction & conformance** — scope, notation (EBNF dialect),
   normativity, the divergence policy, program well-formedness phases
   (lex → parse → resolve → type → context → async → emit; which errors
   belong to which phase).
2. **Lexical structure** — source encoding; comments; identifiers; the
   keyword list; literals: integers (bases, `i8…u64`/`f`/`f32`/`n`
   suffixes, range checking), floats, strings, interpolated `i"…"`
   (escapes, `{expr}` holes), multiline strings, `true`/`false`/`null`;
   operators & punctuation; trivia and token separation.
3. **Grammar** — the full EBNF: programs and items (module files, `import`/
   `use`/`export`, `fun` (+ `async`/`external`/`macro` modifiers, `[extern]`
   attributes), `struct`, `enum`, `trait`, `impl … with …`, attributes and
   derives, `macro {}` blocks); statements and expressions (`let`/`mut`,
   destructuring `let`, assignment incl. compound forms, the operator set
   with the **precedence & associativity table**, calls/methods/indexing/
   member and static accessors, closures and closure types (incl. `async`
   and `context` clauses), `if`/`else`, `match` and the pattern grammar,
   `for … in` / conditional `for`, `jump`, `ret`, blocks-as-expressions,
   `async expr` / `await` / `const` prefix forms and their capture extents,
   `!` / `?.`, `is`, tuples, list literals, struct initializers,
   `&`/`&mut`/`*`, `_`).
4. **Names, modules, and packages** — scopes and shadowing; the resolution
   order (type position vs value position); modules as files; the three
   import namespaces (`std`/`pkg`/dependency) and load model; `export`;
   what `use` does; collision rules.
5. **The type system** — the type grammar (nominal types + generic
   arguments, tuples, closure types, view types, `void`/`any`); primitive
   types; declarations (structs, enums, traits + defaults + `B = Self`
   parameters, impls incl. `impl T<type P: Bound>` binders and conditional
   impls); generic binding and inference (expectation-directed typing, how
   call-site substitutions bind, return-type-only inference, deferral),
   bound checking, monomorphization; coercions (named fn → closure and its
   eligibility rules; there are NO implicit numeric coercions); equality of
   types.
6. **The memory model** — value semantics (the copy rules); places; views
   `&`/`&mut` as second-class borrows: creation, the confinement rules (no
   storing, no returning past inferred `borrows`, no crossing `await`),
   parameter conventions (`own`, `&mut` receivers), view iteration
   (`for e in &mut c`, `*e`); the `borrows` effect and its inference;
   aliasing rules (rule 1–4 as specified in `memory-management.md`, with
   the shipped static subset marked normative and the dynamic remainder
   marked future).
7. **Execution & async** — program start (`fun main`, top-level statements,
   completion semantics); evaluation order; panics; the async model:
   asyncness inference (direct-call infection; what does NOT infect —
   calls through closure values), `async expr`/`async {}` spawn semantics
   and `Promise<T>`, `await`, async closure types and their seams
   (parameter/`let` only), the divergence rule (async into plain non-void
   parameter is an error; void = spawn), interaction with views (§6).
8. **Contexts** — `Context<T>` semantics; `run` extent; `get` static
   coverage (the fence: every possibly-uncovered read is a compile error;
   the exemption rules), `get_safe`; capture-at-creation; `context`
   clauses on parameter closure types and injected-closure threading; the
   value-use restriction on context-reading functions.
9. **Const evaluation** — the `const` prefix extent (weak capture to the
   expression end); eligibility (compile-time-known free variables, no
   host calls); evaluation model (the expansion interpreter) and
   serialization of results; `emit` and build assets; determinism
   requirements.
10. **Macros** — the expansion model (load settling, the once-only
    registry, expansion epilogue, generated code re-entering the load
    loop); `macro fun`, attribute invocation, `[derive(…)]`, `[macro]`
    knobs, `macro {}` blocks (item vs expression position); the hermetic
    macro world and `macro_std`; hygiene (site counters, scoped names);
    fuel/depth limits; splicing and re-analysis; the `[service]` /
    `[rpc]` / `[expose]` attributes as std-provided macros (their
    contract, not their expansion).
11. **The platform model** — platforms (`node`/`deno`/`bun`/`browser`/
    `none`); std layering and the import gate; packages, `vilan.toml`
    (normative schema), workspaces, dependency resolution; externs: the
    four binding forms and their semantics; what codegen guarantees
    (entrypoint invocation, process exit on main completion, `[build]`
    knobs as implementation-defined).
12. **Appendix** — the precedence table, keyword table, grammar collected
    in one place, lang-item table (which std items the language itself
    depends on).

## 5. Method

Per chapter: read the implementing code (lexer/parser for 2–3, analyzer
passes for 4–7, context.rs / const / macros.rs / manifest+platform for
8–11), transcribe the *rules* (not the algorithms), test every claim with a
gated example, and where a claim can't be exercised in an example (pure
rejection cases), state the error class. Divergences found while writing
become implementation notes + backlog entries — writing the spec IS an
audit pass over the compiler.

## 6. Delivery

- **Phase A — the core language**: chapters 1–7 (+ the appendix skeleton).
  Everything a reader needs to predict what plain vilan programs mean.
- **Phase B — the ambient & meta systems**: chapters 8–11, appendix
  completed, tour chapters cross-linked to their normative sections.

## 7. Open decisions

1. **Venue**: `docs/spec/` inside the book (gate coverage, cross-linking)
   — or a standalone top-level `SPEC.md`?
2. **Formalism level**: precise English + EBNF + gated examples — or
   heavier formalism (typing judgments / operational semantics rules)?
3. **Divergence policy**: spec states INTENT with implementation notes
   linking known gaps — or spec documents the implementation exactly as it
   behaves today?
4. **Phasing**: A then B as above — or all twelve chapters in one pass?
