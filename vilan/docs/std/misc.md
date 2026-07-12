# Misc — reference

The small modules: `std::io`, `std::promise`, `std::context`,
`std::crypto`, `std::jwt`, `std::asset`.

## std::io

```vilan,fragment
fun print(message: any)                     // console.log
fun panic(message: str)                     // abort with a message
fun assert(condition: bool, message: str)   // panic when false
```

`panic` is for unreachable states (expected failures are `Result`). Note
`panic`'s value types as `Any` — annotate a binding whose match arms mix a
panic with values ([gotchas](../appendix/gotchas.md)). `assert` is the
`vilan test` failure mechanism.

## std::promise

```vilan,fragment
external struct Promise<T>;
impl Promise<type T> {
	fun all(promises: List<Promise<T>>): List<T>   // async; implicitly awaited
}
```

Promises only arise from spawning (`async expr`); see the
[async tour](../tour/async.md). Keep the promise instead of the result by
spawning the `all` itself: `let pending = async Promise::all(promises);`.

## std::context

Ambient values with dynamic extent — the machinery under `owner_scope` and
`turn_scope`:

```vilan,fragment
impl Context<type T> {
	fun new(): Context<T>
	fun run<U>(self, value: T, body: || U): U   // establish for the body's extent
	fun get(self): T                            // read (compile error if possibly absent)
	fun get_safe(self): Option<T>               // read, absence as None
}
```

- `get` is **statically covered**: the compiler proves every call path runs
  inside a `run` — an uncovered read is a compile error, not a runtime
  `None`.
- Closures capture their contexts **at creation**; parameters declare
  context needs with the `context` clause
  ([functions & closures](../tour/functions-and-closures.md)).
- Async-safe by construction: a continuation sees the value captured at
  creation, across awaits and interleaved extents.

Define module-level contexts for app-wide ambients (a session identity on
the server is the canonical use).

## std::crypto

```vilan,fragment
fun random_bytes(length: i32): Bytes        // cryptographically secure
fun random_uuid(): str
fun equals_constant_time(a: Bytes, b: Bytes): bool   // timing-safe compare
```

WebCrypto-backed (async where the host is). For password hashing on the
server, bind the host's sync primitives as externs (the kolt pilot uses
node's `pbkdf2Sync`) — candidates for std promotion.

## std::jwt

HS512 JSON Web Tokens; claims are any `Wire` type:

```vilan,fragment
async fun sign_hs512<C: Wire>(secret: Bytes, claims: C): str
async fun verify_hs512<C: Wire>(secret: Bytes, token: str): Option<C>
fun decode_claims<C: Wire>(segment: str): Option<C>   // decode WITHOUT verifying
```

`verify_hs512` checks the signature (constant-time) before yielding claims;
`decode_claims` is for non-security introspection only.

## std::asset

```vilan,fragment
fun emit(kind: str, line: str)   // compile-time only: append to a build asset
```

Callable only from `const` evaluation — it's how `std::style` writes the
CSS file (`emit("css", rule)`). A browser build with emissions produces
`<entry>.css` beside `<entry>.js`.
