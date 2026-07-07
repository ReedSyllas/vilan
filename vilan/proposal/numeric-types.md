# Sized numeric types (backlog F2, roadmap #15)

Status: **SHIPPED 2026-07-07.** Adds the sized
integer family `i8`/`u8`/`i16`/`u16`/`i64`/`u64` and `f32` beside today's
`i32`/`u32`/`f64`/`BigInt`, plus two semantic repairs the slice surfaced:
truncating integer division and range-checked integer literals.

## 1. The decision that shapes everything: collapse, don't emulate

On the JS backend every sized integer type lowers to a **plain JS number**,
and `f32` lowers to `f64`. Widths are **nominal**: distinct types with
distinct literals that do not mix (the existing no-implicit-conversion rule),
whose arithmetic is the native JS operator. No per-operation masking, no
wrapping — matching the shipped `i32`/`u32` precedent (`u32` masks only its
bitwise/shift forms, where JS's signed operators would otherwise be wrong).
Width semantics become *real* (wrapping, exact 64-bit) on a future non-JS
backend (backlog F3/F4), where the same nominal types gain real
representations; JS-target programs get the API expressiveness (typed wire
shapes, byte-flavored signatures) at zero per-op cost.

### i64/u64 are f64-backed, not BigInt-backed — measured

The alternative lowering for the 64-bit types was JS `BigInt`. Profiled
2026-07-07 (node v24, best-of-5, 20M-iteration loops shaped like the
compiler's actual output — plain locals, plain operators; memory via
1M-element plain arrays under `--expose-gc`):

| workload | f64 + `Math.trunc` | BigInt | float advantage |
|---|---|---|---|
| add/mul/mod mix | 61.1 ms | 317.9 ms | **5.2×** |
| division-heavy | 23.2 ms | 328.3 ms | **14.1×** |
| adds at 2^40 magnitude | 9.4 ms | 59.2 ms | **6.3×** |
| 1M-element array | 8.0 MB | 32.0 MB | **4×** |

(V8 unboxes number arrays into SMI/double backing stores; every BigInt is a
heap cell behind a pointer. BigInt division allocates per operation —
`Math.trunc` is free.) This confirms the earlier in-house finding that float
is the better carrier. Consequences, stated honestly:

- `i64`/`u64` values are **exact within ±2^53** (`f64`'s integer window) and
  nominal beyond it. Their literals are range-checked to that window (§3) so
  a value you *write* is never silently corrupted.
- **`BigInt` remains the arbitrary-precision escape hatch** — already in the
  language with `n` literals and native truncating division — for arithmetic
  that must be exact past 2^53.

## 2. Truncating integer division (a semantic repair)

Shipped behavior at the start of this slice: `7 / 2` on `i32` evaluated to
`3.5` — an `i32`-typed expression holding a fraction, a type lie inherited
from lowering `/` to JS's float division for every numeric type.

**New rule: `/` on any integer primitive truncates toward zero** (`Math.trunc`
on JS; the Rust/C convention). `f32`/`f64` keep float division; `BigInt`
keeps its native (already truncating) division. (Vilan has no `%` operator —
see §5's discovery note; when one lands it will be exact under this rule.)

Mechanics: the analyzer records the verdict per division node — concrete
integer operands directly, generic operands by their constraint, resolved at
each monomorphization (the `bitwise_u32`/`bitwise_generic_lhs` channel's
shape). The `impl iN with Div` bodies in `number.vl` are concrete, so generic
`T: Div` dispatch inherits truncation with no extra machinery. Compound
`a /= b` follows the same verdict. This **changes observable behavior** of
existing programs that divided integers; every affected corpus golden is
regenerated only after run-verifying the new output.

## 3. Literals and their ranges

Suffix forms, alongside the existing `u32`/`f`/`f64`/`n`:

```
5i8   200u8   5i16   60000u16   5i64   5u64   2.5f32
0xFFu8   0x7Fi8                     // hex rides the existing hex+suffix lexer
```

Unsuffixed integer literals keep their default (`i32`; `f64` in float
contexts). **Integer literals are range-checked at compile time** by their
resolved type — new for `i32`/`u32` too:

| type | accepted literal range |
|---|---|
| `u8` / `u16` / `u32` | `0 ..= 2^n - 1` |
| `i8` / `i16` / `i32` | `0 ..= 2^(n-1)` |
| `i64` | `0 ..= 2^53` |
| `u64` | `0 ..= 2^53` |

The signed bound admits `2^(n-1)` itself (e.g. `128i8`) because the minimum
value is written as unary minus over a literal (`-128i8`) and the literal is
checked before the minus applies — a documented looseness in place of
context-sensitive checking. The 64-bit bound is `f64`'s exact-integer window
(§1); the error for a larger literal names `BigInt` as the escape hatch.
Floats are never range-checked.

## 4. Conversions: explicit, value-converting `as_*`

No implicit numeric conversions (unchanged). Every numeric primitive gains
the explicit family

```
.as_i8()  .as_u8()  .as_i16()  .as_u16()  .as_i32()  .as_u32()
.as_i64() .as_u64() .as_f32()  .as_f64()
```

with **Rust-`as` semantics**: truncate any fraction toward zero, then fold
into the target's range (two's-complement wrap for the sized integers; `u32`
folds by modulo 2^32, `i32` by the same fold at width 32). So
`(300i32).as_u8() == 44`, `(-1i32).as_u8() == 255`, `(3.9f64).as_i32() == 3`.
These are ordinary vilan functions — pure arithmetic over a truncating
quotient and the new `f64.trunc()` (`[extern("Math.trunc")]`, joining
`floor`/`ceil`/`round`) — so they cost one call, not per-op overhead, and
need no compiler support.
`BigInt` gets `.as_f64()` (host `Number()`); numeric→`BigInt` conversion is
deferred until something needs it (recorded).

## 5. The std surface — stamped by the macro engine

Each new type mirrors `i32`'s core trait surface: `Default`, `PartialEq`,
`Add`/`Sub`/`Mul`/`Div` (bodies are the native operator, reached only through
generic dispatch — the shipped pattern), plus `Debug` and `Json` where `i32`
has them (`JSON.stringify` externs; `f64`-backed 64-bit values serialize as
numbers). The bitwise/shift traits stay `i32`/`u32`-only (the hex/bitwise
slice's deliberate line). `Wire` slots for the new types are a protocol
change and are deferred (recorded).

The per-type impl families and the `as_*` ladder were **generated once by a
macro and are checked in as plain source** — not expanded at build time,
because the slice surfaced an engine constraint worth recording:
`number.vl` is loaded inside every macro WORLD (through std's lib surface),
and a world expands with an EMPTY macro scope (macro-engine.md §10's
recursion guard) — so a world-loaded std file must never *dispatch* macros.
The flagship `macro numeric_family(..)` pattern is instead realized as a
pinned test (`a_macro_stamps_a_numeric_family` in `inference.rs`), which
stamps an operator family through the builders and drives it through generic
dispatch. Everything appends after `number.vl`'s hand-written items, so
programs that never touch a new type emit byte-identical JS (unreachable
functions are never emitted).

One more discovery recorded here: **vilan has no `%` operator** — the
conversion folds are spelled with a truncating quotient
(`x - (x / m).trunc() * m`). A remainder operator is a natural follow-up
(backlog H), and with §2's rule it would be exact for every integer type.

## 6. Compiler surface (small, by design)

- `number.vl` declares the seven as `external struct`; the analyzer's
  primitive registration, native-operator whitelist, scalar-primitive lists,
  and literal-suffix map extend to them (the `i32` pattern throughout).
- The literal range check runs post-solve over the RECORDED literal types
  (suffix, seeded annotation expectation, or the solver's map); a literal
  typed only through a constraint (a struct-initializer field, a call
  argument) is skipped rather than re-inferred out of context — never a
  false positive; threading those expectations into the record is the
  follow-up.
- The truncating-division verdict + `Math.trunc` emission (§2); the
  interpreter gains the `Math.trunc` native (one arm).
- Everything else — impls, conversions — is vilan source.

## 7. Out of scope (recorded, with triggers)

- **Wrapping/checked arithmetic on JS** — take with a non-JS backend (F3/F4),
  where widths are real; a JS `wrapping_add` family would cost every op.
- **`f32` rounding** (`Math.fround` per op) — nominal `f32` suffices for API
  shape; fround if a wire/interop consumer needs bit-exact `f32`.
- **`Wire` 64-bit/narrow slots** — protocol change; take with the next codec
  revision.
- **Numeric→`BigInt` conversions, `parse_i8`-style parse family** — with
  their first consumer.
- **`vilan/outdated/` pruned** with this slice (F2's note): every sketch
  there predates shipped features (numbers.vl → this proposal; context,
  signals, lifetime, http-server, ui-framework → shipped subsystems).
