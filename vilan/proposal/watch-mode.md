# `--watch` mode (roadmap P5)

`vilan build`/`check`/`test`/`run`, with `--watch`, re-run the command whenever a
source file changes — the standard edit→save→rebuild loop. Small, independent DX win
(roadmap P5, sized S); no language semantics, so this note just settles the design
decisions before building.

## Scope

- `--watch` is a boolean flag on **`build`**, **`check`**, **`test`**, **`run`** (not
  `fmt`). Without it, every command behaves exactly as today.
- Each round **re-runs the whole command** (re-resolve the project from `vilan.toml`,
  recompile, re-report) — so editing `vilan.toml`, adding a file, or changing a
  dependency is picked up uniformly, with no separate incremental path to keep correct.

## What is watched

The command's path argument decides the watched directory, walked **recursively**:

- an explicit **directory** → that directory (a workspace root covers every member);
- a **file** → its parent directory (where its `pkg::` siblings live);
- **omitted** → the nearest enclosing project root (the directory with `vilan.toml`),
  else the working directory.

Only **`.vl`** files are tracked. This is the key invariant: the compiler's own output
(`<file>.js`, `dist/*.js`, `.parse.out`, …) is never a `.vl` file, so a build can never
trigger its own rebuild. `std` is not watched (it changes rarely; editing `std` itself
means running `vilan check vilan/std --watch`, where `std` *is* the root). — a later
refinement could watch exactly the files the compiler loaded (`Program.sources`).

## How (polling, not an OS watcher)

A dependency-free **poll**: every ~300 ms, snapshot each watched `.vl` file → its
last-modified time and compare to the previous snapshot; any edit, addition, or removal
re-runs the action. Chosen over an event crate (`notify`) because Vilan source trees are
small (the scan is negligible), it adds no dependency, and it works offline/headless.
If trees ever grow enough to matter, swapping in an event-based watcher is a localized
change behind the same loop.

## `run --watch` — restart on change

`build`/`check`/`test` run to completion each round. **`run`** is long-lived (a server),
so each round must **stop the previous process before starting the new one** (otherwise
a server holds its port and the restart fails): kill the prior child, rebuild, spawn the
new one **without waiting**, and hold its handle for the next round. The spawn shares the
node-launch path with the blocking `run` (one `spawn_node` helper; blocking waits, watch
holds the handle). On interactive `Ctrl-C`, the child shares the terminal's process group
and receives the same `SIGINT`, so both stop together — no signal handler needed.

## Limitations (acceptable for v1)

- Poll latency up to the interval (~300 ms) before a change is noticed.
- `std` (and other out-of-tree libraries) aren't watched.
- `run --watch`: a non-interactive kill of `vilan` (e.g. `SIGTERM` to it alone, outside a
  terminal) can orphan the last child; interactive `Ctrl-C` is clean (process group).
- With `run`'s trailing args, `--watch` must precede the file (`vilan run --watch app.vl`).

## Test plan

Unit tests (in the CLI binary) for the pure logic: `watch_roots` (file→parent, dir→dir)
and `scan_vl` (tracks `.vl` only — never a sibling `.js` — and a snapshot changes on a
file addition). The poll/spawn loop is verified manually (start `build --watch`, touch a
`.vl`, observe a rebuild).
