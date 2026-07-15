# P6 follow-ups — the post-completion ladder (2026-07-02)

P6 (transport/RPC) shipped end to end: phases 0–6 of `transport-rpc.md` §11, the
codec arc (§6.1/§6.2 — prerequisites, visitor, both codecs, the single-pass
re-plumb), the WebSocket transport (§5), and a solver pass that killed both
known silent miscompiles. This file ranks what remains, with enough context to
re-prioritize later. Agreed order: **1 → 2 → 3**, then 4+5 bundled behind a
benchmark-justified design pass.

> **Status (2026-07-03): the ladder is COMPLETE — all five items ✅, and the "Final
> fixes" audit below is fully landed.** What remains beyond this file is proposal-sized
> (see "Further out").

## 1. RPC-over-WebSocket multiplexing — ✅ DONE (2026-07-02, 2d82cc7: 2.5× HTTP)

Today a WS client still makes RPC calls over HTTP POST *beside* the socket
(exactly as beside SSE). Benchmarks: ~1.2 ms per awaited localhost HTTP call,
nearly all round-trip overhead — over the already-open socket a small call is
one frame each way. Also the deployment story (one connection to authenticate/
proxy) and, later, the symmetric pipe permits server→client calls.

Design (settled with this slice — see §5): channel prefixes at the TRANSPORT
seam, not in the envelope — `d:<frame>` for duplex/reactive traffic,
`r:<id>:<frame>` for RPC, correlation id owned by the socket transport. The
§4.1 envelope is untouched (no wire-format change); the multiplex is pure
framing. v1 is text frames (JSON codec); binary-over-WS is bundled with #5
(same event-kind plumbing on both ends). `connect_socket`'s duplex gains a
`transport()` view implementing `Transport` with a pending-call map.

## 2. The two remaining pins with real bite — ✅ DONE (2026-07-02)

- **`bound_dispatch_prefers_the_trait_method_on_a_name_collision`** — an
  inherent method and a trait default sharing a name: a bound call picks the
  *inherent* one. Deterministic but silently wrong-METHOD dispatch — the most
  correctness-relevant item left in the pin ledger.
- **`calling_an_unannotated_closure_parameter_defers`** — `|resolve| {
  resolve(); }` fails where `|resolve: || void|` works; the free-call subject
  deferral doesn't cover unannotated closure params. Bit std twice already
  (the `sleep` and `connect_socket` promise executors). Same deferral family
  as the fixed Bug C′.

Both fixed: the resolved trait is recorded per bound call and emission
dispatches on that trait's surface (override, else default — an inherent name
collision can't shadow it); and a free call whose subject is an unannotated
closure parameter now defers like the method paths always did. The pin ledger
is down to two, neither a bug: **trait-argument binders** (`impl X with
Trait<type S: Bound>`) are a missing *feature* with a clean error — notable as
the alternative route to the trait-shaped visitor (#4); and the **impl-binder
declaration-order** pin has a trivial workaround (reorder declarations).

## 3. `Client::connect` — ✅ DONE (2026-07-02)

§4.2 promised connect-time contract enforcement; apps hand-roll it (the todo
client's connect → attach → subscribe dance, plus an optional `verify()`). A
generated `Client::connect(url, codec)` would open the duplex, verify the
contract hash (refusing cleanly on drift — Q6's actual *enforcement*), run the
attach handshake, and wire the `RemoteSource` mirrors. Mostly generation work
over machinery that now exists; the todo app's wiring collapses to two lines.

## 4. Trait-shaped Wire visitor — ✅ DONE (2026-07-02: direct paths +18%/+14%)

The generic-trait-method miscompile that forced §6.1's closure-record pivot is
fixed, so `describe<S: Serializer>` monomorphizing to zero cost is expressible.
Shipped exactly as predicted: traits `Serialize`/`Deserialize` carry the
visitor; the writers/readers implement them natively; the closure records
remain ONLY as the codec-as-a-value erasure (they now `impl` the traits by
delegation — a struct field and a same-named trait method coexist, probed), so
the RPC seam and `Codec` didn't change at all. Direct entry points
(`encode_json`/`decode_json`, new `encode_binary`/`decode_binary`) skip the
records: measured +18% json / +14% binary on the 25-todo round-trip
(51.3k/29.9k per sec vs the 42.5k/25.6k baseline), with the codec path
unregressed. En route, the own-generic ordered-values channel was extended to
FREE calls (bound statics like `T::rebuild(reader)` monomorphize correctly —
the last piece the trait shape needed) and bound-static dispatch gained the
same inherent-shadowing protection as methods.

## 5. Reactive protocol on codecs — ✅ DONE (2026-07-03: the ladder's last rung)

`expose`/`Update` moved off JSON-strings onto the codec — and further than
planned: the mirrors came out **typed end to end**, not frame-typed.
`DuplexTransport` carries `Frame` (SSE stays text-only: a binary send through
`SplitDuplex` panics loudly at the first Subscribe; WS carries both kinds via
tag bytes — `0x64` duplex, `0x72`+LE-id RPC — which also removed
`SocketTransport`'s binary panic: **binary RPC rides the socket**, pinned by a
real-network test). The runtime lost its JSON mirror signals entirely:
`expose<T: Wire>` stores a per-channel *starter* that subs the typed source on
the first Subscribe and single-passes each value into an `Update` envelope
(nothing retained for unwatched channels — strictly better than the old
always-on mirrors); `source<T: Wire>` returns `RemoteSource<T>`
(`Signal<Option<T>>` cache replacing the `""` sentinel; malformed updates
dropped, sticky-checked per frame). The generated client emits
`RemoteSource<Element>` from each `[expose]`d field's `Signal<Element>` type
(the element is now part of the contract surface/hash) and binds `source<T>`
through annotated lets — app code subscribes to *values*:
`client.todos.sub(|list| …)`. The vestigial `Protocol` trait and the
`ReactiveFrame` derive type are gone; `ReactiveServer/Client::new` take the
codec; `register_session` threads `RpcProtocol`'s, so `serve_service` kept its
signature.

En route, THREE compiler findings (the probe-first rule paying out again):
struct-literal fields do NOT direct a generic call's type parameter (two calls
cross-unify — hence the annotated-let generation); bare `ret;` was not legal
grammar yet `rpc_server.vl` shipped one — which exposed that **package-module
parse errors were silently swallowed** (`load_package_module` discarded
recovery errors: the broken statement compiled to *nothing*). Both fixed at
the root: `ret`'s expression is optional (a void early-return, pinned), and
module lex/parse errors now fail the build naming file+line (workspace test).
The ret-value-vs-return-type check turned out not to exist at all — pinned as
two `#[ignore]`s.

Boundary counts after the slice: `std/src/rpc.vl` **7 → 1** (the multiplex id
parse), every client-side mirror decode gone (todo client, rpc example,
realtime benchmark). What remains in the table is exactly the "small fix" and
"cosmetic" rows below.

## Final fixes — the leftover-JSON audit — ✅ ALL LANDED (2026-07-03)

Audited every `to_json`/`from_json` use in std and the example/benchmark
packages (enforced by `crates/vilan-core/tests/json_boundary.rs` — per-file
counts; a change there must be deliberate and re-sanctioned). End state: the
table holds exactly TWO lines — the WS handshake's header read (`JsonValue`
as the documented dynamic-object accessor) and the throughput benchmark's
deliberate `to_json` (it *measures* the derive path). How each row closed:

- **Id parses → `str.parse_i32`**: `str.parse_i32()` already existed as an
  intrinsic but was parseInt-LIBERAL (`"1.5"` → `Some(1)`, `"12x"` →
  `Some(12)`) and completely unpinned — both parse helpers are now STRICT
  (whole trimmed text, i32 range-checked; corpus `parse-i32.vl` pins 12
  cases) and the root fix landed: **`.connection` is an `i32`**, parsed once
  at the handshake (`parse_announced` — a malformed announcement panics as a
  broken-server error), so the generated `connect`, the benchmark, and every
  test site just pass it through. `route_socket_frame` drops frames with
  unparseable ids; `rpc_server`'s two ids default to `-1` (matches nothing).
- **Error prints → `[derive(Wire, Debug)]`** on `RpcError` — prints render
  `Remote("unknown method: …")` via `.debug()`; the enum Debug derive
  handles payload variants (probed).
- **Persistence → the codec**: the todo server writes/reads `todos.json`
  through `encode`/`decode(json_codec(), …)` — and the decode being
  *validating* surfaced the real bug below.

**Severity find en route**: the JSON codec's reader called `JSON.parse`
unguarded — `decode(json_codec(), Frame::Text("garbage"))` THREW instead of
`Err`, which meant **one malformed `/rpc` body killed the whole server
process** (contradicting wire.vl's documented pre-poisoned-reader contract).
Fixed at the root with a guarded `str.try_parse_json(): Option<JsonValue>`
intrinsic; `decode_json` + the codec reader pre-poison on `None`
(`"malformed JSON"`, sticky). Pinned at both seams
(`malformed_json_frames_fail_sticky_instead_of_crashing`: decode → `Err`,
protocol → `Failure(Decode)`) and verified live (corrupt `todos.json` boots
empty; a garbage POST gets a Failure envelope and the server keeps serving).
Per-type `from_json` stays trusting — backlog I3 (validating `from_json`)
is unchanged.

## Further out (own proposals)

The macro engine (roadmap #9 — would eventually subsume the derive/service
generators), WASM/native backends (backlog F3/F4),
`?`/try (Q10, which unblocks `arg -> Result` ergonomics and guarded JSON
parse), per-platform library body type-check. (Map/Set struct keys — I1 —
shipped 2026-07-14, `proposal/hashable-keys.md`.)
