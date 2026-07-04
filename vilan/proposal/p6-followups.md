# P6 follow-ups — the post-completion ladder (2026-07-02)

P6 (transport/RPC) shipped end to end: phases 0–6 of `transport-rpc.md` §11, the
codec arc (§6.1/§6.2 — prerequisites, visitor, both codecs, the single-pass
re-plumb), the WebSocket transport (§5), and a solver pass that killed both
known silent miscompiles. This file ranks what remains, with enough context to
re-prioritize later. Agreed order: **1 → 2 → 3**, then 4+5 bundled behind a
benchmark-justified design pass.

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

## 3. `Client::connect` — small, closes a promised loop

§4.2 promised connect-time contract enforcement; apps hand-roll it (the todo
client's connect → attach → subscribe dance, plus an optional `verify()`). A
generated `Client::connect(url, codec)` would open the duplex, verify the
contract hash (refusing cleanly on drift — Q6's actual *enforcement*), run the
attach handshake, and wire the `RemoteSource` mirrors. Mostly generation work
over machinery that now exists; the todo app's wiring collapses to two lines.

## 4. Trait-shaped Wire visitor — unblocked, wait for a number

The generic-trait-method miscompile that forced §6.1's closure-record pivot is
fixed, so `describe<S: Serializer>` monomorphizing to zero cost is expressible.
Hold because: (a) the win is unmeasured — codec work is nanoseconds against the
~1.2 ms network tax; it only matters for hot local encode loops; (b) a real
design wrinkle: the runtime's codec-as-a-VALUE (chosen at wiring time)
fundamentally needs erasure — a `Codec` record cannot return a trait-typed
serializer without trait objects. Traits likely become an *additional*
zero-cost path for direct `encode_json<T>`-style calls while the RPC seam
keeps the records. Needs its own §6.1 amendment when taken.

## 5. Reactive protocol on codecs — pairs with #1's framing, not standalone

`expose`/`Update` move off JSON-strings onto the codec: binary realtime (half
the bytes, per the payload benchmarks). Requires a binary-capable duplex (WS —
SSE is text-only forever, so `DuplexTransport` must accommodate a text-only
transport gracefully), client-side WS binary events (`binaryType`, typed
`data` access), and the reactive mirrors becoming frame-typed. Touches the
same duplex framing #1 touches — bundle to reshape that seam once.

## Further out (own proposals)

The macro engine (roadmap #9 — would eventually subsume the derive/service
generators), WASM/native backends (backlog F3/F4), Map/Set struct keys (I1),
`?`/try (Q10, which unblocks `arg -> Result` ergonomics and guarded JSON
parse), per-platform library body type-check.
