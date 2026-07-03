# benchmarks — measuring the transport and batching claims

The phase-6 validation suite (`proposal/transport-rpc.md` §11): the RPC
library's claims as numbers instead of assertions.

```
vilan run vilan/benchmarks
```

Four sections:

- **Payload sizes** — the same values as bare JSON, as binary-codec frames
  (§6.2), and inside today's RPC envelopes. This makes the JSON
  **double-encoding** visible (payloads are JSON-escaped inside JSON — the §6
  status cost) and brackets the binary codec's win before the runtime rides it.
- **Coalescing** — update frames counted at the wire over an in-process duplex:
  lone sets emit one frame each; a `batch` of 100 emits **one**; an RPC
  handler's writes coalesce into one frame per subscribed source alongside the
  reply (the wire turn). These counts are exact invariants — CI pins them
  (`crates/vilan-cli/tests/benchmarks.rs`).
- **RPC round-trip throughput** — sequential (each call awaited: honest
  latency, no pipelining) over `LocalTransport` and over real HTTP on
  localhost via `serve_rpc`.
- **Realtime fan-out** — three live SplitDuplex sessions (real SSE + POST)
  subscribed to one signal through the generated `[service(Client)]` stub; 50
  mutations from a driver; reports per-session update-frame counts
  (deterministic: subscribe + 1 per mutation) and the settle time.

Timings are machine-dependent — the harness and the frame-count invariants are
what CI asserts. Re-run after the §6.2 runtime re-plumb to compare the JSON
baseline against binary frames end-to-end.
