# Platforms

One language, several runtimes. A package builds for **node** (the
default), **deno**, **bun**, or the **browser** — set `target` in
`vilan.toml`, or pass `--platform` on the CLI.

The standard library is layered so each build only sees what its platform
can actually do. Import a server module in a browser build and you get a
clear compile error at the import, not a runtime crash. That's the whole
idea of this chapter.

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
`common` library holds the service definition and the payload types, and
because both sides import it, it may only use base-layer std:

```
app/
  vilan.toml       [project] packages = ["common", "client", "server"]
  common/          [library] — service + types (base layer only)
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
