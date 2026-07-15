# Hashable keys for `Map` / `Set` (backlog I1)

Status: **PROPOSED 2026-07-14**.

## 1. The problem

`Map` / `Set` are `external struct`s over the native JS `Map` / `Set`, and JS
keys objects by **reference** (SameValueZero). Primitive keys work — JS keys
`str`/numbers/`bool` by value — but a vilan struct lowers to a positional array
(`Point { x = 1, y = 2 }` → `[1, 2]`), and two by-value-equal structs are
*distinct* JS arrays:

```vilan
mut seen: Set<Point> = Set::new();
seen.insert(Point { x = 1, y = 2 });
seen.contains(Point { x = 1, y = 2 });   // false — reference miss
```

It compiles (there is no bound today) and is silently wrong at runtime. The
same is true for tuples, lists, and enums (all arrays). So aggregate keys are a
footgun, and the fix is to key by *value*, which on a JS backend means
canonicalizing the key.

## 2. The decision

Introduce a reusable **`Hashable` trait** whose method produces a **`Hash`** — an
opaque canonical-key value — and bound the collections `Map<K: Hashable, V>` /
`Set<K: Hashable>`. Under the hood a `Hash` is `JSON.stringify` of the key (the
raw value for a primitive, since JS already keys those by value); on the JS
backend a canonical-keyed native `Map` is O(1).

The point of a *trait returning a value*, rather than a bare marker, is
**reuse**: anyone can bound their own container on `K: Hashable`, call
`k.hash()`, and key a `Map<Hash, …>` of their own — and anyone can hand-write
`impl Hashable` to key by a subset of fields (`fun hash(self): Hash {
self.id.hash() }`), the vilan analogue of a custom Rust `Hash`. A marker trait
would gate keys at compile time and give callers nothing at runtime; `Hashable`
+ `Hash` is a real capability.

## 3. Semantics

### 3.1 The trait

```vilan
trait Hashable {
    fun hash(self): Hash;
}
```

### 3.2 The `Hash` value

`Hash` is an opaque, compiler-known type (like `JsonValue` / `Bytes`) whose
runtime representation **is** its canonical key: the raw value for a primitive,
a `JSON.stringify` string for an aggregate. Properties:

- **Usable as a native `Map`/`Set` key** — it lowers to a string or primitive,
  which JS keys by value.
- **`==`-comparable** — native `===` on the underlying value (`impl Hash with
  PartialEq`), so a user can compare two hashes.
- **`Hashable` itself** — `impl Hash with Hashable { fun hash(self): Hash {
  self } }` (a hash is already a canonical key), so `Map<Hash, V>` works
  directly.

Keeping `Hash` opaque (not a plain `struct Hash { key: str }`) seals the
representation: callers can't peek at or depend on the internal string, so it
stays the swap seam (§5).

### 3.3 The uniform hasher

One builtin does the work for every stock impl:

```vilan
external fun canonical_hash<T>(value: T): Hash;   // intrinsic
```

Lowering (primitive-preserving, so primitive keys stay raw and fast):

```js
function __hash(value) {
	// number/string/boolean are value-keyed by JS as-is; aggregates (arrays)
	// canonicalize to their JSON string.
	return (typeof value === "object" && value !== null) ? JSON.stringify(value) : value;
}
```

Every stock `Hashable` impl is the one-liner `fun hash(self): Hash {
canonical_hash(self) }`.

### 3.4 Who implements `Hashable`

- **Primitives** — `str`, `bool`, every sized numeric (`i8`…`u53`, `f32`,
  `f64`): the one-liner impl.
- **Structs / enums** — `[derive(Hashable)]`, which first verifies every field
  (every variant payload) is itself `Hashable` — the recursive-syntactic check
  `[derive(Wire)]` already uses — then emits the one-liner impl. A non-`Hashable`
  field (a closure, `Set`, `Map`, `Shared`, a view) is a compile error naming
  the field. So the derive stays trivial (a check plus a one-liner), while the
  trait remains open to hand-written impls.
- **Tuples and `List<T: Hashable>`** — a blanket / structural impl (both lower
  to arrays; value semantics makes them safe keys — §3.6).

Not `Hashable` in v1: closures, `Set`/`Map`, `Shared<T>`, `Promise`, view types.

### 3.5 `Map` / `Set` dispatch through the trait

**Decided 2026-07-14: genuine per-call dispatch** (over the simpler canonical
shortcut) — so a hand-written `hash()` is honored inside std collections too.
`Map`/`Set` become thin vilan wrappers over a raw `NativeMap<K, V>` (the current
`Map` intrinsics, extracted to `std::native_map` — a JS `Map` keyed by whatever,
here a `Hash` which is a string/primitive JS keys by value). Their methods call
`key.hash()` (a normal, monomorphized method call):

- `Map<K: Hashable, V>` wraps a native `Map<Hash, (K, V)>`.
  `insert(k, v)` stores `(clone(k), v)` under `k.hash()`;
  `get`/`contains_key`/`remove` look up `k.hash()`; `keys()` / `values()` map
  the stored pairs back through `__clone`, so iteration returns real `K`s, not
  hashes. Insertion order is preserved (JS `Map` is ordered).
- `Set<K: Hashable>` wraps a native `Map<Hash, K>` (a `Map`, not a `Set`, so the
  original key is recoverable): `insert` stores `clone(k)` under `k.hash()`;
  `contains`/`remove` test `k.hash()`; iteration yields the stored originals.

(The raw native-keyed map — the current `Map`/`Set` intrinsics, now keyed by a
`Hash`, which is a string/primitive JS keys by value — becomes an internal
layer the public collections wrap.)

### 3.6 Value semantics make aggregate keys safe

Unlike Rust (where a `Vec` key risks mutation-under-borrow), vilan clones the
key on insert (`__clone(k)`), so the stored key and its canonical string are a
**snapshot** — mutating the original aggregate afterward can't desync the map.

### 3.7 Canonical / injective, and the float corner

Within one `Map<K, V>` the key type is fixed, so the canonical form is injective
(equal values → equal keys; different values → different keys), and cross-type
collisions can't happen. Bare primitive keys use JS `Map` SameValueZero
directly, so even `NaN` and `±0` behave as normal map keys there. The one corner
is a **float nested inside an aggregate key**: `JSON.stringify(NaN)` is `"null"`
and `-0`/`+0` both become `"0"`, so those collide inside a struct/list key. v1
documents this rather than special-casing.

## 4. Custom impls and consistency

A hand-written `impl Hashable` may hash by a subset of fields — that is the
feature. As in Rust, the obligation is on the author to keep it consistent with
whatever equality the key is used under (two keys that compare unequal but hash
equal will collide in a `Set`). The derived impl hashes the full value, so it is
consistent by construction; custom impls are at the author's discretion.

## 5. The seam (the swap story)

`Hashable` + the opaque `Hash` + `canonical_hash` is the whole abstraction
boundary. Each later change is invisible to `Map<K: Hashable, V>` callers and
their `[derive(Hashable)]`:

- **A real hash table** on a native/WASM backend — back the raw map differently;
  `Hash`'s representation is sealed, so nothing downstream cares.
- **A tighter canonical encoding** — change `__hash`.
- **Primitive fast-paths / tuning** — internal to the raw layer.

## 6. Migration

- Add the `Hashable` impls for the primitives; add the `K: Hashable` bound to
  `Map` / `Set`; rewrite `Map`/`Set` as the wrappers of §3.5.
- The rewrite re-emits **every** `Map`/`Set` program (method calls through the
  wrapper; the raw layer stores `(key, value)` pairs; `Set` is a native `Map`).
  No non-primitive keys exist in the corpus, examples, or kolt today (verified),
  so nothing needs a new `[derive(Hashable)]` and there is no behavior migration
  — only golden regeneration, each runtime-verified. Iteration order is
  preserved, so behavior is identical.
- **Identity keys are out of scope for v1.** A program that keyed by reference
  identity — a `Shared<T>` — no longer compiles under `K: Hashable`. None exist
  today; if wanted later it gets its own `Identity`/`ByRef` key trait (native
  reference keying), distinct from value keying. This is the disambiguation of
  the "`Shared` via JS reference check" path.

## 7. Slices

1. **`std::hash`** — the `Hashable` trait, the opaque `Hash` type (`==`,
   self-`Hashable`), the `canonical_hash` intrinsic (`__hash` helper +
   interpreter arm — a new codegen helper needs a native interpreter
   implementation or the equivalence gate breaks), and the primitive impls. Pin
   the primitive round-trips and `Hash` equality.
2. **`Map` / `Set` rewrite** — the vilan wrappers over the raw native-keyed
   layer, dispatching `key.hash()`; `(key, value)` storage; iteration returning
   originals. Regenerate + runtime-verify the primitive-key goldens; confirm
   order preserved.
3. **`[derive(Hashable)]`** — the recursive all-fields validation + the
   one-liner impl, for structs and enums; the non-`Hashable`-field error; tuples
   / `List<T: Hashable>`.
4. **Docs** (`std/collections`): value-keyed `Map`/`Set`, `[derive(Hashable)]`,
   using `Hashable`/`Hash` to build your own container, the float corner, and the
   `Shared`-for-identity note.

## 8. Test plan

Pins (inference `assert_compiles_and_runs` unless noted): a `Set<Point>` and
`Map<Point, V>` round-trip by value (`insert` then `contains`/`get` with a
*fresh* equal struct hits); `keys()`/iteration return real structs in insertion
order; a nested struct, an enum, a tuple, and a `List<i32>` key each work;
mutating the original after insert doesn't desync (value-semantics snapshot); a
custom `impl Hashable` (hash by one field) makes two differing values collide as
intended; `k.hash() == k.hash()` for equal `k`, and a user-built `Map<Hash, V>`
works; a struct key without `[derive(Hashable)]` is rejected (`assert_fails`); a
struct with a non-`Hashable` field fails the derive naming the field; primitive
`Map`/`Set` stay correct. Corpus: primitive-key `Map`/`Set` programs regenerated
(byte-diff verified, runtime-identical) + a new `struct-keys.vl` golden.

## 9. Recorded v1 bounds (deferred)

- **Real hashing / non-JS backends** — the `Hashable` + opaque-`Hash` seam is
  exactly what makes this a later, user-invisible change.
- **Identity (`Shared`/by-reference) keys** — a separate key trait (§6).
- **Float-key canonicalization** (`NaN`/signed-zero collisions inside aggregate
  keys) — documented, not special-cased.
- **`Hash` ordering / persistence** — `Hash` is an in-memory keying value, not a
  stable serialization format; no cross-run stability is promised.
