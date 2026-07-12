# Styling

`std::style` is typed, atomic CSS built at **compile time**: you construct a
`Style` value inside a `const` expression, the compiler evaluates it and
emits the CSS rules into the build's stylesheet, and at runtime the style is
just a set of class names to put on elements. No runtime rule construction,
no style recalculation — dynamism goes through CSS custom properties.

```vilan,browser
import std::ui::{ view, View, mount_root };
import std::style::{ style, space, Style, Color, Length, Display, FlexDirection };

let card = const style()
	.display(Display::Flex)
	.flex_direction(FlexDirection::Column)
	.gap(space(2))
	.padding(space(4))
	.radius(space(1))
	.background(Color::gray(100));

fun main() {
	let _root = mount_root("app", || {
		view("div").styled(card).child(view("p").text("hello"))
	});
}
```

## The model

- `style()` starts an empty style; every method returns a new style with one
  more **property slot** filled.
- Each slot becomes one atomic CSS rule (one class, one declaration),
  deduplicated across the whole build — two styles sharing
  `padding(space(4))` share the class.
- `view.styled(card)` sets the element's classes (`card.class_list()`).
- Construction must happen in a `const` — that's when rules are emitted. At
  runtime you may still **select and merge** already-constructed styles
  (`a + b`, picking one of two styles in an `if`): the rules already exist.

`+` combines named styles per property, right side wins:

```vilan,fragment
let button = const style().padding_x(space(3)).radius(space(1));
let primary = const button + style().background(Color::blue(600)).color(Color::white());
```

## Values

- **`Length`** — `Length::px(1.0)`, `Length::rem(1.5)`, `Length::pct(50.0)`,
  `Length::auto()`, and `Length::var("--w")` (a custom-property reference —
  see dynamic values below).
- **`space(step)`** — the spacing scale (`space(1)` = 0.25rem, Tailwind-like
  steps); the usual argument to `padding`/`gap`/`margin`/`radius`.
- **`Color`** — `Color::white()/black()/transparent()`, `Color::hex("#663399")`,
  and stepped ramps: `Color::gray(300)`, `Color::blue(600)`,
  `Color::red(500)`, `Color::green(500)`.
- Enums for keyword properties: `Display`, `Position`, `FlexDirection`,
  `AlignItems`, `JustifyContent`, `TextAlign`, `Cursor`, `Overflow`.

Anything not covered has an escape hatch:

```vilan,fragment
.raw("font-family", "system-ui, sans-serif")
.with_length("scroll-margin-top", space(4))
.with_color("outline-color", Color::blue(300))
```

## States and breakpoints

Pseudo-class variants take an **inner** style and condition all of its slots:

```vilan,fragment
let button = const style()
	.background(Color::blue(600))
	.hover(style().background(Color::blue(500)))
	.focus(style().raw("outline", "2px solid"))
	.disabled(style().opacity(0.5));
```

Available: `.hover`, `.focus`, `.active`, `.disabled`, `.first`, `.last`,
`.dark` (the `prefers-color-scheme` variant), and `.pseudo(name, inner)` for
anything else. Breakpoints the same way: `.sm(inner)`, `.md(inner)`,
`.lg(inner)`, `.xl(inner)`, or `.media(min_width, inner)`. A breakpoint
cannot wrap another media-conditioned style (compile-time panic).

## Dynamic values

Runtime-varying styling goes through **CSS custom properties**: the style
declares `Length::var("--w")`, and the element binds the variable to a
signal with `style_var`:

```vilan,browser
import std::ui::{ view, View, mount_root };
import std::style::{ style, Style, Length, Color };
import std::reactive::Signal;

let bar = const style()
	.height(Length::rem(0.5))
	.width(Length::var("--progress"))
	.background(Color::green(500));

fun main() {
	let progress = Signal::new("40%");
	let _root = mount_root("app", || {
		view("div").styled(bar).style_var("--progress", progress)
	});
}
```

The rule is emitted once at compile time; only the variable's value changes
at runtime. This is the entire dynamic channel — there is deliberately no
way to construct new rules at runtime.

## Traps

- `style()` chains **outside** a `const` fail to compile ("emission outside
  const") — construct in a `const`, select/merge at runtime.
- `+` is per-property override, not CSS specificity — the right operand's
  slot replaces the left's for the same property/condition.
- The class attribute is whatever `styled` set — combining `.class(name)`
  and `.styled(style)` on one view overwrites; put custom classes in the
  style via `raw` or use one mechanism per element.

Full method table: the [style reference](../std/style.md).
