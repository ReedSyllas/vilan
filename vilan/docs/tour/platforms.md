# Platforms

One language, several runtimes: a package builds for **node** (default),
**deno**, **bun**, or the **browser** (`target` in `vilan.toml`, or
`--platform` on the CLI). The std library is layered so each build sees only
what its platform can actually do — a mismatch is a compile error at the
`import`, not a runtime surprise.

## The std layers

- **Base** — platform-neutral, always available: collections, option/result,
  strings, numbers, `reactive`, `shared`, `time`, `json`/`wire`/`binary`,
  `rpc` (the client machinery), `style`, `fetch`, `crypto`, …
- **Browser layer** — `std::dom`, `std::ui`, `std::router`, `std::storage`.
  Importing these in a node build: compile error.
- **Process layer** (node/deno/bun) — `std::db`, `std::http`, `std::fs`,
  `std::process`, `std::rpc_server`. Importing these in a browser build:
  compile error.

The error points at your import and names the platform, e.g. building
`import std::ui` for node fails with a cross-target diagnostic.

## Full-stack packages

A client/server app is a `[project]` workspace: a shared `[library]` package
holding the `[service]` struct and payload types (base-layer code only), a
browser `[package]` for the client, a node `[package]` for the server. Each
package compiles for its own platform against the same shared library — the
compiler checks that the shared code stays platform-neutral.

```
app/
  vilan.toml       [project] packages = ["common", "client", "server"]
  common/          [library] — service + types (base layer only)
  client/          [package] target = "browser"
  server/          [package] (node)
```

`vilan build .` at the root builds every package (the client bundle typically
lands where the server can serve it).

## Libraries can be layered too

A `[library]` may declare platform overlays — a base root plus per-platform
roots (`[library.layer]` entries with `platform = […]`) — so a dependency
can ship a browser implementation and a node implementation of the same
module behind one import. Most libraries don't need this; std itself is the
main user.

## Externs — talking to the host

Std's own platform bindings are ordinary vilan declarations, and apps can
write them too:

```vilan,fragment
// A host global / module import (node:crypto), bound as a function:
[extern("node:crypto", "randomBytes")]
external fun random_bytes_sync(length: i32): HashBuffer;

// An opaque host object with methods/properties:
external struct HashBuffer;
impl HashBuffer {
	[extern(method, "toString")]
	external fun to_string_encoded(self, encoding: str): str;
}

// An async host function — promise-returning; callers implicitly await:
[extern("node:timers/promises", "setTimeout")]
async external fun sleep(ms: i32): void;
```

Extern forms: `[extern("module", "name")]` (a module import),
`[extern("global.path")]` (a dotted global like `history.pushState`),
`[extern(method, "name")]` / `[extern(get, "prop")]` / `[extern(set, "prop")]`
on impl members. Externs are per-platform by nature — keep them in
platform-specific packages (or std's layers), and prefer promoting a proven
binding into std over copying it between apps.

## Assets

`std::asset::emit(kind, content)` writes a build asset from `const` code —
it's how `style()` emits CSS. Browser builds produce `<entry>.js` (+
`<entry>.css` when styles emitted); serve them plus an HTML shell from your
server (the [services guide](../guide/services.md) shows the fallback shape).
