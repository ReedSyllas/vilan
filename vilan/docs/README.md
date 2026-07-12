# The vilan documentation

How to use the vilan language, its standard library, and the frameworks built
on top of it. (Design history and rationale live in `vilan/proposal/` — those
documents record how things were built; these record how to use them.)

## Parts

- **[Tour](tour/)** — the language, informally: syntax, types, closures,
  async, the memory model. Start here if you're new or need a syntax
  reminder. (A formal specification is planned; the tour is its practical
  companion.)
- **[Guides](guide/)** — the frameworks, task-oriented: reactive state,
  building UI, styling, routing, services & RPC. Each reads front to back
  and links into the reference for exact signatures.
- **[std reference](std/)** — the standard library, signatures-first: one
  page per module group, each item with its signature, semantics, an
  example, and traps. Go here to answer "what were the parameters again?".
- **[Appendix](appendix/)** — gotchas checklist and glossary.

## Conventions

- Examples are complete programs unless explicitly labelled a fragment —
  copy, `vilan build`, run.
- **Every example compiles as part of the test suite** (`cargo test --test
  docs`): a fenced ` ```vilan ` block must compile for the node target,
  ` ```vilan,browser ` for the browser target, ` ```vilan,norun ` compiles
  but needs external services to actually run, and ` ```vilan,fragment ` is
  prose-only (used sparingly, always labelled).
- Maintenance rule: a change to std, a framework, or the language updates the
  affected docs page **in the same commit**.

## Contents

### Tour
| Chapter | Covers |
|---|---|
| [Functions & closures](tour/functions-and-closures.md) | `fun`, closure types, named-fn coercion, async closures and their seams, context clauses |
| [The memory model](tour/memory-model.md) | value semantics, views, `mut`/`own`, `Shared`, `Arena`/`Handle` |
| [Async](tour/async.md) | implicit await, `async expr` spawn, promises, timers |

*(Chapters on getting started, values & types, data & traits, control flow,
macros & const, and platforms arrive with Phase 2.)*

### Guides
| Chapter | Covers |
|---|---|
| [Reactive state](guide/reactive.md) | signals, derived state, effects, ownership & disposal, turns, `optimistic`, `Draft` |
| [Building UI](guide/ui.md) | `view` chaining, binds, events, lists, conditionals, mounting |
| [Styling](guide/styling.md) | `const` typed styles, lengths/colors, dynamic values |
| [Routing](guide/routing.md) | enum routes, `parse`/`href`, `link`, `swap`, navigation |
| [Services & RPC](guide/services.md) | `[service]`/`[rpc]`/`[expose]`, Wire, mirrors, reconnection, the server side |

### std reference
| Page | Modules |
|---|---|
| [reactive](std/reactive.md) | the full `std::reactive` API |
| [style](std/style.md) | the full `std::style` API |
| [rpc](std/rpc.md) | `std::rpc` — transports, clients, frames |
| [browser](std/browser.md) | `std::dom`, `std::ui`, `std::router`, `std::storage` |

*(The remaining module pages arrive with Phase 2.)*
