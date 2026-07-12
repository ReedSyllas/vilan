# Encoding — reference

JSON (`std::json`), the codec-agnostic wire layer (`std::wire`), the binary
codec (`std::binary`), raw bytes (`std::bytes`), and base64
(`std::base64`).

The short version: derive `Json` for JSON in/out at app boundaries, derive
`Wire` for rpc payloads, and let the codecs do the rest. Everything below
"Derives" here is plumbing you only meet when building custom transports or
parsers.

## JSON

```vilan,fragment
trait Json { fun to_json(self): str; }        // encode
trait FromJson {                              // decode
	fun from_json(text: str): FromJson;
	fun from_json_value(value: JsonValue): FromJson;
}
```

`[derive(Json)]` implements both from a struct/enum's shape; scalars,
`List`, and `Option` nest:

```vilan
import std::print;
import std::json::Json;

[derive(Json)]
struct Point {
	x: i32,
	y: i32,
}

fun main() {
	let point = Point { x = 1, y = 2 };
	let text = point.to_json();
	print(text);
	let back = Point::from_json(text);
	print(back.x);
}
```

Untyped inspection, when the shape isn't known up front:

```vilan,fragment
external struct JsonValue;
fun parse_json_value(text: str): JsonValue    // panics on bad JSON
str.try_parse_json(): Option<JsonValue>       // the safe form
value.field(name: str): JsonValue
value.tag(): str                              // "object" | "array" | "string" | …
value.elements(): List<JsonValue>
value.is_null(): bool
```

`json_codec(): Codec` is the JSON wire codec for rpc (see below).

## The wire layer (`std::wire`)

The codec-agnostic serialization protocol under `derive(Wire)` and rpc:

- `trait Serialize` / `trait Deserialize` — visitor-style value
  description (`begin_struct`/`field`/`str_value`/`i64_value`/…). The
  wire scalars: `str`, `bool`, `i32`, `u32`, `i64`, `f64` (+ lists,
  options, structs, enum variants).
- `Frame` — one encoded message.
- `Codec` — a matched writer/reader pair: `json_codec()` (`std::json`,
  readable) or `binary_codec()` (`std::binary`, compact). Client and
  server must agree.

`[derive(Wire)]` requires every field to be Wire, recursively, checked at
the derive site. You implement `Serialize`/`Deserialize` by hand only for
types with a custom encoding.

## Bytes

An immutable-length byte array (`Uint8Array` underneath) — the currency of
the binary codec, crypto, and websockets:

```vilan,fragment
impl Bytes {
	fun alloc(size: i32): Bytes
	fun len(self): i32
	fun get(self, index: i32): i32
	fun set(self, index: i32, value: i32)
	fun slice(self, from: i32, to: i32): Bytes
	fun fill(self, value: i32, from: i32, to: i32): Bytes
	fun copy_into(self, source: Bytes, offset: i32)
	fun concat(a: Bytes, b: Bytes): Bytes     // static
	fun to_hex(self): str
}

// UTF-8 text ↔ bytes
fun encode_utf(text: str): Bytes
fun decode_utf(bytes: Bytes): str
```

Lower still: `ByteBuffer`/`DataView` (host ArrayBuffer access,
`read_f64`/`write_f64`) — the binary codec's float channel.

## Binary codec (`std::binary`)

```vilan,fragment
fun binary_codec(): Codec
fun encode_binary<T: Wire>(value: T): Bytes
fun decode_binary<T: Wire>(bytes: Bytes): T
struct BinaryWriter { … }   // write_byte / write_i32 / write_str / finish(): Bytes
```

Same model as JSON, compact layout. `i64` values ride as f64 bit patterns —
exact to 2^53.

## Base64 (`std::base64`)

URL-safe alphabet, no padding — the JWT flavor:

```vilan,fragment
fun encode_url(bytes: Bytes): str
fun decode_url(text: str): Option<Bytes>
```
