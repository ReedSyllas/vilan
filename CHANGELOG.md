# Changelog

vilan is a fast-moving alpha. Minor versions (`0.X`) may change the
language, the standard library, and the wire protocol without a
deprecation period; patch versions are fixes. Each release below links
the highlights — the [book](https://reedsyllas.github.io/vilan/) always
tracks the latest state.

## v0.3.0 — 2026-07-13

**The toolchain updates itself.** `vilan upgrade` finds the newest
release, verifies its checksum, proves the downloaded binary runs, and
swaps `vilan` and `vilan-lsp` in place; `vilan upgrade --check` only
reports. This is the CLI's one network touchpoint, and it runs only
when you ask. (v0.2.0 installs predate the command — re-run the install
script once to pick it up; it updates in place.)

**Rpc handlers can await.** An `[rpc]` method body can now call
`sleep_for`, another service, or any async API. The reply is sent when
the body finishes, and the wire turn holds across the awaits — signal
writes before and after a suspension still reach every client as one
coalesced update beside the reply.

Also fixed and improved:

- No-argument `[rpc]` methods previously ran outside the wire turn, so
  each of their signal writes was broadcast as its own update. They now
  batch exactly like argument-taking methods.
- The VS Code extension finds the language server in `~/.vilan/bin`, so
  a `vilan upgrade` reaches the editor with no extra step.

## v0.2.0 — 2026-07-13

The first public release.

**The toolchain is self-contained.** The `vilan` binary carries the
standard library inside it and materializes it on first use — download
one file (plus `vilan-lsp` beside it) and `vilan run hello.vl` works
from any directory, with no checkout and no configuration.
`vilan --version` reports the exact build.

**What's in the box:**

- The language: value semantics (assignment copies), no `null` and no
  exceptions (`Option`/`Result` with `!` and `?.`), implicit `await`,
  second-class views with compile-time invalidation checks, generics,
  traits, enums with payloads, pattern matching, and a macro system.
- `std`: collections, strings, sized numerics (`i8`–`u53`, `f32`/`f64`),
  json, time, random, crypto/jwt/base64, fetch, fs/http/process (node),
  dom/storage (browser) — platform-layered, checked at compile time.
- Fine-grained reactive UI (`std::reactive`, `std::ui`): signals bind to
  individual DOM properties; no virtual DOM; automatic cleanup; a typed
  enum-based router; compile-time styling.
- The service layer: one struct is the client/server contract —
  `[expose]`d signals mirror live to every client, `[rpc]` methods are
  typed calls, the wire contract is hashed and checked at connect, and
  reconnects resync automatically.
- The tools: `vilan build / check / run / fmt / test` (all with
  `--watch`), a language server (diagnostics, hover, go-to-definition,
  references, rename — into `std` too), and a VS Code extension,
  prebuilt as a `.vsix` on every release.
- The book: a JS/TS-developer-first guide from
  [Coming from JavaScript](https://reedsyllas.github.io/vilan/tour/coming-from-javascript.html)
  through a full-stack walkthrough app, plus a language spec — every
  example compiled by CI.

Install:

```sh
curl -fsSL https://github.com/ReedSyllas/vilan/releases/latest/download/install.sh | sh
```
