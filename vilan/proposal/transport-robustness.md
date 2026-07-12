# Transport robustness — reconnect, backoff, re-subscription (K6)

Status: **SHIPPED 2026-07-11** (same day). Findings while landing it:
B21 (a dependency-`[service]` consumer unit without its own `std::rpc`
import mistypes the generated `connect` — scope/order-sensitive expansion
resolution, pinned `#[ignore]`d with a minimal repro; the conversion helper
`dial_for_service` stays in std regardless, where it types once); on node a
COMPLETED `main` exits the process (`process.exit` codegen), so a
reconnecting client must hold `main` open; within one drain, a mirror's
subscriber can observe the state flip before the pending rejection lands
(recorded turn semantics — the e2e asserts that order). The Kolt
gap (`kolt-migration.md` §2.6): "Railway parity for `SocketTransport`:
reconnect with backoff, request retry policy, and mirror RE-SUBSCRIPTION on
reconnect." Railway (`kolt/client/src/common/connection.ts`) is the parity
reference; where Railway has a gap we fix it rather than reproduce it.

## 1. What Railway actually does (the parity baseline)

- **Retry**: linear — `DefaultRetryDelay = 1000ms`, `DefaultRetryAttempts =
  10`; the attempt budget RESETS on a successful open; `reconnect()` is
  re-entrancy-guarded (one scheduled retry at a time). After exhaustion it
  stays down.
- **State**: `isReady` — a reactive signal the UI binds ("reconnecting…").
- **Calls while down**: fail fast (`throw` before send).
- **Pending calls at drop**: a gap — the `#requests` map is never flushed, so
  in-flight promises DANGLE forever. We fix this (reject with a typed error)
  instead of matching it.
- **Mirrors**: Railway's observable half is commented out; vilan's
  `RemoteSource` mirrors are ahead — re-subscription is new ground, guided by
  the protocol we already have.

## 2. The model

### 2.1 Connection state is a signal

```vilan
enum ConnectionState { Connected, Reconnecting, Closed }
```

`SocketDuplex` carries `state: Signal<ConnectionState>`; `SocketTransport`
exposes it (`connection_state()`), so a client binds it straight into the UI
(`show`/`bind_text` on "reconnecting…") — the Railway `isReady` shape, typed.

### 2.2 The duplex survives; the socket inside it is replaced

Everything downstream of `SocketDuplex` — `bridge`, `ReactiveClient`,
`SocketTransport`, the generated client — holds the SAME duplex across
reconnects. The `WebSocket` (and the server-announced connection id) become
`Shared` cells the reconnect loop swaps:

- `socket: Shared<WebSocket>`, `connection: Shared<i32>`, plus `url` (to
  redial), `state`, and `on_reconnect` hooks.
- On `close`/`error`: pending calls REJECT (typed, see 2.4), `state` becomes
  `Reconnecting`, and the loop dials with backoff: fresh socket → the same
  `__conn:<id>` handshake → swap the cells → run the re-attach hooks → then
  `state = Connected`. After the attempt budget: `state = Closed`, stays down
  (Railway parity).
- Hooks are stored plain (`List<|| void>` — an `async` closure TYPE lives
  only at parameter/`let` seams, J2 v1) and re-marked `async || void` at a
  `let` when run, so each call awaits: hooks complete in order. The state
  flips to `Connected` FIRST — the hooks' own rpc calls (`__attach`) need a
  usable transport — so the state signal leads the mirror resync by a beat.

### 2.3 Backoff

Doubling from 250ms, capped at 4s, 10 attempts, budget reset on success:
250, 500, 1000, 2000, 4000, 4000… Better than Railway's linear second
(fast first retry for the blip case, capped pressure for the outage case),
same attempt budget and reset rule. Sleeps ride `std::time::sleep_for`
(K5's shape). Not configurable in v1 — a knob belongs on the generated
`connect` once real use asks for it. Jitter: recorded beyond-v1 (matters at
fleet scale, not for Kolt's single server).

### 2.4 Request policy: fail fast, reject pending — never blind-retry

- **Transport failure is representable**: `Transport.call` returns
  `Promise<Result<Frame, str>>` (three impls, all in rpc.vl); `call<T>` maps
  `Err(reason)` to `RpcError::Transport(reason)`. No sentinel frames.
- **Pending at drop**: every in-flight call completes with
  `Err("connection lost")` → the caller sees `Err(RpcError::Transport(..))`.
  (The Railway dangle, fixed.)
- **New calls while not Connected**: immediate
  `Err(RpcError::Transport("not connected"))` — Railway's fail-fast, typed.
- **No automatic retry of REQUESTS**: a dropped call may have executed
  server-side (`create_workspace` is not idempotent) — blind retry is a
  correctness bug. The app owns retry; the typed error and the state signal
  give it everything it needs. (An opt-in idempotent-retry annotation is
  recorded beyond-v1.)

### 2.5 Mirror re-subscription

A reconnect lands on a NEW server connection: new connection id, and
`__attach` mints NEW channel ids. The `__attach` reply is ordered by the
service's exposed-field order, so rebinding is positional:

- `RemoteSource` gains shared cells — `channel: Shared<i32>`, `subscribe:
  Shared<Frame>` — and a `wanted: Shared<bool>` set by `sub`. The
  `ReactiveClient` route holds the SAME channel cell, so one `rebind(channel,
  subscribe_frame)` moves both; a wanted source re-sends `Subscribe`, and the
  server's immediate current-value `Update` resyncs the mirror (the same
  mechanism that seeds a fresh subscribe today).
- The generated `connect` registers ONE reconnect hook: re-run the
  `__contract` check (the server may have REDEPLOYED — on drift the
  connection closes for good rather than desyncing), re-call `__attach` with
  the new connection id, `rebind` each mirror positionally.

### 2.6 Initial connect is robust too

`connect_socket` returns `Result<SocketDuplex, str>` and applies the same
dial-with-backoff to the FIRST connection (today a dead server hangs the
handshake forever). The generated `connect` propagates it (`!`).

## 3. Server side: nothing

`serve_connected` already tears down per-connection state on close
(`on_disconnect`), and a reconnect is just a new connection that attaches
fresh channels. The mirrors' resync is the ordinary subscribe path.

## 4. Beyond v1 (recorded)

- Backoff jitter; configurable schedule/budget on the generated `connect`.
- Opt-in idempotent request retry (an `[rpc(retry)]`-style annotation).
- `SplitDuplex` (SSE) reconnect — the WebSocket transport is the production
  path today; the SSE fallback keeps fail-fast semantics.
- Half-open detection (heartbeat/ping) — a silent-death socket only surfaces
  on the next send today.
