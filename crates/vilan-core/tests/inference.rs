//! Compile-outcome tests for the type inference / generic resolution paths that
//! have been the source of recurring bugs. Each case asserts whether a source
//! compiles cleanly or fails, run through the real pipeline on a large-stack
//! worker (so a recursion bug surfaces as an error, not an aborted suite).
//!
//! `#[ignore]`d tests are KNOWN BUGS (see vilan/proposal/analyzer-refactor.md):
//! they assert the *desired* outcome, so removing `#[ignore]` when the bug is
//! fixed turns them green — that's how we track progress against the plan.

use std::path::{Path, PathBuf};

use vilan_core::{BuildOptions, Target, analyze_source, transform};

fn std_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std/src")
}

/// Compile a source through the full pipeline (analyze → context → infer →
/// transform) on a 256 MB-stack worker, matching the CLI. Returns the emitted JS
/// on a clean compile, or the diagnostics. A panic becomes an error rather than
/// aborting the test process.
fn compile(source: &str) -> Result<String, Vec<String>> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let leaked: &'static str = Box::leak(source.into_boxed_str());
                let (program, errors) = analyze_source(
                    leaked,
                    &std_root(),
                    Path::new("."),
                    Path::new("test.vl"),
                    Some(Target::Node),
                );
                match program {
                    Some(program) if errors.is_empty() => {
                        transform(&program, &BuildOptions::default())
                            .map_err(|error| vec![error.msg])
                    }
                    _ => Err(errors.into_iter().map(|error| error.msg).collect()),
                }
            }))
            .unwrap_or_else(|_| Err(vec!["compiler panicked".to_string()]))
        })
        .expect("spawn worker")
        .join()
        .unwrap_or_else(|_| {
            Err(vec![
                "compiler thread aborted (likely a stack overflow)".to_string(),
            ])
        })
}

#[track_caller]
fn assert_compiles(source: &str) {
    if let Err(errors) = compile(source) {
        panic!("expected a clean compile, got: {errors:#?}");
    }
}

#[track_caller]
fn assert_fails(source: &str) {
    assert!(
        compile(source).is_err(),
        "expected a compile error, but it compiled cleanly"
    );
}

/// The analyzer's non-fatal warning messages (e.g. unused `[must_use]` results).
fn warnings(source: &str) -> Vec<String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let leaked: &'static str = Box::leak(source.into_boxed_str());
            let (program, _errors) = analyze_source(
                leaked,
                &std_root(),
                Path::new("."),
                Path::new("test.vl"),
                Some(Target::Node),
            );
            program
                .map(|program| {
                    program
                        .warnings
                        .into_iter()
                        .map(|warning| warning.msg)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .expect("spawn worker")
        .join()
        .unwrap_or_default()
}

/// Compile, then execute the emitted JS with `node`, returning its stdout. A
/// compile failure or a non-zero exit becomes `Err`. This catches *runtime*
/// miscompiles — a program that type-checks but emits the wrong code (e.g. a
/// generic dispatch that resolves to `undefined`) — which `assert_compiles`
/// alone cannot see.
fn compile_and_run(source: &str) -> Result<String, Vec<String>> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let js = compile(source)?;
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("vilan_test_{}_{unique}.js", std::process::id()));
    std::fs::write(&path, js).map_err(|error| vec![error.to_string()])?;
    let output = std::process::Command::new("node").arg(&path).output();
    let _ = std::fs::remove_file(&path);
    match output {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Ok(output) => Err(vec![String::from_utf8_lossy(&output.stderr).into_owned()]),
        Err(error) => Err(vec![format!("could not run node: {error}")]),
    }
}

#[track_caller]
fn assert_compiles_and_runs(source: &str, expected_stdout: &str) {
    match compile_and_run(source) {
        Ok(stdout) => assert_eq!(stdout, expected_stdout, "stdout mismatch"),
        Err(errors) => panic!("expected a clean run, got: {errors:#?}"),
    }
}

// --- Regression guards (must keep passing) ----------------------------------

#[test]
fn generic_method_calls_generic_methods_on_self() {
    // Bug A (fixed): `update` calls both `self.set` and `self.get` — two generic
    // method calls on the same receiver. This used to overflow the compiler.
    assert_compiles(
        r#"
        import std::shared::Shared;
        struct Cell<T> { value: Shared<T> }
        impl Cell<type T> {
            fun new(value: T): Cell<T> { Cell { value = Shared::new(value) } }
            fun get(self): T { self.value.read() }
            fun set(self, value: T) { self.value.write() = value; }
            fun update(self, f: |T| T) { self.set(f(self.get())); }
        }
        fun main() { let c = Cell::new(0); c.update(|n| n + 1); }
        "#,
    );
}

#[test]
fn reactive_map_sub_and_set_with() {
    assert_compiles(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner };
        fun main() {
            let owner = Owner::new();
            let count = Signal::new(0);
            let doubled = count.map(|n| n * 2);
            owner.take(doubled.sub(|n| print(n)));
            count.set_with(|n| n + 1);
        }
        "#,
    );
}

#[test]
fn owner_disposes_subscriptions_across_re_renders() {
    // A2: the leak fix. Mimics `bind_each` — `source` drives re-renders; each
    // render disposes the previous rows' subscriptions (`rows.dispose()`) and
    // creates fresh ones. After several renders only the *current* rows fire, so
    // the count stays bounded (a leak would give 6, not 2).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::reactive::{ Signal, Owner };
        fun main() {
            let source = Signal::new(0);
            let data = Signal::new(0);
            let rows = Owner::new();
            let fires = Shared::new(0);
            let outer = Owner::new();
            outer.take(source.sub(|_| {
                rows.dispose();
                rows.take(data.sub(|_| { fires.write() = fires.read() + 1; }));
                rows.take(data.sub(|_| { fires.write() = fires.read() + 1; }));
            }));
            source.set(1);
            source.set(2);
            fires.write() = 0;
            data.set(99);
            print(fires.read());
        }
        "#,
        "2\n",
    );
}

#[test]
fn generic_dispatch_to_extern_impl() {
    // A trait method on a generic, dispatching to a primitive's `[extern]` impl.
    assert_compiles(
        r#"
        import std::print;
        import std::display::{ Display, format };
        fun show<T: Display>(x: T): str { x.to_string() }
        fun main() { print(format(42)); print(show("hi")); }
        "#,
    );
}

#[test]
fn return_type_only_generic() {
    // A generic fixed only by the return type (no argument binds it).
    assert_compiles(
        r#"
        import std::print;
        import std::default::Default;
        fun make<T: Default>(): T { T::default() }
        fun main() { let n: i32 = make(); print(n); }
        "#,
    );
}

#[test]
fn collection_json_roundtrip() {
    assert_compiles(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        fun main() {
            let nums: List<i32> = List::from_json("[1,2,3]");
            print(nums.to_json());
        }
        "#,
    );
}

#[test]
fn nested_generic_containers() {
    // `Option<List<i32>>` etc. — generic args nested several deep must resolve.
    assert_compiles(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        fun main() {
            let x: Option<List<i32>> = Some([1, 2, 3]);
            match x {
                Some(let list) => print(list.len()),
                None => print(0),
            }
        }
        "#,
    );
}

#[test]
fn recursion_self_and_mutual() {
    assert_compiles(
        r#"
        import std::print;
        fun fib(n: i32): i32 { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
        fun is_even(n: i32): bool { if n == 0 { true } else { is_odd(n - 1) } }
        fun is_odd(n: i32): bool { if n == 0 { false } else { is_even(n - 1) } }
        fun main() { print(fib(10)); print(is_even(4)); }
        "#,
    );
}

#[test]
fn calling_a_non_function_still_errors() {
    // A real error must still be reported (not silently swallowed).
    assert_fails(
        r#"
        struct Point { x: i32 }
        fun main() { let p = Point { x = 1 }; p(); }
        "#,
    );
}

#[test]
fn generic_struct_infers_type_arg_from_literal() {
    // A generic struct built by literal infers its parameter from the field
    // value (`Box { value = 5 }` -> `Box<i32>`), so a later method dispatches
    // against the concrete element. Previously the initializer dropped the
    // inferred arg (`Box<>`), leaving `T` abstract.
    assert_compiles(
        r#"
        import std::print;
        import std::display::Display;
        struct Box<T> { value: T }
        impl Box<type T> { fun get(self): T { self.value } }
        fun main() { let b = Box { value = 5 }; print(b.get().to_string()); }
        "#,
    );
}

#[test]
fn generic_struct_infers_type_arg_from_constructor() {
    // The same inference through a static constructor: `Box::new(5)` binds the
    // *impl's* `T` from the argument even though `new` declares no generics of
    // its own. (Bug B in disguise — `Signal::new(0).map(|n| ..)` left `n`
    // abstract only because `count` itself was an abstract `Signal<T>`.)
    assert_compiles(
        r#"
        import std::print;
        import std::display::Display;
        struct Box<T> { value: T }
        impl Box<type T> {
            fun new(value: T): Box<T> { Box { value = value } }
            fun get(self): T { self.value }
        }
        fun main() { print(Box::new(5).get().to_string()); }
        "#,
    );
}

#[test]
fn generic_call_on_closure_parameter() {
    // Bug B (fixed): a closure passed to a generic method (`count.map(|n|
    // n.to_string())`) used to type `n` as an abstract generic, so the method
    // call on it couldn't dispatch. The real cause was that `Signal::new(0)`
    // left `count` as an abstract `Signal<T>`; with construction now inferring
    // `Signal<i32>`, `n` is `i32` and `to_string` dispatches.
    assert_compiles(
        r#"
        import std::print;
        import std::reactive::Signal;
        import std::display::Display;
        fun main() {
            let count = Signal::new(0);
            let label = count.map(|n| n.to_string());
            label.sub(|s| print(s));
        }
        "#,
    );
}

#[test]
fn format_through_nested_generic() {
    // Bug C (fixed): a generic function passing its type parameter to another
    // generic call (`show<T: Display>(x) { format(x) }`) used to leave the nested
    // `format` un-monomorphized — its `value.to_string()` resolved to the empty
    // abstract `Display::to_string`, printing `undefined`. The cause was a binding
    // direction: the call reconciled argument-against-parameter, so a generic
    // argument bound *its own* constraint instead of the callee's. Reconciling
    // parameter-first binds `format`'s `U = T`, so it monomorphizes per `show`
    // instantiation.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::{ Display, format };
        fun show<T: Display>(x: T): str { format(x) }
        fun main() { print(show(7)); print(show("hi")); }
        "#,
        "7\nhi\n",
    );
}

#[test]
fn chained_derive_binds_method_generic_from_closure_return() {
    // A chained `derive` (`count.map(|n| n * 2).map(|m| format(m))`) used to
    // emit `undefined`: the first `derive<U>` left its result `Signal<U>` abstract
    // because `U` (its *own* generic) was never bound from the closure's return
    // type, so the second `derive` saw an abstract element. Method calls now bind
    // their own generics from arguments, like free-function calls do.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::Signal;
        import std::display::format;
        fun main() {
            let count = Signal::new(3);
            let label = count.map(|n| n * 2).map(|m| format(m));
            label.sub(|s| print(s));
            count.set(10);
        }
        "#,
        "6\n20\n",
    );
}

#[test]
fn format_in_closure_argument() {
    // Bug c′ (fixed): a free generic function called with an unannotated closure
    // parameter (`count.map(|n| format(n))`) emitted `undefined`. The call
    // resolved while `n` was still `Unknown` (its type lands only once `derive`
    // resolves), committed with no generic binding, and was never revisited.
    // Fixed by deferring the call while an argument is an unknown closure
    // parameter — the same rule the method-call resolver already applies to an
    // unknown closure *receiver* — so it re-resolves once `n` becomes `i32`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::Signal;
        import std::display::format;
        fun main() {
            let count = Signal::new(0);
            let label = count.map(|n| format(n));
            label.sub(|s| print(s));
            count.set(5);
        }
        "#,
        "0\n5\n",
    );
}

#[test]
fn method_closure_param_inferred_from_argument_generic() {
    // A method's own generic bound from a (nested) argument must reach its closure
    // parameters: `pick<T, K>(rows: List<List<T>>, key: |T| K, get: |T| i32)` typed
    // `|p| p.id`'s `p` as the abstract `T` until the own-generic binding ran first.
    // This is the `bind_each(source: Signal<List<T>>, |todo| todo.id, ..)` shape.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        struct P { id: i32 }
        struct Holder { tag: i32 }
        impl Holder {
            fun pick<T, K>(self, rows: List<List<T>>, key: |T| K, get: |T| i32): i32 {
                get(rows[0][0])
            }
        }
        fun main() {
            let h = Holder { tag = 0 };
            print(h.pick([[P { id = 42 }]], |p| p.id, |p| p.id).to_string());
        }
        "#,
        "42\n",
    );
}

#[test]
fn logical_or_operator() {
    // `||` is logical-or: binds looser than `&&`, short-circuits, and an empty
    // closure `|| body` still parses (it's tried before the operator).
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun boom(): bool { print("evaluated"); true }
        fun main() {
            let a = "x";
            print(a == "x" || a == "y");
            print(a == "z" || a == "y");
            print(a == "x" && false || a == "x");
            print(true || boom());
            let f = || 7;
            print(f());
        }
        "#,
        "true\nfalse\ntrue\ntrue\n7\n",
    );
}

#[test]
fn reactive_combine_variadic() {
    // The driving example: `combine` is variadic over its inputs' distinct types
    // via a mapped-tuple parameter, yielding a `Signal` of the tuple that
    // recomputes when any input changes. The consumer destructures the tuple with
    // a closure tuple binder.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        import std::reactive::{ Signal, combine };
        fun main() {
            let a = Signal::new(1);
            let b = Signal::new("x");
            let c = Signal::new(true);
            let combined: Signal<(i32, str, bool)> = combine((a, b, c));
            combined.sub(|(n, s, flag)| print(i"{n.to_string()} {s} {flag}"));
            a.set(2);
            b.set("y");
        }
        "#,
        "1 x true\n2 x true\n2 y true\n",
    );
}

#[test]
fn tuple_comprehension_over_mapped_source() {
    // A tuple comprehension `(x in xs => e)` maps each element of a mapped-tuple
    // source through the body, typing as `(U in T: <body>)`. Here `source.len()`
    // collapses `(List<i32>, List<str>)` to `(i32, str) = T`. Lowers to a runtime
    // `.map`, so it's arity-independent.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun lengths<T: (2..)>(sources: (U in T: List<U>)): T {
            (source in sources => source.len())
        }
        fun main() {
            let (a, b) = lengths(([1, 2, 3], ["a", "b"]));
            print(i"{a.to_string()} {b.to_string()}");
        }
        "#,
        "3 2\n",
    );
}

#[test]
fn mapped_tuple_forward_expansion() {
    // A mapped tuple type with a concrete source expands element-wise:
    // `(U in (i32, str): List<U>)` is `(List<i32>, List<str>)`, so each binding
    // dispatches concretely.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun main() {
            let pair: (U in (i32, str): List<U>) = ([1, 2], ["x", "y", "z"]);
            let (nums, strs) = pair;
            print(i"{nums.len().to_string()} {strs.len().to_string()}");
        }
        "#,
        "2 3\n",
    );
}

#[test]
fn mapped_tuple_inverted_inference() {
    // A generic function over a mapped parameter infers the source tuple `T` from
    // the argument by inverting the template per element: `id(([1,2,3], ["a","b"]))`
    // binds `T = (i32, str)`, so the result mapped type re-expands to
    // `(List<i32>, List<str>)`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun id<T: (2..)>(sources: (U in T: List<U>)): (U in T: List<U>) { sources }
        fun main() {
            let (nums, strs) = id(([1, 2, 3], ["a", "b"]));
            print(i"{nums.len().to_string()} {strs.len().to_string()}");
        }
        "#,
        "3 2\n",
    );
}

#[test]
fn tuple_arity_bounds_parse() {
    // The tuple-bound grammar — `(..)`, `(2..)`, `(..10)`, and a per-element
    // bound `(2..: Display)` — parses and the parameter behaves as a generic
    // tuple. (Arity isn't enforced, mirroring trait bounds, which aren't either.)
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun any<T: (..)>(x: T): T { x }
        fun two<T: (2..)>(x: T): T { x }
        fun small<T: (..10)>(x: T): T { x }
        fun shown<T: (2..: Display)>(x: T): T { x }
        fun main() {
            let (a, b) = two((1, 2));
            let (c, d, e) = any((3, 4, 5));
            print(i"{a.to_string()} {b.to_string()} {c.to_string()} {d.to_string()} {e.to_string()}");
        }
        "#,
        "1 2 3 4 5\n",
    );
}

#[test]
fn nested_tuple_flat_lowering() {
    // A nested tuple stores flat (`((1,2),3)` -> `[1,2,3]`), so a matching nested
    // pattern reads flat offsets and a sub-tuple capture reslices — all behaviorally
    // transparent. Distinct types are preserved: the pattern must match the nesting.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun main() {
            let a = (1, 2);
            let b = (a, 3);
            let ((x, y), z) = b;
            print(i"{x.to_string()} {y.to_string()} {z.to_string()}");
            let (pair, last) = b;
            let (pa, pb) = pair;
            print(i"{pa.to_string()} {pb.to_string()} {last.to_string()}");
        }
        "#,
        "1 2 3\n1 2 3\n",
    );
}

#[test]
fn parameter_tuple_destructuring() {
    // A tuple binder in parameter position — both a function parameter
    // (`fun f((a, b): T)`) and a closure parameter (`|(a, b)|`) — destructures,
    // typing each binding from the matched tuple element.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun sum_pair((a, b): (i32, i32)): i32 { a + b }
        fun apply(pair: (i32, str), f: |(i32, str)| str): str { f(pair) }
        fun main() {
            print(sum_pair((3, 4)).to_string());
            print(apply((7, "x"), |(n, label)| i"{n.to_string()}{label}"));
        }
        "#,
        "7\n7x\n",
    );
}

#[test]
fn nested_parameter_tuple_destructuring() {
    // A nested tuple binder in a closure parameter, dispatched through a generic
    // reactive `derive` so the parameter type is inferred, not annotated.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun main() {
            let f = |(a, (b, c)): (i32, (i32, str))| i"{a.to_string()} {b.to_string()} {c}";
            print(f((1, (2, "z"))));
        }
        "#,
        "1 2 z\n",
    );
}

#[test]
fn let_tuple_destructuring() {
    // `let (a, b, c) = tuple` destructures, typing each binding from the tuple's
    // element types (so a method call on a binding dispatches concretely).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun pair(): (i32, str) { (7, "x") }
        fun main() {
            let (a, (b, c)) = (1, (2, 3));
            let (n, label) = pair();
            print(i"{a} {b} {c} {n.to_string()} {label}");
        }
        "#,
        "1 2 3 7 x\n",
    );
}

// --- Transparent references (implicit place, explicit value) ----------------

#[test]
fn transparent_references_write_through() {
    // R5: assigning *through* a view writes to its referent with no `*` — a view
    // binding, a `&mut` parameter, a re-borrow, a `borrows`-returning call, and a
    // captured `Option<&mut T>`, for plain `=` and compound `+=` / `/=`. Reading a
    // view as a value keeps its explicit `*`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        fun add_ten(x: &mut i32) { x += 10; }
        fun same(x: &mut i32): &mut i32 borrows x { x }
        struct Cell { value: i32 }
        impl Cell { fun slot(&mut self): Option<&mut i32> { Some(&mut self.value) } }
        fun main() {
            mut a: i32 = 10;
            let b: &mut i32 = &mut a;
            let c: &mut i32 = b;
            b = 20;
            print(i"{a} {*b} {*c}");
            add_ten(&mut a);
            print(i"{a} {*b}");
            add_ten(b);
            print(i"{a} {*b}");
            same(c) /= 10;
            print(i"{a} {*b}");
            mut cell = Cell { value = 100 };
            match cell.slot() {
                Some(let s) => { s += 5 }
                None => {}
            }
            print(cell.value);
        }
        "#,
        "20 20 20\n30 30\n40 40\n4 4\n105\n",
    );
}

#[test]
fn transparent_references_reject_deref_assignment() {
    // R6: `*` is value extraction (an rvalue) and may not be an assignment
    // target — write `v = …`, not `*v = …`.
    assert_fails(
        r#"
        fun main() { mut a = 5; let v: &mut i32 = &mut a; *v = 9; }
        "#,
    );
}

#[test]
fn transparent_references_reject_mut_view_binding() {
    // R7: a view binding cannot be `mut` — a view cannot be rebound.
    assert_fails(
        r#"
        fun main() { mut a = 5; mut v: &mut i32 = &mut a; v = 9; }
        "#,
    );
}

#[test]
fn transparent_references_reject_view_into_value_binding() {
    // R1: a value annotation cannot bind a view — write `*` to copy the value out.
    assert_fails(
        r#"
        fun main() { mut a = 5; let v: &mut i32 = &mut a; let b: i32 = v; }
        "#,
    );
}

#[test]
fn transparent_references_reject_value_into_view_binding() {
    // R1: a view annotation (`&mut T`) cannot bind a value.
    assert_fails(
        r#"
        fun main() { mut a = 5; let v: &mut i32 = &mut a; let b: &mut i32 = *v; }
        "#,
    );
}

// --- A1: `Shared::write(): &mut T borrows self` -----------------------------

#[test]
fn shared_write_view_rebinds_and_mutates_through_handles() {
    // Writing a whole value through the view rebinds the cell's slot, so every
    // handle (a clone) sees it; a method call mutates in place. The rebind must
    // NOT merge — the old aggregate-view `Object.assign` path would have left a
    // stale tail (len 3 then 4 instead of 1 then 2).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        fun main() {
            let a: Shared<List<i32>> = Shared::new([1, 2, 3]);
            let b = a.clone();
            a.write() = [9];
            print(b.read().len());
            a.write().push(8);
            print(b.read().len());
        }
        "#,
        "1\n2\n",
    );
}

#[test]
fn own_parameter_is_a_mutable_copy() {
    // `own x: T` consumes a copy the callee may mutate freely — reassign a scalar,
    // or rebind an aggregate — without affecting the caller (an aggregate is
    // cloned at the call site). Reassigning an `own` parameter used to be rejected
    // ("cannot assign to this expression"); it is now allowed.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun bump(own x: i32): i32 { x += 1; x }
        fun grow(own xs: List<i32>): i32 { xs = [7, 8, 9, 10]; xs.len() }
        fun main() {
            mut a = 10;
            print(bump(a)); // 11
            print(a);       // 10 — caller untouched
            mut list = [1, 2];
            print(grow(list)); // 4
            print(list.len()); // 2 — caller untouched
        }
        "#,
        "11\n10\n4\n2\n",
    );
}

#[test]
fn shared_write_is_a_view_not_a_value() {
    // `write()` returns a view (`&mut T`), so binding its result to a value slot
    // is rejected (transparent references R1) — use `read()` or `*`.
    assert_fails(
        r#"
        import std::shared::Shared;
        fun main() { let c = Shared::new(5); let x: i32 = c.write(); }
        "#,
    );
}

// --- R8: no implicit borrow at the call site -------------------------------

#[test]
fn r8_explicit_borrow_and_reborrow() {
    // A `&`/`&mut` parameter takes an explicit `&[mut] place`, or an existing
    // view forwarded (re-borrowed) — both compile.
    assert_compiles(
        r#"
        fun bump(x: &mut i32) { x += 1; }
        fun via(y: &mut i32) { bump(y); }
        fun main() { mut a = 0; bump(&mut a); via(&mut a); }
        "#,
    );
}

#[test]
fn r8_method_receiver_is_implicitly_borrowed() {
    // R8 exempts the `self` receiver: `c.inc()` on a `&mut self` method needs no
    // `&mut c` at the call site.
    assert_compiles(
        r#"
        struct C { v: i32 }
        impl C { fun inc(&mut self) { self.v = self.v + 1; } }
        fun main() { mut c = C { v = 0 }; c.inc(); }
        "#,
    );
}

#[test]
fn r8_reject_implicit_borrow() {
    // Passing a bare value place to a `&mut` parameter is rejected — there is no
    // implicit borrow (a scalar would otherwise emit a broken `(base,key)` read).
    assert_fails(
        r#"
        fun bump(x: &mut i32) { x += 1; }
        fun main() { mut a = 0; bump(a); }
        "#,
    );
}

// --- [must_use] -------------------------------------------------------------

#[test]
fn must_use_dropped_result_warns() {
    // A dropped `[must_use]` result (a bare statement) is a warning.
    let messages = warnings(
        r#"
        [must_use]
        fun make(): i32 { 42 }
        fun main() { make(); }
        "#,
    );
    assert!(
        messages.iter().any(|message| message.contains("must_use")),
        "expected a must_use warning, got {messages:?}"
    );
}

#[test]
fn must_use_consumed_result_no_warning() {
    // Binding, discarding with `let _`, or passing as an argument consumes the
    // result — no warning.
    let messages = warnings(
        r#"
        import std::print;
        [must_use]
        fun make(): i32 { 42 }
        fun consume(x: i32) { print(x); }
        fun main() {
            let a = make();
            let _ = make();
            consume(make());
            print(a);
        }
        "#,
    );
    assert!(
        messages.is_empty(),
        "expected no warnings, got {messages:?}"
    );
}
