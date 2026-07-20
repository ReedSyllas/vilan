# Resources

> Normative rules: spec [§6.8 Resources and destruction](../spec/memory.md).

Almost everything in vilan is a value: it copies, and the copy is yours (the
[memory model](memory-model.md) chapter). A few things can't work that way.
A database handle that copied would close twice. A task owner that copied
would cancel the wrong tasks. These are **resources**: values with a single
owner, that *move* instead of copying, and that are torn down
deterministically when their owner's scope ends.

You mark one with `resource`, and — if it needs cleanup — give it a `Drop`:

```vilan
import std::print;
import std::drop::Drop;

resource struct Guard {
	label: str,
}

impl Guard with Drop {
	fun drop(&mut self) {
		print(i"dropped {self.label}");
	}
}

fun main() {
	let first = Guard { label = "first" };
	let second = Guard { label = "second" };
	print("body");
}
```

That program prints `body`, then `dropped second`, then `dropped first`.
Two things to notice: `drop` ran on its own at the end of `main`, with no
call from you — and the two guards tore down in **reverse** order, the way a
stack unwinds.

## Moving, not copying

Binding a resource to a new name *moves* it. The old name is now empty, and
using it is an error:

```vilan,fragment
let a = Guard { label = "a" };
let b = a;           // a moves into b
print(a.label);      // ✗ error: use of `a` after it was moved
```

The compiler points at the use and, in a note, at the move: a resource has
one owner, so once you hand it over the old binding is spent. The same
happens when you pass a resource to an `own` parameter, return it, or match
it by value.

## Loaning instead of moving

Usually you don't want to give a resource away — you want to *use* it and
keep it. Lend a **loan** (a view, `&` or `&mut`), exactly as with ordinary
values:

```vilan
import std::print;
import std::drop::Drop;

resource struct Guard { label: str }
impl Guard with Drop {
	fun drop(&mut self) { print(i"dropped {self.label}"); }
}

fun inspect(&g: Guard) {
	print(i"inspecting {g.label}");
}

fun main() {
	let g = Guard { label = "g" };
	inspect(&g);
	inspect(&g);
	print("done");
}
```

A loan changes no ownership, so `g` is still yours after each call — the
program prints `inspecting g` twice, then `done`, then `dropped g` when
`main` ends. Method calls (`&self`, `&mut self`) are loans too; that is how
a resource's own methods reach it without consuming it.

## Teardown happens on every exit

Because teardown is tied to the owner's scope, it runs however the scope
ends — falling off the bottom, an early `ret`, a `jump` out of a loop, even
a panic unwinding through. There are no drop flags and nothing to remember:
if a binding still owns a resource when control leaves the scope, it drops.

## Tearing down early: `drop(x)`

Sometimes you want a resource gone *before* its scope ends. Move it into
`drop`:

```vilan
import std::print;
import std::drop::{ Drop, drop };

resource struct Guard { label: str }
impl Guard with Drop {
	fun drop(&mut self) { print(i"dropped {self.label}"); }
}

fun main() {
	let a = Guard { label = "a" };
	let b = Guard { label = "b" };
	drop(a);                 // a is torn down right here
	print("after drop(a)");
}
```

This prints `dropped a`, `after drop(a)`, `dropped b`. `drop` takes its
argument by move, so `a` is spent at that line — there is no `close()` to
call and no way to use `a` afterward by mistake. (On plain data, `drop(x)`
just means "I'm done with this"; it does nothing.)

## Conditional teardown: `Option.take`

A resource can live in an `Option`, which is the one container that holds
one. `take()` moves the resource out and leaves `None` behind — exactly what
"tear it down only if it's there" needs:

```vilan
import std::print;
import std::drop::{ Drop, drop };
import std::option::Option::{ self, Some, None };

resource struct Guard { label: str }
impl Guard with Drop {
	fun drop(&mut self) { print(i"dropped {self.label}"); }
}

fun main() {
	mut slot: Option<Guard> = Some(Guard { label = "held" });
	match slot.take() {
		Some(let g) => drop(g),
		None => {},
	}
	print("after take");
}
```

After the `take`, `slot` is `None`, so nothing drops a second time at the
end of `main`. `take` is also how a resource leaves a struct field — the one
sanctioned way to move a resource out of something that is still alive.

## A real resource: `Database`

The std `Database` is a resource: opening one gives you a handle that closes
itself on drop. A short-lived database closes when its function returns; a
server that runs forever wants the opposite, so it keeps the database at
**module level**, where it lives for the whole process and never drops:

```vilan,norun
import std::print;
import std::db::{ Database, Row };
import std::option::Option::{ self, Some, None };

let db: Database = Database::open(":memory:");

fun setup() {
	db.exec("CREATE TABLE items (name TEXT)");
	db.prepare("INSERT INTO items (name) VALUES (?)").run(["widget"]);
}

fun count_items(): i32 {
	let statement = db.prepare("SELECT COUNT(*) AS n FROM items");
	match statement.first([]) {
		Some(let row) => row.integer("n"),
		None => 0,
	}
}

fun main() {
	setup();
	print(i"items: {count_items()}");
}
```

Every function reaches `db` by loan (a method call is a loan), never by
moving it. A module-level resource is loan-only for exactly this reason:
moving or `drop`ing it would close the shared handle out from under the rest
of the program, so the compiler rejects that. When you *do* want a database
that closes at the end of a scope, open it in a local instead — or `drop(db)`
to close it early.

A **closure** may reach a module-level resource too. A closure that
references `db` isn't capturing an owner — it borrows the same
process-lifetime storage, per call, exactly as a function does. This is what
gives the module-level idiom its reach: request handlers, injected hooks, and
background tasks can all touch the database, as long as it lives at module
level.

```vilan,norun
import std::db::Database;
import std::shared::Shared;

let db: Database = Database::open(":memory:");

fun main() {
	db.exec("CREATE TABLE account (username TEXT)");
	// A hook closure that reaches the module-level `db` — not a captured
	// owner, just a per-call loan of process-lifetime storage.
	let insert = |username: str| {
		db.prepare("INSERT INTO account (username) VALUES (?)").run([username]);
	};
	let hook = Shared::new(insert);
	hook.read()("alice");
}
```

A *local* resource is different: a closure that captures one would become a
second owner, so that stays rejected (below).

## Owning background work: `OwnedNursery`

A closure can't capture a *local* resource (it would become a second owner),
which is a problem for background tasks that need to outlive the function
that starts them. `OwnedNursery` is the answer: a resource that *owns* the tasks
spawned inside its `enter`, and cancels them when it drops.

```vilan,norun
import std::print;
import std::time::sleep;
import std::task::OwnedNursery;

fun main() {
	let owner = OwnedNursery::new();
	owner.enter(|| {
		let _ = async {
			sleep(50);
			print("background work");
		};
	});
	print("enter returned without joining");
}
```

Unlike a `nursery`, `enter` does not wait for the spawned work — it returns
right away, and the tasks keep running under `owner`. When `owner` drops (at
the end of `main`, or at an explicit `drop(owner)`), the tasks are cancelled.
That is the whole point: the *owner's* lifetime bounds the work, and the
owner is an ordinary resource the scope rules already know how to tear down.

## Traps

- **"Why can't I use it again?"** You moved it. Loan it (`&x` / `&mut x`, or
  a method call) instead of binding, passing, or returning it by value.
- **A resource can't go in a `List`, `Map`, or `Set`** — the compiler can't
  see inside those. Use an `Option`, or a struct field.
- **A closure or spawn can't capture a *local* resource.** Pass a loan into
  the call, give the resource to a struct (or an `OwnedNursery`) that owns the
  closure's lifetime, or keep it at **module level** — a module global is
  loan-only and process-lifetime, so a closure may reach it without becoming
  an owner.
- **`Drop` is only for resources**, must be exactly `fun drop(&mut self)`,
  and must be synchronous and context-free (no `await`, no signal writes).
  Cancel owned tasks through an `OwnedNursery` rather than awaiting them.
