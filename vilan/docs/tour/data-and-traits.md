# Data and traits

> Normative rules: spec [§5 The type system](../spec/types.md).

## Structs

A struct is a named record type:

```vilan
import std::print;

struct Task {
	id: i32,
	name: str,
}

fun main() {
	let task = Task { id = 1, name = "write docs" };
	print(task.name);
}
```

Literal fields use `=`, not `:`. When a field's value is a binding with
the same name, you can write it once: `Task { id, name }`. There are no
field defaults — `derive(Default)` (below) covers the all-defaults case.

If you're coming from TypeScript: a struct is like an interface plus an
object literal in one, except it's a real nominal type. Two structs with
identical fields are still different types.

## Enums

An enum is a type with a fixed set of variants, and variants can carry
data. If you've used discriminated unions in TypeScript, this is that
idea with language support:

```vilan
import std::print;

enum Shape {
	Circle(f64),
	Rectangle(f64, f64),
	Point,
}

fun area(shape: Shape): f64 {
	match shape {
		Shape::Circle(let radius) => 3.14159 * radius * radius,
		Shape::Rectangle(let width, let height) => width * height,
		Shape::Point => 0.0,
	}
}

fun main() {
	print(area(Shape::Rectangle(3.0, 4.0)));
}
```

`match` takes the value apart, and the compiler checks that you handled
every variant. Add a variant later and every `match` that misses it
becomes a compile error — that's the feature.

`Option` and `Result` are ordinary enums from std. No special cases.

## impl — methods and statics

Methods live in `impl` blocks, separate from the data:

```vilan
import std::print;

struct Counter {
	value: i32,
}

impl Counter {
	// A static: no self. Called as Counter::new().
	fun new(): Counter {
		Counter { value = 0 }
	}

	// A method: self is the receiver (a value; &mut self to mutate in place).
	fun doubled(self): i32 {
		self.value * 2
	}

	fun bump(&mut self) {
		self.value += 1;
	}
}

fun main() {
	mut counter = Counter::new();
	counter.bump();
	print(counter.doubled());
}
```

The `&mut self` on `bump` matters: it means "mutate the actual receiver,
not a copy". Plain `self` receives a copy, like every other value in
vilan. The [memory model](memory-model.md) chapter makes this precise.

## Generics and bounds

Type parameters work on functions, structs, enums, and impls. A bound
constrains what the code may do with the parameter:

```vilan
import std::print;
import std::compare::PartialOrd;

struct Pair<T> {
	first: T,
	second: T,
}

impl Pair<type T: PartialOrd> {
	fun larger(self): T {
		if self.first > self.second {
			self.first
		} else {
			self.second
		}
	}
}

fun main() {
	let pair = Pair { first = 3, second = 8 };
	print(pair.larger());
}
```

Note the impl-side syntax: `impl Pair<type T: PartialOrd>`. The `type T`
declares the parameter at the impl, and the bound says these methods
exist only when `T` can be compared.

> **Going deeper.** Generics are monomorphized: each concrete use of a
> generic function or impl compiles to its own specialized code, so
> generic dispatch has no runtime cost. This is unlike TypeScript, where
> generics are erased. It also means the compiler checks bounds at each
> call site, not at the declaration alone.

## Traits

A trait declares a capability. `impl Type with Trait` provides it. Trait
methods can have default bodies written in terms of the required ones:

```vilan
import std::print;

trait Greet {
	fun name(self): str;

	// A default, in terms of the required method.
	fun greet(self): str {
		"hello, " + self.name()
	}
}

struct Robot {
	id: i32,
}

impl Robot with Greet {
	fun name(self): str {
		i"unit-{self.id}"
	}
}

fun main() {
	print(Robot { id = 7 }.greet());
}
```

Traits are like interfaces, with two differences worth knowing. They're
implemented explicitly (`impl Robot with Greet`), never structurally.
And they appear as *bounds* on generics (`T: Greet`) rather than as
standalone types — `let x: Greet = …` is a compile error. When you want
"one of several things at runtime", use an enum.

Operators are traits too. `+` dispatches through `Add`, `==` through
`PartialEq`, `<` through `PartialOrd`, and so on. Implement the trait and
your type gets the operator. `std::time` does exactly this so that
`instant + duration` works.

## Derives

`[derive(…)]` generates trait impls from a type's shape, so you don't
write the boilerplate:

| Derive | Gives you |
|---|---|
| `PartialEq` | structural `==` |
| `Debug` | `.debug()` — a developer-facing rendering |
| `Default` | `Default::default()` built from the fields' defaults |
| `Json` | JSON encode/decode (`std::json`) |
| `Wire` | serialization for rpc payloads (`std::wire`) |

```vilan
import std::print;
import std::debug::Debug;

[derive(PartialEq, Debug)]
struct Point {
	x: i32,
	y: i32,
}

fun main() {
	let a = Point { x = 1, y = 2 };
	let b = Point { x = 1, y = 2 };
	print(a == b);
	print(a.debug());
}
```

The standard shape for a type that crosses the wire is
`[derive(Wire, PartialEq, Debug)]`.

> **Going deeper.** Derives are ordinary macros, and you can write your
> own — see [Macros & const](macros-and-const.md). `Wire` and `Json`
> check that every field is itself serializable, recursively, and report
> at the derive site when one isn't.
