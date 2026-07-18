# Async polymorphism: adaptation, `sync` contracts, scopes, and the parallelism spine

**Status: Part A SHIPPED 2026-07-17** (four slices: `sync` marker 3b5e1db,
std audit 5fb9eb8, adaptation + snapshot 176fe8a, docs — spec §7.4 rewritten).
Deltas from the design, all recorded in place: the snapshot is implemented as
shallow-copy iteration inside async adapted instances (sound because element
aliasing doesn't exist under value semantics — A.5); dual lowering (A.6)
collapsed to nothing because the List combinators are vilan source, not
intrinsics; `settle_all` is not yet minted (the two-`map` idiom works today —
open question stands). **Part B is the seed of the J1 execution-model phases;
Part C is a design record, explicitly not v1.**

Decisions in this document were made 2026-07-17 (backlog J2's last open
channel). The headline calls are the user's: adaptation is the default and
sequential; the sync-contract marker is spelled **`sync`**; concurrency is an
opt-in idiom over spawn; void positions keep spawn semantics.

---

## 0. Where this sits

Vilan's async model is *inferred coloring*: a function is async because its
body awaits or because it calls something async; calls to async functions are
implicitly awaited; return types stay plain values. For closure **values**
(no fixed callee) asyncness rides the type — `async |T| U`, accepted on
parameters, `let` annotations, struct fields, and function return types — and
unannotated bindings adopt asyncness from what they hold. A divergence check
refuses an async closure flowing into a plain value-returning position at
every boundary (parameter, field, declared return). All of that is shipped
(J2, closed 2026-07-17).

The one remaining refusal is the useful one this proposal removes:

```vilan
fun fetch_id(url: str): i32 {
    sleep(1);              // async — inferred
    url.len()
}

let ids = urls.map(|url| fetch_id(url));
//        ^^^ error: `fn` receives an async closure, but its type awaits nothing
```

No higher-order function accepts an async closure today. The refusal is
sound — `map`'s body doesn't await — but the fix should not be a colored API
(`map_async`) or a blanket `async` parameter (which would color every sync
call site in every program).

Survey conclusion (recorded so it isn't re-litigated): Go's model on a JS
host *is* this model (which calls can suspend must be decided statically;
Go-on-JS pays a scheduler to discover it at runtime); Pony's capabilities
would replace vilan's memory model rather than extend it; Rust's explicit
futures are the infection this language exists to avoid, and Rust's stalled
"keyword generics" work marks effect-polymorphic HOFs as the hard kernel —
which vilan's whole-program monomorphization (no `dyn`, no fn pointers)
makes uniquely cheap; Gleam's per-target split (BEAM processes vs JS
promises) is the fragmentation to avoid. What the current model lacks is not
a different coloring story but the structured layer above it (Part B) and a
sendability rule shared with future parallelism (Part C).

---

## Part A — Monomorphize-by-asyncness ("adaptation")

### A.1 The rule

A plain, value-returning closure parameter is **asyncness-polymorphic**.
Each call site instantiates the function with the actual asyncness of its
closure arguments, exactly as it already instantiates by type arguments:

- **Async instance** (an argument closure is async): every call through that
  parameter is awaited; the instance itself colors async; its callers await
  it and color accordingly. Emission is a distinct monomorphized instance.
- **Sync instance** (all closure arguments sync): byte-identical to today.
  No awaits, no coloring, native lowerings preserved.

The instantiation key gains a per-closure-parameter asyncness bit beside the
type substitution. Precedent: platform requirements are already computed
per-instantiation (platform-coloring, 8772aef); asyncness is a second effect
axis on the same machinery.

**The sequential contract.** An adapted call awaits each callback before the
next begins. `array.map` over 100 elements whose callback takes 1 second
takes 100 seconds. Effects are ordered exactly as the sync version orders
them; adaptation never introduces interleaving between elements beyond what
each await itself admits. Concurrency is opt-in (A.7).

### A.2 The `sync` marker — a synchronous contract

Some callbacks are part of a synchronous protocol: reactive recomputation,
comparators, `turn` bodies. Adaptation there would break invariants at a
distance (the reactive graph's propagation is synchronous by design —
glitch-freedom, drain affinity). The author opts out per parameter:

```vilan
fun map<U>(self, f: sync |T| U): Signal<U>      // Signal::map — recompute is sync
fun turn(policy: FlushPolicy, body: sync || )    // turn_async is the async flavor
```

`sync |T| U` means: *this closure's completion is part of my synchronous
protocol; its call is used as-is and never awaited.* Passing an async
closure to a `sync` position is a compile error. The message names the why
and the steer, per the diagnostics standard (B4/B6):

> `f` requires a synchronous closure (`sync |T| U`) — recomputation is part
> of the reactive graph's synchronous protocol. For async work, use
> `turn_async` / `Draft` / `optimistic`.

(The steer text is per-site; std's `sync` positions each get a wording pass.)

Parameter positions therefore have three states:

| declaration      | async argument                     | sync argument |
| ---------------- | ---------------------------------- | ------------- |
| `\|T\| U` (plain)  | adapts (async instance, awaited)   | sync instance |
| `sync \|T\| U`     | **error** (sync contract)          | as-is         |
| `async \|T\| U`    | awaited (declared channel)         | awaited (no-op await) |

`async`-marked parameters are *not* polymorphic — they force the async
instance regardless of the argument, for closures whose provenance adoption
cannot see. `sync` is only meaningful on parameters: fields and returns
already have a two-state story (plain = refuses async stores via the
divergence check; `async` = awaited channel) and do not adapt.

Grammar: `sync` is a **contextual keyword** in closure-type position (like
`context`): it lexes as an identifier and only means the contract directly
before a closure type, so `sync`-named values stay legal.

### A.3 Void positions keep spawn semantics

Adaptation applies to **value-returning** closures only. An async closure
into a plain *void*-returning parameter stays what it is today: legal,
spawned, fire-and-forget — UI handlers and turn bodies ride this, and the
`turn` / `turn_async` distinction stays deliberate. So the full rule is:

- non-void plain parameter: adapts;
- void plain parameter: spawns (unchanged);
- `sync` parameter: refuses async arguments (any return type);
- `async` parameter: awaits (unchanged).

This preserves every existing program's semantics: today's legal programs
only ever put async closures where they spawn or where the channel is
declared; the newly-legal programs are exactly the ones that were refused.

### A.4 v1 exclusions (recorded, not solved)

- **Adaptation covers closures the body *calls*.** A body that stores a
  parameter closure into a field, returns it, or writes it into any typed
  position uses the existing rules (the field/return divergence checks catch
  lies). `fun compose(f, g): |A| C { |a| g(f(a)) }` with an async `f` stays
  an error at the return — the returned closure's asyncness *depends on* the
  parameter's, which is an effect variable connecting two positions (the
  full effect-row problem). v2 horizon at most; `compose` is rare.
- **Transitive adaptation is NOT excluded**: passing the parameter onward as
  an argument to another adaptive function is a call-position flow —
  `fun helper<T,U>(xs: List<T>, f: |T| U) { xs.map(f) }` instantiates
  `helper` async-in-`f`, which instantiates `map` async. The bit rides the
  instantiation chain; only *escape* into storage/returns is excluded.
- **Externs are implicitly `sync`** for value-returning closure parameters
  (host code cannot await a vilan closure); void extern callbacks keep spawn
  (a `setTimeout` handler that awaits is a spawn, as today).
- **Container elements**: `List<|| T>` element types accept no markers (J2
  record) and calls through elements do not adapt; unchanged, future work.

### A.5 Snapshot semantics for adapted receivers

An adapted `map` cannot hold a view of its receiver across the callback
awaits — that is exactly what no-view-across-await forbids, and the rule is
right: during an await, arbitrary interleaved code (turns, handlers, other
spawns) runs, and *anyone* who can reach the viewed root may mutate or
reallocate it. Note the two tempting loosenings and why they fail:

- "the closure can't reach the view" — necessary but insufficient; the
  hazard is the interleavable world, not just the callee;
- "prove the view isn't mutated" — unverifiable against that same world.

The sound options are escape analysis on the *root* (a local that never
escapes — no `Shared`, no capture, never passed outward by view — is
unreachable by interleaved code, so the borrow is safe) or snapshotting.

**Decision: adapted std higher-order functions iterate a snapshot** — one
copy of the receiver taken at the call. This is the better *observable*
semantics, not just a checker dodge: an awaiting `map` traverses the
receiver as of the call; interleaved mutations do not tear the traversal.
Under value semantics "you got a copy" is the least surprising rule in the
language. The escape-based borrow is recorded as a later, purely-internal
optimization (it must not change observable behavior, which the snapshot
contract pins).

### A.6 Host-lowered functions: dual lowering

Where a sync instance lowers to a host intrinsic, the async instance emits a
vilan loop body (with awaits + the snapshot); where the function is ordinary
vilan source, both instances emit from the same body. Consequence to accept:
a distant `sleep` added deep in a callback silently moves a `map` from the
native fast path to an emitted sequential loop. That is the cost of
consistency, and it is only paid by call sites that actually went async; the
tooling mitigation is an "async because …" origin chain on hover (A.8).

`array.map(|x| async { work(x) })` involves a **sync** closure returning a
promise value — sync instance, native lowering, `List<Promise<T>>` result.
The concurrency opt-in costs nothing.

### A.7 Concurrency is an idiom, plus one helper

```vilan
// start all (sync closure returning promises), then settle in order:
let ids = urls
    .map(|url| async fetch_id(url))    // List<Promise<i32>> — all in flight
    .map(|p| await p);                 // adapts; total ≈ max, not sum
```

A std helper can name the second half (`settle_all(List<Promise<T>>):
List<T>` or a `.settle()` method; `std::promise`'s gathered form already
exists — pick one surface at implementation, don't add two).

**Failure semantics, stated:** a started promise that rejects before its
settle pass is reached is a *late unhandled rejection* if the pass is
abandoned (a panic between the two maps, a short-circuiting combinator).
v1 documents this hazard; the real fix is Part B — inside a scope, every
spawn settles at scope exit, absorbed or propagated, never orphaned.

**`Promise<T>` under value semantics must be pinned at implementation:** it
is a *handle* (copy = the same promise), never `__clone`d — a deep copy of a
pending promise is nonsense. `async-promise-all.vl` suggests the emission
already behaves; the rule needs a pin.

### A.8 Diagnostics and tooling

- Errors arising inside an adapted instance (a `sync` violation reached
  transitively, a view error in a user HOF that borrows across the new
  awaits) are **instantiation-dependent**. They attribute with origin
  chains, platform-coloring style: *"async instance required by the call at
  main.vl:12 → helper → map"*. This is the acknowledged cost of
  monomorphized effects; the chains are the mitigation.
- The `sync`-violation message carries the per-site steer (A.2).
- LSP: hover on a call can show the chosen instance's asyncness with its
  origin chain (rides the existing coloring-hover machinery). Polish, not a
  gate.

### A.9 Std audit (initial; finalized at implementation)

- **Adapt** (plain parameters): `List::map/filter/each/find/reduce/sort_by`,
  `Option::map/and_then/unwrap_or_else`, `Result::map/map_err/and_then`,
  retry/walk-style helpers.
- **`sync`**: `Signal::map/effect/set_with`, `bind_each` and render
  callbacks, reactive comparators/keys, `turn`, `batch`.
- **Spawn (void, unchanged)**: `ui.on`, `dispatcher.on` handlers,
  reconnect hooks.
- Every flip is its own reviewed line in the implementing commit; `sort_by`
  adapting (sequential awaited comparisons over the snapshot) is included
  unless the audit finds a reason not to.

### A.10 Test plan (pins before behavior ships)

adaptation runs sequentially (effect ORDER pinned, not wall time); sync
instance byte-identical (corpus); `sync` refusal message + steer; void spawn
preserved; snapshot observation (mutation during awaits doesn't tear);
transitive `helper → map`; store/return exclusions still refused; extern
refusal; the opt-in idiom compiles native (golden) and runs; mixed
closure-parameter arity (one async, one sync); `Promise<T>` never cloned.

---

## Part B — Nurseries (the J1 execution-model seed) — **SHIPPED 2026-07-18**

Shipped in four slices: `Task<T>` (ae2d675), the nursery core (9b85534),
cancellation + the AbortSignal bridge (24e4dd7), docs (this commit).
Implementation deltas from the design below:

- **Settle-time failure reaction**: a failing owned task notifies its
  nursery AT SETTLE (`__fail`: latch, abort, wake the drain) and the join
  races each child against the wake — a fast failure behind a slow healthy
  sibling reacts immediately, and "earliest-settled wins" is structural
  (the latch), no sequence stamps needed.
- **Owned tasks never default-report** — the nursery observes them; the
  unobserved-failure report is for free-floating tasks only. Post-failure
  stragglers spawned into a dead nursery are silently absorbed (owned).
- **Body-cancellation semantics** (was unrecorded): `cancel()` kills
  children, not the body — code after it runs and the value returns; a
  body SUSPENDED on cancellable IO when the signal fires observes the
  rejection, which propagates as the nursery's outcome (body-throw rule).
- **Registration mechanics**: spawns are SAFE reads of `ambient_nursery`
  in the context pass (a new demand kind riding the whole strict/safe
  apparatus), engaged only when `nursery` is called somewhere — loading
  `std::task` alone keeps every program byte-identical. The body parameter
  is an injected-clause closure (`context ambient_nursery`), so the
  literal takes its own hidden parameter; an awaiting body rides Part A
  adaptation into the machinery.
- **Holes this forced open and closed**: a directly-applied async closure
  literal (the lowered `run` body) never counted as an await point —
  latent miscompile for ANY async run body, fixed in subject_awaits /
  awaited_calls / the J3 initializer check; extern refusals now honor the
  typed channel (a DECLARED `async |…| T` extern parameter is the host's
  contract to await — `__nursery_run`); WrapSome thread forms now trigger
  Option variant resolution (spawn demand creates covered→safe boundaries
  with no `get_safe` in the program); spec §7.1's exit claim corrected
  (the host exits when no live handles remain, not "when `main`
  completes").
- **Still open**: `Task<Task<T>>` assimilation (JS thenables flatten —
  same divergence Promise always had, typed one level deeper than
  runtime); per-task cancel handles (race composes from nursery-scoped
  cancel, so deferred until a real need); the free-spawn lint (std's own
  audit found NOTHING to migrate — every std spawn is either a returned
  `Task` or object-lifetime work a function-scoped nursery cannot own,
  each now comment-marked as deliberate; the lint waits on the
  resource-owner story). The abort-in-flight `fetch` e2e is CLOSED:
  `crates/vilan-cli/tests/cancellation.rs` cancels a fetch against a
  hanging endpoint and joins in ~3s instead of 60.

Original direction (decisions recorded 2026-07-18, all implemented):

- **`nursery(body)`** — a `std::nursery` FUNCTION, not syntax (adaptation
  makes an awaiting body just work): spawns created within its dynamic
  extent are joined at exit; the nursery's value is its body's value; it
  returns only when all children settle. Registration is **dynamic-extent
  via `context`** (the handle threads as a scoped value, like the reactive
  ambient owner) — a helper called inside spawns into it without plumbing.
  **DECIDED: the name is `nursery`** (`scope` has too many meanings), and
  **v1 is explicit** — `main` is NOT an implicit root; free-floating spawns
  keep today's behavior, with a lint later once std itself is scoped.
- **Errors — DECIDED, first-observed**: a body throw wins if it happens
  before the join; otherwise the earliest-settled rejection. The nursery
  stops awaiting the rest, **absorbs** their eventual rejections (no late
  unhandled rejections), and re-raises the winner at the join with an
  origin chain naming the spawn site. Abort-caused rejections classify as
  CANCELLATION (absorbed), never as a competing first error.
- **Cancellation is cooperative and honest about JS — with the AbortSignal
  bridge**: the nursery owns a host `AbortController`; its signal rides the
  same context value as the token, and std's host-IO wrappers (`fetch`,
  `sleep`, sockets) pass it to the host op. First error (or an explicit
  `n.cancel()`) aborts the controller, so in-flight HOST IO genuinely
  cancels and the join is fast; pure-compute loops still check
  `cancelled.get()` at their own points. Nested nurseries chain signals.
  Instrumenting every implicit await is the possible v2 (cost measured
  first); native targets can preempt better later.
- **`Task<T>` as the substrate** (2026-07-18): `async expr` lowers to a
  std `Task<T>` — a HANDLE (like `Shared`; copy = same task; never
  `__clone`d, dissolving Part A's Promise-under-value-semantics pin) that
  wraps the host promise plus its abort handle and spawn-site origin, and
  **attaches the absorption handler at construction** — unhandled
  rejections become structurally impossible program-wide, not just inside
  nurseries (an abandoned task outside any nursery gets an orderly default
  report). Host promises wrap at the std extern seam; raw `Promise<T>`
  remains for direct host interop. Tasks stay EAGER (run to first
  suspension synchronously, as §7.3 specifies) — a cold task would be a
  semantic break for no benefit. NOTE: a global `Promise` polyfill is NOT
  viable — JS async functions return the intrinsic %Promise% regardless of
  the patched global, and species games don't capture it; owning the
  lowering and the std boundary replaces it.

Token ergonomics — **DECIDED as recommended (user delegated 2026-07-18)**:
the structural AbortSignal bridge (std IO reads the ambient signal, no
token threading) + the `nursery(|n| …)` handle variant for
cancel-from-within. The `Task` surface shipped as `Task::settle_all` +
`Task::race`; the race idiom is `race` + `n.cancel()` (cancel-after-settle
is a no-op for the winner, so nursery-scoped cancel suffices).

---

## Part C — Parallelism appendix (design record; not v1, forecloses nothing)

- **Sendability is the shared spine.** Plain values cross any concurrency
  boundary by construction (value semantics — no aliases); `Shared<T>`,
  views, and non-`Wire` closures do not. The check is platform-coloring-
  shaped machinery, and `Wire` already answers serialization.
- **JS lowering**: workers + `Wire`; a parallel scope mirrors Part B's scope
  with worker execution (`par` / `worker_map` — surface deliberately
  unspecified here).
- **Native future**: the same discipline scales to threads; fork-join over
  immutable second-class views is provably race-free by construction, which
  is the safe first shared-memory extension; actors + supervision (the BEAM
  idea worth keeping) are a possible std layer above it, never core.
- Async (interleaving) and parallelism (simultaneity) share sendability and
  the scope vocabulary and **stay separate in scheduling semantics** — one
  vocabulary, different machines.

---

## Decisions and open questions

**Decided (2026-07-17):** default-adapt for plain non-void closure params;
sequential contract; marker spelled `sync` (contextual keyword);
void = spawn preserved; snapshot semantics for adapted std receivers;
effect-dependent returns excluded in v1; externs implicitly `sync`
(non-void); concurrency via the spawn-then-settle idiom + one helper.

**Open, Part A (settle at implementation):** ~~the helper's surface~~
(SHIPPED: `Task::settle_all` + `Task::race`, statics on the handle);
`sort_by` inclusion; ~~`Promise<T>` handle pin~~ (dissolved — `Task<T>` is
a class-instance handle, `__clone` passes it through); `sync` steer
wording per std site.

**Part B (SHIPPED 2026-07-18):** ~~scope keyword~~ (`nursery`, a std
function); ~~implicit root scope~~ (explicit v1); ~~token ergonomics~~
(structural bridge + handle variant); await-point instrumentation stays
NOT taken (the bridge covers IO; compute loops poll `is_cancelled`).
