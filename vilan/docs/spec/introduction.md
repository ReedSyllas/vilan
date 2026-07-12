# Spec §1 — Introduction & conformance

This is the vilan language specification: the normative definition of what
programs mean and which programs are rejected. The [tour](../tour/) teaches;
this defines. Where the two disagree, this document wins.

## 1.1 Scope and normativity

- Unmarked prose in `docs/spec/` is **normative**.
- *Implementation notes* (italic paragraphs beginning "Implementation
  note:") are **not** normative: they record where the current compiler is
  known to diverge from this specification (linking the tracked gap), or
  where behavior is deliberately implementation-defined.
- The specification states **intent**. A compiler behavior that contradicts
  normative text is a compiler bug, even before it is fixed; such gaps are
  tracked in `vilan/proposal/backlog.md` and pinned as `#[ignore]`d tests.

Deliberately implementation-defined (a conforming implementation may vary):
diagnostic wording and spans; the shape of emitted JavaScript (subject to
§7's observable guarantees); compilation performance; the order in which
independent diagnostics are reported.

The standard library's API is specified by the [std reference](../std/) and
is not part of this document, except for the **lang items** the language
itself depends on (appendix §A.4): `Option`, the `Try`/`Lift` traits, the
operator traits, `PartialEq`/`PartialOrd`, `Context`, `Promise`, `List`,
and the primitive types' declarations.

## 1.2 Program processing phases

A vilan program is processed in phases; each phase's errors preclude the
next. A **conforming program** passes all of them.

1. **Lexing** (§2) — source text to tokens.
2. **Parsing** (§3) — tokens to the syntax tree.
3. **Macro expansion & loading** (Phase B) — module loading, attribute/
   derive/block expansion, splicing. Generated code re-enters phases 1–2.
4. **Name resolution** (§4) — names to declarations.
5. **Type checking** (§5) — types, generic binding, bound checking.
6. **Memory checking** (§6) — view confinement, the aliasing rules.
7. **Context checking** (Phase B) — ambient-value coverage.
8. **Async inference** (§7) — asyncness, seam checking.
9. **Emission** (§7.1 for the observable guarantees).

An error's phase is observable only in which other errors accompany it; a
conforming implementation may report errors from several phases at once.

## 1.3 Notation

Grammar productions use this EBNF dialect:

```text
production = alternative | alternative ;
sequence   = item item ;
[ x ]        optional
{ x }        zero or more
( x )        grouping
"literal"    a keyword or fixed token
IDENT        a token class (small caps, defined in §2)
```

Code blocks tagged `vilan` are conforming programs (compile-checked by the
test suite). Blocks tagged `vilan,fragment` are fragments or deliberate
counter-examples; a counter-example always states its error class in the
surrounding prose.

## 1.4 Source files, packages, programs

A **module** is one source file (`.vl`, UTF-8). A **package** is a directory
with a `vilan.toml` manifest declaring `[package]` (an application) or
`[library]`; a `[project]` manifest groups packages into a workspace
(normative manifest schema: §11, Phase B). A **program** is an application
package compiled for a **platform** (§11): its entry module, the modules it
transitively imports, its dependencies' modules, and the standard library
subset they reach.
