# Functions & closures

> Normative rules: spec [§3 Grammar](../spec/grammar.md), [§5.8 Coercions](../spec/types.md), [§7.4 Async closures](../spec/execution.md).

## Functions

`fun` declares a function. The last expression in the body is the return
value, and `ret` returns early:

```vilan
import std::print;

fun clamp(value: i32, low: i32, high: i32): i32 {
	if value < low {
		ret low;
	}
	if value > high {
		ret high;
	}
	value
}

fun main() {
	print(clamp(15, 0, 10));
}
```

Notice there is no `return` on the last line. A bare expression at the
end of a block is the block's value. You'll see this everywhere in vilan.
`if`, `match`, and plain blocks all work the same way.

Generic functions take type parameters. Bounds say what the body is
allowed to do with them:

```vilan,fragment
fun largest<T: PartialOrd>(a: T, b: T): T { … }
```

## Closures

A closure is an inline function value. Where JavaScript writes
`x => x * 2`, vilan writes `|x| x * 2`. Parameter types are usually
inferred from where the closure is used. Annotate them when they aren't:

```vilan
import std::print;

fun apply(seed: i32, transform: |i32| i32): i32 {
	transform(seed)
}

fun main() {
	print(apply(21, |n| n * 2));
	let label = |count: i32| i"{count} items";
	print(label(3));
}
```

Closure **types** are written `|T| U`. A closure with no parameters is
`|| U`, and one that returns nothing is `|| void`. These appear as
parameter types, in `let` annotations, and as struct fields.

Closures capture their surroundings **by value** at the moment they are
created. vilan copies, remember. When a closure needs to share mutable
state with its creator, they hold a `Shared` cell together. The
[memory model](memory-model.md) explains that pattern.

## Named functions as closures

When a function already does what your closure would do, pass the
function itself:

```vilan
import std::print;
import std::reactive::Signal;

fun exclaim(text: str): str {
	text + "!"
}

fun main() {
	let words = Signal::new("hello");
	let loud = words.map(exclaim);   // instead of .map(|w| exclaim(w))
	print(loud.get());
}
```

This works for plain vilan functions. It does not work for generic
functions, methods, `async` functions, or externs. For those, write the
small wrapping closure — the compiler will tell you when you hit one.

## Async closures

You can skip this section until you start storing callbacks that do
async work.

A closure type can carry an `async` marker: `async |T| U`. Calls through
a value of that type are awaited automatically, the same way direct
calls to async functions are (see [Async](async.md)). The marker is
allowed in exactly two places: parameter types and `let` annotations.

```vilan,fragment
struct Draft<T> {
	commit: |T| Option<str>,          // struct fields store the plain type
}
…
let commit: async |T| Option<str> = self.commit;   // re-mark at a let
let outcome = commit(value);                        // this call awaits
```

That "store plain, re-mark at a `let`" dance is the standard pattern for
async callbacks kept in struct fields, because the marker doesn't exist
on field types yet.

There is one more rule, and it works in your favor. Passing an async
closure where a plain closure is expected is an error if the plain type
returns a value — you would receive a promise pretending to be the
value. But if the plain type returns `void`, it is allowed, and the call
becomes fire-and-forget. This is why UI event handlers can await things
freely without any ceremony.

## Context clauses

You will *use* this feature constantly without writing it. It is how the
UI framework passes things like "the current owner" invisibly. You only
write it yourself when building framework-level helpers, so feel free to
skim this on a first read.

A parameter's closure type can declare that the closure reads an ambient
**context**:

```vilan,fragment
fun mount_root(id: str, body: (|| View) context owner_scope): Owner
fun turn<T>(policy: FlushPolicy, body: (|| T) context turn_scope): T
```

When you pass a closure literal into such a parameter, the ambient value
(the current `Owner`, the current `Turn`) is threaded to it at the call
site, through any depth of ordinary function calls in between. This is
the machinery behind "every `effect` registers with the nearest
boundary" in the UI layer. Your component functions never mention
owners, and ownership still flows to the right place.

> **Going deeper.** Two rules keep contexts sound. First, closures
> capture their contexts when they are *created*. A closure created
> outside a scope and called inside it would see nothing, so the
> compiler rejects that shape outright. Second, a function that reads a
> context can't be passed around as a plain value, because the context
> channel would be severed. Both produce clear errors when you hit them.

## Traps

- Calling a method-call result directly doesn't parse yet. Instead of
  `self.hook.read()(a, b)`, bind first:
  `let hook = self.hook.read(); hook(a, b)`.
- A closure bound to a local and then called directly
  (`let f = |i| …; f(3)`) doesn't infer its parameter type from the
  call. Annotate the parameter.
- Chained element access on a call result (`pair().1`, `read()[i]`) can
  lose the element type. Bind, then access.
