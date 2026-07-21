# The handwritten frontend — replacing chumsky (H6)

> **Status: DRAFT 2026-07-21 — for review.** Backlog H6 (L) + E4 (rides this).
> The user pulled the trigger early (2026-07-21, the structural-improvements
> arc): the recorded triggers ("release builds past ~1s, LSP latency, the next
> grammar fight") are not yet met — a cold release build of the todo client is
> ~0.47–0.51s — but the item is taken deliberately as structural investment
> while the language is alpha and the grammar is still small enough to move.
> Grounded in a full codebase scout (2026-07-21); every count and file:line
> below was verified that day.

## 0. What exists (the verified ground)

- **Two chumsky parsers, not one**: the lexer (`lexer.rs`, 373 lines — a
  chumsky parser over `&str`) and the grammar (`parser.rs`, 2163 lines, one
  ~1600-line `parser_with` function). H6 replaces both.
- **The deepest coupling is `span.rs`, not error types**: `Span =
  chumsky::span::SimpleSpan`, read across ~15 core modules (`.start`/`.end`/
  `.into_range()`/`Span::new`/`From<Range>`; byte offsets, half-open). Error
  types (`Rich`) leak into exactly three `lib.rs` rendering functions.
- **The two-pass cheap/rich split** (2026-07-08 perf arc): `parse_clean` runs
  the grammar with a zero-size error type, failures re-run with `Rich`. A
  handwritten parser is fast-and-rich in one pass — the split, the
  `CustomParseError` bridge, and the double rustc instantiation all dissolve.
- **The cost premise still holds**: ~95% of a cold compile is lex+parse
  (callgrind, todo client 2.43B Ir; token wrap+compare alone ≈17%); this is
  chumsky's structural overhead (ordered `choice`, boxed recursion,
  per-primitive wrapping), not a fixable pathology. Expected win: **3–5× on
  parse** (todo client ~0.47s → ~0.10–0.15s release; the debug binary gains
  most), plus a cheaper vilan-core rustc build.
- **Grammar inventory**: 66 `Node` variants, 44 `Token` variants, ~85
  `.labelled` production names (a ready-made nonterminal list), 8
  `Recursive::declare` slots + 5 inline recursions, a 13-level operator tower,
  the H.1 `condition_expression` (struct-literal-free heads), i-strings
  desugared **in the lexer**, trivia dropped (never attached) with the
  linear-scan fix pinned.
- **Recovery surface** (the true cost center, per the original backlog note):
  10 `nested_delimiters` sites (each → empty/placeholder/`Node::Error`), the
  LSP-critical trailing-`.` member recovery (`p.|` still types its receiver —
  pinned at `member_completion_on_incomplete_receiver`), its `?.` sibling, the
  misplaced-`resource` steer, and the lexer's skip-then-retry. Only the `p.`
  case is pinned as an observable today; the delimiter sites are exercised
  only indirectly.
- **Proof primitives that already exist**: `parse_fast_path.rs` compares two
  frontends by `Debug`-string — **span-inclusive**, since `Span: Debug` and
  `Node` deliberately derives only `Debug`; the corpus byte-gate; fmt's
  re-lex-and-compare safety net (which silently turns `fmt` into a no-op if
  the token stream drifts — a quiet failure mode this arc must test loudly).
- **Interface constraints**: four parse fold sites (lib.rs, CLI, module
  loader, macro expansion) + macros' fifth bare `parse_clean`; two
  content-keyed caches hold `&'static` ASTs **borrowing `&'src str`** — the
  output shape `Spanned<NodeList<'src>>` and the leak model must survive; the
  formatter runs the real parser (not just the lexer) and reprints raw
  un-lifted trees.

## 1. Goals and non-goals

**Goals.** (1) A dependency-free, single-pass, fast-and-rich lexer + parser
producing the *identical* AST — spans included, byte-for-byte — with equal or
better diagnostics and recovery. (2) Control: contextual decisions, curated
errors, and hand-designed recovery sync points become ordinary code, ending
the grammar fights (`condition_expression`, the split-shift hack, the
`try_map` one-off). (3) The E4 door: a handwritten parser is the substrate
sub-file incremental parsing rides later.

**Non-goals.** No grammar changes in this arc (byte-identical trees is the
gate; language work freezes on this path while it runs). No AST redesign, no
`PartialEq` derive (the `Debug`-string differential is deliberate — it is
span-inclusive for free and adds no surface). No incremental parsing yet
(E4 is explicitly out; this arc builds where it will live). No parser
generator — recursive descent with precedence climbing, by hand, is the
point.

## 2. Architecture

**Lexer**: one hand-rolled pass over `&str` → `Vec<(Token, Span)>`. Longest-
match operators, keywords classified at lex time, `<`/`>` always `Ctrl` (the
parser reassembles span-adjacent pairs into shifts, as today), `=>` an `Op`,
trivia skipped in place (linear, the pinned property), i-strings desugared in
the lexer via a mode stack (text → `( "" + "text" + ( holes … ) )` token
sequence with every token carrying the literal's span — the formatter's
verbatim-recovery contract).

**Parser**: recursive descent over `&[(Token, Span)]` (the chumsky
`ValueInput` wrapper vanishes). The ~85 labelled productions become named
functions; the 13-level tower becomes precedence climbing with a table; the
H.1 restriction becomes a boolean in the parser state
(`no_struct_literal_heads`), not a parallel grammar. Errors are values
accumulated in the parser (cheap until rendered); recovery is explicit sync
points: delimiter matching reproduces the 10 `nested_delimiters` behaviors,
statement/item boundaries synchronize on `;`/`}`/item keywords, and the
trailing-`.`/`?.` member recovery is a first-class case in the postfix loop.

**Span**: a new two-field `struct Span { start: u32, end: u32 }` in `span.rs`
with the exact API surface consumed today (`.start`/`.end`/`.into_range()`/
`.to_end()`/`Span::new`/`From<Range<usize>>`, `Debug` rendering compatible
with the differential), dropping the chumsky type — the alias means the ~15
consumer modules recompile unchanged. (Settled: own struct over keeping
`SimpleSpan` as a type-only dependency — the whole point is zero chumsky.)

**Errors**: the native error type carries what `Rich` carried (found/expected
sets, labels, custom messages); `lib.rs`'s three `Rich`-rendering functions
are replaced one-for-one, and `parse_error_hint`'s text-stopgap dissolves
into real parser knowledge (its own doc comment has been waiting for this).

## 3. The proof strategy — differential first, cutover last

The chumsky frontend **stays in-tree as the oracle** for the whole arc and is
deleted only at cutover. The order is deliberately pins-first:

1. **S0 — pin the ground before moving it.**
   - Per-site recovery pins against the *current* parser: each of the 10
     delimiter sites + the `?.` member case + the `resource` steer gets an
     observable test (the partial tree shape or the LSP behavior it exists
     for), joining the existing `p.` pin. These pins are the recovery
     contract the new parser must hold.
   - The corpus-scale differential harness: a test target that runs BOTH
     frontends in one process over corpus + std + `vilan/examples` + the docs
     examples, asserting `Debug`-equal (= span-equal) trees — scaling
     `parse_fast_path.rs`'s exact pattern.
   - A loud fmt-net test: formatter output token-compares against the oracle
     across the corpus, so the silent no-op failure mode has a tripwire.
   - A fresh callgrind split of lex-vs-parse cost (the profiling profile
     exists) so S1's win is measurable on its own.
2. **S1 — the lexer**, differentially: token streams (with spans) byte-equal
   to the chumsky lexer over every source the harness covers, including the
   i-string and trivia edge cases; the lexer's own pins carry over.
3. **S2 — expressions + types**: atoms, the postfix chain, the tower, `is`/
   logical tiers, closures, types with binders/contexts. The differential
   runs the new lexer + new expression parser embedded inside the oracle
   grammar? No — too invasive; instead S2's harness parses expression-shaped
   fixtures + the corpus files whose items happen to be expression-dominated,
   and full-file equality arrives at S3. (The slice boundary is a build
   convenience, not a proof boundary — S3 is where the gate is total.)
4. **S3 — items, statements, patterns, macro forms, imports**: the full
   grammar. Gate: the differential harness green over every covered source;
   the corpus byte-gate green running the NEW frontend end-to-end.
5. **S4 — recovery + rich errors**: the S0 recovery pins hold against the new
   parser; error quality reviewed against the diagnostics standard site by
   site (messages may *improve* — byte-parity of diagnostics is explicitly
   not required — but every changed message passes the ledger's verdict
   discipline); LSP e2e (completion on broken mid-edit sources) green.
6. **S5 — cutover**: the four fold sites + formatter + fast-path plumbing
   move to the new frontend; `parse_clean`/`parser_with`/`CustomParseError`
   and the chumsky dependency are deleted; `span.rs` swaps to the owned
   struct; the differential harness retires into a regression corpus for the
   new parser alone; perf measured before/after (callgrind + wall-clock,
   recorded in the backlog entry). The corpus byte-gate and full suite are
   the final word.

**Freeze discipline**: grammar-touching work (new syntax, parser fixes) holds
during S1–S5, or lands in both frontends with the differential as the referee
— decided per case, with "hold" the default. The arc should be short enough
(the grammar is ~2.5k lines total) that this does not bite.

## 4. Risks, named

- **Span drift** — the differential is span-inclusive by construction; any
  drift fails loudly. The one subtlety is spans inside *recovered* trees
  (placeholder nodes carry the delimiter span today — pinned at S0).
- **fmt's silent no-op** — the S0 tripwire converts it to a loud failure.
- **Recovery regression** — the S0 per-site pins are written before any new
  code exists; the LSP's completion e2e is the integration check.
- **The `&'static` borrow model** — the new parser keeps the borrowing output
  shape; the caches never notice.
- **Scope creep into grammar cleanup** — tempting once the grammar is plain
  code; deferred by rule. Cleanups queue behind the cutover as ordinary
  backlog items with their own tests.

## 5. What this unlocks (recorded, not in scope)

- **E4 sub-file incremental parsing** — tree reuse across edits needs parser
  control chumsky doesn't give; it rides the new frontend's design (parse
  functions that can start at a statement boundary) but ships separately.
- **Watch/LSP latency** — compounds with E12's cross-round parse cache: the
  cache dodges re-parsing unchanged files, H6 makes the changed file's parse
  3–5× cheaper; together a watch round approaches analyzer-bound.
- **Grammar evolution** — contextual keywords and curated parse errors stop
  being fights (the `lazy` keyword lands post-cutover into plain code).

## 6. Open calls — wanted before S1 (recommendations inline)

- **(a) Diagnostics policy at cutover**: allow improved (not byte-identical)
  parse errors, gated by the diagnostics standard + per-changed-message
  review (recommendation), vs strict parity first then improve. Parity-first
  doubles the error-rendering work for no user value.
- **(b) The freeze**: hold grammar-touching changes during the arc
  (recommendation; the B30 `lazy` keyword slots in after cutover), vs
  dual-landing with the differential as referee.

## 7. Slices (suite-gated; the differential harness is the extra gate)

S0 pins + harness + callgrind split → S1 lexer → S2 expressions/types →
S3 full grammar (total differential + corpus-on-new-frontend) → S4 recovery +
errors (S0 pins hold; LSP e2e) → S5 cutover (delete chumsky, own Span,
measure, record). Each slice lands only with the differential green over
everything it claims; nothing user-visible changes until S5.
