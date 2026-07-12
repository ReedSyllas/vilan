# Gotchas

A checklist of idioms that trip people up — each with the working shape.
Grown as findings land (the backlog's "idiom traps", promoted here).

## Language

- **Calling a method-call result doesn't parse.**
  `self.hook.read()(a, b)` → bind first: `let hook = self.hook.read();
  hook(a, b)`.
- **Chained access on a call result loses the element type.**
  `pair().1`, `shared.read()[i]` → bind, then access:
  `let (a, b) = pair(); b`.
- **A closure bound to a local and called directly doesn't infer its
  parameter.** `let f = |i| …; f(3)` → annotate: `let f = |i: i32| …;`.
- **`match` can't be an operator operand.**
  `(match x { … }) + 1` → bind the match to a local first.
- **`ret` inside a match leg doesn't make the leg divergent** for type
  unification — annotate the binding the match flows into if legs mix
  returns and values.
- **`panic(…)` types as `Any`** and absorbs match unification — annotate
  the surrounding binding when a leg panics.
- **i64 literals in binary operations need suffixes.**
  `stamp + 1000` on an `i64` → `stamp + 1000i64` (the bare literal is
  `i32`; the mismatch report is currently spanless).

## Reactive & UI

- **Annotate `effect` closure parameters** (backlog B23): on a generic
  payload, `entry.effect(|current| …)` can type the parameter as the
  abstract `T` — write `|current: Option<Task>| …`.
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
