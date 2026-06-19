# Full-stack example

A Node server and a browser client, **both written in Vilan**, that share a
module. `vilan.toml` declares the two entries:

```toml
[server]
entry = "server.vl"

[client]
entry = "client.vl"
```

- `server.vl` — a Node program (`std::http` + `std::fs`). It reads the compiled
  client bundle once at startup and serves the HTML shell on every path, the
  bundle at `/client.js`, and a small JSON-less API at `/api/hello`.
- `client.vl` — a browser program (`std::dom` + `std::fetch`). It mounts into the
  server's `<div id="app">` and has a button that `fetch`es `/api/hello` and shows
  the reply — a live client→server round-trip.
- `shared.vl` — `pkg::shared`, imported by **both** entries. It uses only core
  std, so it compiles for `--target node` and `--target browser` alike; the
  platform gate rejects it if it ever reaches for a Node- or browser-only module.

## Run

```sh
vilan run .
```

This builds `dist/client.js` (browser) and `dist/server.js` (Node), then starts
the server. Open <http://localhost:3000> — the page loads the client bundle,
which renders a heading using the same `greeting` the server logs at startup.

Or build the bundles without running:

```sh
vilan build .          # writes dist/server.js + dist/client.js
node dist/server.js    # then run the server yourself
```

`dist/` is generated and not checked in.
