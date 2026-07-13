# The rendered docs site (D1 polish, item 5)

## 1. Problem

`vilan/docs/` is now a complete book (tour, guides, reference, spec,
appendices — ~45 pages), but it reads as files in an editor. The two
things a learner is missing are **search** ("where was `bind_each`
again?") and **navigation** (a sidebar, prev/next, working cross-page
links). Both are exactly what a static-site renderer provides, and the
content was deliberately written to need no rewriting for one: plain
markdown, relative `.md` links, fenced code, ASCII diagrams in `text`
fences.

This item was sequenced last so the content could settle first. It has:
the editorial pass, glossary, walkthrough, error index, and diagrams are
all in.

## 2. Goals and non-goals

Goals:

- Full-text search and a sidebar over the existing tree, with zero
  content rewrites — the markdown stays the source of truth and keeps
  working as plain files.
- vilan syntax highlighting in code blocks.
- One-command local build and serve; no daemon, no lockfiles, no node
  toolchain in the repo.
- The suite protects the site's structure the way it protects the
  content (no rotting nav).

Non-goals (recorded):

- Publishing/hosting (GitHub Pages or similar) — decoupled; the build
  output is a static directory, so hosting is a follow-up decision, not
  an architectural one.
- Versioned docs — no releases yet.
- Rendering the proposals (`vilan/proposal/`) — design history stays
  files-first.

## 3. Tooling: the real decision

Three honest options:

**(a) mdBook** — the Rust ecosystem's standard book renderer. One
binary (`cargo install mdbook`), driven by a `SUMMARY.md` table of
contents. Built-in: client-side full-text search, sidebar, prev/next,
light/dark themes, mobile layout. Costs: a hand-kept `SUMMARY.md`
alongside the README index (drift risk — mitigated below), and syntax
highlighting via highlight.js, which doesn't know vilan (mitigated
below: we ship a small custom grammar).

**(b) A hand-rolled generator** (a small crate in the workspace).
Full control, zero external tools, and one genuinely attractive
capability: token-accurate highlighting using the *real vilan lexer* —
the compiler is right there. Costs: we own templates, the nav tree,
anchor generation, a search index, and responsive CSS — a real project
(days, plus ongoing maintenance), spent on web plumbing rather than the
language. The lexer-true-highlighting idea survives as a v2 trigger: if
mdBook ever chafes, the exit is cheap because the content never adapted
to the tool.

**(c) A JS-ecosystem generator** (VitePress/Docusaurus). Nicest
out-of-box UX, but drags a node dependency tree and lockfile into the
repo permanently. Off-brand for this project; not seriously considered.

**Recommendation: (a)** — mdBook with a custom vilan highlight grammar.
Setup is roughly an hour, the tool is one versioned binary, and the
content remains tool-agnostic markdown throughout.

## 4. Layout (under option a)

```
vilan/docs/
  book.toml          — mdBook config (src = ".", build-dir = "book")
  SUMMARY.md         — the sidebar tree (hand-kept, suite-checked)
  theme/
    vilan.js         — highlight.js grammar for vilan + fence-tag shim
  book/              — build output (gitignored)
  …the existing tree, untouched
```

- `mdbook serve vilan/docs` for live-reload writing; `mdbook build` for
  the static output.
- `SUMMARY.md` mirrors the README's tables (Tour → Guides → Reference →
  Spec → Appendix). The README stays the index for file-based readers;
  SUMMARY.md is the sidebar. Both are hand-kept.
- The **fence-tag shim**: our fences carry harness tags
  (` ```vilan,browser `, ` ```vilan,fragment `) that highlight.js would
  treat as unknown languages. A few lines in `theme/vilan.js` normalize
  the class (strip everything after the comma) before highlighting, so
  every vilan block highlights identically and `text` diagram fences
  stay plain.
- The grammar itself is regex-level, not lexer-level: keywords, types,
  numbers with suffixes, strings + `i"…{holes}…"`, attributes
  (`[derive(…)]`), comments. Good enough for docs; lexer-true
  highlighting is the recorded v2.

## 5. Keeping it honest (suite integration)

The site must not rot while the gate watches only content. One new check
in the docs harness (`crates/vilan-core/tests/docs.rs`):

- every `.md` under `vilan/docs/` (minus `book/`) appears in
  `SUMMARY.md`, and every `SUMMARY.md` entry points at a file that
  exists — so adding a page without wiring the sidebar, or moving one
  without fixing it, fails the suite.

The site *build* itself stays out of the suite: it would make every test
run depend on an installed external binary. `mdbook` is optional
tooling; the check above is what CI needs.

The maintenance rule extends naturally: a new docs page lands with its
`SUMMARY.md` line in the same commit (the check enforces it).

## 6. Delivery

One slice: `book.toml` + `SUMMARY.md` + the grammar/shim + the suite
check + a "reading the docs as a website" note in the README. Verify by
building and spot-checking search, the sidebar, cross-page links,
diagram rendering, and highlighted vilan blocks in both themes.

## 7. Decisions (settled with the user, 2026-07-12)

1. **Tooling**: mdBook + the custom vilan grammar.
2. **The SUMMARY consistency check**: yes, in the suite.
3. **Highlighting**: the custom grammar ships in this slice.
4. **Publishing**: GitHub Pages is IN scope — a workflow builds and
   deploys the book (the user wants a shareable URL; enabling Pages in
   the repo settings remains a one-time console step).
