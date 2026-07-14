# Validating per-type `from_json` (backlog I3)

Status: **SHIPPED 2026-07-14**. Both `FromJson` methods return
`Result<Self, str>`; scalars check the JSON type before coercing (a new
`JsonValue.kind()` intrinsic behind `is_number`/`is_string`/`is_bool`/
`is_array`); the struct derive presence-checks each field (naming a missing
one) and threads leaves with `!`; the enum derive validates the tag; `from_json`
parses non-crashingly via `try_parse_json`. Both the macro derives (json.vl) and
the Rust fallback (analyzer.rs) emit the new bodies and self-carry
`import std::result::Result`. Migration: the reader reads scalars through raw
`coerce_*` externs (its own sticky-error model unchanged), `rpc_server`'s
websocket-key read uses `unwrap_or`, and a latent missing `import std::io::panic`
in rpc.vl (previously satisfied by `RpcError`'s enum derive injecting it) was made
explicit. 13 inference pins + 8 pre-existing `from_json` tests migrated + 3 corpus
goldens; `docs/std/encoding.md` updated. Deferred (§7): structured error paths,
number-range validation, one-reader dedup.

## 1. The problem

`std::json` has two decoders, and they disagree on trust.

The **codec seam** (`decode_json<T: Wire>(text): Result<T, str>`, over a
sticky `JsonReader`) treats its input as untrusted: a missing field, a wrong
shape, or text that isn't JSON at all becomes a **decode error**, never a
crash. json.vl states the rule outright (line 88):

> a wire frame is attacker-supplied input, so the codec must refuse it as a
> decode error, never a crash.

The **per-type convenience surface** (`FromJson::from_json(text): Self` and
`from_json_value(value): Self`, derived by `[derive(Json)]`) contradicts that
rule. Both methods return `Self` directly, and every leaf trusts its input:

- `i32::from_json_value(value)` is `[extern("Number")]` — `Number(undefined)`
  is `NaN`, `Number("abc")` is `NaN`. No error.
- `str::from_json_value(value)` is `[extern("String")]` — `String(undefined)`
  is `"undefined"`. No error.
- `bool::from_json_value(value)` is `[extern("Boolean")]` — every non-empty
  string is `true`. No error.
- the derived struct decoder reads `value.field(name)` for each field;
  `field` on a **missing** key yields `undefined`, which the leaves then
  coerce to garbage.
- `str::from_json(text)` is `[extern("JSON.parse")]` — and `JSON.parse` on
  malformed text **throws**, i.e. crashes, the exact failure the codec seam
  was hardened away from.

So `Person::from_json("{\"name\":\"a\"}")` where `Person` needs `age: i32`
produces `Person { name = "a", age = NaN }` and flows onward — a silent
corruption — while `Person::from_json("not json")` crashes the process.

## 2. The decision

Extend the codec seam's rule to the per-type surface: **decoding is a
`Result`, never a crash.** Both `FromJson` methods return `Result<Self, str>`
(the `str` error matches `decode_json` exactly). This is not a new
philosophy — it is the one json.vl already documents, applied to the decoder
that skipped it.

`Result` over `panic` (the backlog's fallback option) because the input is
untrusted: a file, an RPC body, a config string. A panic would regress the
codec's own severity fix (a malformed `/rpc` body once killed the server;
now it is a sticky decode error). The `!` / `?.` operators (try-and-lift,
shipped) make threading and call-site handling ergonomic — this is the
"interacts with `?`/try" the backlog anticipated.

## 3. Semantics

### 3.1 The trait

```
trait FromJson {
    fun from_json(text: str): Result<FromJson, str>;
    fun from_json_value(value: JsonValue): Result<FromJson, str>;
}
```

`from_json_value` is the recursive workhorse; `from_json` is the text entry
point. Their relationship is fixed and can be a **trait default**, so each
impl writes only `from_json_value`:

```
fun from_json(text: str): Result<Self, str> {
    match text.try_parse_json() {
        Some(let value) => Self::from_json_value(value),
        None => Err("not valid JSON"),
    }
}
```

`try_parse_json` already exists (the non-crashing parse behind the reader) —
this closes the `JSON.parse`-throws hole for free.

### 3.2 Leaves: validate the JSON type

Scalar decoders check the value's JSON type before coercing:

| type   | accepts        | on mismatch                     |
|--------|----------------|---------------------------------|
| `i32`/`u32`/`f64` | JSON number | `Err("expected a number")` |
| `str`  | JSON string    | `Err("expected a string")`      |
| `bool` | JSON boolean   | `Err("expected a boolean")`     |

This needs a minimal type-check primitive on `JsonValue`, alongside the
existing `is_null`/`tag`/`elements`/`field`: **`is_number()`**,
**`is_string()`**, **`is_bool()`**, **`is_array()`**, **`is_object()`**
(`typeof`/`Array.isArray` intrinsics). The raw coercions (`Number`/`String`/
`Boolean`) remain, now guarded — a checked shell over the same fast leaf.

### 3.3 Structs: presence names the field

The derived `from_json_value` checks each field's presence first (so the
message names it), then recurses, threading failures with `!`:

```
fun from_json_value(value: JsonValue): Result<Person, str> {
    if !has_json_field(value, "name") { ret Err("missing field `name`") }
    if !has_json_field(value, "age")  { ret Err("missing field `age`") }
    Ok(Person {
        name = str::from_json_value(value.field("name"))!,
        age  = i32::from_json_value(value.field("age"))!,
    })
}
```

`has_json_field` (`Object.hasOwn`) already exists. A field of the wrong
*shape* reports the leaf's type message (v1); path-annotated messages
("`age`: expected a number") are recorded v2 (§7).

**Extra fields are ignored** (accept an object with keys beyond the struct) —
the forward-compatible default a wire format wants.

### 3.4 Enums, `Option`, `List`

- **Enum** (externally tagged): `value.tag()` must name a known variant, else
  `Err("unknown variant `X` for `Shape`")`; the payload's arity/shape is
  checked as the variant's fields are, recursing with `!`.
- **`Option<T>`**: `value.is_null()` → `Ok(None)`; else
  `Ok(Some(T::from_json_value(value)!))`.
- **`List<T>`**: `value.is_array()` else `Err("expected an array")`; each
  element recurses with `!`, short-circuiting on the first bad element.

### 3.5 Interaction with `!` / `?.` / try

`Result<T, str>` is a `Try` type, so the shipped operators apply directly:

- inside a decoder, `leaf!` early-returns the `Err` (proven to thread through
  struct-literal field position);
- at a call site, `Team::from_json(text)!` in a `Result`-returning function,
  `Team::from_json(text)?.field` for optional chaining, or an explicit
  `match`.

Round-trips that were `T::from_json(t)` become `T::from_json(t)!` (or a
match). That `!` is the honest cost of decoding untrusted text; it is no
longer a silent success that might be garbage.

## 4. Relationship to `Wire` / `decode_json`

Both decoders stay. `Json`/`FromJson` is the human-readable, reflective
per-type surface (`Team::from_json(text)`, self-describing `{"x":1}`);
`Wire`/`decode_json` is the codec-abstracted one (`decode_json::<Team>(text)`
over any `Codec`). After this change they agree on trust — both validate,
both return `Result<T, str>` — even though the code paths differ (value-walk
vs. sticky reader). Collapsing the two onto one reader is a real
simplification but a **separate** refactor (§7), not part of I3.

## 5. Migration

Blast radius (measured): the `FromJson` trait + its scalar/`Option`/`List`
impls + the two derive macros (`struct_json_impls`, `enum_json_impls`), and
these callers —

- `std::process::rpc_server` — `str::from_json_value(headers.field(
  "sec-websocket-key"))`: the header is present by the WS handshake protocol,
  so `!` inside the (already fallible) handshake path, or a named decode
  error if absent.
- corpus `derive-json.vl`, `derive-enum.vl`, `json-roundtrip.vl` — the
  round-trip assertions gain `!` (they decode text the same program just
  encoded, so they still succeed); goldens regenerate and are runtime-verified.

No non-test program outside std consumes the raw `from_json` surface today
(the Wire/codec path superseded it for RPC), so the change is contained.

## 6. Slices

1. **Primitives + scalars.** Add the `JsonValue` type checks; rewrite the
   five scalar `FromJson` impls as checked vilan (validate → coerce),
   returning `Result`. Add the trait's default `from_json` over
   `try_parse_json`. Pin scalar accept/reject + the malformed-text case.
2. **`Option`/`List`.** Null/array checks + element recursion.
3. **Struct derive.** Presence checks + `!`-threaded literal in
   `struct_json_impls`; regenerate the affected macro output. Pin
   missing-field, wrong-type-field, extra-field-ignored, nested.
4. **Enum derive.** Tag validation + payload shape in `enum_json_impls`. Pin
   unknown-variant + wrong-arity.
5. **Migration + goldens.** `rpc_server` caller; corpus round-trips gain `!`;
   regenerate + runtime-verify goldens; docs (`std/json` page + errors
   appendix decode-error entry).

## 7. Recorded v1 bounds (deferred)

- **Structured errors.** v1 error is a flat `str`. A JSON-pointer path
  (`age`, `items[3].x`) threaded through the recursion is v2 — it wants an
  error-context combinator (`map_err`-shaped) at each frame.
- **Number range.** A JSON number decoding into `i32`/`u32` truncates (the
  existing `Number` behavior); v1 does not reject out-of-range magnitudes.
  Ties to the numeric-types range-check work.
- **One reader.** Routing `FromJson` through the validated `JsonReader` (or
  vice-versa) to delete the duplicate decoder is a follow-up refactor.
- **`to_json` is unaffected** — encoding is total; only decode gains a
  `Result`.

## 8. Test plan

Pins (inference.rs unless noted): scalar accept + each scalar reject +
malformed-text → `Err`; missing field names it; wrong-type field → `Err`;
extra field ignored; nested struct propagates; `Option` null→None /
value→Some; `List` non-array → `Err` and bad-element short-circuit; enum
unknown-tag → `Err`; a full round-trip still succeeds through `!`. Corpus:
the three round-trip programs updated (byte goldens regenerated,
runtime-verified). Docs gate: the `std/json` page's decode examples compile.
