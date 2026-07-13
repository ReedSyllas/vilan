# vilan

vilan is a language for building full-stack web apps. It compiles to
JavaScript and runs on node and in the browser, but it is not JavaScript:
values are copied instead of shared, there is no `null` and no exceptions,
`await` is implicit, and the compiler checks the things you usually find
out at runtime.

It ships as one coherent stack. The language, a standard library, a
fine-grained reactive UI layer, typed compile-time styling, an enum-based
router, and a service layer where the server exposes live-synced state and
typed rpc methods from a single struct — no REST endpoints, no schema
files, no client SDK to regenerate.

> **Status: fast-moving alpha.** The language changes weekly and there are
> no stability promises yet. It is, however, real: the test suite holds
> ~670 tests, every example in the documentation is compiled by CI, and
> the repo contains a working full-stack example app.

## A taste

Reactive state, from the first page of the guide:

```vilan
import std::print;
import std::reactive::{ Signal, Owner, run_with_owner };

fun main() {
	let count = Signal::new(0);
	let owner = Owner::new();
	run_with_owner(owner, || {
		count.effect(|value: i32| print(value));
	});
	count.set(1);
	count.set(2);
}
```

And the full-stack model — one struct is the entire client/server
contract. Exposed signals mirror live to every connected client, and
`[rpc]` methods are callable remotely with typed results:

```vilan,fragment
[service(NotesClient)]
struct NotesStore {
	[expose] notes: Signal<List<Note>>,
	…
}

impl NotesStore {
	[rpc]
	fun add_note(self, token: str, title: str): i32 { … }
}

// browser side:
let client = NotesClient::connect("/", json_codec())!;
let _sync = client.notes.sub(|list| …);   // live-synced, typed
```

The [full-stack walkthrough](vilan/docs/guide/walkthrough.md) builds a
working notes app — sign-in, live sync between windows, an editor that
saves as you type — in about 500 lines, and that app lives in
[`vilan/examples/walkthrough/`](vilan/examples/walkthrough/) where the
test suite builds it on every run.

## Why it feels different

- **Values copy.** Assigning or passing data gives the receiver its own
  copy. Sharing is explicit and typed — a whole class of
  spooky-action-at-a-distance bugs doesn't exist.
- **No `null`, no exceptions.** Absence is `Option`, failure is `Result`,
  and the `!` and `?.` operators keep both ergonomic.
- **`await` is implicit.** Calling an async function just gives you the
  value. You only write `async` to *opt out* of waiting.
- **Fine-grained reactive UI.** Signals bind to individual DOM
  properties. No virtual DOM, no re-renders, and cleanup is automatic by
  construction.
- **The wire is checked.** Payload types derive `Wire`; client and server
  compare a contract hash at connect; mirrors resync themselves after
  reconnects.
- **Docs that can't rot.** Every example in the book is compiled by the
  test suite.

## Getting started

Install the toolchain (Linux, macOS, or Windows via WSL; you'll also
need [node](https://nodejs.org) to run what you build):

```sh
curl -fsSL https://github.com/ReedSyllas/vilan/releases/latest/download/install.sh | sh
```

That puts `vilan` and `vilan-lsp` in `~/.vilan/bin` and prints the PATH
line to add. Each [release](https://github.com/ReedSyllas/vilan/releases)
also carries `vilan-vscode.vsix` — the VS Code extension (highlighting,
diagnostics, hover, go-to-definition, rename), installed via
"Extensions: Install from VSIX". Or build from source (Rust required):

```sh
git clone https://github.com/ReedSyllas/vilan
cd vilan
cargo install --path crates/vilan-cli   # installs the `vilan` binary
```

Then:

```sh
echo 'import std::print;

fun main() {
	print("hello");
}' > hello.vl

vilan run hello.vl
```

From there, read the book. It starts with
[Coming from JavaScript](vilan/docs/tour/coming-from-javascript.md) and
ends with the full-stack walkthrough:

- **Rendered** (search + sidebar): https://reedsyllas.github.io/vilan/ —
  or locally, `cargo install mdbook && mdbook serve vilan/docs`.
- **As files**: start at [vilan/docs/README.md](vilan/docs/README.md).

## Repository structure

```
crates/
  vilan-core/      the compiler: lexer → parser → analyzer → transformer
  vilan-cli/       the `vilan` binary (build / check / run / fmt / test)
  vilan-lsp/       the language server
editors/vscode/    the VS Code extension (grammar + LSP client)
vilan/
  std/             the standard library, written in vilan
  docs/            the book: tour, guides, reference, spec (mdBook)
  examples/        runnable examples, incl. the walkthrough app
  test/            the codegen corpus (byte-identical golden files)
  proposal/        design documents — how and why things were built
.github/           CI: docs build + deploy to Pages
```

## Development

```sh
cargo test    # the whole suite: compiler, corpus, docs gate, examples
```

Three test layers keep the project honest: unit and behavior pins in
`crates/vilan-core/tests/`, a golden-file codegen corpus in `vilan/test/`
(byte-identical, deliberately), and the docs gate, which extracts and
compiles every fenced example in `vilan/docs/` — including the ones on
this page.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in vilan by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms
or conditions.
