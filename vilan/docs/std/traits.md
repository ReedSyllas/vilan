# Core traits — reference

The traits behind the operators and the derive set: `std::compare`,
`std::default`, `std::operators`.

## std::compare

```vilan,fragment
trait PartialEq<B = Self> {
	fun eq(self, b: B): bool;      // ==
	fun ne(self, b: B): bool;      // != (default: !eq)
}
trait Eq with PartialEq {}

enum Ordering { Less, Equal, Greater }

trait PartialOrd<B = Self> with PartialEq<B> {
	fun partial_compare(self, b: B): Option<Ordering>;
	fun lt(self, b: B): bool;      // <   (defaults over partial_compare)
	fun le(self, b: B): bool;      // <=
	fun gt(self, b: B): bool;      // >
	fun ge(self, b: B): bool;      // >=
}

trait Ord with Eq + PartialOrd {
	fun compare(self, b: Self): Ordering;
	fun min(self, b: Self): Self;
	fun max(self, b: Self): Self;
	fun clamp(self, min: Self, max: Self): Self;
}
```

- `==`/`!=` dispatch through `PartialEq`; `<`/`<=`/`>`/`>=` through
  `PartialOrd`. Numbers, `str`, and `bool` implement them in std.
- For your own types, `[derive(PartialEq)]` gives structural equality —
  the usual path. Implement `PartialOrd`/`Ord` by hand when ordering is
  meaningful (`Instant` does this in std).
- The `B = Self` parameter allows cross-type comparison impls; you'll
  rarely need it.

## std::default

```vilan,fragment
trait Default {
	fun default(): Self;
}
```

Zero for numbers, `""` for `str`, `false` for `bool`.
`[derive(Default)]` composes fields' defaults. Used as a bound by helpers
like `unwrap_or_default` and `List.sum`.

## std::operators — the operator traits

Each operator dispatches through a trait; implement the trait, get the
operator:

| Trait | Operator | | Trait | Operator |
|---|---|---|---|---|
| `Add<B = Self>` | `+` | | `Shl` | `<<` |
| `Sub` | `-` | | `Shr` | `>>` |
| `Mul` | `*` | | `BitAnd` | `&` |
| `Div` | `/` | | `BitOr` | `\|` |
| `Rem` | `%` | | `BitXor` | `^` |

The `B = Self` parameter types the right-hand side — mixed-operand impls
are how std's `Instant + Duration` works:

```vilan
import std::print;
import std::operators::Add;

struct Celsius {
	degrees: f64,
}

impl Celsius with Add {
	fun add(self, b: Celsius): Celsius {
		Celsius { degrees = self.degrees + b.degrees }
	}
}

fun main() {
	let morning = Celsius { degrees = 20.5 };
	let rise = Celsius { degrees = 1.5 };
	print((morning + rise).degrees);
}
```

Compound assignment (`+=`, `/=`, …) rides the same impls. Note: the
operator's result type is `Self` (the left operand's type).

## std::operators — `Try` and `Lift`

The machinery behind `!` and `?.`
([control flow](../tour/control-flow.md)):

```vilan,fragment
enum Verdict<T, B> { Good(T), Bad(B) }

trait Try<T, B> {
	fun verdict(self): Verdict<T, B>;   // split into good/bad
	fun from_bad(bad: B): Self;         // rebuild from the bad half (for propagation)
}
trait Lift {}                           // opt-in marker for ?.
```

`Option` and `Result` implement both in std. A custom two-outcome type that
implements `Try` gets `!`; adding the `Lift` marker gets `?.`.
