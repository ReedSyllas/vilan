# Full-stack example

A Node server and a browser client, **both written in Vilan**, that share a
package. The project is a **multi-package workspace** — its root `vilan.toml`
lists three members, each with its own `vilan.toml` and target:

```toml
[project]
packages = ["common", "client", "server"]
```

- `common/` — `[package] target = "none"`, a pure library (core std only). Both
  apps `import common::greeting`; as a `none` package it compiles into either
  bundle, and the platform gate rejects it if it ever reaches for a Node- or
  browser-only module.
- `server/` — `[package] target = "node"`, depending on `common` via a `path`
  dependency. It reads the compiled client bundle once at startup and serves the
  HTML shell on every path, the bundle at `/client.js`, and a small API at
  `/api/hello` (whose body uses `common::greeting`).
- `client/` — `[package] target = "browser"`, also depending on `common`. It
  mounts into the server's `<div id="app">` and has a button that `fetch`es
  `/api/hello` and shows the reply — a live client→server round-trip.

## Run

```sh
vilan run .
```

This builds `dist/client.js` (browser) and `dist/server.js` (Node) — the
workspace's single `node` member, the server, is then started. Open
<http://localhost:3000> — the page loads the client bundle, which renders a
heading using the same `common::greeting` the server logs at startup.

Or build the bundles without running:

```sh
vilan build .          # writes dist/server.js + dist/client.js
node dist/server.js    # then run the server yourself
```

`dist/` is generated and not checked in.
