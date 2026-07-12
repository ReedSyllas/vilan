# Macros & const

Two compile-time tools: `const` folds *values*; macros generate *code*. Both
run in the compiler — the emitted JS carries only results.

## `const` — compile-time evaluation

`const expr` evaluates the expression at compile time and serializes the
plain-data result in place:

```vilan
import std::print;

fun squares(): List<i32> {
	mut result: List<i32> = [];
	for i in [1, 2, 3, 4] {
		result.push(i * i);
	}
	result
}

let TABLE = const squares();   // the emitted JS holds the literal list

fun main() {
	let folded = const 1 + 2 * 3;
	print(folded);
	print(TABLE.len());
}
```

- `const` captures **weakly** — everything to the expression's end folds;
  parenthesize to narrow: `(const square(4)) + square(2)` runs the second
  call at runtime.
- Free variables must be compile-time-known: imports, literals, immutable
  bindings whose initializers are themselves const.
- Only host-independent code can fold — `const now()` is an error ("unknown
  host call").

The styling system is `const`'s flagship customer: `const style()…` chains
evaluate at compile time and emit CSS (see [Styling](../guide/styling.md)).

## Attribute macros and derives

A macro is a vilan function that runs at expansion time and returns source
text to splice. `[derive(…)]` and `[service(…)]` are this mechanism; you can
write your own:

```vilan
import std::print;
import std::display::{ Display, format };

macro fun derive_display(item: Item): Source {
	import macro_std::source;
	import macro_std::meta::{ Item, Source, StructItem };
	import macro_std::option::Option::{ self, Some, None };

	let target = match item.as_struct() {
		Some(let found) => found,
		None => StructItem { name = "?", fields = [] },
	};
	mut arms = "";
	mut first = true;
	for field in target.fields {
		if first {
			first = false;
		} else {
			arms = arms + " + \", \" + ";
		}
		arms = arms + i"\"{field.name}=\" + format(self.{field.name})";
	}
	source(i"impl {target.name} with Display \{
	fun to_string(self): str \{
		import std::display::format;
		{arms}
	\}
\}
")
}

[derive_display]
struct Point {
	x: i32,
	y: i32,
}

fun main() {
	print(format(Point { x = 1, y = 2 }));
}
```

The mechanics:

- A `macro fun` body is **hermetic**: it compiles against `macro_std` (the
  macro-world std: `source`, `meta` with `Item`/`StructItem`/…, and the
  basics), not your program — its imports are its own.
- It receives the annotated item as data (`item.as_struct()`, fields with
  names) and returns `Source` — text built with interpolation. Literal
  braces in generated code are escaped `\{` `\}`.
- The returned source splices before analysis: generated code is
  type-checked like anything you wrote.

## `macro { … }` blocks

An anonymous macro, expanded immediately — in **item** position it stamps
out items; in **expression** position it folds to a value:

```vilan
import std::print;

macro fun labeled(name: str, value: i32): str {
	i"fun {name}(): i32 \{ {value} \}\n"
}

macro {
	mut generated = "";
	mut index = 0;
	for index < 3 {
		generated = generated + labeled(i"constant_{index}", index * 10);
		index = index + 1;
	}
	source(generated)
}

fun main() {
	print(constant_0() + constant_1() + constant_2());
}
```

Use item-position blocks for families of near-identical items; use `const`
(not an expression-position macro) when you're just folding a value.

## When to reach for which

| Need | Tool |
|---|---|
| a computed constant / lookup table | `const` |
| CSS, assets, anything emitted at build time | `const` calling std emitters |
| an impl derived from a type's shape | a derive macro |
| a family of similar items | `macro { … }` in item position |
| transforming a whole item (a `[service]`-like) | an attribute macro |

Limits worth knowing: macro expansion is fueled (a runaway macro is a
compile error, tunable via `[macro] fuel`/`depth` in `vilan.toml`), and
macros see one item at a time — there is no whole-program reflection.
