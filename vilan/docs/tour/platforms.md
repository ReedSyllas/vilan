# Platforms

One language, several runtimes. A package builds for **node** (the
default), **deno**, **bun**, or the **browser** — set `target` in
`vilan.toml`, or pass `--platform` on the CLI.

The standard library is layered so each build only uses what its platform
can actually do. Call a server function from code a browser build can
reach, and you get a clear compile error naming the call chain — not a
runtime crash. That's the whole idea of this chapter.

> **Going deeper.** The check is on *reachable code*, not on imports. A
> file may import `std::fs` and compile for the browser, as long as no
> code the browser entry can reach actually calls into it. The compiler
> colors every function with the platforms it can run on (seeded by the
> std layers, flowing through calls — the same way `async` is inferred),
> and checks the colors only along paths that start at your `main`. When
> a path crosses onto the wrong platform, the error shows that path.
> The editor shows the same information as you write: violations appear
> as live diagnostics at the offending call, and hovering a function
> shows its inferred requirement and how it got it — e.g. ``requires the
> `process` layer of `std` (via `save → write_file (std::fs)`)``.

## The std layers

- **Base** — platform-neutral, available everywhere: collections,
  `Option`/`Result`, strings, numbers, `reactive`, `shared`, `time`,
  json/wire/binary, the rpc client machinery, `style`, `fetch`,
  `crypto`, and friends.
- **Browser layer** — `std::dom`, `std::ui`, `std::router`,
  `std::storage`. Browser builds only.
- **Process layer** (node/deno/bun) — `std::db`, `std::http`, `std::fs`,
  `std::process`, `std::rpc_server`. Server builds only.

## Full-stack packages

A client + server app is a workspace of three packages. The shared
`common` library holds the payload types both sides speak (base-layer
std only); the service itself can live in the server package, next to
its resources — the client depends on the server package and imports
the generated client from it (see
[Services](../guide/services.md)):

```
app/
  vilan.toml       [project] packages = ["common", "client", "server"]
  common/          [library] — payload types (base layer only)
  client/          [package] target = "browser"
  server/          [package] (node)
```

`vilan build .` at the root builds every package. The compiler checks
each one against its own platform, including that `common` stays
platform-neutral.

## Externs — talking to the host

You'll mostly consume host bindings through std. But when you need a
node API or browser API that std doesn't wrap yet, you can bind it
yourself with an extern declaration. This is exactly how std's own
bindings are written:

```vilan,fragment
// A function from a host module (node:crypto):
[extern("node:crypto", "randomBytes")]
external fun random_bytes_sync(length: i32): HashBuffer;

// An opaque host object, with methods bound one by one:
external struct HashBuffer;
impl HashBuffer {
	[extern(method, "toString")]
	external fun to_string_encoded(self, encoding: str): str;
}

// An async host function — promise-returning; callers implicitly await:
[extern("node:timers/promises", "setTimeout")]
async external fun sleep(ms: i32): void;
```

The four binding forms:

| Form | Binds |
|---|---|
| `[extern("module", "name")]` | an import from a host module |
| `[extern("global.path")]` | a dotted global, like `history.pushState` |
| `[extern(method, "name")]` | a method on a host object |
| `[extern(get, "prop")]` / `[extern(set, "prop")]` | a property read / write |

Keep externs in platform-specific packages (they are host-specific by
nature). When a binding proves itself, consider promoting it into std
rather than copying it between apps.

## Assets

Browser builds produce `<entry>.js`, plus `<entry>.css` when styles were
emitted. Your server serves those two files and an HTML shell — the
[services guide](../guide/services.md) shows the standard fallback shape.

> **Going deeper.** Build assets come from `std::asset::emit(kind,
> content)`, callable only during `const` evaluation. The styling
> system's `const style()` chains call it to write CSS rules. Libraries
> can also declare platform overlays of their own (a base root plus
> per-platform roots in `[library.layer]`), which is how std itself is
> layered — most libraries never need this.
