# Data and traits

## Structs

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

Literal fields use `=`. A field whose value is a binding of the same name
can be written once (`Task { id, name }`). There are no field defaults —
`derive(Default)` (below) covers the all-defaults case.

## Enums

Variants may carry payloads; `match` consumes them:

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

`Option` and `Result` are ordinary enums from std — same syntax, no special
cases.

## impl — methods and statics

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

## Generics and bounds

Type parameters go on functions, structs, enums, and impls; bounds constrain
what the body may do:

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

Note the impl-side binder syntax: `impl Pair<type T: PartialOrd>` declares
the parameter (`type T`) and its bound at the impl. Generics monomorphize —
each concrete instantiation gets its own compiled code, so generic dispatch
has no runtime cost.

## Traits

A trait declares a capability; `impl Type with Trait` provides it. Trait
methods can have default bodies:

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

Traits appear as **bounds** (`T: Greet`) — vilan has no trait *objects* yet
(`let x: Greet = …` is a compile error), so dynamic dispatch goes through
enums or closures instead.

Operator traits from `std::operators` overload the operators: `Add`
(`+`), `Sub`, `Mul`, `Div`, `Rem`, the bit ops, and `PartialEq`/`PartialOrd`
from `std::compare` for `==`/`<`. `std::time`'s `Instant + Duration` is std
code doing exactly this.

## Derives

`[derive(…)]` generates impls from the shape of a struct/enum:

| Derive | Gives |
|---|---|
| `PartialEq` | structural `==` |
| `Debug` | `.debug()` — a developer-facing rendering |
| `Default` | `Default::default()` from the fields' defaults |
| `Json` | JSON encode/decode (`std::json`) |
| `Wire` | wire serialization for rpc payloads (`std::wire`) |

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

Derives are ordinary macros (user-definable — see
[Macros & const](macros-and-const.md)); `Wire`/`Json` require every field to
be wire/json-able recursively, checked at the derive site.
