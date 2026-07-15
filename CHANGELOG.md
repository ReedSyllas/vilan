# Changelog

vilan is a fast-moving alpha. Minor versions (`0.X`) may change the
language, the standard library, and the wire protocol without a
deprecation period; patch versions are fixes. Each release below links
the highlights — the [book](https://reedsyllas.github.io/vilan/) always
tracks the latest state.

## v0.6.1 — 2026-07-15

**`&mut bool` write-through, fixed.** A writable view of a boolean *local* —
`let v = &mut flag`, or passing `&mut flag` to a function — silently did
nothing; the write never reached the original. v0.6.0 introduced `&mut bool`
views but boxed only number and string locals, so a boolean's backing cell
was missing and the write landed nowhere. Views of boolean *list elements*
and *struct fields* were already correct; this fixes the bare-local case,
the `v = !*v` toggle included.

## v0.6.0 — 2026-07-15

**Map and Set key by value.** A struct, enum, or `List` works as a key
once it derives `Hashable` (`[derive(Hashable)]`) — two equal values are
the same key, and a freshly-built equal key finds the entry a stored one
made. Scalar keys (`i32`, `str`) still work directly. Hand-write
`impl Hashable` to key by a subset of fields, or to build your own
hash-keyed structure on the `Hash` value the trait returns.

**Decoding validates.** A generated `from_json` returns
`Result<Self, str>` and checks the shape of what it is handed — a missing
field, a wrong JSON type, an absent required value — and returns an `Err`
with a reason instead of a struct half-built from garbage. Round-tripping
your own types across the wire or through a file is safe by construction.

**A view crosses to a value explicitly.** Reading a scalar view's value
requires the `*`. `print(v)` for a `&mut i32` used to leak the view's
internal `(base, key)` representation; it now tells you to write `*v`. The
language never silently converts a view to a value — storing one where it
would escape was already an error, and this closes the read half.

**`Option<&mut T>`, built inline.** `match Some(&mut a) { Some(let v) => … }`
constructs a mutable-view option on the spot and writes through it — the
direct form, the conditional `match if c { Some(&mut x) } else { None }`,
and forwarding a `&mut` parameter. It is a transient, so it may view a
local: it never outlives the `match`. Bind it to a `let` and it escapes,
rejected as before.

**`&mut bool` writes through — and toggles.** A writable view of a boolean
now lowers like any other scalar view, so `v = true` reaches the original;
and toggling reads naturally, `v = !*v`. (The toggle also needed a lexer
fix: adjacent prefix operators like `!*`, `!!`, and `-*` were fusing into
one bogus token and failing to parse — a space was the only workaround.)

## v0.5.1 — 2026-07-14

**A type name isn't a value.** `let q = Point;` used to compile, quietly
binding the constructor object; now it's an error that points you at the
fix — construct the type, name a variant, or call a static. This also
closes a trap the v0.5.0 grammar could spring: `if p == Point { … } { … }`
(a struct-literal comparison a user meant, written without parentheses)
parsed `p == Point` against the type object and ran. It now reports
`` `Point` is a type, not a value `` at the name instead of misbehaving at
runtime. Traits, type parameters, and module names get the same check.

## v0.5.0 — 2026-07-14

**Your types order themselves.** `<` `<=` `>` `>=` now dispatch through
`PartialOrd` — implement (or derive) `partial_compare` and the
operators just work, `started < deadline` on instants included. v0.4.0
steered you to calling `lt` by hand; that detour is over.

**Platform checking follows the instantiation.** A generic function is
checked with the types each call actually binds — `save(disk_store)` in
the server entry charges `std::fs` there and only there, while
`save(memory_store)` in the browser entry stays clean. Before, one
colored instantiation could taint every use of the generic.

**Boundaries you can declare: `[platform("browser")]`.** Inference
still colors everything; a fence turns intent into a checked promise —
verified on every compile, for every host the pattern names, libraries
included. Reach outside it and the error renders the chain from the
fenced function.

**Struct literals are operands.** `Point { x = 1, y = 2 } == p`
compares and `Rect { .. }.area()` chains — no more binding to a local
first. Conditions keep the brace for the block (`if Foo { … }` stays a
condition and a block), so a literal in a condition is parenthesized:
`if p == (Point { x = 1 }) { … }`.

**A local module may share a std name.** `pkg::ui` is always your
`ui.vl`; `std::ui` is always std's. Resolution is scoped by the import
root, so naming a module `ui`, `json`, or `io` no longer collides with
— or silently loses to — the standard library. (`pkg::` also no longer
accidentally aliases std modules you never wrote.)

**Hover tells the whole story.** The editor now renders the full
declaration — signature with parameter names, generics with their
bounds, struct and enum bodies, an `async` prefix when inference adds
one — plus the `//` doc block above the item, its `[platform]` fence,
and the inferred platform requirement with its via-chain.

Also fixed and improved:

- Impl binders: a `type T` binder impl declared before the subject's
  other impls no longer misresolves, and binders in trait-argument
  position (`impl X with Wire<type F>`) register and dispatch.

## v0.4.0 — 2026-07-14

**Platform checking moved from imports to reach.** A build may import
any module; what's checked is what your entry can actually *run into*.
Every function — and now every module-level `let` — carries an inferred
platform requirement, and a browser build that reaches `std::fs` fails
with the whole call chain (`main → boot → load → exists (std::fs)`),
anchored at your call site. Since imports stopped being the boundary, a
service can live next to its resources — the database, the filesystem —
and the client imports the generated stub from that very module; the
injected-closure ceremony is gone. The editor shows all of it live:
violations as you type, and hover tells you what a function requires
and via which path it got there.

**One package, many entries.** A client + server app no longer needs
three packages. Declare two entries in one `[package]` —

```toml
[entry.client]
target = "browser"

[entry.server]
```

— and `vilan build` compiles each for its own target into
`dist/<name>.js` (browser bundles first, so a serving entry finds them
fresh), `vilan run` starts the node entry, and `vilan check` checks
them all. Packages can also depend on each other by path, so the
multi-package shape still scales when you want it. The legacy
`[server]`/`[client]` manifest form is retired; the error names the
replacement. The docs walkthrough app is rewritten in the
single-package shape — its service holds its database directly.

**Module initializers are honest.** A top-level `let` runs iff
something reachable references it — the same rule emission uses — so a
dropped binding's callees (and their `import … from "node:…"` lines,
which previously leaked into every browser bundle and broke it at
module parse) never emit. And an initializer that calls an async
function is now a clean compile error instead of a value that is
secretly a promise.

**Comparisons type-check.** `true < 3`, `1 == "a"`, and mixed-width
typed operands used to compile into coercing JS comparisons; they are
errors now. A bare integer literal still adapts to its peer
(`stamp < 1000` stays fine on an `i53`). Ordering a user-defined type
errors honestly — `PartialOrd`'s operator dispatch isn't wired yet, so
the compiler steers you to its `lt`/`le`/`gt`/`ge` methods rather than
emitting a JS object comparison that is always `false`.

**Tuples have positional access.** `pair.0`, `pair.1`, chains like
`nested.0.1`, and assignment through `mut` bindings — all over the
tuple's flat storage, so a nested write mutates the tuple, never a
copy. Destructuring is no longer the only way in.

Also fixed and improved:

- Iterator-protocol `next()` calls, indexing subjects, destructuring
  subjects, and functions passed as values are now all visible to
  platform checking and async inference — each was a blind spot that
  could hide a platform requirement or an await.
- Two build units writing the same `dist/<name>.js` are rejected at
  build instead of silently overwriting each other.
- `vilan upgrade` prunes stale materialized-std cache directories after
  a successful swap.
- `[macro]` in a manifest no longer warns as an unknown key.
- `std::time`'s documented instant comparison was wrong at runtime
  (`started < deadline` always produced `false`); the docs now use
  `lt` and the compiler rejects the old form.

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
