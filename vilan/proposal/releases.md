# Releases — installation and updates

## 1. Problem

vilan is public, but the only way to run it is to clone the repo and build
with cargo. The target audience is JS/TS developers who may not have a
Rust toolchain and shouldn't need one. They need: a one-command install, a
one-command update, prebuilt binaries per platform, and an editor
extension they can install without a marketplace account.

Underneath the packaging sits one real design problem: **an installed
binary cannot find std.** The compiler loads `vilan/std` from disk —
`$VILAN_STD`, else a path baked at compile time pointing into the repo
checkout. Both are meaningless on a user's machine. Every other decision
in this document is plumbing; this one is architecture.

## 2. Goals and non-goals

Goals:

- Install without Rust: download-and-run binaries for the major
  platforms, plus an install script in the style the audience knows
  (rustup/deno/bun).
- One-command update from the CLI itself.
- A self-contained toolchain: `vilan` works with no repo checkout, no
  side-by-side directories, no environment variables.
- Prebuilt `.vsix` for the VS Code extension in every release.
- Reproducible, automated releases: tag → CI builds, tests, packages,
  publishes. No hand-built artifacts.
- Privacy-clean artifacts: release binaries carry no build-machine paths
  (`--remap-path-prefix`), no phone-home behavior of any kind.

Non-goals (recorded, revisitable):

- Package managers (Homebrew, AUR, winget, apt) — after the direct
  channel proves out.
- crates.io publishing — the audience isn't cargo-first; low value now.
- Versioned documentation — the site tracks `main` (recorded in
  docs-site.md); releases link to it.
- Auto-update daemons or background update checks — updates happen when
  the user asks, full stop.

## 3. The std problem (the architectural piece)

**Decision to make: embed std in the binary.** The whole standard library
is 420K of `.vl` source (+28K macro_std) — embedded as compile-time data
it costs less than half a megabyte uncompressed, and it makes the binary
the complete toolchain. Compiler and std version together atomically,
which the wire-contract hashing and derive machinery already assume.

Resolution order becomes:

1. `$VILAN_STD` — explicit override, unchanged (power users, testing).
2. The ancestor walk — unchanged (developing IN this repo keeps loading
   std from the working tree, so std hacking needs no rebuild).
3. **The embedded copy** — replaces today's baked repo path as the
   final fallback. This is what every installed binary uses.

Implementation (refined after reading the loader): the std pipeline is
filesystem-shaped end to end — layer probing, `read_dir` module listing,
`macro_std` resolved as a sibling directory of `std`, manifests read
from disk. Teaching all of that a virtual filesystem would touch the
most battle-tested code in the compiler for no user-visible gain. So the
binary **embeds the two package trees and materializes them on first
use**: a `build.rs` in `vilan-core` embeds every `.vl` + `vilan.toml`
under `vilan/std` and `vilan/macro_std` (with a content hash), and the
fallback writes them once to `~/.vilan/std-cache/<hash>/` (atomic
tmp-dir + rename; temp-dir fallback if home is unavailable) and returns
that real path. The loader is untouched. LSP go-to-definition into std
keeps landing on real files. The content-hash key means a rebuilt dev
binary never sees a stale cache, and `vilan upgrade` swaps versions
without any sync step. The LSP replaces its baked-path fallback with the
same call (fixing the kolt-shape fragility for installed binaries).

**Pre-compiled std: measured, deferred.** Embedding *parsed* std (the
caching plan's deferred tier) was considered here. Measured on the
release binary: `check` on a hello is ~100ms end to end and a full
walkthrough build (client + server + macro worlds) is ~500ms — and the
in-process parse cache already amortizes std for watch mode, the LSP,
and multi-target builds, so pre-parsing could save only a few tens of
milliseconds on cold CLI runs. Against that: serializing an AST built
on borrowed `&'static str`, a build-script bootstrap on the compiler's
own parser, and a stale-artifact bug class. Not worth it at today's
numbers. If std parse cost ever shows up, the right shape is a
runtime-written warm cache beside the materialized std (generated on
first use by the binary's own parser, content-hash keyed) — no
build-time serialization, no staleness.

## 4. Versioning

One version for the whole toolchain — `vilan`, `vilan-lsp`, the
extension, and std move together, because they are coupled in fact
(embedded std, wire contracts, LSP protocol assumptions). Scheme:

- `0.MINOR.PATCH`, tags `v0.2.0`, `v0.2.1`, …
- Pre-1.0 semantics: minor bumps may break anything (the alpha promise);
  patch bumps are fixes. Bump minor liberally.
- The first public release is **v0.2.0** (0.1.0 was the pre-public
  internal number; a visible jump marks the boundary).
- `vilan --version` prints `vilan 0.2.0 (<short-sha>)` so bug reports
  are precise.
- `CHANGELOG.md` at the root, hand-written per release in the docs'
  voice: what changed, what breaks, how to migrate. The release workflow
  refuses to tag a version with no changelog section (a grep gate, same
  spirit as the docs gate).

## 5. Installation channels

**Phase 1 (this proposal):**

- **GitHub Releases** — the canonical artifact store. Per release:
  - `vilan-<version>-<target>.tar.gz` (`.zip` on Windows), each
    containing `vilan` + `vilan-lsp`.
  - `vilan-vscode-<version>.vsix` — the extension, prebuilt.
  - `sha256sums.txt`.
- **The install script** —
  `curl -fsSL https://github.com/ReedSyllas/vilan/releases/latest/download/install.sh | sh`:
  detects OS/arch, downloads the right tarball, unpacks `vilan` and
  `vilan-lsp` into `~/.vilan/bin`, prints the PATH line to add. The
  script itself is a release asset (and lives in the repo under
  `scripts/`), so it needs no separate hosting. Idempotent: re-running
  it updates in place.
- **From source** stays first-class for Rust users:
  `cargo install --path crates/vilan-cli` (already in the README).

**Targets:** `x86_64-unknown-linux-musl` (static — one binary for every
distro and WSL), `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`,
`aarch64-apple-darwin`. Windows: decision below — native
`x86_64-pc-windows-msvc` marked experimental, or WSL-only at first (the
runtime story is node-based either way).

**Phase 2 (recorded):** npm distribution (`npm i -g vilan` /
`npx vilan`) via the esbuild pattern — platform binaries as
`optionalDependencies`. For a JS/TS audience this is likely the single
highest-adoption channel; it earns its own slice once the direct channel
is proven. Homebrew tap alongside it.

## 6. Updates

- **`vilan upgrade`** — a new CLI subcommand:
  1. queries the GitHub Releases API for the latest tag,
  2. compares to its own version; prints "already newest" or the
     changelog url,
  3. downloads the platform asset, verifies the sha256,
  4. swaps itself atomically (write to temp, rename over — with the
     rename-the-running-exe dance on Windows), updating `vilan-lsp`
     beside it.
  - `vilan upgrade --check` does steps 1–2 only.
- **No passive checks.** The CLI never touches the network unless the
  user runs `upgrade`. This is a privacy stance, stated in the docs.
- **The extension**: point it at `~/.vilan/bin/vilan-lsp` in its binary
  search order (it already searches release/debug/cargo locations), so
  `vilan upgrade` updates the LSP the editor uses with no extra step.
  Extension updates themselves are a new `.vsix` per release until a
  marketplace listing lands (recorded for Phase 2 — publishing needs a
  publisher account decision).

## 7. The release pipeline

`.github/workflows/release.yml`, triggered by pushing a `v*` tag:

1. **Gate**: full `cargo test` on linux (the suite: 669 tests, corpus,
   docs gate, walkthrough build, hygiene).
2. **Changelog check**: `CHANGELOG.md` contains a section for this
   version.
3. **Build matrix**: the targets above, `--release` with
   `RUSTFLAGS=--remap-path-prefix` mapping `$HOME` and the workspace to
   neutral names — release binaries carry no build paths (the
   going-public discipline, mechanized).
4. **Package**: tarballs + vsix (`vsce package` in `editors/vscode`,
   pinned via `npx --yes @vscode/vsce`) + `sha256sums.txt` + the install
   script.
5. **Publish**: `gh release create v<version>` with the changelog
   section as the release notes, all assets attached.

Cutting a release is then: update `CHANGELOG.md` + bump versions (one
script: `scripts/bump-version.sh` rewrites the three crate manifests and
the extension's `package.json`), commit, tag, push the tag. Everything
after is CI.

## 8. Delivery

- **Slice 1 — the self-contained binary**: embed std (+macro_std),
  rewire the fallback order, `vilan --version` with sha. Pins: an
  installed-binary smoke test (build, copy the binary to a temp dir
  outside the repo, compile a hello with no `VILAN_STD` and no checkout).
- **Slice 2 — the pipeline**: release workflow, packaging, install
  script, CHANGELOG, version-bump script, v0.2.0 tagged as the
  first public release.
- **Slice 3 — `vilan upgrade`**: the subcommand + extension search-path
  addition. Ships in v0.3.0 (users of v0.2.0 update by re-running the
  install script once; from then on, `vilan upgrade`).

## 9. Decisions (settled with the user, 2026-07-13)

1. **Windows**: WSL-only at first, documented. Native binaries wait for
   someone who can verify them.
2. **Install prefix**: `~/.vilan/bin` — own directory, clean uninstall,
   `vilan upgrade` owns it.
3. **First public version**: v0.2.0.
4. **npm channel**: Phase 2, its own slice soon after the direct channel
   proves out.
