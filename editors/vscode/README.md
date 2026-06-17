# Vilan for VS Code

Language support for the Vilan language, backed by the `vilan-lsp` language
server.

## Features

- Syntax highlighting (TextMate grammar)
- Live diagnostics
- Hover (inferred types)
- Go to definition — including jumping into the `std` package
- Find references
- Rename (locals, parameters, fields)
- Document outline

## Setup

1. Build the language server from the workspace root:

   ```sh
   cargo build --release -p vilan-lsp
   ```

2. Point the extension at it (Settings → `vilan.server.path`), e.g.
   `/path/to/vilan/target/release/vilan-lsp`, or put `vilan-lsp` on your `PATH`.

   The server locates the `std` sources by searching upward from the open file
   for `vilan/std/src`. Override with `vilan.stdPath` if needed.

## Developing the extension

```sh
npm install
npm run build      # bundle to out/extension.js
```

Then press <kbd>F5</kbd> in VS Code to launch an Extension Development Host with
the extension loaded. Open a `.vl` file to activate it.
