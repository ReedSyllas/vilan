# The diagnostics ledger (diagnostics-standard.md §5)

Every `diagnostics.push` site, its message head, and its audit verdict.
Verdicts: **QUALIFIES** (rules met, pin cited) · **RE-ANCHOR** (fails
A-rules) · **REWORD** (fails B-rules) · **DEMOTE** (cascade) ·
**NOTE-NEEDED** (wants C3) · *(blank = unreviewed)*. Line numbers are the
snapshot at generation; the message head is the stable key. Updated per
audit batch, in the batch's commit.

| # | Site | Message head | Verdict |
|---|------|--------------|---------|
| 1 | analyzer.rs:1856 | `` | |
| 2 | analyzer.rs:1960 | `{label} of `[derive(Wire)]` type `{type_name}` is `{rendered` | |
| 3 | analyzer.rs:2043 | `{label} of `[derive(Hashable)]` type `{type_name}` is `{rend` | |
| 4 | analyzer.rs:2106 | `{label} of `[rpc]` method `{method_name}` is `{rendered}`, ` | |
| 5 | analyzer.rs:2118 | `{label} of `[rpc]` method `{method_name}` must declare a Wir` | |
| 6 | analyzer.rs:2150 | `{label} is `[expose]`d, but its element `{rendered}` is not ` | |
| 7 | analyzer.rs:2165 | `{label} is `[expose]`d, but its type `{rendered}` is not a ` | |
| 8 | analyzer.rs:3167 | `a view cannot escape its scope: it may not be returned, stor` | |
| 9 | analyzer.rs:3872 | `an async function cannot take {form} parameters: the view wo` | |
| 10 | analyzer.rs:3946 | `` | |
| 11 | analyzer.rs:3998 | `an async closure cannot capture the view '{name}': the captu` | |
| 12 | analyzer.rs:4420 | `cannot reseat a view to '{name}', which goes out of scope be` | |
| 13 | analyzer.rs:4700 | `cannot mutate immutable '{name}'; {advice} to allow mutation` | |
| 14 | analyzer.rs:4758 | `cannot mutate immutable '{name}'; {advice} to allow mutation` | |
| 15 | analyzer.rs:4864 | `a view can't be read as a value here; write `*` to copy the ` | |
| 16 | analyzer.rs:4894 | `cannot take a writable view of immutable '{name}'; {advice} ` | |
| 17 | analyzer.rs:4938 | `view binding '{name}' cannot be `mut`: a view cannot be rebo` | |
| 18 | analyzer.rs:4958 | `` | |
| 19 | analyzer.rs:5019 | `a `{kind}` parameter takes a view; pass `{kind} <place>` (th` | |
| 20 | analyzer.rs:5688 | `an array length must be a non-negative integer literal ` | |
| 21 | analyzer.rs:5711 | `the `?` lifts this condition to an `Option`/`Result`, which ` | |
| 22 | analyzer.rs:5731 | `vilan has no const declarations — write `let x = const ..`` | |
| 23 | analyzer.rs:5772 | `` | |
| 24 | analyzer.rs:5827 | `a tuple position is a bare number (`.0`, `.1`) — drop the ` | |
| 25 | analyzer.rs:5885 | `expected a method name after `.`` | |
| 26 | analyzer.rs:5895 | `expected a field or method name after `.`` | |
| 27 | analyzer.rs:5959 | `a `[T; n]` array type isn't a value; write an array ` | |
| 28 | analyzer.rs:6081 | `a `macro fun` must be a top-level item` | |
| 29 | analyzer.rs:6155 | `the invocation `macro {name}(..)` was not expanded — splice ` | |
| 30 | analyzer.rs:6184 | `this `macro { .. }` block was not expanded — a block cannot ` | |
| 31 | analyzer.rs:6201 | ``export` is a module-level item and cannot appear inside a b` | |
| 32 | analyzer.rs:6326 | `an `external` function cannot have a body` | |
| 33 | analyzer.rs:6375 | `function '{}' must have a body or be declared `external`` | |
| 34 | analyzer.rs:6532 | `a bare `?` (expression lifting) is not supported in this pos` | |
| 35 | analyzer.rs:6548 | ``?` lifts nothing here — the region is the whole expression;` | |
| 36 | analyzer.rs:6566 | ``!` cannot run after a `?` inside a lifted expression — it ` | |
| 37 | analyzer.rs:6636 | ``!` requires the nearest enclosing function to declare an `O` | |
| 38 | analyzer.rs:6697 | `a `context` clause is only supported on a closure type` | |
| 39 | analyzer.rs:6788 | `a destructuring `let` requires a value` | |
| 40 | analyzer.rs:6802 | `cannot assign through `*`: a view is written through directl` | |
| 41 | analyzer.rs:6816 | `a lifted chain (`?.`) is not an assignment target` | |
| 42 | analyzer.rs:6860 | `struct '{}' must declare a body or be declared `external`` | |
| 43 | analyzer.rs:7310 | `a closure type is not valid here (expected an expression)` | |
| 44 | analyzer.rs:7318 | `a `context`-typed closure type is not valid here (expected a` | |
| 45 | analyzer.rs:7328 | `an `async` closure type is not valid here (expected an expre` | |
| 46 | analyzer.rs:7337 | `a mapped tuple type is not valid here (expected an expressio` | |
| 47 | analyzer.rs:7427 | `a `context` clause is only supported on a closure type` | |
| 48 | analyzer.rs:7715 | `cannot find '{}' in this scope` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 49 | analyzer.rs:7726 | `'{}' is not an enum variant` | |
| 50 | analyzer.rs:7764 | `variant '{}' does not belong to the matched enum` | |
| 51 | analyzer.rs:7773 | `cannot match an enum variant against type {}` | |
| 52 | analyzer.rs:7807 | `variant '{}' carries {} {}, but the pattern has {}` | |
| 53 | analyzer.rs:7867 | `this pattern binds {} {}, but the array's length is {}` | |
| 54 | analyzer.rs:7884 | `cannot destructure {rendered} as a fixed array — ` | |
| 55 | analyzer.rs:7915 | `literal pattern of type {} cannot match type {}` | |
| 56 | analyzer.rs:8025 | `an `async` closure type is only supported on parameters and ` | |
| 57 | analyzer.rs:8033 | `a `context` clause is only supported on a parameter's closur` | |
| 58 | analyzer.rs:8740 | `this array literal has {} element{}, but its type is `[_; {l` | |
| 59 | analyzer.rs:8770 | `Expected {expected} (this literal's element type), but got {` | |
| 60 | analyzer.rs:8847 | `Expected {expected} (this literal's element type), but got {` | |
| 61 | analyzer.rs:10184 | ``self` import has no enclosing namespace` | |
| 62 | analyzer.rs:10204 | `cannot find module '{}' to import` |QUALIFIES — A4 segment anchor (E7 pass 1 pins) |
| 63 | analyzer.rs:10240 | `cannot find '{}' in the imported path` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 64 | analyzer.rs:10483 | `Expected {} {}, but got {} instead.` | |
| 65 | analyzer.rs:10549 | `Expected {}, but got {} instead.{}` |QUALIFIES — B2 + B3 note (B13 first-call origin); pin a_conflicting_later_call… |
| 66 | analyzer.rs:10577 | `Expected {} {}, but got {} instead.` | |
| 67 | analyzer.rs:10605 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 68 | analyzer.rs:10644 | `Expected {} {}, but got {} instead.` | |
| 69 | analyzer.rs:10722 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 70 | analyzer.rs:10776 | `cannot call '{name}': it is a struct, not a function — const` | |
| 71 | analyzer.rs:10803 | `cannot call a non-function value` | |
| 72 | analyzer.rs:10886 | `{} has no method '{}'` | |
| 73 | analyzer.rs:10895 | ``len` takes no arguments` | |
| 74 | analyzer.rs:11186 | `{} has no method '{}'{}` | |
| 75 | analyzer.rs:11200 | `cannot call method '{}' on {}` | |
| 76 | analyzer.rs:11213 | `cannot call '{member_name}' on a value of bare trait type ` | |
| 77 | analyzer.rs:11248 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 78 | analyzer.rs:11288 | `Expected {} {}, but got {} instead.` | |
| 79 | analyzer.rs:11336 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 80 | analyzer.rs:11376 | `a tuple comprehension's source must be a mapped tuple, got {` | |
| 81 | analyzer.rs:11454 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 82 | analyzer.rs:11490 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 83 | analyzer.rs:11554 | `Expected {}, but got {} instead.` |QUALIFIES — B2; reassignments carry the B3 initializer note; pins a_reassignment_mismatch…, an_annotated_variables… |
| 84 | analyzer.rs:11642 | ``?.` flattens into the chain's own `Result`, so the error ty` | |
| 85 | analyzer.rs:11711 | `a bare `?` lifts an `Option` or a `Result` — this is {render` | |
| 86 | analyzer.rs:11729 | `every `?` in one lifted expression must split the same ` | |
| 87 | analyzer.rs:11755 | ``?` short-circuits a lifted expression with the first bad ` | |
| 88 | analyzer.rs:11807 | `this lifted expression flattens into its own `Result`, so th` | |
| 89 | analyzer.rs:11889 | ``?.` lifts an `Option`, a `Result`, or a type opting in with` | |
| 90 | analyzer.rs:11904 | ``?.` needs a container with an element type — this is {rende` | |
| 91 | analyzer.rs:11935 | ``?.` on {rendered} needs a `{member_name}` method — the Lift` | |
| 92 | analyzer.rs:12011 | `a bare `ret` exits a closure whose body yields {tail_rendere` | |
| 93 | analyzer.rs:12025 | `the closure's body ends without a value, but this `ret` retu` | |
| 94 | analyzer.rs:12036 | `this `ret` returns {value_rendered}, but the closure's body ` | |
| 95 | analyzer.rs:12114 | ``!` on an `Option` returns `None` early, so the enclosing fu` | |
| 96 | analyzer.rs:12150 | ``!` returns this `Result`'s error as-is, so the error types ` | |
| 97 | analyzer.rs:12160 | ``!` on a `Result` returns the error early, so the enclosing ` | |
| 98 | analyzer.rs:12199 | ``!` needs a value implementing `Try` (an `Option`, a `Result` | |
| 99 | analyzer.rs:12222 | `the `Try` impl is missing `verdict`/`from_bad`` | |
| 100 | analyzer.rs:12258 | ``!` on a `Try` type returns `from_bad(..)`, which rebuilds {` | |
| 101 | analyzer.rs:12321 | `match guard must be a bool, but got {}` | |
| 102 | analyzer.rs:12373 | `match is not exhaustive: missing {}` | |
| 103 | analyzer.rs:12385 | `match is not exhaustive: add a catch-all `_` leg` | |
| 104 | analyzer.rs:12428 | `match legs have mismatched types: expected {}, but got {} in` | |
| 105 | analyzer.rs:12479 | `unknown struct: {}` | |
| 106 | analyzer.rs:12495 | `cannot initialize a non-struct: {}` | |
| 107 | analyzer.rs:12506 | `Expected {} {}, but got {} instead.` | |
| 108 | analyzer.rs:12536 | `struct '{}' has no field '{}'` | |
| 109 | analyzer.rs:12564 | `Expected {}, but got {} instead.` |QUALIFIES — B2 (both sides rendered), value-anchored (A4) |
| 110 | analyzer.rs:12704 | `` | |
| 111 | analyzer.rs:12718 | `subject is not a struct: {}` | |
| 112 | analyzer.rs:12770 | `struct '{}' has no field '{}'` | |
| 113 | analyzer.rs:12788 | `cannot access field '{}' on type {}` | |
| 114 | analyzer.rs:12853 | `cannot index this List: its element type is never determined` | |
| 115 | analyzer.rs:12874 | `index {literal_index} is out of range for an array of length` | |
| 116 | analyzer.rs:12894 | `cannot index {} (only a `List` or `[T; n]` array is indexabl` | |
| 117 | analyzer.rs:12992 | `cannot find '{}' in this scope` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 118 | analyzer.rs:13014 | ``use` requires a namespace (a module or an enum)` | |
| 119 | analyzer.rs:13030 | `cannot find '{}' in the `use` path` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 120 | analyzer.rs:13083 | `` | |
| 121 | analyzer.rs:13100 | `cannot find '{}' in this scope` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 122 | analyzer.rs:13139 | `cannot assign to this expression` | |
| 123 | analyzer.rs:13205 | `` | |
| 124 | analyzer.rs:13230 | `cannot find '{}' in module '{}'` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 125 | analyzer.rs:13251 | `cannot resolve `{member_name}` here: {subject_str} is not a ` | |
| 126 | analyzer.rs:13383 | `cannot find '{}' in {}{}` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 127 | analyzer.rs:13405 | `cannot find '{}' in module '{}'` |QUALIFIES — B4 steer added (batch 1); pins: an_unknown_value_steers…, an_unknown_name_gets_no_bogus_steer, B.27 family |
| 128 | analyzer.rs:13422 | `cannot access '{}' on an unconstrained type parameter` | |
| 129 | analyzer.rs:13466 | `no bound of this type parameter ({}) has a member '{}'` | |
| 130 | analyzer.rs:13487 | `cannot find trait '{}'` |QUALIFIES — B4 steer added (batch 1); pin: an_unknown_trait_steers… |
| 131 | analyzer.rs:13499 | `'{}' is not a trait` | |
| 132 | analyzer.rs:13567 | `'{}' does not implement trait '{}': missing '{}'` | |
| 133 | analyzer.rs:13736 | `this {construct} is `{label}`, but a condition must be `bool` | |
| 134 | analyzer.rs:13815 | ``{symbol}` takes `bool` operands; the {side} operand is `{la` | |
| 135 | analyzer.rs:13831 | ``bool` has no ordering — `{symbol}` models `PartialOrd`, whi` | |
| 136 | analyzer.rs:13855 | ``{symbol}` compares two values of the same type, but the ` | |
| 137 | analyzer.rs:13973 | `type '{type_name}' does not implement the `{trait_name}` ope` | |
| 138 | analyzer.rs:13991 | `cannot find context `{name}` in this scope` |QUALIFIES — context-pass pins |
| 139 | analyzer.rs:14006 | `duplicate context `{name}` in this clause` | |
| 140 | analyzer.rs:14062 | `unknown numeric suffix `{suffix}`{hint}` | |
| 141 | analyzer.rs:14129 | `the literal `{whole}` is out of range for `{name}` ({range})` | |
| 142 | analyzer.rs:14143 | `type of struct initializer could not be resolved` | |
| 143 | analyzer.rs:14148 | `type of accessor subject could not be resolved` | |
| 144 | analyzer.rs:14153 | `type of variable '{}' could not be resolved` | |
| 145 | analyzer.rs:14167 | `type of function call arguments could not be resolved` | |
| 146 | analyzer.rs:14190 | `cannot index this List: its element type is never determined` | |
| 147 | analyzer.rs:14210 | `type of match expression could not be resolved (subject: {})` | |
| 148 | analyzer.rs:14284 | `the type of '{name}' is never fully determined: `{rendered}`` | |
| 149 | analyzer.rs:15270 | `` | |
| 150 | analyzer.rs:16224 | ``{importer}` imports `pkg::{module}`, but `{module}` is not ` | |
| 151 | analyzer.rs:16633 | `library at `{}` has no `lib.vl`` | |
| 152 | analyzer.rs:16674 | `library `{library_name}`'s base `lib.vl` re-exports `{module` | |
| 153 | analyzer.rs:16776 | `module `{name}` is ambiguous: both `{name}.vl` and `{name}/l` | |
| 154 | analyzer.rs:17062 | `` | |
| 155 | async_infer.rs:190 | `` | |
| 156 | async_infer.rs:280 | `` | |
| 157 | macros.rs:369 | `a `macro { .. }` block cannot appear inside macro code — the` | |
| 158 | macros.rs:386 | `the `macro_std` package was not found beside `std` — macros ` | |
| 159 | macros.rs:421 | `a macro named `{name}` is already defined in this module` | |
| 160 | macros.rs:490 | `a macro body may import only from `macro_std` — the macro wo` | |
| 161 | macros.rs:876 | ``[service]` expanded before std::rpc's `service` macro was ` | |
| 162 | macros.rs:957 | `this `macro { .. }` block was not registered — see the file'` | |
| 163 | macros.rs:1054 | `the built-in derive generators produced invalid vilan ({mess` | |
| 164 | macros.rs:1073 | `no macro named `{name}` is in scope` | |
| 165 | macros.rs:1082 | ``{name}` is a macro HELPER (its signature is not a macro sha` | |
| 166 | macros.rs:1097 | `macro `{name}` is invocation-shaped (it takes no `Item`) — c` | |
| 167 | macros.rs:1133 | `no macro named `{name}` is in scope` | |
| 168 | macros.rs:1145 | ``{name}` is a macro HELPER (its signature is not a macro sha` | |
| 169 | macros.rs:1161 | `macro `{name}` is attribute-shaped (it takes an `Item`) — us` | |
| 170 | macros.rs:1211 | `macro expansion did not settle after {cap} rounds — the chai` | |
| 171 | macros.rs:1235 | `{label}'s definition did not compile` | |
| 172 | macros.rs:1257 | `{label} failed at expansion time: {message}` | |
| 173 | macros.rs:1284 | `{label} generated invalid vilan ({message}) — the ` | |
| 174 | macros.rs:1298 | `{label} must generate a single expression here (it is ` | |
| 175 | macros.rs:1311 | `{label} generated a `macro {{ .. }}` block — macros cannot ` | |
| 176 | macros.rs:1333 | `{label} generated invalid vilan ({message}) — the ` | |
| 177 | macros.rs:1346 | `{label} generated a `macro fun` — macros cannot define ` | |
| 178 | macros.rs:1358 | `{label} generated a `macro {{ .. }}` block — macros cannot ` | |
| 179 | platform_color.rs:110 | `unknown platform pattern `{pattern_text}` in `[platform(…)]`` | |
| 180 | platform_color.rs:232 | `` | |
