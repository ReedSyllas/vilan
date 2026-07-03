# Bits and bytes — the binary floor (backlog I2, codec prerequisites 1–2)

The language floor under the binary codec (transport-rpc.md §6) and the WebSocket
frame codec: hex literals, bitwise/shift operators, and a `Bytes` type. Agreed
2026-07-02 as the first prerequisites of the codec slice.

## 0. A found miscompile that gates everything: flat JS emission

Probing for this feature found a **pre-existing codegen bug**: the JS printer
emits `Binary` operands flat, with no parenthesization —

```vilan
print((1 + 2) * 3);   // prints 7 — emitted as `1 + 2 * 3`
print(0 - (a - b));   // a=1, b=2: prints -3 — emitted as `0 - a - b`; correct is 1
```

The parse tree is right; the emission loses grouping. It has survived because no
corpus/example program yet wrote a parenthesized subexpression under a
tighter-binding operator. It gates this proposal absolutely: vilan's bitwise
precedence (Rust-style, below) intentionally differs from JS's (C-style — `&`
binds *looser* than `==` in JS), so correct emission of `(a & b) == c` **requires**
precedence-aware parentheses; there is no by-luck path.

**Fix (root cause, first slice):** the JS printer parenthesizes a `Binary`
operand by JS precedence — left operand when it binds looser than the parent,
right operand when it binds looser **or equal** (safe for non-associative `-`,
`/`, and for `+`'s string/number mixing; the redundant parens this adds to
associative chains are a readability cost only). Pinned by compile-and-run
tests; any corpus golden this changes was a latent miscompile and is inspected
by hand before regeneration.

## 1. Hex literals

`0x` followed by one or more `[0-9a-fA-F]` digits, with the usual optional type
suffix: `0xFF`, `0x80000000u32`, `0xDEADn` (BigInt). A hex literal is an integer
literal in another spelling — same typing rules as decimal (suffix, else
context, else `i32`), emitted to JS verbatim (JS hex is identical). Note
`0xFFf` is all hex digits (`f` is a digit, not the `f64` suffix) — a fractional
hex type must be written decimal; and today's lexer mis-reads `0xFF` as `0`
with suffix `xFF`, so the hex rule precedes the decimal rule.

## 2. Bitwise and shift operators

Five new binary operators on integers, each overloadable through a
`std::operators` trait exactly like `+`/`Add`:

| op   | trait / method        | JS emission (`i32`)      | JS emission (`u32`)         |
| ---- | --------------------- | ------------------------ | --------------------------- |
| `&`  | `BitAnd` / `bit_and`  | `a & b`                  | `(a & b) >>> 0`             |
| `\|` | `BitOr` / `bit_or`    | `a \| b`                 | `(a \| b) >>> 0`            |
| `^`  | `BitXor` / `bit_xor`  | `a ^ b`                  | `(a ^ b) >>> 0`             |
| `<<` | `Shl` / `shl`         | `a << b`                 | `(a << b) >>> 0`            |
| `>>` | `Shr` / `shr`         | `a >> b` (arithmetic)    | `a >>> b` (logical)         |

- **Semantics**: operands and result are the same integer type, taken from the
  left operand like the other arithmetic ops. `i32` gets JS's native ToInt32
  behavior (which *is* two's-complement i32). `u32` results are normalized with
  `>>> 0` because JS bitwise is signed — without it `0x80000000u32 | 0` comes
  back negative; and `>>` on `u32` is the logical shift `>>>`. `BigInt` (`n`)
  uses the JS operators natively with **no** `>>> 0` (it is arbitrary-precision;
  the wrap would be wrong). Shift counts follow the host (`i32`/`u32`: count
  masked to 5 bits by JS).
- **Precedence** (Rust's, not C's — high to low): unary, `* /`, `+ -`,
  `<< >>`, `&`, `^`, `|`, comparisons, `is`, `&&`, `||`. So `a & b == c` is
  `(a & b) == c` — the useful reading, not C's footgun. All left-associative.
- **Lexing/parsing**: `^` joins the operator charset (previously unused). `&`
  and `|` already lex as operators; the new *infix* layers coexist with prefix
  `&`/`&mut` (views) and `|…|` closures because those occupy operand position,
  not operator position. `<`/`>` are control tokens (generics), so `<<`/`>>`
  are parsed as two **span-adjacent** control tokens in expression position —
  `a << b` is a shift, `a < < b` stays a parse error, and `List<List<T>>`
  (type position) is untouched.
- **v1 exclusions**: no unary `~` (write `x ^ 0xFFFFFFFFu32`; add later if it
  earns its keep), no compound assigns (`&=`, `<<=`, …), no bitwise on `bool`
  (use `&&`/`||`). Misuse on non-integer scalars (`str & str`) is currently as
  unchecked as `str - str` — the native-operand looseness is a pre-existing,
  shared hardening item, not widened here.

## 3. `Bytes`

A byte buffer over the host `Uint8Array` — the codec's output/input and the
future WebSocket frame unit. A new **base-layer** std module `std::bytes`
(`Uint8Array` exists in browser, node, deno, and bun alike):

```vilan
external struct Bytes;

impl Bytes {
	fun alloc(size: i32): Bytes;              // zero-filled
	fun len(self): i32;                        // byteLength
	fun get(self, index: i32): i32;            // 0–255 (reads past end: host 'undefined' — see note)
	fun set(self, index: i32, value: i32);     // stores value & 0xFF (host semantics)
	fun slice(self, from: i32, to: i32): Bytes;
	fun concat(a: Bytes, b: Bytes): Bytes;     // new buffer, a then b
}

fun encode_utf8(text: str): Bytes;             // TextEncoder
fun decode_utf8(bytes: Bytes): str;            // TextDecoder (moves beside fetch's)
```

Growable building (the serializer's need) is a small vilan `BytesBuilder` over
chunked `Bytes` + a final `concat`/copy — library code, no new externs beyond
the above. Fixed `[T; n]` arrays generally stay backlog I2; `Bytes` is the
codec substrate only.

## 4. Tests

- Emission: the §0 parenthesization pins; `(a & b) == c`, `a | b ^ c & d`
  (vilan precedence preserved through JS's different one).
- Value: each op on `i32`; the `u32` high-bit cases (`0x80000000u32 | 0`,
  `0xFFFFFFFFu32 >> 28`, `1u32 << 31`); hex literal values incl. suffixes and
  `0xDEADn`; shift-vs-generics coexistence (`List<List<i32>>` in the same file
  as `a >> b`); `a < < b` stays an error.
- Overload: a struct implementing `BitAnd` dispatches `&` to `bit_and` (and the
  no-impl diagnostic names the trait, mirroring `Add`).
- `Bytes` round-trips: alloc/set/get/len/slice/concat; utf8 encode→decode
  identity; the `& 0xFF` store semantics.
- Formatter round-trip for every new operator and hex spelling.
