# Spec §A — Appendix

## A.1 Operator precedence

Tightest to loosest; binary levels are left-associative (§3.7):

| Level | Operators |
|---|---|
| 1 | `::` paths · calls · `.member` · `[index]` · `!` (try) · `?.` |
| 2 | prefix `!` `-` `await` `async` `&` `&mut` `*` |
| 3 | `*` `/` `%` |
| 4 | `+` `-` |
| 5 | `<<` `>>` (span-adjacent) |
| 6 | `&` |
| 7 | `^` |
| 8 | `\|` |
| 9 | `==` `!=` `<` `<=` `>` `>=` |
| 10 | `is` |
| 11 | `&&` |
| 12 | `\|\|` |

Above level 12 sit the expression forms (closures, blocks, `if`,
`match`, `for`, `let`, `ret`, assignment), and above those the
top-tier-only forms: `const expr` and struct initializers (§3.8).

## A.2 Reserved words

```text
async  await  borrows  const else    enum   export  external
for    fun    if       impl  import  in     is      jump
let    macro  match    mod   mut     null   own     ret
struct trait  type     use   with    true   false
```

Contextual (identifier everywhere else): `context`, `void`, `self`,
`Self`, `break`/`continue` (after `jump`), and the attribute names
`derive` `service` `extern` `must_use` `rpc` `trait_only` `doc`
`expose`.

## A.3 Literal suffixes

`i8 i16 i32 i53 u8 u16 u32 u53 f f32 f64 n` — §2.3 (unknown suffixes
error; `i64`/`u64` were renamed to `i53`/`u53`). Unsuffixed: integer
→ `i32`, fractional → `f64`.

## A.4 Lang items

Std declarations the language itself depends on:

| Item | Module | Language use |
|---|---|---|
| primitives (`bool`, `str`, numerics, `BigInt`) | `std::boolean`/`string`/`number` | literal types |
| `List<T>` | `std::list` | list literals, `for` |
| `Option<T>` | `std::option` | `?.` results, view-returning lookups |
| `Try`, `Verdict`, `Lift` | `std::operators` | `!`, `?.` (§5.10) |
| `Add Sub Mul Div Rem Shl Shr BitAnd BitOr BitXor` | `std::operators` | operators (§5.7) |
| `PartialEq`, `PartialOrd` | `std::compare` | `==`/ordering (§5.7) |
| `Iterator`/`Iterable` | `std::iterator` | `for … in` |
| `Task<T>` | `std::task` | `async`/`await` (§7.3) |
| `Promise<T>` | `std::promise` | host-interop promises (§7.3) |
| `Context<T>` | `std::context` | context clauses (§8, Phase B) |
| `panic`, `assert` | `std::io` | divergence, `vilan test` |

## A.5 Phase B (pending chapters)

§8 Contexts · §9 Const evaluation · §10 Macros · §11 The platform model
& manifests. Until they land, the corresponding tour chapters
([macros & const](../tour/macros-and-const.md),
[platforms](../tour/platforms.md)) and `std::context`'s
[reference entry](../std/misc.md) are the best available descriptions.
