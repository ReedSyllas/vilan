# Browser example

A Vilan client that runs in the browser, built on the `std::dom` platform layer.

## Build

```sh
vilan build --target browser client.vl
```

This emits `client.js` — an ES module that uses DOM globals
(`document.createElement`, `addEventListener`, …) with no Node host imports and
no `process.exit`.

## Run

Open `index.html` in a browser. It provides the `<div id="app">` mount point and
loads `client.js` as a module. (Serve the directory over HTTP — e.g. any static
server — if your browser restricts ES modules over `file://`.)

You should see the heading, a live greeting that echoes whatever you type into
the name field (read via `query_selector` + `value` on each `input` event), a
"Clear" button that resets the field (`set_value`), and an "Add a note" button
whose paragraphs remove themselves when clicked (`remove`).

## Notes

- `client.vl` only imports `std::dom` and other universal (core) modules, so it
  compiles for `--target browser`. Importing a Node-layer module (`std::http`,
  `std::fs`, `std::process`) here is a compile error.
- The full-stack flow — a Vilan `std::http` server that serves this bundle, and a
  shared module compiled for both targets — is the next step.
