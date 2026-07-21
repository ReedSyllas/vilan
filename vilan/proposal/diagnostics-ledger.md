# The diagnostics ledger (diagnostics-standard.md §5)

Every `diagnostics.push` site, its message head, and its audit verdict.
Verdicts: **QUALIFIES** (rules met, pin cited) · **RE-ANCHOR** (fails
A-rules) · **REWORD** (fails B-rules) · **DEMOTE** (cascade) ·
**NOTE-NEEDED** (wants C3) · *(blank = unreviewed)*. Line numbers are the
snapshot at generation; the message head is the stable key. Updated per
audit batch, in the batch's commit.

**Batch 7 (continuation), 2026-07-21.** The standard's "180/180 —
AUDIT COMPLETE" is the 2026-07-16 snapshot. The arcs that shipped after it
(C4 resources/destruction, R11 generic-resource, B29 conformance, A13 HMR
transfer, R12 resource-`any`, the async-polymorphism transitive checks)
added 23 new `diagnostics.push` sites — enumerated by diffing `9f59099..HEAD`
(20 new analyzer messages + 3 async_infer; the two `[rpc]`-Wire sites and the
`async` closure-type-position site only relocated). All 23 verdict QUALIFIES:
each was born inside a proven, pinned arc and already follows the B-rules
(rendered types, one-action steers, rule statements) with a family pin; span
quality spot-checked (B29 anchors at the offending parameter/return). The
five `could not be resolved` residual rows (142–145, 147) are finalized
**DEMOTE** — the `!self.diagnostics.is_empty()` guard suppresses them behind
any real error, verified with a multi-use-site pin. No new cross-source note
producer points into `std` for a user-caused condition (backlog item 11 /
E11): the only into-`std` notes are the bound- and trait-declaration notes,
both control cases — see that item's finding.

| # | Site | Message head | Verdict |
|---|------|--------------|---------|
| 1 | analyzer.rs:1856 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 2 | analyzer.rs:1960 | `{label} of `[derive(Wire)]` type `{type_name}` is `{rendered` |QUALIFIES — recursive all-fields checks name the offending field (derive pins) |
| 3 | analyzer.rs:2043 | `{label} of `[derive(Hashable)]` type `{type_name}` is `{rend` |QUALIFIES — derive all-fields check (hashable pins) |
| 4 | analyzer.rs:2106 | `{label} of `[rpc]` method `{method_name}` is `{rendered}`, ` |QUALIFIES — §4.2 contract checks (transport pins) |
| 5 | analyzer.rs:2118 | `{label} of `[rpc]` method `{method_name}` must declare a Wir` |QUALIFIES — §4.2 contract checks (transport pins) |
| 6 | analyzer.rs:2150 | `{label} is `[expose]`d, but its element `{rendered}` is not ` |QUALIFIES — Wire-element checks name the field + type (transport pins) |
| 7 | analyzer.rs:2165 | `{label} is `[expose]`d, but its type `{rendered}` is not a ` |QUALIFIES — Wire-element checks name the field + type (transport pins) |
| 8 | analyzer.rs:3167 | `a view cannot escape its scope: it may not be returned, stor` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 9 | analyzer.rs:3872 | `an async function cannot take {form} parameters: the view wo` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 10 | analyzer.rs:3946 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 11 | analyzer.rs:3998 | `an async closure cannot capture the view '{name}': the captu` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 12 | analyzer.rs:4420 | `cannot reseat a view to '{name}', which goes out of scope be` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 13 | analyzer.rs:4700 | `cannot mutate immutable '{name}'; {advice} to allow mutation` |QUALIFIES — B4 advice names the fix (mutability pins) |
| 14 | analyzer.rs:4758 | `cannot mutate immutable '{name}'; {advice} to allow mutation` |QUALIFIES — B4 advice names the fix (mutability pins) |
| 15 | analyzer.rs:4864 | `a view can't be read as a value here; write `*` to copy the ` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 16 | analyzer.rs:4894 | `cannot take a writable view of immutable '{name}'; {advice} ` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 17 | analyzer.rs:4938 | `view binding '{name}' cannot be `mut`: a view cannot be rebo` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 18 | analyzer.rs:4958 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 19 | analyzer.rs:5019 | `a `{kind}` parameter takes a view; pass `{kind} <place>` (th` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 20 | analyzer.rs:5688 | `an array length must be a non-negative integer literal ` |QUALIFIES — B6 + the const-length roadmap note (fixed-arrays pins) |
| 21 | analyzer.rs:5711 | `the `?` lifts this condition to an `Option`/`Result`, which ` |QUALIFIES — lift family (chain + region pins) |
| 22 | analyzer.rs:5731 | `vilan has no const declarations — write `let x = const ..`` |QUALIFIES — const-eval family (21 pins): capability/free-variable wording with reference spans |
| 23 | analyzer.rs:5772 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 24 | analyzer.rs:5827 | `a tuple position is a bare number (`.0`, `.1`) — drop the ` |QUALIFIES — flat-storage family (tuple .0 pins, 12) |
| 25 | analyzer.rs:5885 | `expected a method name after `.`` |QUALIFIES — parse-adjacent, dot-anchored (H18 pins) |
| 26 | analyzer.rs:5895 | `expected a field or method name after `.`` |QUALIFIES — dot-anchored recovery (H18 pins) |
| 27 | analyzer.rs:5959 | `a `[T; n]` array type isn't a value; write an array ` |QUALIFIES — B6 steer to the literal forms |
| 28 | analyzer.rs:6081 | `a `macro fun` must be a top-level item` |QUALIFIES — engine family: site-anchored, previews (macro-engine pins) |
| 29 | analyzer.rs:6155 | `the invocation `macro {name}(..)` was not expanded — splice ` |QUALIFIES — engine family: site-anchored, previews (macro-engine pins) |
| 30 | analyzer.rs:6184 | `this `macro { .. }` block was not expanded — a block cannot ` |QUALIFIES — engine family: site-anchored, previews (macro-engine pins) |
| 31 | analyzer.rs:6201 | ``export` is a module-level item and cannot appear inside a b` |QUALIFIES — H2 body-export rule (scoped-import pins) |
| 32 | analyzer.rs:6326 | `an `external` function cannot have a body` |QUALIFIES — B6 declaration-shape rule |
| 33 | analyzer.rs:6375 | `function '{}' must have a body or be declared `external`` |QUALIFIES — B6 declaration-shape rule |
| 34 | analyzer.rs:6532 | `a bare `?` (expression lifting) is not supported in this pos` |QUALIFIES — lift family (chain + region pins) |
| 35 | analyzer.rs:6548 | ``?` lifts nothing here — the region is the whole expression;` |QUALIFIES — expression-lifting pins (15) |
| 36 | analyzer.rs:6566 | ``!` cannot run after a `?` inside a lifted expression — it ` |QUALIFIES — expression-lifting pins (15) |
| 37 | analyzer.rs:6636 | ``!` requires the nearest enclosing function to declare an `O` |QUALIFIES — Origin-labeled reachability chains (platform-coloring pins) |
| 38 | analyzer.rs:6697 | `a `context` clause is only supported on a closure type` |QUALIFIES — coverage-fence family (ambient-owner pins); B6 names run/extent rules |
| 39 | analyzer.rs:6788 | `a destructuring `let` requires a value` |QUALIFIES — Origin-labeled reachability chains (platform-coloring pins) |
| 40 | analyzer.rs:6802 | `cannot assign through `*`: a view is written through directl` |QUALIFIES — view-invalidation E1/E2/E3 family (~25 pins); event-named wording |
| 41 | analyzer.rs:6816 | `a lifted chain (`?.`) is not an assignment target` |QUALIFIES — B6 (lift/place pins) |
| 42 | analyzer.rs:6860 | `struct '{}' must declare a body or be declared `external`` |QUALIFIES — B6 declaration-shape rule |
| 43 | analyzer.rs:7310 | `a closure type is not valid here (expected an expression)` |QUALIFIES — B6 (type-position rule) |
| 44 | analyzer.rs:7318 | `a `context`-typed closure type is not valid here (expected a` |QUALIFIES — coverage-fence family (ambient-owner pins); B6 names run/extent rules |
| 45 | analyzer.rs:7328 | `an `async` closure type is not valid here (expected an expre` |QUALIFIES — B6 (type-position rule) |
| 46 | analyzer.rs:7337 | `a mapped tuple type is not valid here (expected an expressio` |QUALIFIES — flat-storage family (tuple .0 pins, 12) |
| 47 | analyzer.rs:7427 | `a `context` clause is only supported on a closure type` |QUALIFIES — coverage-fence family (ambient-owner pins); B6 names run/extent rules |
| 48 | analyzer.rs:7715 | `cannot find '{}' in this scope` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 49 | analyzer.rs:7726 | `'{}' is not an enum variant` |QUALIFIES — payload-arity + resolution wording (match pins) |
| 50 | analyzer.rs:7764 | `variant '{}' does not belong to the matched enum` |QUALIFIES — payload-arity + resolution wording (match pins) |
| 51 | analyzer.rs:7773 | `cannot match an enum variant against type {}` |QUALIFIES — payload-arity + resolution wording (match pins) |
| 52 | analyzer.rs:7807 | `variant '{}' carries {} {}, but the pattern has {}` |QUALIFIES — payload-arity + resolution wording (match pins) |
| 53 | analyzer.rs:7867 | `this pattern binds {} {}, but the array's length is {}` |QUALIFIES — array-destructure count check (destructuring pins) |
| 54 | analyzer.rs:7884 | `cannot destructure {rendered} as a fixed array — ` |QUALIFIES — B6 names the pattern's domain (destructuring pins) |
| 55 | analyzer.rs:7915 | `literal pattern of type {} cannot match type {}` |QUALIFIES — B2 both sides (match pins) |
| 56 | analyzer.rs:14180 | `an `async` closure type is only supported on parameters, `let` annotations, struct fields, and function return types` |QUALIFIES — B6 marker position; relocated + reworded (widened to struct fields / return types) since the snapshot, pin re-confirmed (inference.rs:18940) |
| 57 | analyzer.rs:8033 | `a `context` clause is only supported on a parameter's closur` |QUALIFIES — coverage-fence family (ambient-owner pins); B6 names run/extent rules |
| 58 | analyzer.rs:8740 | `this array literal has {} element{}, but its type is `[_; {l` |QUALIFIES — count-vs-type wording (fixed-arrays pins) |
| 59 | analyzer.rs:8770 | `Expected {expected} (this literal's element type), but got {` |QUALIFIES — unified list/array element wording (heterogeneous-literal pins) |
| 60 | analyzer.rs:8847 | `Expected {expected} (this literal's element type), but got {` |QUALIFIES — unified list/array element wording (heterogeneous-literal pins) |
| 61 | analyzer.rs:10184 | ``self` import has no enclosing namespace` |QUALIFIES — B6 import-shape rule |
| 62 | analyzer.rs:10204 | `cannot find module '{}' to import` |QUALIFIES — A4 segment anchor (E7 pass 1 pins) |
| 63 | analyzer.rs:10240 | `cannot find '{}' in the imported path` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 64 | analyzer.rs:10483 | `Expected {} {}, but got {} instead.` |QUALIFIES — arity anchors at the arguments (they ARE the problem) |
| 65 | analyzer.rs:10549 | `Expected {}, but got {} instead.{}` |QUALIFIES — B2 + B3 note (B13 first-call origin); pin a_conflicting_later_call… |
| 66 | analyzer.rs:10577 | `Expected {} {}, but got {} instead.` |QUALIFIES — arity anchors at the arguments (they ARE the problem) |
| 67 | analyzer.rs:10605 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 68 | analyzer.rs:10644 | `Expected {} {}, but got {} instead.` |QUALIFIES — arity anchors at the arguments (they ARE the problem) |
| 69 | analyzer.rs:10722 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 70 | analyzer.rs:10776 | `cannot call '{name}': it is a struct, not a function — const` |QUALIFIES — B6 steer; subject-anchored (batch 3) |
| 71 | analyzer.rs:10803 | `cannot call a non-function value` |QUALIFIES — REWORDED to render the type + subject-anchored (batch 3); pin a_non_function_call_names… |
| 72 | analyzer.rs:10886 | `{} has no method '{}'` |QUALIFIES — RE-ANCHORED to the method name (batch 3); pins a_no_method_error_anchors…, an_array_no_method… |
| 73 | analyzer.rs:10895 | ``len` takes no arguments` |QUALIFIES — arity anchors at the arguments (they ARE the problem) |
| 74 | analyzer.rs:11186 | `{} has no method '{}'{}` |QUALIFIES — RE-ANCHORED to the method name (batch 3); pins a_no_method_error_anchors…, an_array_no_method… |
| 75 | analyzer.rs:11200 | `cannot call method '{}' on {}` |QUALIFIES — RE-ANCHORED to the method name (batch 3) |
| 76 | analyzer.rs:11213 | `cannot call '{member_name}' on a value of bare trait type ` |QUALIFIES — B6 (B4-family pins) |
| 77 | analyzer.rs:11248 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 78 | analyzer.rs:11288 | `Expected {} {}, but got {} instead.` |QUALIFIES — arity anchors at the arguments (they ARE the problem) |
| 79 | analyzer.rs:11336 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 80 | analyzer.rs:11376 | `a tuple comprehension's source must be a mapped tuple, got {` |QUALIFIES — flat-storage family (tuple .0 pins, 12) |
| 81 | analyzer.rs:11454 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 82 | analyzer.rs:11490 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 83 | analyzer.rs:11554 | `Expected {}, but got {} instead.` |QUALIFIES — B2; reassignments carry the B3 initializer note; pins a_reassignment_mismatch…, an_annotated_variables… |
| 84 | analyzer.rs:11642 | ``?.` flattens into the chain's own `Result`, so the error ty` |QUALIFIES — B4 map_err/ok_or steers (§9 pins) |
| 85 | analyzer.rs:11711 | `a bare `?` lifts an `Option` or a `Result` — this is {render` |QUALIFIES — expression-lifting pins (15) |
| 86 | analyzer.rs:11729 | `every `?` in one lifted expression must split the same ` |QUALIFIES — expression-lifting pins (15) |
| 87 | analyzer.rs:11755 | ``?` short-circuits a lifted expression with the first bad ` |QUALIFIES — expression-lifting pins (15) |
| 88 | analyzer.rs:11807 | `this lifted expression flattens into its own `Result`, so th` |QUALIFIES — expression-lifting pins (15) |
| 89 | analyzer.rs:11889 | ``?.` lifts an `Option`, a `Result`, or a type opting in with` |QUALIFIES — expression-lifting pins (15) |
| 90 | analyzer.rs:11904 | ``?.` needs a container with an element type — this is {rende` |QUALIFIES — lift family (chain + region pins) |
| 91 | analyzer.rs:11935 | ``?.` on {rendered} needs a `{member_name}` method — the Lift` |QUALIFIES — lift family (chain + region pins) |
| 92 | analyzer.rs:12011 | `a bare `ret` exits a closure whose body yields {tail_rendere` |QUALIFIES — ret-checking family (B10 pins) |
| 93 | analyzer.rs:12025 | `the closure's body ends without a value, but this `ret` retu` |QUALIFIES — ret-checking family (B10 pins) |
| 94 | analyzer.rs:12036 | `this `ret` returns {value_rendered}, but the closure's body ` |QUALIFIES — ret-checking family (B10 pins) |
| 95 | analyzer.rs:12114 | ``!` on an `Option` returns `None` early, so the enclosing fu` |QUALIFIES — try/lift operator family (B11 pins) |
| 96 | analyzer.rs:12150 | ``!` returns this `Result`'s error as-is, so the error types ` |QUALIFIES — try/lift operator family (B11 pins) |
| 97 | analyzer.rs:12160 | ``!` on a `Result` returns the error early, so the enclosing ` |QUALIFIES — try/lift operator family (B11 pins) |
| 98 | analyzer.rs:12199 | ``!` needs a value implementing `Try` (an `Option`, a `Result` |QUALIFIES — try/lift operator family (B11 pins) |
| 99 | analyzer.rs:12222 | `the `Try` impl is missing `verdict`/`from_bad`` |QUALIFIES — B6 names the Try contract (user-Try pins) |
| 100 | analyzer.rs:12258 | ``!` on a `Try` type returns `from_bad(..)`, which rebuilds {` |QUALIFIES — try/lift operator family (B11 pins) |
| 101 | analyzer.rs:12321 | `match guard must be a bool, but got {}` |QUALIFIES — the guard twin of B28 (existing check) |
| 102 | analyzer.rs:12373 | `match is not exhaustive: missing {}` |QUALIFIES — names the missing variants / the catch-all steer (match pins) |
| 103 | analyzer.rs:12385 | `match is not exhaustive: add a catch-all `_` leg` |QUALIFIES — names the missing variants / the catch-all steer (match pins) |
| 104 | analyzer.rs:12428 | `match legs have mismatched types: expected {}, but got {} in` |QUALIFIES — leg-body anchors (E7 pass-1 pins) |
| 105 | analyzer.rs:12479 | `unknown struct: {}` |QUALIFIES — B4 import steer (batch 7); pin an_unknown_struct_steers… |
| 106 | analyzer.rs:12495 | `cannot initialize a non-struct: {}` |QUALIFIES — B6 |
| 107 | analyzer.rs:12506 | `Expected {} {}, but got {} instead.` |QUALIFIES — arity anchors at the arguments (they ARE the problem) |
| 108 | analyzer.rs:12536 | `struct '{}' has no field '{}'` |QUALIFIES — field-anchored (E7 pins) |
| 109 | analyzer.rs:12564 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 110 | analyzer.rs:12704 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 111 | analyzer.rs:12718 | `subject is not a struct: {}` |QUALIFIES — B2 renders the type |
| 112 | analyzer.rs:12770 | `struct '{}' has no field '{}'` |QUALIFIES — field-anchored (E7 pins) |
| 113 | analyzer.rs:12788 | `cannot access field '{}' on type {}` |QUALIFIES — B2 both sides; member-anchored |
| 114 | analyzer.rs:12853 | `cannot index this List: its element type is never determined` |QUALIFIES — B4 annotate steer (B16 pins) |
| 115 | analyzer.rs:12874 | `index {literal_index} is out of range for an array of length` |QUALIFIES — literal-OOB compile error (fixed-arrays pins) |
| 116 | analyzer.rs:12894 | `cannot index {} (only a `List` or `[T; n]` array is indexabl` |QUALIFIES — B6 names the indexable types |
| 117 | analyzer.rs:12992 | `cannot find '{}' in this scope` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 118 | analyzer.rs:13014 | ``use` requires a namespace (a module or an enum)` |QUALIFIES — Origin-labeled reachability chains (platform-coloring pins) |
| 119 | analyzer.rs:13030 | `cannot find '{}' in the `use` path` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 120 | analyzer.rs:13083 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 121 | analyzer.rs:13100 | `cannot find '{}' in this scope` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 122 | analyzer.rs:13139 | `cannot assign to this expression` |QUALIFIES — place-model rule (assignment pins) |
| 123 | analyzer.rs:13205 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 124 | analyzer.rs:13230 | `cannot find '{}' in module '{}'` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 125 | analyzer.rs:13251 | `cannot resolve `{member_name}` here: {subject_str} is not a ` |QUALIFIES — B2 renders the subject |
| 126 | analyzer.rs:13383 | `cannot find '{}' in {}{}` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 127 | analyzer.rs:13405 | `cannot find '{}' in module '{}'` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 128 | analyzer.rs:13422 | `cannot access '{}' on an unconstrained type parameter` |QUALIFIES — const-eval family (21 pins): capability/free-variable wording with reference spans |
| 129 | analyzer.rs:13466 | `no bound of this type parameter ({}) has a member '{}'` |QUALIFIES — B6 names the bound channel (B12 pins) |
| 130 | analyzer.rs:13487 | `cannot find trait '{}'` |QUALIFIES — B4 steer added (batch 1); pin: an_unknown_trait_steers… |
| 131 | analyzer.rs:13499 | `'{}' is not a trait` |QUALIFIES — B2 renders the subject |
| 132 | analyzer.rs:13567 | `'{}' does not implement trait '{}': missing '{}'` |QUALIFIES — impl-anchored; REFINEMENT TAKEN (notes finale): renders the signature to declare + a CROSS-SOURCE note at the trait's declaration; pin a_missing_trait_member… |
| 133 | analyzer.rs:13736 | `this {construct} is `{label}`, but a condition must be `bool` |QUALIFIES — B28 pins (6) |
| 134 | analyzer.rs:13815 | ``{symbol}` takes `bool` operands; the {side} operand is `{la` |QUALIFIES — B2 names side + type (B24 pins) |
| 135 | analyzer.rs:13831 | ``bool` has no ordering — `{symbol}` models `PartialOrd`, whi` |QUALIFIES — B6 + compare steer (B24 pins) |
| 136 | analyzer.rs:13855 | ``{symbol}` compares two values of the same type, but the ` |QUALIFIES — B24 wording (pins) |
| 137 | analyzer.rs:13973 | `type '{type_name}' does not implement the `{trait_name}` ope` |QUALIFIES — B6 operator steer (B24/B25 pins) |
| 138 | analyzer.rs:13991 | `cannot find context `{name}` in this scope` |QUALIFIES — context-pass pins |
| 139 | analyzer.rs:14006 | `duplicate context `{name}` in this clause` |QUALIFIES — coverage-fence family (ambient-owner pins); B6 names run/extent rules |
| 140 | analyzer.rs:14062 | `unknown numeric suffix `{suffix}`{hint}` |QUALIFIES — B4 rename hint (i53 rename pins) |
| 141 | analyzer.rs:14129 | `the literal `{whole}` is out of range for `{name}` ({range})` |QUALIFIES — range-checked literals, B2-shaped range text (numeric-types pins) |
| 142 | analyzer.rs:14143 | `type of struct initializer could not be resolved` |DEMOTE — same suppressed-residual family as 143–145/147 (was mis-verdicted QUALIFIES); guarded by `!self.diagnostics.is_empty()`, surfaces only as the lone signal. Pin one_unresolved_name_does_not_cascade_across_many_use_sites |
| 143 | analyzer.rs:14148 | `type of accessor subject could not be resolved` |DEMOTE — post-solve residual, guarded by `!self.diagnostics.is_empty()` (a symptom of an upstream failure, B5); surfaces only as the lone signal. Multi-use-site pin one_unresolved_name_does_not_cascade_across_many_use_sites |
| 144 | analyzer.rs:14153 | `type of variable '{}' could not be resolved` |DEMOTE — post-solve residual, guarded by `!self.diagnostics.is_empty()` (B5); surfaces only as the lone signal. Multi-use-site pin one_unresolved_name_does_not_cascade_across_many_use_sites |
| 145 | analyzer.rs:14167 | `type of function call arguments could not be resolved` |DEMOTE — post-solve residual, guarded by `!self.diagnostics.is_empty()` (B5); surfaces only as the lone signal. Multi-use-site pin one_unresolved_name_does_not_cascade_across_many_use_sites |
| 146 | analyzer.rs:14190 | `cannot index this List: its element type is never determined` |QUALIFIES — B4 annotate steer (B16 pins) |
| 147 | analyzer.rs:14210 | `type of match expression could not be resolved (subject: {})` |DEMOTE — post-solve residual, guarded (its own `residuals_are_cascade` gate, B5); surfaces only as the lone signal. Multi-use-site pin one_unresolved_name_does_not_cascade_across_many_use_sites |
| 148 | analyzer.rs:14284 | `the type of '{name}' is never fully determined: `{rendered}`` |QUALIFIES — B4 annotate steer (Map-sweep pins) |
| 149 | analyzer.rs:15270 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 150 | analyzer.rs:16224 | ``{importer}` imports `pkg::{module}`, but `{module}` is not ` |QUALIFIES — L1/E.10 module-shape rules (module_resolution pins) |
| 151 | analyzer.rs:16633 | `library at `{}` has no `lib.vl`` |QUALIFIES — L1 surface checks (workspace pins) |
| 152 | analyzer.rs:16674 | `library `{library_name}`'s base `lib.vl` re-exports `{module` |QUALIFIES — H2 body-export rule (scoped-import pins) |
| 153 | analyzer.rs:16776 | `module `{name}` is ambiguous: both `{name}.vl` and `{name}/l` |QUALIFIES — B6 names both candidates (module pins) |
| 154 | analyzer.rs:17062 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 155 | async_infer.rs:190 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 156 | async_infer.rs:280 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 157 | macros.rs:369 | `a `macro { .. }` block cannot appear inside macro code — the` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 158 | macros.rs:386 | `the `macro_std` package was not found beside `std` — macros ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 159 | macros.rs:421 | `a macro named `{name}` is already defined in this module` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 160 | macros.rs:490 | `a macro body may import only from `macro_std` — the macro wo` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 161 | macros.rs:876 | ``[service]` expanded before std::rpc's `service` macro was ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 162 | macros.rs:957 | `this `macro { .. }` block was not registered — see the file'` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 163 | macros.rs:1054 | `the built-in derive generators produced invalid vilan ({mess` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 164 | macros.rs:1073 | `no macro named `{name}` is in scope` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 165 | macros.rs:1082 | ``{name}` is a macro HELPER (its signature is not a macro sha` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 166 | macros.rs:1097 | `macro `{name}` is invocation-shaped (it takes no `Item`) — c` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 167 | macros.rs:1133 | `no macro named `{name}` is in scope` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 168 | macros.rs:1145 | ``{name}` is a macro HELPER (its signature is not a macro sha` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 169 | macros.rs:1161 | `macro `{name}` is attribute-shaped (it takes an `Item`) — us` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 170 | macros.rs:1211 | `macro expansion did not settle after {cap} rounds — the chai` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 171 | macros.rs:1235 | `{label}'s definition did not compile` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 172 | macros.rs:1257 | `{label} failed at expansion time: {message}` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 173 | macros.rs:1284 | `{label} generated invalid vilan ({message}) — the ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 174 | macros.rs:1298 | `{label} must generate a single expression here (it is ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 175 | macros.rs:1311 | `{label} generated a `macro {{ .. }}` block — macros cannot ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 176 | macros.rs:1333 | `{label} generated invalid vilan ({message}) — the ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 177 | macros.rs:1346 | `{label} generated a `macro fun` — macros cannot define ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 178 | macros.rs:1358 | `{label} generated a `macro {{ .. }}` block — macros cannot ` |QUALIFIES — expansion diagnostics site-anchored w/ output previews (macro-engine pins); analyzer errors INSIDE generated code re-anchor at the attribute (batch 5 redirect; pin a_diagnostic_in_generated_code…) |
| 179 | platform_color.rs:110 | `unknown platform pattern `{pattern_text}` in `[platform(…)]`` |QUALIFIES — B6 lists the accepted forms; pattern-anchored ([platform] fence pins) |
| 180 | platform_color.rs:232 | `` |QUALIFIES — reviewed in the batch-7 sweep: B6-shaped rule statement from a designed arc; pin via its family's suite |
| 181 | analyzer.rs:2641 | `{label} of `[derive(Wire)]` type `{type_name}` is the resourc` |QUALIFIES — C4 resource-not-Wire variant (B2 renders the resource type, B4 plain-data-handle steer); pin derive_wire_rejects_a_resource_field |
| 182 | analyzer.rs:2729 | `{label} of `[derive(Hashable)]` type `{type_name}` is the res` |QUALIFIES — C4 resource variant; pin derive_hashable_rejects_a_resource_field |
| 183 | analyzer.rs:2767 | `{label} of `[derive(PartialEq)]` type `{type_name}` is the re` |QUALIFIES — C4 resource variant; pin derive_partialeq_rejects_a_resource_field |
| 184 | analyzer.rs:2796 | ``{rendered}` implements `Drop` but is not a resource — destru` |QUALIFIES — C4 §3/§11 double-close rule (B6 + declare-`resource` steer); pins `declare it a `resource``, `is not a resource` |
| 185 | analyzer.rs:2928 | ``{}`'s `{}` declares {} type parameter(s), but `{}` declares ` |QUALIFIES — B29 conformance (B2 both counts + match steer; conformance_note = declaration control); B29 pins |
| 186 | analyzer.rs:2988 | ``{}`'s `{}` takes {} parameter(s), but `{}` declares {} — matc` |QUALIFIES — B29 arity (B2 + match-the-list steer); B29 pins |
| 187 | analyzer.rs:3015 | ``{}`'s `{}` takes no receiver / a `{}` receiver, but `{}` decl` |QUALIFIES — B29 receiver presence (B6 + give-the-receiver steer); B29 pins |
| 188 | analyzer.rs:3065 | ``{}`'s `{}` receives `{}` / parameter {} is {}, but `{}` decl` |QUALIFIES — B29 receiver/param convention; pin `match the receiver convention` |
| 189 | analyzer.rs:3089 | `parameter {position} of `{}`'s `{}` is `{actual_label}`, but ` |QUALIFIES — B29 param type (B2 both sides, anchored at the parameter — A1 verified); pin `match the declared type` |
| 190 | analyzer.rs:3132 | ``{}`'s `{}` returns `{actual_label}`, but `{}` declares `{exp` |QUALIFIES — B29 return type (B2 + match-the-return steer); pin `match the declared return type` |
| 191 | analyzer.rs:3924 | ``{container_name}` cannot hold the resource `{rendered}` — a ` |QUALIFIES — C4 native-container-resource (B6 + `Option`/struct-field steer); pin `cannot hold the resource` |
| 192 | analyzer.rs:4018 | `the resource `{rendered}` cannot be used where `any` is expec` |QUALIFIES — R12 resource-to-`any` (B2 + debug-print steer); r12_rejects_* pins |
| 193 | analyzer.rs:4145 | ``{rendered}` cannot cross a hot swap / is a generic type para` |QUALIFIES — A13 HMR transfer (B6 + stash-plain-data steer, same-file `only plain data transfers` note); `cannot cross a hot swap` pins |
| 194 | analyzer.rs:6456 | `use of `{name}` after it was moved` (+6 affine arms) |QUALIFIES — C4 affine-rule family, 7 arms one push (B6 + loan/`Option`+take steers; UseAfterMove carries a same-file `moved here` note); pins `after it was moved`, `no partial moves`, `moved on one path`, `declared outside this loop`, `cannot capture the resource`, `module-level resource` |
| 195 | analyzer.rs:6639 | ``{name}` is not move-clean when instantiated with a resource ` |QUALIFIES — R11 own-generic-leak; primary at the instantiation (A2), cross-file note into the generic body is user↔user (no std `own`-generic leaks — see E11 finding); `not move-clean` pins |
| 196 | analyzer.rs:6727 | ``{name}` is not move-clean … pass a resource to `drop<T>`` |QUALIFIES — R11 drop-forward; `not move-clean` pins |
| 197 | analyzer.rs:7290 | ``{name}` is not move-clean when instantiated with a resource ` |QUALIFIES — R11 move-violation family (per-violation summary + steer); `not move-clean` pins |
| 198 | analyzer.rs:14193 | `a `sync` closure contract is only supported on parameters` |QUALIFIES — B6 marker position (async-polymorphism A.2); pin `a `sync` closure contract is only supported on parameters` |
| 199 | analyzer.rs:24914 | ``drop` for `{subject}` is async — teardown must be synchronou` |QUALIFIES — C4 §5 sync-teardown (B6 + OwnedNursery steer); pin `teardown must be synchronous` |
| 200 | analyzer.rs:24940 | ``drop` for `{subject}` requires an ambient context — teardown` |QUALIFIES — C4 context-free teardown (B6 + hand-work-to-owner steer); pin `teardown must be context-free` |
| 201 | async_infer.rs:1208 | `this call passes an async closure that reaches `{parameter}`,` |QUALIFIES — async-polymorphism transitive sync (B6 + move-async-outside steer; cross-file note at the forwarding site is user↔user — std takes sync closures directly, so its violations are DIRECT, reported at the global check); pin `forwarded into the `sync` parameter` |
| 202 | async_infer.rs:1272 | `this call passes an async closure that reaches the host (`ext` |QUALIFIES — async-polymorphism transitive extern (B6; forwarding note user↔user as above); pin `cannot await a vilan closure` |
| 203 | async_infer.rs:1329 | `an async closure cannot adapt a trait/generic-dispatched call` |QUALIFIES — async-polymorphism dispatch refusal (B6 + bind-concretely / declare-the-param-async steer, no note); pin `cannot adapt a trait/generic-dispatched call` |
