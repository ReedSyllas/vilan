# Hello vilan

vilan compiles to JavaScript and runs on node (also deno/bun) and in the
browser. One toolchain binary, `vilan`, does everything.

## A first program

```vilan
import std::print;

fun main() {
	print("hello");
}
```

`fun main` is the entrypoint — it runs automatically (no `main()` call at
the top level). Save as `hello.vl`, then:

```sh
vilan run hello.vl      # build + run
vilan build hello.vl    # writes hello.js
vilan check hello.vl    # type-check only, no output
```

## The CLI

| Command | Does |
|---|---|
| `vilan build [path]` | compile to `<file>.js` (no path: the nearest `vilan.toml`'s entry) |
| `vilan check [path]` | type-check, report diagnostics, write nothing |
| `vilan run [path] [args…]` | build and run; trailing args reach `process::args()` |
| `vilan fmt [paths…]` | format sources in place (`--check` to verify only) |
| `vilan test [path]` | run `*_test.vl` files (pass = exit 0; a failed `assert` panics) |

Useful flags: `--platform node|deno|bun|browser` (alias `--target`)
overrides the package's target; `--watch` rebuilds/re-runs/re-checks on
source changes; `--stdout` (build) prints the JS instead of writing it.

## Projects: `vilan.toml`

A directory with a `vilan.toml` is a package. An **app**:

```toml
[package]
name = "hello"
target = "browser"          # node (default) | deno | bun | browser
# root = "."                # source root (default .)
# entry = "src/main.vl"     # entrypoint (default main.vl under root)

[package.dependencies]
common = { path = "../common" }
```

A **library** (no entry — it's imported, not run):

```toml
[library]
name = "common"
```

A **workspace** listing several packages (built together with
`vilan build .`):

```toml
[project]
packages = ["common", "client", "server"]
```

A `[build]` section selects codegen presets and per-feature overrides
(`debug`/`release` presets; `indent`, `spaces`, `debug-names` knobs).

## Imports

```vilan,fragment
import std::print;                          // one item
import std::reactive::{ Signal, combine };  // several
import std::option::Option::{ self, Some, None };  // a type + its variants
import pkg::routes::{ Route, parse };       // a sibling module of this package
import common::{ Task, KoltClient };        // a dependency, by its name
```

Three namespaces: `std::` (the standard library), `pkg::` (your own
package's modules — `pkg::routes` is `routes.vl` next to your entry), and
each dependency under its name. A module is just a `.vl` file; there is no
separate module declaration.

## The full-stack shape

A client/server app is a workspace of three packages — the layout the
[services guide](../guide/services.md) assumes:

```
app/
  vilan.toml            [project] packages = ["common", "client", "server"]
  common/               [library] — the [service] struct, shared types
  client/               [package] target = "browser", depends on common
  server/               [package] target = node, depends on common
```

The compiler checks platform compatibility per package: `std::ui` in a node
build, or `std::db` in a browser build, is a compile error at the import
(see [Platforms](platforms.md)).
