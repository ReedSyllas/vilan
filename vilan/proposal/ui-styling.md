# UI styling — typed atomic styles, compiled

Status: **CORE SHIPPED 2026-07-10** — `std::style` (same day as the whole
prerequisite stack: `const`, the asset channel, this). Shipped: `Style` as a
slot map (`media:pseudo:property` → class + declaration), the builder chain
(~30 properties), `Color`/`Length`/`space` tokens with `:root` var emission
(per-use lines, deduplicated — no theme-emission coordination needed),
pseudo/breakpoint/dark conditions with one-level stacking, `raw`, pure-vilan
djb2 class hashing (content-addressed; the corpus and the example produce the
SAME class name for the same rule — cross-program determinism proven),
`View.styled` + `View.style_var` + the `set_style_property` DOM extern, 12
pins, corpus `style.vl` with **both `.js` and `.css` goldens**, and a styled
`counter` in the reactive-ui example emitting `app.css`. Implementation
findings: condition combinators re-emit the inner chain's declarations under
their selector (the inner's base rules also emit — the recorded
over-approximation); the interpreter's `new Map`/`new Set` host arms learned
their entries argument (serialized const Maps arrive populated). Remaining,
recorded: `bind_styled(Signal<Style>)` (compose via `bind_class(sig.map(..))`
meanwhile), dark×pseudo stacking, the html `<link>` scaffold, `vilan fmt`
chain splitting, the property-list long tail, and the first draft's items
(critical CSS with A7, dead-style elimination via liveness).

The first draft proved styles
through a macro DSL; this revision — the syntax refinement, settled with the
user — made styling **expression-flavored**: an ordinary typed API riding
the general `const` compile-time-evaluation feature
(`proposal/const-eval.md`), which this system is the forcing use case for.
The semantic layer (atomic lowering, last-wins merge, custom-property
theming, the deduplicating asset channel) carries over from the first draft
unchanged; what changed is *who evaluates it* — the language, not a macro —
so hover, go-to-def, typed diagnostics, functions, impls, and operator sugar
all work out of the box, with no DSL toolchain to build. (The macro draft
survives in git history; its §8 rejection rationale now lives here, inverted.)

## 0. The problem

`std::ui` builds and updates DOM; nothing styles it. Handwritten CSS is the
current answer and an unacceptable one long-term: global names, cascade
surprises, dead rules, styles far from their components.

The best mainstream model — Tailwind-style atomic utilities — earns its keep
with locality, a market-tested design system, and a stylesheet that
*plateaus* (n components share one bounded set of single-purpose rules). Its
chronic pains are one root cause — **styling as strings**: long class
utterances (composition has no names), merges resolved by stylesheet order
rather than authoring order (`tailwind-merge` re-parses strings at runtime to
guess intent), and variant assembly (CVA) paying string-parsing costs per
render because the build discarded the structure the author had.

A compiler that owns the pipeline keeps that structure. Styles here are
**typed values, constructed at compile time, lowered to deduplicated atomic
CSS**; merge is value semantics, not cascade semantics.

## 1. The model

```vilan
import std::ui::style::{ style, space, Color, Display };

let card = const style()
    .display(Display::Flex)
    .padding(space(4))
    .background(Color::gray(50))
    .hover(style().background(Color::gray(100)));

let active = const style().padding(space(6));

view.class(card + active);   // padding resolves to space(6) — LAST WINS, always
```

- **The builder chain is the construction surface** (settled with the user):
  `style()` opens a chain; each property method merges one property in, last
  wins — so the per-property map algebra is unchanged underneath, and calling
  a property method on an EXISTING style is extend-with-override
  (`base.background(blue(600))`). Chosen over free property functions for
  vilan-specific reasons: one `style` import instead of a per-property list
  that grows and collides (`color`, `display` as user locals), and `.`-
  completion over the whole property surface — the discoverability the
  expression-flavored pivot was for. `+` (`impl Style with Add`) remains the
  combinator for NAMED styles (variants). Implementation note: `vilan fmt`
  should split (or preserve) multiline chains — check when the std module
  lands.
- **A `Style` value** is a map from property-slot → atomic class name. Each
  `(property, value, condition)` triple lowers to one CSS rule with a
  **content-hashed class name** (never a counter — deterministic across
  builds; readable names under the `debug-names` codegen knob). Program-wide
  line-dedup in the asset channel is what makes the stylesheet plateau.
- **Merge is a record update, not a cascade.** Each property contributes
  exactly one class, so the merged map *is* the resolution — specificity
  fights are structurally impossible. Fully-const merges fold to a
  precomputed map; runtime merges of const styles are a small map union.
  String parsing never happens.
- **Construction happens inside `const` expressions; selection and merging
  are runtime.** This is the load-bearing rule (§3). (`const` is the
  weak-precedence expression keyword of `const-eval.md` — `let card = const
  ..` is the idiom, and ordinary `let` bindings mean no special naming or
  mutability rules for styles.)
- **Variants are just code** — CVA dissolves into the language:

  ```vilan
  let primary = const base.background(Color::blue(600)).color(Color::white());
  let danger = const base.background(Color::red(600)).color(Color::white());

  fun button_style(kind: Kind): Style {
      match kind {
          Kind::Primary => primary,
          Kind::Danger => danger,
      }
  }
  ```

- **Long class strings become names** — ordinary bindings, co-located with
  their component, tree-shaken (F6), composed like any value.

## 2. Tokens, themes, conditions

### 2.1 Tokens: const functions over scales, custom properties underneath

`space(4)` is a const-evaluated function over the scale — **the scale is
data, the validation is const evaluation** (`space(37)` fails the build with
a spanned error; no macro-time property table needed). What it *returns*
distinguishes two token kinds:

- **Themeable tokens** (spacing, colors, typography) resolve to **CSS custom
  properties**: `padding(space(4))` lowers to `.pA3 { padding: var(--space-4) }`
  plus one `:root { --space-4: 1rem }` declaration from the theme. The
  compiler needs token *identities*; values stay a CSS-side concern — so
  re-theming and dark mode are property swaps with zero recompilation, and
  signal-driven dynamic values ride the same channel
  (`width(var("--w"))` + `view.style_var("--w", signal)`).
- **Structural tokens** (breakpoints) resolve to **literal values at const
  time** — media queries cannot read custom properties, and const evaluation
  reads the breakpoint constants directly. The first draft needed a
  compile-time config knob here; `const` dissolves it — breakpoints are plain
  std constants a future theme layer can override like any value.

v1 ships std defaults stolen wholesale from the market-tested scales
(Tailwind's spacing scale, color ramps, type scale). **Color tokens are
namespaced on the `Color` type** (settled with the user): the type must exist
for property signatures anyway, associated functions are the established
idiom (`List::new`, `FlushPolicy::AtEnd`), and one import covers every ramp —
`Color::gray(50)`, `Color::blue(600)`, `Color::white()`, plus `Color::hex(..)`
as the typed escape. The completion flow is the point: `.background(` → the
parameter is `Color` → `Color::` lists the ramps. `space(n)` stays a bare
function — one function is not clutter, and `padding(space(4))` keeps the
familiar reading. **What `space(4)` computes to** (settled with the user): a
`Length` carrying the token's IDENTITY — rendered CSS `var(--space-4)`; the
theme's `:root` block supplies the magnitude, and const evaluation validates
the scale step. **Units namespace on `Length`** (the parameter type, the
Color rule again): `Length::px(37)`, `Length::rem(1.5)`, `Length::pct(50)`,
`Length::auto()`, and `Length::var("--w")` — the typed end of the dynamic
channel, pairing with `view.style_var("--w", signal)`. The representation
stays OPAQUE (constructors may render to CSS text immediately; structure for
`calc(..)`/unit arithmetic is deferred) — users never match on a `Length`,
so public variants buy nothing. An arbitrary value mints one atomic class
per distinct value, Tailwind-arbitrary style: the escape, not the norm,
bounded by dedup. Theme *values* are overridable day one (custom properties
are just CSS); theme *extension* (new ramps/namespaces) is deferred.

### 2.2 Conditions

Condition combinators wrap a `Style`, lowering each wrapped property to an
atomic rule with the condition baked in:

- **Pseudo**: `.hover(s)`, `.focus(s)`, `.active(s)`, `.disabled(s)`,
  `.first(s)`, `.last(s)` → `.hB7:hover { .. }`.
- **Breakpoints**: `.md(s)` → `@media (min-width: 768px) { .. }` (values
  from §2.1's structural tokens).
- **Dark mode**: `.dark(s)` → `:root[data-theme="dark"] .dC9 { .. }` —
  explicit, SSR-friendly control; an auto `prefers-color-scheme` mode is a
  recorded refinement.
- Condition methods take a `Style` built by its own chain
  (`.hover(style().background(..))`); one pseudo + one media may stack;
  deeper nesting deferred.

### 2.3 The escape hatch

The typed property surface covers the core that styles 90% of real UI
(layout, spacing, color, typography, borders, radius, shadow, transition —
the ~60-function std list, to be written out in slice 3). The tail does not
block: `raw("mask-image", "linear-gradient(..)")` lowers to an atomic rule
like any other, minus value validation. Plain string classes coexist
untouched (`view.class_name("leaflet-container")`) for third-party CSS.

## 3. The construct-in-const rule (variant completeness)

The expression model's one hard problem: CSS for *every* variant must exist
at build time, but a runtime `match` never evaluates its unchosen arms — a
style constructed at runtime would have classes whose rules were never
emitted. The rule that keeps the system sound:

> **Styles construct inside `const` expressions; runtime code selects and
> merges.**

Mechanically free: property functions bottom out in `std::asset::emit`
(const-eval.md §3), which is **const-only** — so a runtime construction is a
static error at the construction site ("styles are compile-time values —
build them in a `const` expression"), enforced by call-graph reachability,
not convention. Selection (`match` over const styles) and merging (`+` as map
union over already-emitted rules) stay ordinary runtime code. This is the
constraint StyleX arrived at from the other direction, here falling out of
the capability model instead of a lint.

## 4. Compiler & std additions

Almost everything is `const-eval.md`'s: the evaluator (exists — the macro
interpreter), the `const` binding form, the const-only bit, the asset
channel with its dedup/ordering/emission (CSS ordering: base < pseudo <
media, then lexical). On top, this proposal adds only:

- **`std::ui::style`** — `Style` (the property map), `Add`, the property
  functions, condition combinators, token functions, `raw`. Pure std vilan.
- **`View.class(style: Style)`** — renders the joined class string (cached
  per map identity); reactive class switching composes with the existing
  turn/ownership machinery, staying a predictable map union under any
  interleaving. `view.style_var(name, signal)` writes custom properties for
  dynamic values. Plain-string `class_name` remains.
- **HTML hookup**: browser builds emit `<out>.css`; the html host links it;
  A7's server render later inlines critical CSS via the same channel.
- Server-layer code may hold `Style` values (plain data); platform rules are
  unaffected.

## 5. The Tailwind bridge (supported, sidecar, not the foundation)

Unchanged from the first draft: real Tailwind integrates today with near-zero
compiler work — its scanner regex-walks `**/*.vl` for class-shaped strings;
pass them through `view.class_name(..)`; run the CLI beside
`vilan build --watch` (the `[build] run` hooks item, A9, makes that
pleasant). Worth documenting as the familiarity option and escape hatch. Not
the foundation: its pains live in the string representation, and fixing merge
for real Tailwind means maintaining its per-version utility semantics inside
our compiler — the wrong home for someone else's database.

## 6. Implementation plan (slices)

1. **`const` core** (const-eval.md slices 1–4: grammar, analyzer, evaluator
   pass, serialization) — independently landable and useful.
2. **The asset channel** (const-eval.md slice 5) + the const-only capability
   bit and its call-graph check.
3. **`std::ui::style` core**: `Style` + `Add` + `class(..)`, the property
   functions and token scales, `:root` theme emission; the motivating corpus
   program (byte-stable CSS golden beside the JS golden).
4. **Conditions**: the pseudo set, breakpoints, `dark`; channel ordering
   rules pinned.
5. **`raw`, `style_var`, docs** + the Tailwind-bridge writeup.
6. (With A7, later) critical-CSS inlining; liveness-tied asset emission
   (dead-style elimination); theme extension; auto dark mode.

## 7. Open questions

- The v1 property-function list (write out the ~60 in slice 3's design note).
- `Style` equality/hashing (memoized class strings suggest yes).
- ~~Whether method sugar ships in v1~~ — settled: the builder chain IS the
  surface; free property functions are not shipped.
- ~~Naming convention for style consts~~ — dissolved by the expression-form
  `const`: styles are ordinary `let` bindings, no special convention needed.

## 8. Alternatives rejected

- **The macro DSL** (this proposal's own first draft) — semantics identical,
  but every consumer pays the DSL toll: no hover/go-to-def/typed diagnostics
  inside the block, custom syntax highlighting, macro-grade error spans. The
  expression form gets the whole toolchain for free and composes with
  functions/impls/match natively. Kept in git history as the record.
- **Runtime CSS-in-JS** — per-render style work and SSR collection machinery;
  the industry is walking away from it for reasons vilan would inherit.
- **Compiler-maintained Tailwind semantics** (typed class strings +
  compile-time merge) — couples the compiler to an external project's
  per-version utility database. The sidecar (§5) covers familiarity.
