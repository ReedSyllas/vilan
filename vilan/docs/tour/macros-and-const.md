# Macros & const

vilan has two tools that run at compile time. `const` computes *values*
ahead of time. Macros generate *code*. In both cases the emitted
JavaScript carries only the results, never the computation.

Most days you'll use these indirectly: `[derive(…)]` is a macro, and
`const style()` is how the styling system works. Writing your own comes
up rarely, so treat the second half of this chapter as reference.

## `const` — compute it at compile time

Put `const` in front of an expression and the compiler evaluates it
during the build, then writes the *result* into the output as a literal:

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

Three rules to know:

- `const` captures **greedily**: everything to the end of the expression
  folds. Parenthesize to narrow it — in
  `(const square(4)) + square(2)`, the second call runs at runtime.
- The expression can only use things the compiler can know: literals,
  imports, and immutable bindings whose own initializers are const.
- No host calls. `const now()` is an error, because the answer wouldn't
  be a constant.

The flagship user is styling: `const style()…` chains evaluate at build
time and emit CSS. See the [styling guide](../guide/styling.md).

## Derive macros — impls from a type's shape

You've already seen `[derive(PartialEq, Debug)]`. A derive is a macro: a
function that runs at compile time, receives the type it annotates as
*data*, and returns source code to splice into the program. You can
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

How to read that:

- `macro fun` declares the macro. Its body is ordinary vilan, but it
  compiles against `macro_std` — a small compile-time standard library
  with `source`, the `meta` types (`Item`, `StructItem`, …), and the
  basics. Its imports are its own; it can't reach into your program.
- The macro receives the annotated item as data. `item.as_struct()`
  gives the struct's name and fields.
- It returns `Source`: text, usually built with interpolation. Literal
  braces in generated code are escaped as `\{` and `\}`.
- The returned source is spliced in *before* type checking, so generated
  code is checked exactly like code you wrote by hand.

## `macro { … }` blocks

An anonymous macro that expands on the spot. In item position it stamps
out a family of items; in expression position it folds to a value:

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

For plain value folding, prefer `const` — reach for an expression-position
macro block only when you're generating *code*, not computing a value.

## Choosing the right tool

| You need | Reach for |
|---|---|
| a computed constant or lookup table | `const` |
| CSS or other build-time assets | `const` calling std's emitters |
| an impl derived from a type's shape | a derive macro |
| a family of near-identical items | `macro { … }` in item position |
| transforming a whole item (like `[service]` does) | an attribute macro |

> **Going deeper.** Macro expansion is fueled: a runaway macro is a
> compile error rather than a hung build, and the limits are tunable via
> `[macro] fuel` / `depth` in `vilan.toml`. Macros see one item at a
> time — there is no whole-program reflection. The `[service]`,
> `[rpc]`, and `[derive(Wire)]` attributes you meet in the guides are
> this same mechanism, shipped in std.
