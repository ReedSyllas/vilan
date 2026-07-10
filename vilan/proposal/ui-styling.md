# UI styling — typed atomic styles, compiled

Status: **PROPOSAL** (2026-07-10). The semantics below are the contract being
proposed; **every piece of surface syntax is provisional** and expected to be
refined before implementation — the merge rule, the lowering, the asset
channel, and the theming mechanics are the load-bearing decisions.

## 0. The problem

`std::ui` builds and updates DOM; nothing styles it. Handwritten CSS is the
current answer and an unacceptable one long-term: global names, cascade
surprises, dead rules, and styles living far from the components they dress.

The best mainstream model — Tailwind-style atomic utilities — earns its
popularity with locality (styles in the markup), a market-tested design system
(spacing scales, color ramps, breakpoints), and output that *plateaus*: n
components share one bounded set of single-purpose rules. But its three
chronic pains are one root cause — **styling as strings**:

1. **Long class strings.** Composition has no names, so every use site carries
   the full utterance.
2. **Unpredictable merges.** `class="p-2" + " p-4"` is resolved by *stylesheet
   order*, not authoring order; `tailwind-merge` exists to re-parse strings at
   runtime and guess the author's intent.
3. **Runtime variant cost.** CVA and friends assemble/parse class strings per
   render because the build discarded the structure the author had.

A compiler that owns the whole pipeline does not have to discard that
structure. The proposal: **styles are typed values; the compiler lowers them
to atomic CSS at build time; merge is value semantics, not cascade
semantics.** (This is StyleX's insight, made language-native: Tailwind's
design system with record semantics instead of string semantics.)

## 1. The model

```vilan
import std::ui::style::{ style, space, gray, Display };

let card = style {
    display = Display::Flex,
    padding = space.4,
    background = gray.50,
    hover = style { background = gray.100 },
};
let active = style { padding = space.6 };

view.class(card + active);   // padding resolves to space.6 — LAST WINS, always
```

- A **`style { .. }` block** is a macro block (the macro engine is the
  proving ground; first-class syntax is the promotion path if it earns it).
  At expansion time it validates property names and value shapes, and lowers
  to (a) a small per-property **class map** value and (b) the **atomic CSS
  rules** those properties need, emitted through the asset channel (§3).
- A **`Style`** value is, conceptually, a map from property-slot → atomic
  class name. One rule per `(property, value, condition)` triple, deduplicated
  program-wide, is what makes the stylesheet plateau exactly as Tailwind's
  does.
- **Merge is `+`** (`impl Style with Add`): per-property, right side wins —
  a record update, not a cascade. The compiler guarantees the emitted classes
  agree with the value semantics (each property contributes exactly one class,
  so the merged map *is* the resolution; specificity fights are structurally
  impossible). Fully-static merges constant-fold to a precomputed map; dynamic
  merges are a small map union at runtime — no string parsing, ever.
- **Variants are just code.** CVA dissolves into the language:

  ```vilan
  fun button(kind: Kind): Style {
      base + match kind {
          Kind::Primary => style { background = blue.600, color = white },
          Kind::Danger => style { background = red.600, color = white },
      }
  }
  ```

  Enum-driven variant spaces with static arms fold to per-arm constants.
- **Long strings become names.** Style values are ordinary module-level or
  local bindings — co-located with their component, tree-shaken like any
  binding (F6), composed like any value.

## 2. Tokens, themes, and conditions

### 2.1 Tokens are CSS custom properties (the load-bearing trick)

`space.4` does **not** resolve to `1rem` at compile time. It lowers to a class
whose rule reads a custom property:

```css
.pA3 { padding: var(--space-4); }
:root { --space-4: 1rem; }        /* emitted once, from the theme */
```

The macro therefore needs only token *identities* (static in the source), never
token *values* — which keeps the macro engine's world small (no evaluating
theme code at expansion time) and buys, for free:

- **Theming and dark mode**: a theme is a set of custom-property values;
  `:root[data-theme="dark"] { --gray-50: #111; }` re-skins every component
  with zero recompilation. v1 ships `dark = style { .. }` as a condition
  lowering to `:root[data-theme="dark"] &` (explicit, SSR-friendly control;
  an auto `prefers-color-scheme` mode is a recorded refinement).
- **Dynamic values**: a signal-driven width is the same mechanism —
  `width = var(--w)` in the style, `view.style_var("--w", signal)` at the
  binding site. Dynamism never reopens the runtime-CSS door.

v1 ships std defaults stolen wholesale from the market-tested scales
(Tailwind's spacing scale, color ramps, type scale) as token namespaces
(`space`, `gray`, `blue`, `text`, …). User theme *extension* (new tokens, new
ramps) is deferred; user theme *values* (overriding the custom properties)
works day one because it is just CSS emission.

### 2.2 Conditions: pseudo-classes, breakpoints

Nested style blocks under condition keys lower to per-property atomic rules
with the condition baked in (`hover = style { .. }` → `.hB7:hover { .. }`):

- **Pseudo**: `hover`, `focus`, `active`, `disabled`, `first`, `last` (v1 set).
- **Breakpoints**: `md = style { .. }` → `@media (min-width: 768px) { .. }`.
  Media queries cannot read custom properties, so breakpoint values must be
  macro-static: v1 ships the fixed std set (`sm`/`md`/`lg`/`xl`, Tailwind's
  values); making them configurable is a recorded refinement (it needs a
  compile-time knob — the `[build]` options system is the natural home).
- Conditions nest one level in v1 (`md = style { hover = .. }` deferred).

### 2.3 The escape hatch

The typed property table covers the core that styles 90% of real UI (layout,
spacing, color, typography, borders, radius, shadow, transition — ~60
properties, std-maintained data). The long tail does not block:

```vilan
raw("mask-image", "linear-gradient(to bottom, black, transparent)")
```

lowers to an atomic rule like any other (hashed class, deduplicated), just
without value validation. Plain string classes coexist untouched
(`view.class_name("leaflet-container")`) for third-party CSS.

## 3. Compiler additions

### 3.1 The asset channel (the one genuinely new compiler feature)

Macros emit *code*; a stylesheet is not code. The style macro's expansion
includes specially-attributed module-level constants —

```vilan
[asset("css")]
let _rules = ".pA3{padding:var(--space-4)}\n.hB7:hover{background:var(--gray-100)}";
```

— and the compiler collects every `[asset("css")]` string constant reachable
in the build, **deduplicates by line** (this is what makes per-macro-expansion
emission compose into one plateauing stylesheet with no cross-macro state),
orders rules by kind (base < pseudo < breakpoint, then lexically — determinism
for byte-stable outputs), and writes `<out>.css` beside the `.js`.

Properties of the channel worth stating:

- **General**: nothing about it is styling-specific; it is "compile-time
  accumulated build assets", and A7 SSR wants the same channel for critical
  CSS (the stylesheet already exists at build time — static extraction means
  SSR needs no runtime style collection, unlike CSS-in-JS).
- **Deterministic**: class names are content hashes of
  `(property, value, condition)` — never counters — so parallel/incremental
  compiles and reordered expansion agree. Debug builds can emit readable names
  (`p_space4`) under the existing `debug-names` codegen knob; release keeps
  short hashes. (The macro needs a hash in `macro_std` — small addition; the
  compiler already carries djb2 for `contract_hash`.)
- **Over-approximation, recorded**: v1 accumulates CSS from every *expanded*
  style literal in compiled modules; a style value that is never referenced
  still contributes rules (they are mostly shared atomics, so the cost is
  bounded). Tying asset emission to F6 binding-liveness is the recorded
  refinement.

### 3.2 What the macro engine must support (mostly: nothing new)

Macro blocks, source-text arguments, and the construction API are shipped.
Needed: the `macro_std` hash function (§3.1), and DSL-quality diagnostics —
the style macro must report *spanned* property/value errors, which will
exercise (and possibly extend) the macro error story. If diagnostics prove
too coarse in practice, that is the trigger for promoting `style` to first-
class syntax with analyzer typing — the semantics do not change, only who
checks them.

### 3.3 HTML hookup

`vilan build` for a browser target emits `app.css`; the html host references
it (`<link rel="stylesheet">`). The examples' scaffolds gain the link; A7's
server render later inlines or links it per its own design.

## 4. UI integration

- `View.class(style: Style)` renders the class string (joined map values,
  cached per map identity). Reactive styling composes with the existing
  machinery: a class bound through a signal re-renders the attribute inside
  the normal turn/ownership model — the merge stays a map union, predictable
  under any interleaving.
- `view.style_var(name, signal)` writes a custom property for dynamic values
  (§2.1).
- Server-layer code may construct `Style` values freely (they are plain data);
  platform rules are unaffected.

## 5. The Tailwind bridge (supported, sidecar, not the foundation)

Real Tailwind integrates today with near-zero compiler work: its scanner
regex-walks text for class-shaped strings — point its content config at
`**/*.vl`, pass strings through `view.class_name(..)`, run the CLI beside
`vilan build --watch`. Worth documenting and keeping healthy as the
familiarity option and escape hatch. Not the foundation, because its pains
live in the string representation (fixing merge for real Tailwind would mean
maintaining Tailwind's version- and config-dependent utility semantics inside
our compiler — the wrong home for someone else's database).

The one reusable piece: a small `vilan.toml [build] run = [".."]` hook for
running external tools alongside builds/watch — useful beyond styling;
recorded as its own backlog item.

## 6. Implementation plan (slices)

1. **The asset channel**: `[asset("css")]` collection, dedup, ordering,
   file emission; `[build]` wiring; pins for determinism and dedup.
2. **`Style` + merge in `std::ui::style`**: the map representation, `Add`,
   `class(..)` rendering; pins for last-wins, identity, static-fold parity.
3. **The `style` macro, core properties + tokens**: property table, token
   namespaces, custom-property theme emission; the motivating corpus program.
4. **Conditions**: pseudo set + breakpoints; ordering rules in the channel.
5. **`raw(..)`, `style_var(..)`, docs + the Tailwind-bridge writeup.**
6. (With A7, later) critical-CSS inlining; theme extension; liveness-tied
   asset emission; configurable breakpoints.

## 7. Open questions (deliberately unsettled)

- **Surface syntax** — the user refines this next: `style { .. }` block shape,
  property spelling (`padding = space.4` vs `p = 4`?), condition keys, merge
  operator (`+` vs method), theme/token naming.
- Property table scope for v1 (the ~60 list needs writing out).
- Whether `Style` values participate in equality/hashing (memoized class
  strings suggest yes).

## 8. Alternatives rejected

- **Runtime CSS-in-JS** (styled-components model): per-render style work and
  SSR collection machinery — the industry is walking away from it for
  reasons vilan would inherit.
- **Compiler-maintained Tailwind semantics** (typed class strings +
  compile-time `tailwind-merge`): couples the compiler to an external
  project's per-version utility database. Rejected even as a bridge; the
  sidecar (§5) covers the familiarity case.
- **Tokens as macro-time values** (evaluating theme code at expansion):
  requires the macro world to run theme modules and forfeits free re-theming;
  custom properties (§2.1) dominate it on every axis.
