# The vilan documentation

How to use the vilan language, its standard library, and the frameworks built
on top of it. (Design history and rationale live in `vilan/proposal/` — those
documents record how things were built; these record how to use them.)

## Parts

- **[Tour](tour/)** — the language, informally: syntax, types, closures,
  async, the memory model. Start here if you're new or need a syntax
  reminder — the [specification](spec/) is its normative counterpart.
- **[Guides](guide/)** — the frameworks, task-oriented: reactive state,
  building UI, styling, routing, services & RPC. Each reads front to back
  and links into the reference for exact signatures.
- **[std reference](std/)** — the standard library, signatures-first: one
  page per module group, each item with its signature, semantics, an
  example, and traps. Go here to answer "what were the parameters again?".
- **[Specification](spec/)** — the normative definition: grammar (EBNF),
  the type system's rules, the memory model, execution & async. Where the
  tour teaches, the spec defines; where they disagree, the spec wins.
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
| [Hello vilan](tour/hello-vilan.md) | the CLI, `vilan.toml`, packages & workspaces, imports |
| [Values & types](tour/values-and-types.md) | bindings, primitives & numeric widths, strings & interpolation, tuples, collections |
| [Functions & closures](tour/functions-and-closures.md) | `fun`, closure types, named-fn coercion, async closures and their seams, context clauses |
| [Data & traits](tour/data-and-traits.md) | structs, enums, `impl`, generics & bounds, traits, derives |
| [Control flow](tour/control-flow.md) | `match`/`is`, loops, `ret`, Option/Result idioms, `!` and `?.` |
| [The memory model](tour/memory-model.md) | value semantics, views, `mut`/`own`, `Shared`, `Arena`/`Handle` |
| [Async](tour/async.md) | implicit await, `async expr` spawn, promises, timers |
| [Macros & const](tour/macros-and-const.md) | `const` evaluation, derive macros, `macro { … }` blocks |
| [Platforms](tour/platforms.md) | std layers, full-stack packages, externs, assets |

### Guides
| Chapter | Covers |
|---|---|
| [Reactive state](guide/reactive.md) | signals, derived state, effects, ownership & disposal, turns, `optimistic`, `Draft` |
| [Building UI](guide/ui.md) | `view` chaining, binds, events, lists, conditionals, mounting |
| [Styling](guide/styling.md) | `const` typed styles, lengths/colors, dynamic values |
| [Routing](guide/routing.md) | enum routes, `parse`/`href`, `link`, `swap`, navigation |
| [Services & RPC](guide/services.md) | `[service]`/`[rpc]`/`[expose]`, Wire, mirrors, reconnection, the server side |
| [Persistence & the server](guide/persistence.md) | `std::db` (SQLite), the http server, files, the process |

### std reference
| Page | Modules |
|---|---|
| [collections](std/collections.md) | List, Map, Set, Range, Iterator |
| [option & result](std/option-result.md) | Option, Result and their method sets |
| [strings](std/strings.md) | str, Display, Debug, Into |
| [numbers](std/numbers.md) | the sized numerics, math, random |
| [traits](std/traits.md) | compare, default, the operator traits, Try/Lift |
| [cells](std/cells.md) | Shared, Arena/Handle |
| [time](std/time.md) | Instant, Duration, timers |
| [encoding](std/encoding.md) | json, wire, binary, bytes, base64 |
| [net](std/net.md) | fetch, ws |
| [reactive](std/reactive.md) | the full `std::reactive` API |
| [style](std/style.md) | the full `std::style` API |
| [rpc](std/rpc.md) | `std::rpc` — transports, clients, frames |
| [browser](std/browser.md) | `std::dom`, `std::ui`, `std::router`, `std::storage` |
| [process](std/process.md) | db, http, fs, process, rpc_server |
| [misc](std/misc.md) | io, promise, context, crypto, jwt, asset |

### Specification
| Chapter | Defines |
|---|---|
| [§1 Introduction](spec/introduction.md) | conformance, notation, processing phases |
| [§2 Lexical structure](spec/lexical.md) | tokens, keywords, literals, operators |
| [§3 Grammar](spec/grammar.md) | the full EBNF, precedence, patterns, types |
| [§4 Names & modules](spec/names.md) | scopes, resolution, imports, namespaces |
| [§5 The type system](spec/types.md) | types, generics & inference, traits, coercions, `!`/`?.` |
| [§6 The memory model](spec/memory.md) | the four rules, views, projections, the await rule |
| [§7 Execution & async](spec/execution.md) | entrypoint, evaluation order, the async model |
| [§A Appendix](spec/appendix.md) | precedence & keyword tables, lang items |

*(Spec Phase B — contexts, const, macros, the platform model — pending.)*
