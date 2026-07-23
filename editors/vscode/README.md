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
- Inlay type hints
- Semantic highlighting (from the analyzer)
- **Organize Imports** — sorts imports into canonical order (identical to
  `vilan fmt`) and prunes unused ones. Run it from the Source Action menu
  (`source.organizeImports`), or on save (see settings below).

## Settings

| Setting | Type | Default | What it does |
| --- | --- | --- | --- |
| `vilan.server.path` | string | `vilan-lsp` | Path to the `vilan-lsp` executable (on `PATH`, or absolute). |
| `vilan.stdPath` | string | `""` | Path to the `std` source root (`vilan/std/src`). Overrides auto-discovery; sets `VILAN_STD` for the server. |
| `vilan.inlayHints.enabled` | boolean | `true` | Show inlay type hints. Applies live. |
| `vilan.semanticTokens.enabled` | boolean | `true` | Use analyzer-based semantic highlighting; when off, the TextMate grammar is used. Applies live. |
| `vilan.completion.functionCall` | `none` \| `parensOnly` \| `full` | `full` | How completion inserts a function or method call. |
| `vilan.organizeImports.onSave` | boolean | `false` | Run Organize Imports before each save. |

`vilan.organizeImports.onSave` is handled by the extension's own on-save hook, so
it leaves your `editor.codeActionsOnSave` untouched. If you'd rather drive it
through that standard mechanism, leave this off and add
`"editor.codeActionsOnSave": { "source.organizeImports": "explicit" }` instead —
organizing is a fixed point, so having both on is harmless.

Pruning is conservative: it never runs while the file has errors, never removes a
re-export, and keeps any import that derive-generated code references.

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
