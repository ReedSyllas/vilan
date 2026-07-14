# Gotchas

A checklist of idioms that trip people up — each with the working shape.
Grown as findings land (the backlog's "idiom traps", promoted here).

Arriving with an error message in hand? The [error index](errors.md) is
organized by message instead of by topic.

## Language

- **Chained element access on a call result loses the element type.**
  `shared.read()[i]` → bind, then index: `let list = shared.read();
  list[i]`.
- **`match` can't be an operator operand.**
  `(match x { … }) + 1` → bind the match to a local first.
- **A bare integer literal adapts to its peer; two typed variables
  don't.** `stamp + 1000` and `stamp < 1000` are fine on an `i53` (the
  literal takes the peer's type), but mixing two differently-typed
  *variables* in a comparison is an error — there are no implicit
  conversions; convert with `as_*` or unify the declarations.

## Reactive & UI

- **`shared.read()` is a copy** — `shared.read().push(x)` is lost; write
  through the cell: `shared.write().push(x)`.
- **Mutate signal lists with `set_with`**, never by mutating a `get()`
  result (also a copy).
- **`bind_value` fights remote updates** — for server-backed fields use
  `bind_draft`.
- **`show` keeps bindings live** while hidden; use `when` to drop state and
  subscriptions.
- **Disposal doesn't cancel the in-flight wave**: a subscriber already
  queued in the draining turn may fire once more; only *later* deliveries
  are guaranteed gone.

## Services & the wire

- **Contract-mismatch errors on connect usually mean a leaked old server**
  still holding the port — `ss -tlnp | grep <port>`, kill by PID.
- **`desc` is an SQL keyword** — name the column `description` (any SQL
  keyword as a column name fails in `CREATE TABLE`).
- **Value semantics cross the wire**: a mirrored list is a fresh copy per
  update; mutate via rpcs, never by writing the client's mirror signal.

## Process & testing

- **A completed node `main` exits the process** — long-lived
  clients/servers must hold `main` open.
- **`pkill -f <pattern>` can match your own shell's command string** — kill
  by tracked PID.
- **Rebuild the debug binary before regenerating corpus goldens** — a stale
  binary silently writes wrong goldens.
