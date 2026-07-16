# `[T; n]` — fixed-length arrays (backlog I2)

`List<T>` is vilan's only sequence today: a growable, heap-shaped JS array behind a
nominal `Struct(list_id, [T])`. It is the right default, but it is the *only* option,
so every fixed-size buffer — a codec's scratch, an RGBA pixel, a 4×4 matrix row, a
lookup table — pays for length-mutability it never uses and carries no size in its
type. `[T; n]` is the contiguous, fixed-length companion: the length is part of the
type, there is no `push`/`pop`, and (crucially, on a future native/WASM backend —
F3/F4) it is inline storage with none of a growable vector's overhead. On the JS
backend it lowers to the same plain array; the whole v1 win is *type-level* — a size
the compiler knows and a resize it forbids.

`Bytes` (`bits-and-bytes.md`) is the specialized `u8` buffer over `Uint8Array`; this is
the general element-typed form it left on the backlog.

## 1. The type

`[T; n]` — element type `T`, length `n` a compile-time constant. `[i32; 4]`,
`[bool; 8]`, `[Point; 3]`, nested `[[f64; 4]; 4]`.

**Representation — a new `Type::Array(TypeId, usize)`.** `Type` today
(`type_.rs:19-51`) is `Struct(Id, Vec<TypeId>)` / `Tuple(Vec<TypeId>)` / … — the
argument vectors hold *types*, with nowhere to put a number, so the length can't ride
`List`'s `Struct(list_id, [T])` shape. `Type::Array(element, len)` carries the length
in the type itself; `#[derive(PartialEq, Eq)]` then makes `[i32; 3]` and `[i32; 4]`
**distinct types** for free — the fixed-length contract falls out of type equality.
`len: usize` (not a `TypeId`) — v1 lengths are concrete (see §4); const-generic lengths
are deferred (§7).

## 2. Literals

Two forms, because `[a, b, c]` is already `List<i32>` and must stay so (every existing
program and corpus depends on it):

- **Repeat `[value; n]`** — the flagship. `[0; 16]` is a sixteen-zero `[i32; 16]`;
  `[Point { x = 0, y = 0 }; 3]` is three independent points. New expression grammar
  (`Node::Repeat(value, len)`), unambiguous against `Node::List` — the `;` after the
  first element is the fork (a comma continues a list). `value` is evaluated **once**
  and copied into each slot (value semantics — §3), so a non-scalar element is cloned
  per slot, not aliased.
- **List literal `[a, b, c]`, context-directed.** Unchanged by default — it is
  `List<T>`. When the *expected* type is `[T; n]` (an annotated `let`, a parameter, a
  return, a field), the same literal elaborates to the fixed array instead, and its
  element count must equal `n` (else a spanned error). `let row: [i32; 3] = [1, 2, 3]`.
  This mirrors how `expr!` and empty `[]` already take direction from their expectation.

## 3. Semantics

- **Value-copied like everything.** `[T; n]` classifies as a cloneable aggregate
  (`is_cloneable_aggregate`, `analyzer.rs:2599`), so `mut b = a` deep-copies through the
  existing `clone_sites`/`__clone` path — no new runtime. (`__clone` already deep-copies
  JS arrays.)
- **Indexed, bounds-checked.** `arr[i]` reads, `arr[i] = v` writes, `&[mut] arr[i]` is a
  scalar/aggregate element view — all reusing `__at`/`__at_put`/`__at_view`
  (shape-compatible with a JS array). `resolve_subscript` (`analyzer.rs:11980`, today
  List-only) gains an `Array` arm yielding `T`. A **literal** index known out of range
  (`arr[9]` on `[i32; 4]`) is a **compile error** — the length is in the type; a dynamic
  index keeps the runtime bounds check.
- **No resize.** No `push`/`pop`/`insert` — they don't exist on the type. The contract
  is "exactly `n`, always".
- **`for x in arr`** iterates the `n` elements (`iterable_element_type`,
  `analyzer.rs:7666`, gains an `Array` arm → `T`); `for e in &mut arr` gives element
  views, exactly like `List`.
- **The length is in the type** (`[i32; 4]` — you already know it is 4). `.len()`
  ships as slice 2 — see §10; the generic `[T; N].len()` → `N` waits on const-length
  lengths (§7).

## 4. Length `n` — literals in v1

`n` is an **integer literal** (`[i32; 4]`, `[0; 16]`). Evaluated to a `usize` at type /
literal resolution and stored in `Type::Array`. A `const`-named length (`[u8; SIZE]`)
wants `const_eval` (`const_eval.rs`) threaded into type resolution — real, but its own
slice (§7). This keeps v1's new surface to "a literal in the two new grammar spots".

## 5. Lowering (JS backend)

`[T; n]` **is** a JS array — same runtime as `List`, so index/view/clone/iteration reuse
the List lowering unchanged. Only the two literals are new:

- `[a, b, c]` directed to an array → `js::Node::Array([a, b, c])` (identical to the List
  literal — the array-ness is type-only).
- `[value; n]` → for a **scalar** `value`, `Array(n).fill(value)`; for an **aggregate**,
  `Array.from({ length: n }, () => __clone(value))` so each slot is independent (the
  value expression is hoisted to a temp and evaluated once). A new `__repeat(value, n,
  clone?)` helper may fold both.

Corpus stays byte-identical (nothing uses `[T; n]` yet; the directed `[a,b,c]` case only
arises under a new `[T; n]` annotation).

## 6. Scope of v1

Type `[T; n]` (`Type::Array`), the repeat literal `[v; n]`, context-directed `[a,b,c]`,
indexing (read / write / `&[mut]` view / literal-OOB compile error), value-copy, `for`
iteration (by value and by view). Nested arrays (`[[T; m]; n]`) fall out of the
recursion. (`.len()` shipped separately as slice 2 — §10.)

## 7. Deferred (recorded)

- **Const-named / const-generic lengths** — `[u8; SIZE]`, `fun f<const N>(a: [T; N])`.
  Needs `const_eval` in type resolution, then length as a const-expr id in the type.
- **`List` ↔ `[T; n]` conversion** — `arr.to_list()` / `list.to_array<n>()` (the latter
  is fallible — length mismatch). Explicit methods, not coercion (no silent conversion).
- **Multi-dimensional sugar / slicing** — `arr[1..3]` (needs a slice/range type first).
- **Destructuring** `let [a, b, c] = arr`.

## 8. Implementation surface (from the read)

- **`type_.rs`**: add `Type::Array(TypeId, usize)`. It is matched exhaustively in ~dozens
  of places — each is a touch-point that must gain an arm: `reconcile_type`,
  `pretty_print_type`, `is_cloneable_aggregate`, `iterable_element_type`,
  `resolve_subscript`, `substitute_type`/monomorphization, `get_type`/rendering. This
  fan-out is the bulk of the work; most arms are one line (recurse on the element).
- **Parser**: a `[T; Ctrl(';') n]` production in the type `choice((…))` (`parser.rs:1575`)
  → new `Node::ArrayType(inner, len)`; a `[value; n]` expression production beside
  `Node::List` (`parser.rs:331`) → `Node::Repeat`. `[`/`]`/`;`/numbers already lex.
- **Analyzer walk**: a `walk_type_node` arm for `Node::ArrayType` (the catch-all at
  `analyzer.rs:7656` panics until then); an `Expr::Repeat` walk + inference; the
  context-direction hook where `Expr::List` reads its expectation
  (`infer_type_inner` List arm, `analyzer.rs:8283`); `resolve_subscript` +
  `iterable_element_type` + `is_place_expr` Array arms. **Not** the `List` name-discovery
  / element-slot machinery (`[T; n]` is structural, concrete `T`/`n`).
- **Transformer**: `Expr::Repeat` emission (§5); everything else reuses List lowering.
- **`.len()`**: a fold to the constant when the subject types as `[T; n]`.

## 9. Test plan (per case, as always)

Type parse + round-trip (`[i32; 4]`, nested, in params/returns/fields); repeat literal
scalar (`[0; 4]`) and aggregate (independent copies — mutate one, others unchanged);
context-directed `[a,b,c]` (right count runs; wrong count spanned error); index read /
write / `&mut` view write-through; literal-OOB compile error; dynamic-index runtime
bounds panic; value-copy (`mut b = a; b[0]=…` leaves `a`); `for` by value and by `&mut`
view; `[i32; 3]` ≠ `[i32; 4]` (a mismatched assign is a spanned error); corpus
byte-identical; a docs example. Interpreter-equivalence for every runnable pin.

## 10. Slice 2 — `.len()` (the fold)

`arr.len()` on a `[T; n]` subject returns **`i32`** (matching `List.len()`), value `n`.

- **Resolution, not a member.** `[T; n]` is structural — there is no impl to look up.
  `resolve_method_call` gains a `Type::Array` intercept: `len` with zero arguments (and
  no generic arguments) resolves the call directly to a new `Expr::ArrayLen(subject, n)`;
  a non-empty argument list is a spanned error (`` `len` takes no arguments ``), and any
  other method name keeps the standard "`[i32; 4]` has no method 'x'" error.
- **The fold preserves evaluation order.** A **pure** subject (an identifier, a field
  path — anything `expr_has_side_effects` clears) folds to the literal `n` and the
  subject is not emitted. A **side-effectful** subject (`make().len()`,
  `grid[i].len()` — a subscript can panic, so it counts) emits `subject.length` **in
  place** instead: the array is a plain JS array, so `.length` is exactly `n`, the
  subject evaluates exactly once in source order (no statement hoisting, which could
  reorder it against sibling operands), and no new JS node shapes are needed — it is
  the same lowering `List.len()`'s intrinsic uses. The fold is the optimization; the
  in-place property read is the correctness fallback.
- **Nested arrays**: `grid.len()` is the outer length; `grid[0].len()` the inner —
  and the subscript keeps its bounds check (it is the side-effectful case above).
- **Views are transparent**: a view of an array reads as the array, so `.len()`
  through `&arr` / a `for … in &grid` binder resolves the same way.
- **Generic `[T; N].len()` → `N`** stays deferred with const-generic lengths (§7).
- **Fan-out** (the recorded `Type`-variant lesson, applied to `Expr`): the new
  `Expr::ArrayLen` needs arms in `infer_type_inner` (→ `i32`), the call-graph walk
  (the subject must still async-infer and platform-color), the transformer, and
  `expr_has_side_effects` (inherits the subject's). No macro-interpreter change: the
  emission uses only existing JS node shapes.

Tests (per case): pure fold runs (`[0; 4].len() == 4` via a binding); arithmetic use
types as `i32`; nested outer/inner; a side-effectful subject evaluates exactly once
(observable via a `&mut` log) and still yields `n`; `a.len(1)` errors; `a.push(1)`
errors ("no method"); `List.len()` regression-guarded by the existing corpus.
