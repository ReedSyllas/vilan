# Spec §6 — The memory model

Values are copied (§6.1); sharing is asked for explicitly. Every explicit
sharing mechanism — views, projections, `Shared`, `Arena`/`Handle`, and the
resource class (§6.8) — is a *claim* on an owner, and all of them obey the
one law stated in §6.0. The numbered sections that follow are that law's
projections: the four rules (design:
`proposal/memory-management-rev-1.md`), the escape hatches, and resources
(design: `proposal/claims-and-epochs.md`, `proposal/destruction.md`). Rules
1–3 and rule 4's static half are normative and enforced; rule 4's dynamic
remainder is future work, marked below.

## 6.0 The law — owners, epochs, and claims

One relation underlies every sharing mechanism in this chapter; the
sections after it are its projections.

An **owner** is any value with independent existence: a binding, a
container, an arena slot, a counted cell, a resource. Every owner has an
**epoch** — an abstract counter that advances on a fixed set of **events**,
determined by the owner's shape:

| owner shape | epoch events |
|---|---|
| scalar cell (a boxed local) | rebinding, death |
| aggregate (a struct root) | + reassigning an aggregate field out from under an interior view |
| container (`List`, arena slots) | + geometry: resize, insert, remove, reallocation |
| counted cell (`Shared`; the future counted tier, §6.8) | + the strong count reaching zero |
| resource (§6.8) | + the move (the source binding's epoch ends), + `drop` (the final event) |

A **claim** is any alias to a place inside an owner: a `&`/`&mut` view
(§6.3), an `Arena` `Handle`, a `Weak`, a loan of a resource. In the
abstract a claim is the triple *(owner identity, path, epoch at capture)*.

**The law: a claim is valid while its owner's epoch is unchanged.** Nothing
else is forbidden — aliasing is permitted, and writing through an alias is
permitted (§6.4 declines exclusivity); only *using* a claim whose owner's
epoch has advanced is illegal. A **suspension point** (`await`) is the
degenerate event: while a function is suspended, other turns run, so every
owner's epoch must be assumed to advance — `await` bumps the world (§6.6).

A claim's validity is established at exactly one of two times, giving **two
enforcement regimes**:

- **Static discharge — views (§6.3).** The compiler proves that no event
  occurs between a view's capture and its last use. The proof needs a
  *surveyable interval*, which is why views are second-class: lexical
  liveness (declaration to block end) is the interval the compiler can
  audit. Because the proof is total, the access is infallible and free —
  a view compiles to a bare `(base, key)` and checks nothing at runtime.
- **Dynamic carry — handles.** When a claim outlives every surveyable
  interval — stored in a field, kept across `await`, held between turns —
  it carries its epoch as data (a `Handle`'s generation) and every access
  re-establishes validity by comparison, answering `Option<&T>`. The check
  runs at use time, so access is *failable, and the failability is in the
  type*. A handle is a view that survived by carrying its proof obligation
  with it.

The same relation, read at compile time or at use time:

| concern | static (proof) | dynamic (check) |
|---|---|---|
| interior access | `&`/`&mut` views; `borrows` provenance (§6.5) | `Arena.get(handle): Option<&T>` |
| invalidation | rule 4 (§6.4): reassignment, mutating call | generation mismatch → `None` |
| suspension | no view across `await` (§6.6) | handles cross freely; the next access re-validates |
| death | resources: use-after-move; drops placed statically (§6.8) | a stale handle / `Weak` → `None` |
| exclusivity | declined — aliased views and content writes are legal | traps only on *invalidating* overlap (§6.7) |

Reading down the columns: the left is what the compiler proves when
liveness is lexical; the right is the identical relation carried as data
when the claim must outlive the compiler's sight. Sections 6.1–6.8 place
each mechanism in a cell — rules 1–2 (§6.1–§6.2) govern owners that carry
no claims; rule 3 (§6.3) is the static column's interval requirement; rule
4 (§6.4) is the static invalidation cell; `borrows` (§6.5) makes a claim's
origin explicit; the escape hatches (§6.7) are the dynamic column; and
resources (§6.8) are the static **death** cell, where use-after-move is the
compile-time twin of a stale handle's `None`.

## 6.1 Rule 1 — values are copied; copies are semantic

Every binding, assignment, argument pass, field initialization, and
return **copies** the value. After `mut b = a`, mutating `b` never
affects `a` — for primitives, structs, enums, tuples, lists, and every
other value type alike:

```vilan
import std::print;

struct Point { x: i32, y: i32 }

fun main() {
	mut a = Point { x = 4, y = 6 };
	mut b = a;
	b.x = 10;
	print(a.x);   // 4 — b is a semantic copy
}
```

## 6.2 Rule 2 — elision is an optimization, never observable

An implementation may skip a copy (reuse the storage) when no conforming
program can tell — e.g. when the source is never used again. Elision must
not change any program's output; a live view of the source counts as a
use. (This rule licenses the JS backend to alias under the hood; it
grants programs nothing.)

## 6.3 Rule 3 — references are second-class views

`&place` / `&mut place` create a **view**: an alias of a place, readonly
or writable. Views are values of view type (`&T`, `&mut T`) but are
deliberately second-class — a view may not outlive the thing it views:

- A view may be a **parameter** (the caller's place is lent for the
  call), a **short-lived local**, or a **return projecting a parameter**
  (§6.5). One vocabulary, position-dependent defaults: `&mut self` on a
  method, `bump(&mut c)` at a call site, `x: &mut T` in a signature all
  carry the same convention.
- A view may **not** be stored in a struct field, a collection element,
  or a `Signal`/`Shared` payload; may not be returned except through a
  `borrows` projection; may not be captured by a closure that outlives
  the place; may not cross an `await` (§6.6).

Mutating through a view writes the viewed place; reading its value
requires an explicit `*`. A view in **value position** — passed where a
value is expected, used as an operator's operand, or bound to a value
type — is a compile error, never a silent coercion to the pointee (so the
`(base, key)` representation of a scalar view can't leak); write `*v` to
copy the value out. Iteration by view (`for e in &mut list`) binds each
element as a view — assignment and field writes go through; `*e` reads the
element. The parameter conventions:

| Convention | Written | Meaning |
|---|---|---|
| bare | `x: T` | by value (a copy — rule 1) |
| own | `own x: T` | by value, explicitly (documentation of intent) |
| ref | `&x` / `x: &T` | readonly view |
| ref mut | `&mut x` / `x: &mut T` | writable view |

## 6.4 Rule 4 — no invalidating mutation under a live view

While a view of a place is live, mutations that would **invalidate** the
view (replacing the aggregate that contains the place, removing the
element it points into, resizing past it) are forbidden. The compiler
enforces the statically-decidable half: assignments to the viewed root or
an enclosing place while a view is live are rejected.

*Implementation note: the dynamic remainder (aliasing reached through
calls, container-internal invalidation) is tracked future work. When it
lands it enforces the **same event set** as the static rule above (§6.0's
design invariant): a trap fires on a reassignment, a geometry change, or a
death under a live claim — never on a mere overlap of content writes, which
the static rule deliberately permits (aliased views, and writing through
them, are legal). `Shared.read()/write()` are the cell-level form of this
dynamic check (§6.7).*

## 6.5 Projections: `borrows`

A function may return a view **into one of its parameters** — the one
sanctioned escape from rule 3's return ban. The projected parameter is
named by a `borrows` clause, which is **inferred** when the body makes it
evident (a method returning a view of `self` needs no clause):

```vilan,fragment
fun write(self): &mut T borrows self;   // Shared::write — explicit
fun get(&mut self, i: i32): &mut T      // inferred: borrows self
```

At the call site the returned view obeys the same second-class rules,
with the borrow anchored to the projected argument: the argument's place
is treated as viewed while the result is live (rule 4 applies to it).
Returning a view of a **local** is always an error (it would dangle).

`Option<&T>` is permitted as a return type for "a view, maybe" (map
lookups); the `Some` payload obeys the same anchoring. An `Option<&mut T>`
may also be built **inline as a transient** and matched in the same
expression — `match Some(&mut a) { Some(let v) => … }`, including the
conditional form `match if c { Some(&mut x) } else { None } { … }` and
forwarding a bare view parameter (`match Some(p) { … }` for `p: &mut T`).
Because the transient never outlives the `match` that consumes it, its
payload may view a **local** (unlike a returned projection). Binding the
same constructor to a `let` stores the view and is rejected.

## 6.6 Views and suspension

**A view may not be live across an `await`.** Between suspension and
resumption other code runs and may invalidate any place; rather than
extend rule 4 across turns, the language forbids the shape. Re-derive
the view after the suspension:

```vilan,fragment
let row = &mut rows[index];
send(row.id);              // suspends
row.text = "sent";         // ✗ error: view live across await

send(rows[index].id);      // ✓ re-derive
rows[index].text = "sent";
```

This applies to every suspension point: calls to async functions
(implicitly awaited, §7), explicit `await`, and calls through async
closure values.

## 6.7 Library escape hatches (informative)

`Shared<T>` (one shared cell; `read()` copies, `write()` yields a
statement-scoped view of the contents) and `Arena<T>`/`Handle<T>` (stable,
generation-checked identities — handles are plain values, storable where
views are not) are std types built on these rules, not extensions of them.
They are §6.0's **dynamic regime**: a `Handle`'s generation is its owner's
epoch carried as data, and `Arena.get` answers `Option<&T>`. When the
counted tier (§6.8) gives `Shared` a runtime check, that check enforces the
same event set as rule 4 — a reassignment, geometry change, or death of the
cell under another live view traps, while overlapping writes through the
cell do not (the reconciled trap law of §6.0; a `write` view is *not*
exclusive). See [cells](../std/cells.md).

## 6.8 Resources and destruction

*(Design: `proposal/destruction.md`.)* Rule 1 copies values, and a
droppable value cannot survive copying: a copied file handle double-closes,
a copied refcount miscounts. Destruction is therefore not bolted onto the
data world — the world is partitioned. **Data** is everything above: copied
on binding, elided at last use, reclaimed by the host. A **resource** is a
small, explicitly-rooted class with *affine* discipline — one owner at a
time, no copies — whose owner's scope end runs its destructor. This section
is the static **death** cell of §6.0: a resource's move ends the source
binding's epoch, and `drop` is its final event.

The class is two tiers. **Tier 1**, specified here and enforced on JS, is
*unique* resources: one owner, move-only. **Tier 2** (`Shared` as a counted
resource, `Weak`, counted closure environments) is specified against the
future native arc and is out of scope for this section beyond the forward
references above.

### The resource class

- **`resource` is a declaration modifier**, written in `external`'s
  position: `resource struct S`, `resource external struct D`, `resource
  enum E`.
- **Containment infers.** An aggregate — struct, enum, tuple, or fixed
  array `[R; n]` — with a resource field, payload, element, or member type
  *is* a resource, recursively (the `Wire`/`Hashable` all-fields machinery
  with the polarity flipped: any resource member marks the whole). Declaring
  `resource` on such a type is allowed and checked; omitting it never hides
  resource-ness.
- **The modifier is required at leaves.** An `external struct` is opaque, so
  a host-object resource (`Database`) must declare itself one.
- **Per-instantiation for generics.** `Option<Database>` is a resource
  instantiation; `Option<i32>` stays data. Resource-ness of a generic type
  is decided at each instantiation, like the platform and asyncness bits.
- **`Drop` may be implemented only for a resource type** (see below); an
  impl on a data type is an error steering to add `resource`.

### The affine rules

*Move* transfers ownership and leaves the source binding dead; *loan* is the
existing second-class view (`self` / `&` / `&mut` conventions, §6.3), which
changes no ownership and is policed by rule 4.

- **R1 — binding moves.** `let b = a;` transfers ownership; any later use of
  `a` is a compile error naming the move site. No copies ever fire for a
  resource.
- **R2 — overwrite drops.** Assigning onto a binding that still owns a
  resource drops the old value first, then moves the new one in.
- **R3 — parameters.** `self` / `&x` / `&mut x` are loans, unchanged; `own
  x` is a move — and for a resource *only* a move: an `own` argument that is
  not the binding's last use is an error (where a data `own` would silently
  copy).
- **R4 — returns move out**, including through `if` / `match` tails (a
  diverging leg is exempt).
- **R5 — fields.** A struct literal moves resources in. A resource field is
  read only by loan (`self.db.exec(..)`, `&mut self.db`); moving it out of a
  live aggregate is rejected — v1 has no partial moves. The sanctioned
  partial move is `Option` (below).
- **R6 — match consumes.** Matching a resource *by value* consumes the
  subject; pattern captures move the payloads into the arm. Matching a loan
  (`match &self.state`) inspects without consuming.
- **R7 — no conditional moves.** A binding must be moved on every path
  through a scope or on none; moving it on one path only is an error. This
  keeps end-of-scope ownership static — there are no runtime drop flags in
  v1.
- **R8 — no moves in repeatable interiors.** Moving a binding declared
  outside a loop from inside its body is an error (the move would repeat).
- **R9 — closures and spawns cannot capture resources.** Capturing one would
  give the closure a second owner. Injected `context`-clause bodies receive
  resource *parameters* as loans — parameters are per-call, not captures —
  so `nursery(|n| ..)`-shaped APIs are unaffected. A closure referencing a
  **module-level** resource is likewise exempt: a module global is loan-only
  and lives for the process (see *Module-level resources never drop*, below),
  so the closure can never own it and no second owner is created — the
  reference is a per-call loan, exactly like a parameter. Captures of a
  **local** or a **parameter** stay rejected.
- **R10 — no resource elements in the native containers.** `List` / `Map` /
  `Set` and every external generic (`Shared`, `Task`, `Promise`, `Context`)
  reject resource type arguments in v1 — their internals are host code the
  move checker cannot see. `Option` is the sanctioned container (it is a
  vilan enum, checkable under R11).
- **R11 — generics must be move-clean per instantiation.** Instantiating a
  type parameter with a resource type re-checks the instantiated body under
  the affine rules (T := the resource): every T-typed value is used at most
  once as a move, with no captures and no copies. `Option::unwrap(self): T`
  passes; a body that reads its parameter twice fails **at the instantiation
  site**, not inside std. For an `own T` parameter the rule tightens to
  *exactly* once (a generic body is emitted once and so cannot run an
  instantiation-conditional destructor; zero moves would leak).
- **R12 — no coercion to `any`.** A resource passed where `any` is expected
  is an error (`print(db)` included): `any` is a data sink, and the
  discipline must not launder away. Debug-print the fields instead.

### Destruction — the `Drop` trait

```vilan,fragment
trait Drop {
	fun drop(&mut self);
}
```

The body cleans up through the `&mut self` loan; the compiler destroys the
fields *afterward*, in reverse field order. `&mut self` is the exact and
only accepted shape — a by-value `self` could move the value out and keep it
alive (resurrection), a `&self` receiver cannot run the mutating teardown,
an extra parameter cannot be supplied by an inserted call, and a
value-returning body is rejected. Two further restrictions are enforced:

- **`drop` is synchronous.** An `async` or awaiting `drop` body is rejected
  — teardown runs synchronously in v1. (Cancel owned tasks through an
  `OwnedNursery`, whose own `drop` cancels them.)
- **`drop` is context-free.** A `drop` body that requires an ambient context
  (for example, one that writes a `Signal`, which threads the turn as a
  hidden context argument) is rejected: a destructor's call sites are scope
  exits, which thread no context.

A resource without a `Drop` impl is legal — containment alone still enforces
moves and destroys the resource's fields. `Drop` is distinct from the
cooperative `Disposable` protocol, which is the data-world teardown hook
(subscriptions, owners) and is capture-based (exactly why it is not a
resource mechanism).

### Drop timing and order

At the owner's scope end, still-owned resource locals drop in **reverse
declaration order**. A value's own `drop` body runs **before its fields**,
and the fields drop in reverse field order; an enum's payload drops with the
value. **Every exit runs drops** — fall-through, `ret`, `jump break`, `jump
continue` (out of the scopes they leave), and panic unwinding — because a
resource-owning scope lowers to a `try`/`finally` and every exit flows
through the `finally`. Concrete `own` resource *parameters* drop at scope
end like locals (a generic `own T` is required to move out instead, per
R11).

- **Module-level resources never drop.** A top-level `let` resource has
  process lifetime (the serve-forever server's `Database`). It is
  consequently **loan-only**: moving it into a local, an `own` argument, or
  `drop(x)` would hand a process-lifetime resource to a droppable owner and
  is rejected; method calls and `&` / `&mut` passing are accepted.
- **Panic during unwind.** A `drop` that panics while a panic is already
  unwinding replaces the in-flight error (JS `finally` semantics; a native
  backend would abort).
- **Across `await`.** Owning a resource across a suspension is legal —
  frames own their locals, and §6.6's no-view-across-`await` restriction is
  about *loans*, not ownership. Under cancellation a bridged operation
  rejects, the frame unwinds, and drops run.
- **Exit after finally.** When a value-returning `main` owns a resource, its
  process exit is sequenced *after* the teardown `finally` runs, so scope-end
  drops are never skipped by process termination.

### `Option.take` and `Option.replace`

Moving out of a place must leave a valid value behind. One intrinsic pair on
`Option<T>` provides the sanctioned partial move:

```vilan,fragment
impl Option<type T> {
	fun take(&mut self): Option<T>;              // Some(v) -> None here, Some(v) out
	fun replace(&mut self, value: T): Option<T>; // new value in, old contents out
}
```

`self.slot.take()` is how a resource field leaves a live aggregate (R5), and
`match opt.take() { Some(let c) => drop(c), None => {} }` is the
conditional-teardown idiom R7 pushes toward. Both are ordinary std surface,
useful for data too.

### Early teardown — `drop`

```vilan,fragment
fun drop<T>(own value: T) {}
```

Moving a value into `drop` destroys it at that (immediate) scope end instead
of waiting for the owner's scope to close. The call is rewritten at each
site by the concrete argument type: for a resource it lowers to that type's
destructor; for plain data it is a no-op that consumes the argument for its
effects. There is no public `close()` to keep in sync with a destructor —
`drop(x)` is the early form. Inside a generic body a `drop(x)` on a value of
a still-abstract type `T` has no concrete destructor (a generic body is
emitted once, erased), so R11 rejects it **under a resource instantiation**
("the erased body cannot destroy a `T` — move it out to the caller"); a data
instantiation keeps the legitimate no-op consume.

### `OwnedNursery`

`OwnedNursery` (`std::task`) is the resource-owner for object-lifetime
background work that no function-scoped `nursery` can hold. It wraps a
`Nursery` (which stays data — the ambient handle; `Context<R>` is
R10-rejected, so ownership lives only in the wrapper). `enter(body)` runs
`body` with the owner's nursery established as ambient — every task spawned
in the body's dynamic extent registers with the owner — but, unlike
`nursery`, does *not* join: it returns the body's value as soon as the body
settles, leaving the spawned tasks running under the owner. Its `Drop`
cancels the owned nursery, so dropping the owner (at scope end or via
`drop(owner)`) aborts in-flight bridged IO.

Its nursery runs in **detached mode**: because nothing ever joins the owned
children, a child failure that is not a cancellation echo takes the
free-task reporting path (console, with the spawn origin) rather than being
stored for a join, and a child does not cancel its siblings — ownership is
lifetime, not fate-sharing. Cancellation echoes stay silent.

### Services and resources

A `[service]` struct that owns a resource field is itself a resource by
containment, and its generated dispatcher builds per-`[rpc]` handler
closures that capture `self` — which R9 forbids. R9's module-level exemption
does not apply here: `self` is a local store value, not a module global, so
the capture is a genuine second owner. This collision is **by design**: the
sanctioned shape is the module-level idiom — hoist the resource (a
`Database`) to a module-level `let`, and let the store hold only its reactive
state (plain data) and reach the resource by loan. The module-level resource
is process-lifetime and never drops, and reachability keeps its initializer
out of the client bundle.

### Honesty limits

The following are recorded limits the model does not promise, not bugs:

- **A never-settling await leaks its frame's drops.** An *unbridged* await
  that never resolves (nothing cancels it) leaves its frame — and the
  resources the frame owns — undropped. This is structured concurrency's
  concern, not the memory model's.
- **R11/R12 diagnostic residues** (completeness, not soundness). The
  per-instantiation move scan descends into *direct* lexical closures only,
  so a double move of `T` inside a *nested* closure is not separately
  flagged (its capture is still caught); dispatched (trait-typed-receiver)
  callees are not re-discovered by R11's scan or R12's `any`-coercion check,
  matching the standing convention that convention checks skip dispatched
  callees — R11's per-instantiation re-check is the net under that residue.
  A cross-file instantiation may anchor its *primary* span imprecisely (the
  body note carries the correct source). A module-*initializer* global →
  global move is not scanned — benign, since module globals never drop.
