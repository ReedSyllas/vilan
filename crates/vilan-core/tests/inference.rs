//! Compile-outcome tests for the type inference / generic resolution paths that
//! have been the source of recurring bugs. Each case asserts whether a source
//! compiles cleanly or fails, run through the real pipeline on a large-stack
//! worker (so a recursion bug surfaces as an error, not an aborted suite).
//!
//! `#[ignore]`d tests are KNOWN BUGS (see vilan/proposal/analyzer-refactor.md):
//! they assert the *desired* outcome, so removing `#[ignore]` when the bug is
//! fixed turns them green — that's how we track progress against the plan.

use std::path::{Path, PathBuf};

use vilan_core::{BuildOptions, PackageSpec, Platform, Workspace, analyze_source, transform};

fn std_spec() -> PackageSpec {
    vilan_core::manifest::resolve_std(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"),
    )
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
                    &std_spec(),
                    Path::new("."),
                    Path::new("test.vl"),
                    Some(Platform::default()),
                    &Workspace::default(),
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
                &std_spec(),
                Path::new("."),
                Path::new("test.vl"),
                Some(Platform::default()),
                &Workspace::default(),
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

#[test]
fn enum_constructor_propagates_expected_type_to_payload() {
    // Bidirectional inference (B1): a constructor argument is typed against the
    // *expected* enum's arguments, not the abstract parameter. `Ok(Option::from_json
    // (t))` in a `Result<Option<User>, str>` context types `from_json` against
    // `Option<User>`, so it round-trips. (Was a generic-binding-flow bug.)
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        [derive(Json)] struct User { id: i32, name: str }
        fun main() {
            let decoded: Result<Option<User>, str> =
                Ok(Option::from_json("{\"id\":1,\"name\":\"Ada\"}"));
            match decoded {
                Ok(Some(let u)) => print(u.name),
                Ok(None) => print("none"),
                Err(let e) => print(e),
            }
        }
        "#,
        "Ada\n",
    );
}

// --- Known bugs: generic-binding flow (backlog B1, see proposal/type-solver.md) ---
//
// These assert the *desired* behaviour and are `#[ignore]`d because they currently
// produce `undefined` — the two remaining faces of the generic-binding-flow class.
// Remove `#[ignore]` as each lands.

#[test]
fn generic_field_method_dispatch_runs() {
    // `(self.inner).handle(x)` on a generic-bounded field. Field access now
    // substitutes the struct's declared field generic through the subject's actual
    // arguments (`resolve_field_accessor`), so `self.inner` carries the receiver's
    // `T` id rather than the struct definition's — the dispatch binding composes
    // through `current_substitution` and emits the concrete `Doubler::handle`
    // instead of the empty abstract trait method.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Handler { fun handle(self, x: i32): i32; }
        struct Doubler { factor: i32 }
        impl Doubler with Handler { fun handle(self, x: i32): i32 { x * self.factor } }
        struct Wrap<T: Handler> { inner: T }
        impl Wrap<type T: Handler> {
            fun run(self, x: i32): i32 { (self.inner).handle(x) }
        }
        fun main() { let w = Wrap { inner = Doubler { factor = 3 } }; print(w.run(7)); }
        "#,
        "21\n",
    );
}

#[test]
fn generic_field_from_a_variable_dispatches() {
    // Same as above but the field value is a *variable*, so the `Wrap` initializer
    // (priority 1) is reached before `d` is grounded (priority 10) and defers. It
    // must not publish a type while deferred (the unbound parameter would fall back
    // to its constraint, `Wrap<Handler>`), and a pending generic initializer infers
    // as `Unresolved` so `let w = ..` defers instead of grounding on an abstract
    // `Wrap`. With both, `w` grounds to `Wrap<Doubler>` once the initializer
    // resolves, and the dispatch reaches the concrete `Doubler::handle`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Handler { fun handle(self, x: i32): i32; }
        struct Doubler { factor: i32 }
        impl Doubler with Handler { fun handle(self, x: i32): i32 { x * self.factor } }
        struct Wrap<T: Handler> { inner: T }
        impl Wrap<type T: Handler> {
            fun run(self, x: i32): i32 { (self.inner).handle(x) }
        }
        fun main() {
            let d = Doubler { factor = 3 };
            let w = Wrap { inner = d };
            print(w.run(7));
        }
        "#,
        "21\n",
    );
}

#[test]
fn from_json_indirect_element_type_runs() {
    // `decode` returns `Result<Option<User>, str>`; its body is now inferred against
    // that return type (the `ReturnType` constraint), so `Ok(Option::from_json(text))`
    // types `from_json` against `Option<User>` — the constructor propagation (fix #1)
    // then threads `User` into the decode. Previously the body was inferred bottom-up
    // and lowered to the abstract `from_json_value` → `Some(undefined)`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        [derive(Json)] struct User { id: i32, name: str }
        fun decode(text: str): Result<Option<User>, str> { Ok(Option::from_json(text)) }
        fun main() {
            match decode("{\"id\":1,\"name\":\"Ada\"}") {
                Ok(Some(let u)) => print(u.name),
                Ok(None) => print("none"),
                Err(let e) => print(e),
            }
        }
        "#,
        "Ada\n",
    );
}

#[test]
fn deep_dependency_chain_resolves_across_passes() {
    // Ordering test for the dependency-driven re-queue (item 5 v2): each `id` call's
    // generic `T` binds from its argument, which is the *next* `id` call — so the
    // outer calls can only resolve several passes after the innermost. The runner
    // wakes each deferred call when its input lands (with the run-all backstop as a
    // safety net), so the whole nest resolves to `i32` and prints `7`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        fun id<T>(x: T): T { x }
        fun main() {
            let deep = id(id(id(id(id(id(7))))));
            print(format(deep));
        }
        "#,
        "7\n",
    );
}

#[test]
fn from_json_return_type_flows_through_match_arm() {
    // The RPC-client shape: the `from_json` decode sits inside a `match` arm whose
    // enclosing function declares the return type. The return type must reach the
    // arm body *through* the match — `resolve_match` propagates the function's
    // expected type into each leg, so `Ok(Option::from_json(json))` binds `User`
    // even though a `match` sits between the call and the signature. Without the
    // propagation the leg was inferred bottom-up → abstract decoder → `Some(undefined)`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        [derive(Json)] struct User { id: i32, name: str }
        fun decode(tag: str, json: str): Result<Option<User>, str> {
            match tag {
                "ok" => Ok(Option::from_json(json)),
                _ => Err("bad tag"),
            }
        }
        fun main() {
            match decode("ok", "{\"id\":1,\"name\":\"Ada\"}") {
                Ok(Some(let u)) => print(u.name),
                Ok(None) => print("none"),
                Err(let e) => print(e),
            }
        }
        "#,
        "Ada\n",
    );
}

// --- Monomorphization unification (the one `emit_instance` / `call_substitution`
//     path; commit 6b96d3f) and dependency re-queue (item 5 v2) edge cases --------

#[test]
fn multi_parameter_generic_function_instantiations() {
    // The unified emitter keys an instance by its bound types ordered by constraint
    // id; the old free-function emitter keyed by *positional* type arguments. For a
    // two-parameter function those orders coincide (constraint ids are minted in
    // parameter order), and this pins that: `first<A, B>` must instantiate
    // `<i32, str>`, the *swapped* `<str, i32>`, and the same-type `<i32, i32>` as
    // distinct, non-colliding instances — a key bug would cross-wire them.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun first<A, B>(a: A, b: B): A { a }
        fun second<A, B>(a: A, b: B): B { b }
        fun main() {
            print(first(1, "x"));
            print(first("y", 2));
            print(second(1, "z"));
            print(first(3, 4));
        }
        "#,
        "1\ny\nz\n3\n",
    );
}

#[test]
fn multi_parameter_generic_method_monomorphizes() {
    // A two-generic impl whose methods return each parameter — the binding flows
    // through `method_call_substitution` (both `A` and `B` bound from the receiver
    // `Pair<i32, str>`) and field access substitutes the field's declared generic
    // through the receiver's arguments. Both reach the one `emit_instance` path.
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Pair<A, B> { left: A, right: B }
        impl Pair<type A, type B> {
            fun show_left(self): A { self.left }
            fun show_right(self): B { self.right }
        }
        fun main() {
            let p = Pair { left = 7, right = "hi" };
            print(p.show_left());
            print(p.show_right());
        }
        "#,
        "7\nhi\n",
    );
}

#[test]
fn operator_monomorphizes_on_generic_aggregate() {
    // `==` on `Option<Point>` overloads to the aggregate's `eq`, monomorphized
    // against the recorded type-arg substitution — the operator path through
    // `binary_op_dispatch` + `method_call_substitution` into the one emitter.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        [derive(PartialEq)] struct Point { x: i32, y: i32 }
        fun main() {
            let a: Option<Point> = Some(Point { x = 1, y = 2 });
            let b: Option<Point> = Some(Point { x = 1, y = 2 });
            let c: Option<Point> = Some(Point { x = 9, y = 9 });
            if a == b { print("ab-eq") } else { print("ab-neq") }
            if a == c { print("ac-eq") } else { print("ac-neq") }
        }
        "#,
        "ab-eq\nac-neq\n",
    );
}

#[test]
fn single_level_container_from_json_roundtrip_runs() {
    // A single-level `List<i32>` decode: `from_json` calls `from_json_value`, whose
    // element type comes only from the enclosing `List<i32>` instantiation — the
    // inherited-substitution channel of `call_substitution`. Verifies it threads the
    // element type at runtime (the nested case is still open — see the ignored test).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        fun main() {
            let nums: List<i32> = List::from_json("[1,2,3]");
            print(nums.to_json());
        }
        "#,
        "[1,2,3]\n",
    );
}

#[test]
fn nested_container_from_json_roundtrip_runs() {
    // The `List<List<T>>` round-trip (the last row of the type-solver bug table).
    // The inner `List`'s element binding must thread through the *outer*
    // `from_json_value`: `resolve_dispatch` now binds an impl's generics from the
    // concrete receiver type (`bind_generics`) and emits a monomorphized instance,
    // so the nested `T::from_json_value` resolves at each level instead of lowering
    // to the abstract decoder (which yielded `[[undefined,...]]`). Triple nesting
    // exercises the recursion through two intermediate container instances.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        fun main() {
            let grid: List<List<i32>> = List::from_json("[[1,2],[3,4]]");
            print(grid.to_json());
            let deep: List<List<List<i32>>> = List::from_json("[[[1]],[[2,3]]]");
            print(deep.to_json());
        }
        "#,
        "[[1,2],[3,4]]\n[[[1]],[[2,3]]]\n",
    );
}

#[test]
fn mixed_nested_container_from_json_roundtrips() {
    // Mixed nesting through the same monomorphizing dispatch: `Option<List<i32>>`,
    // `List<Option<i32>>` (with a JSON `null` -> `None`), and a `List` of derived
    // structs — each inner decoder is monomorphized for its element via the impl's
    // generics bound from the concrete type.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        import std::option::Option::{ self, Some, None };
        [derive(Json)] struct P { x: i32 }
        fun main() {
            let a: Option<List<i32>> = Option::from_json("[1,2,3]");
            print(a.to_json());
            let b: List<Option<i32>> = List::from_json("[1,null,3]");
            print(b.to_json());
            let c: List<P> = List::from_json("[{\"x\":1},{\"x\":2}]");
            print(c.to_json());
        }
        "#,
        "[1,2,3]\n[1,null,3]\n[{\"x\":1},{\"x\":2}]\n",
    );
}

// --- Method & argument passing (a historically fragile area) -----------------
//   Runtime checks, because the recurring failures here were silent miscompiles
//   (a dispatch resolving to `undefined`, a `&mut` lowering to broken JS) that a
//   compile-only test would pass. Covers: generic-bounded value dispatch
//   (roadmap Tier 1.2 / M2), a method routing its own generic into a nested call
//   (Bug C / B5), auto-deref through a view-returning call (B2), and `&`/`&mut`
//   argument passing (C5 / R8). Two open cases are pinned as ignored tests.

#[test]
fn generic_bounded_value_method_dispatch() {
    // A trait method called on a value of a generic-bounded type (`x: T: Display`)
    // dispatches to the concrete impl per monomorphization, at each call type —
    // not the abstract trait method (which would print `undefined`). Roadmap 1.2.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun describe<T: Display>(x: T): str { x.to_string() }
        fun main() {
            print(describe(42));
            print(describe("hi"));
        }
        "#,
        "42\nhi\n",
    );
}

#[test]
fn generic_bounded_value_operator_dispatch() {
    // `==` on a value of a generic-bounded type (`a: T: PartialEq`) re-resolves to
    // the concrete impl per monomorphization — for a primitive (native `===`) and
    // a `str`. Roadmap 1.2 / generic-equality.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialEq;
        fun same<T: PartialEq>(a: T, b: T): bool { a == b }
        fun main() {
            if same(3, 3) { print("y") } else { print("n") }
            if same(1, 2) { print("y") } else { print("n") }
            if same("a", "a") { print("y") } else { print("n") }
        }
        "#,
        "y\nn\ny\n",
    );
}

#[test]
fn method_routes_own_generic_to_nested_call() {
    // A method on a generic impl passes the impl's type parameter into a *nested*
    // generic call (`format(self.v)`), which must monomorphize for the concrete
    // element at each instantiation (Bug C / B5). The receiver's `T` reaches the
    // nested call through the field access + the inherited substitution.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::{ Display, format };
        struct Wrap<T: Display> { v: T }
        impl Wrap<type T: Display> {
            fun render(self): str { format(self.v) }
        }
        fun main() {
            print(Wrap { v = 7 }.render());
            print(Wrap { v = "hi" }.render());
        }
        "#,
        "7\nhi\n",
    );
}

#[test]
fn auto_deref_through_view_returning_call() {
    // Field and method access on a `borrows` view-returning call: `o.slot().n` and
    // `o.slot().get()` auto-deref the returned `&mut Inner` to reach the inner
    // struct's member (backlog B2). Locks the behavior in (a regression would make
    // the access miss the deref).
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Inner { n: i32 }
        impl Inner { fun get(self): i32 { self.n } }
        struct Outer { inner: Inner }
        impl Outer { fun slot(&mut self): &mut Inner borrows self { &mut self.inner } }
        fun main() {
            mut o = Outer { inner = Inner { n = 5 } };
            print(o.slot().n);
            print(o.slot().get());
        }
        "#,
        "5\n5\n",
    );
}

#[test]
fn mut_view_argument_mutates_through_call_chain() {
    // R8: a `&mut` argument is passed as an explicit `&mut place` and mutates the
    // caller's place; forwarding the view to a further call (`via` -> `bump`)
    // re-borrows it and keeps writing through. Runtime, so the `(base, key)`
    // place-write is exercised end to end.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun bump(x: &mut i32) { x += 1; }
        fun via(y: &mut i32) { bump(y); }
        fun main() {
            mut a = 0;
            bump(&mut a);
            print(a);
            via(&mut a);
            print(a);
        }
        "#,
        "1\n2\n",
    );
}

#[test]
fn mut_view_as_method_argument_mutates() {
    // A `&mut` parameter on a *non-`self`* method argument (`target`) mutates the
    // caller's place across repeated calls — distinct from the implicitly-borrowed
    // `self` receiver. C5 / R8.
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Counter { n: i32 }
        impl Counter { fun add_into(self, target: &mut i32) { target += self.n; } }
        fun main() {
            mut total = 10;
            let c = Counter { n = 5 };
            c.add_into(&mut total);
            c.add_into(&mut total);
            print(total);
        }
        "#,
        "20\n",
    );
}

#[test]
fn mixed_value_view_and_own_arguments() {
    // One call mixing the three argument modes: a by-value `base` (read), a `&mut`
    // view `acc` (writes through to the caller), and an `own scratch` (a private
    // mutable copy the caller never sees). Each must keep its own semantics.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun combine(base: i32, acc: &mut i32, own scratch: i32): i32 {
            acc += base;
            scratch += 100;
            scratch
        }
        fun main() {
            mut a = 1;
            let s = combine(2, &mut a, 7);
            print(a); // 3 — written through the view
            print(s); // 107 — the own copy
        }
        "#,
        "3\n107\n",
    );
}

#[test]
fn reject_bare_value_to_shared_reference_param() {
    // R8 for a shared `&` parameter (the complement of `r8_reject_implicit_borrow`,
    // which covers `&mut`): a bare value place is rejected — pass `& <place>`.
    assert_fails(
        r#"
        fun read_it(x: &i32): i32 { *x }
        fun main() { let a = 5; let n = read_it(a); }
        "#,
    );
}

#[test]
fn generic_mut_view_parameter_writes_through() {
    // A generic `&mut T` view now behaves exactly like a concrete `&mut <T>`. For a
    // scalar pointee (`i32`, `f64`, `str`, `u32`) the read/write goes through the
    // `(base, key)` place-write, decided at monomorphization (the analyzer can't,
    // with `T` abstract — it emitted the aggregate `Object.assign`, leaving `a`
    // unchanged). For an aggregate pointee it stays the in-place copy.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun replace<T>(slot: &mut T, value: T) { slot = value; }
        fun main() {
            mut a = 1;
            replace(&mut a, 9);
            print(a);             // 9 — i32 written through
            mut f = 1.0;
            replace(&mut f, 2.5);
            print(f);             // 2.5 — f64
            mut s = "hi";
            replace(&mut s, "hey");
            print(s);             // hey — str
        }
        "#,
        "9\n2.5\nhey\n",
    );
}

#[test]
fn generic_mut_view_reads_and_swaps() {
    // Reading through a generic `&mut T` view (`*a`) and a `swap<T>` that both reads
    // and writes both views — the place-read `slot[0][slot[1]]` is also picked at
    // monomorphization for a scalar `T`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::Display;
        fun peek<T: Display>(slot: &mut T): str { (*slot).to_string() }
        fun swap<T>(a: &mut T, b: &mut T) { let t = *a; a = *b; b = t; }
        fun main() {
            mut a = 5;
            print(peek(&mut a));
            mut x = 1;
            mut y = 2;
            swap(&mut x, &mut y);
            print(x);
            print(y);
        }
        "#,
        "5\n2\n1\n",
    );
}

#[test]
fn generic_mut_view_of_a_generic_local() {
    // The caller side: a `&mut` of a *generic-typed local* (`mut local = x` where
    // `x: T`) forwarded to another generic view parameter. The local must be boxed
    // and the reference must build the `(base, key)` pair when `T` resolves to a
    // scalar here — decided in the transformer (`generic_referenced_roots`), since
    // the analyzer saw `T` abstract. An aggregate `T` stays unboxed. (Before the
    // fix the scalar case crashed: `slot[0][slot[1]]` on an unboxed value.)
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun inner<T>(slot: &mut T, value: T) { slot = value; }
        fun outer<T>(x: T, value: T): T { mut local = x; inner(&mut local, value); local }
        struct P { x: i32 }
        fun main() {
            print(outer(1, 9));                       // scalar local -> 9
            print(outer(P { x = 1 }, P { x = 9 }).x); // aggregate local -> 9
        }
        "#,
        "9\n9\n",
    );
}

#[test]
fn generic_mut_view_aggregate_pointee_copies_in_place() {
    // The aggregate side of the same parameter: a generic `&mut T` where `T`
    // resolves to a struct rebinds via the in-place copy (not a `(base, key)`
    // write), so the caller's value updates. Guards that the scalar fix didn't
    // change the aggregate path.
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct P { x: i32 }
        fun replace<T>(slot: &mut T, value: T) { slot = value; }
        fun main() {
            mut p = P { x = 1 };
            replace(&mut p, P { x = 9 });
            print(p.x);
        }
        "#,
        "9\n",
    );
}

#[test]
fn bare_trait_value_method_call_is_rejected() {
    // Calling a method on a value typed as a *bare trait* (`let x: Display = 5`)
    // has no concrete type to dispatch to — vilan has no trait objects — and used
    // to silently lower to the empty abstract method (`undefined`). It is now a
    // clean compile error pointing at the generic-parameter / concrete-type fix
    // (backlog B4). The legitimate use of a bare-trait type is a *bound*
    // (`<T: Display>`), exercised by `generic_dispatch_to_extern_impl` et al.
    assert_fails(
        r#"
        import std::display::Display;
        fun main() {
            let x: Display = 5;
            let s = x.to_string();
        }
        "#,
    );
}

#[test]
fn trait_default_self_dispatch_still_runs() {
    // The flip side of the rejection: inside a trait *default* body a `Self`
    // receiver — including a chain through a `Self`-returning method and a
    // non-`self` `Self`-typed parameter — is legitimate and re-dispatches to the
    // concrete type at codegen. Guards that the bare-trait-value check doesn't
    // catch these.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Stepper {
            fun step(self): i32;
            fun twice(self): i32 { self.step() + self.step() }
            fun plus(self, other: Self): i32 { self.twice() + other.step() }
        }
        struct One {}
        impl One with Stepper { fun step(self): i32 { 1 } }
        fun main() {
            let a = One {};
            let b = One {};
            print((a).plus(b));
        }
        "#,
        "3\n",
    );
}

// --- B6: inferred-element list, closure-param field access -------------------

#[test]
fn inferred_list_closure_param_field_access() {
    // A `List::new()` + `push` list has its element type inferred from `push`,
    // which lands (via a `SlotUnification`) *after* a following `map`/`filter`
    // would resolve. A method on such a receiver now defers while a `push`/`run`
    // to fill the slot is still pending, so the closure parameter types against
    // the concrete element and a field access on it works — no `mut xs: List<P>`
    // annotation needed (backlog B6 / roadmap Tier 1.2). Parity with a literal
    // list.
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct P { x: i32 }
        fun main() {
            mut xs = List::new();
            xs.push(P { x = 10 });
            xs.push(P { x = 20 });
            let big = xs.filter(|p| p.x > 15);
            print(big.len());
            let labels = xs.map(|p| p.x);
            print(labels.len());
        }
        "#,
        "1\n2\n",
    );
}

#[test]
fn inferred_list_never_pushed_still_resolves() {
    // The deferral must not strand a `List::new()` that is *never* pushed: with no
    // pending `SlotUnification`, its methods resolve immediately (element stays
    // `Unknown`/`any`) rather than deferring forever.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let xs = List::new();
            print(xs.len());
            let ys = xs.map(|n| 1);
            print(ys.len());
        }
        "#,
        "0\n0\n",
    );
}

#[test]
fn inline_match_on_method_result_field_access() {
    // An inline `match` on a method call that returns `Option<element>`
    // (`match xs.get(0) { Some(let p) => p.x }`) typed its capture `p` only on a
    // late pass; the field accessor on `p` was woken by that resolution but the
    // fixpoint's backstop branch could terminate *before* running the woken
    // constraint (its `wake_ready` result was ignored). The loop now continues
    // while a wake is pending, so the access resolves. Worked when bound to a
    // `let` first (an extra pass) — now works inline too, for `get` and `pop`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        struct P { x: i32 }
        fun main() {
            mut xs = List::new();
            xs.push(P { x = 42 });
            match xs.get(0) {
                Some(let p) => print(p.x),
                None => print(0),
            }
            match xs.pop() {
                Some(let p) => print(p.x),
                None => print(0),
            }
        }
        "#,
        "42\n42\n",
    );
}

#[test]
fn impl_binder_inherits_struct_bound() {
    // `impl Wrapper<type T>` omits the bound the struct declares (`struct
    // Wrapper<T: Greeter>`). The impl can only ever apply to a `Wrapper`, whose
    // existence already requires `T: Greeter`, so the binder *inherits* that
    // bound — and a trait method call on the `T`-typed field resolves, exactly as
    // if `impl Wrapper<type T: Greeter>` had been written.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Greeter { fun greet(self): str; }
        struct Hello { name: str }
        impl Hello with Greeter { fun greet(self): str { "hi " + self.name } }
        struct Wrapper<T: Greeter> { inner: T }
        impl Wrapper<type T> {
            fun run(self): str { (self.inner).greet() }
        }
        fun main() {
            print(Wrapper { inner = Hello { name = "x" } }.run());
        }
        "#,
        "hi x\n",
    );
}

#[test]
fn impl_binder_inherits_multiple_bounds() {
    // A multi-bound declared parameter (`T: A + B`) keeps *both* bounds when
    // inherited: the extra bounds hang off the same constraint id the binder
    // reuses, so methods from either trait resolve on the field.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Named { fun name(self): str; }
        trait Aged { fun age(self): i32; }
        struct Person { n: str, a: i32 }
        impl Person with Named { fun name(self): str { self.n } }
        impl Person with Aged { fun age(self): i32 { self.a } }
        struct Card<T: Named + Aged> { who: T }
        impl Card<type T> {
            fun render(self): str { (self.who).name() }
            fun years(self): i32 { (self.who).age() }
        }
        fun main() {
            let card = Card { who = Person { n = "Ada", a = 36 } };
            print(card.render());
            print(card.years());
        }
        "#,
        "Ada\n36\n",
    );
}

#[test]
fn impl_binder_inherits_per_position_with_multiple_params() {
    // Two declared parameters with *different* bounds — the inherited constraint
    // is matched to the binder by position, not conflated.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Named { fun name(self): str; }
        trait Aged { fun age(self): i32; }
        struct Tag { n: str }
        impl Tag with Named { fun name(self): str { self.n } }
        struct Years { y: i32 }
        impl Years with Aged { fun age(self): i32 { self.y } }
        struct Pair<A: Named, B: Aged> { left: A, right: B }
        impl Pair<type A, type B> {
            fun label(self): str { (self.left).name() }
            fun count(self): i32 { (self.right).age() }
        }
        fun main() {
            let pair = Pair { left = Tag { n = "Ada" }, right = Years { y = 7 } };
            print(pair.label());
            print(pair.count());
        }
        "#,
        "Ada\n7\n",
    );
}

#[test]
fn impl_binder_mixes_explicit_and_inherited_bounds() {
    // One binder restates its bound explicitly, the other infers it — both must
    // resolve. The explicit one already worked; this pins that adding inheritance
    // for the other did not break the mixed form.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Named { fun name(self): str; }
        trait Aged { fun age(self): i32; }
        struct Tag { n: str }
        impl Tag with Named { fun name(self): str { self.n } }
        struct Years { y: i32 }
        impl Years with Aged { fun age(self): i32 { self.y } }
        struct Pair<A: Named, B: Aged> { left: A, right: B }
        impl Pair<type A: Named, type B> {
            fun label(self): str { (self.left).name() }
            fun count(self): i32 { (self.right).age() }
        }
        fun main() {
            let pair = Pair { left = Tag { n = "Ada" }, right = Years { y = 7 } };
            print(pair.label());
            print(pair.count());
        }
        "#,
        "Ada\n7\n",
    );
}

#[test]
fn impl_binder_inherits_enum_bound() {
    // Inheritance works for an enum subject too, not just structs.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Greeter { fun greet(self): str; }
        struct Hello { name: str }
        impl Hello with Greeter { fun greet(self): str { "hi " + self.name } }
        enum Box<T: Greeter> { Full(T), Empty }
        impl Box<type T> {
            fun shout(self): str {
                match self {
                    Box::Full(let inner) => inner.greet(),
                    Box::Empty => "empty",
                }
            }
        }
        fun main() {
            print(Box::Full(Hello { name = "x" }).shout());
        }
        "#,
        "hi x\n",
    );
}

#[test]
fn impl_binder_without_a_declared_bound_stays_unconstrained() {
    // Inheritance only borrows a bound the subject actually declares. An
    // unconstrained `struct Plain<T>` confers nothing, so a trait method call on
    // the `T`-typed field must still be rejected — the fix must not invent bounds.
    assert_fails(
        r#"
        import std::print;
        trait Greeter { fun greet(self): str; }
        struct Plain<T> { inner: T }
        impl Plain<type T> {
            fun run(self): str { (self.inner).greet() }
        }
        fun main() {
            print(0);
        }
        "#,
    );
}

#[test]
#[ignore = "bound inheritance needs the subject type walked first; \
            forward-referencing a struct declared after the impl falls back to \
            requiring the explicit bound"]
fn impl_binder_inherits_bound_from_a_later_declared_struct() {
    // The same program as `impl_binder_inherits_struct_bound`, but with the struct
    // declared *after* the impl. The analyzer is single-pass and resolves the
    // subject type only in `build()`, so the struct's declared bound is not yet
    // available when the impl's binders are registered. Inheritance therefore does
    // not fire here; the explicit `impl Wrapper<type T: Greeter>` form still works.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Greeter { fun greet(self): str; }
        struct Hello { name: str }
        impl Hello with Greeter { fun greet(self): str { "hi " + self.name } }
        impl Wrapper<type T> {
            fun run(self): str { (self.inner).greet() }
        }
        struct Wrapper<T: Greeter> { inner: T }
        fun main() {
            print(Wrapper { inner = Hello { name = "x" } }.run());
        }
        "#,
        "hi x\n",
    );
}

#[test]
fn async_trait_method_through_generic_bound_auto_awaits() {
    // An inferred-async trait method (`fetch` awaits) dispatched through a generic
    // bound (`self.inner: T`, `T: Fetcher`). The call graph used to mis-resolve the
    // dispatch to the trait's *signature* (a bodyless method, never async — the
    // dispatch is keyed by the call id, which `resolve_target` only consulted for
    // `OnType`), so the enclosing `run` was left non-`async` while the transformer,
    // resolving the concrete async impl, still inserted the `await` — `await` inside
    // a non-async function, invalid JS that crashed at load. Async-ness now
    // propagates through the dispatch's candidate impls, so `run` (and its caller
    // `main`) are async and the program runs.
    assert_compiles_and_runs(
        r#"
        import std::print;
        [extern("Promise.resolve")]
        async external fun resolved(value: str): str;
        trait Fetcher { fun fetch(self): str; }
        struct Remote { tag: str }
        impl Remote with Fetcher {
            fun fetch(self): str { await resolved(self.tag) }
        }
        struct Wrapper<T: Fetcher> { inner: T }
        impl Wrapper<type T> {
            fun run(self): str { (self.inner).fetch() }
        }
        fun main() {
            print(Wrapper { inner = Remote { tag = "hi" } }.run());
        }
        "#,
        "hi\n",
    );
}

#[test]
fn async_impl_through_generic_bound_propagates_transitively() {
    // The impl method is async *transitively* — it doesn't `await` itself, it calls
    // an async function — so its async-ness is only settled by the fixpoint. The
    // dispatch must pick that up after propagation, not just from a direct `await`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        [extern("Promise.resolve")]
        async external fun resolved(value: str): str;
        fun load(tag: str): str { await resolved(tag) }
        trait Fetcher { fun fetch(self): str; }
        struct Remote { tag: str }
        impl Remote with Fetcher {
            fun fetch(self): str { load(self.tag) }
        }
        struct Wrapper<T: Fetcher> { inner: T }
        impl Wrapper<type T> {
            fun run(self): str { (self.inner).fetch() }
        }
        fun main() {
            print(Wrapper { inner = Remote { tag = "hey" } }.run());
        }
        "#,
        "hey\n",
    );
}

#[test]
fn mixed_async_and_sync_impls_through_generic_bound_both_run() {
    // Two impls of one trait — one async, one sync — both reached through the bound.
    // The dispatch is conservatively async (some candidate impl awaits), so even the
    // sync instance compiles to an async function; awaiting its non-promise result is
    // a JS no-op, and both instantiations run correctly.
    assert_compiles_and_runs(
        r#"
        import std::print;
        [extern("Promise.resolve")]
        async external fun resolved(value: str): str;
        trait Fetcher { fun fetch(self): str; }
        struct Remote { tag: str }
        impl Remote with Fetcher { fun fetch(self): str { await resolved(self.tag) } }
        struct Local { tag: str }
        impl Local with Fetcher { fun fetch(self): str { self.tag } }
        struct Wrapper<T: Fetcher> { inner: T }
        impl Wrapper<type T> { fun run(self): str { (self.inner).fetch() } }
        fun main() {
            print(Wrapper { inner = Remote { tag = "remote" } }.run());
            print(Wrapper { inner = Local { tag = "local" } }.run());
        }
        "#,
        "remote\nlocal\n",
    );
}

#[test]
fn async_trait_default_body_through_generic_bound_auto_awaits() {
    // The async method is the trait's *default* body (the impl doesn't override it),
    // dispatched through the bound. The candidate is the trait default, not an impl
    // member — so candidate resolution must consider the trait's own declarations.
    assert_compiles_and_runs(
        r#"
        import std::print;
        [extern("Promise.resolve")]
        async external fun resolved(value: str): str;
        trait Greeter {
            fun name(self): str;
            fun greet(self): str { await resolved(self.name()) }
        }
        struct Hello { who: str }
        impl Hello with Greeter { fun name(self): str { self.who } }
        struct Wrapper<T: Greeter> { inner: T }
        impl Wrapper<type T> { fun run(self): str { (self.inner).greet() } }
        fun main() {
            print(Wrapper { inner = Hello { who = "ada" } }.run());
        }
        "#,
        "ada\n",
    );
}

#[test]
fn sync_method_through_generic_bound_is_not_made_async() {
    // The precision guard: a generic dispatch whose trait has *no* async impl must
    // not become async. Asserted structurally — the emitted JS has no `async`/`await`
    // anywhere — so an over-eager propagation (e.g. matching an async method of the
    // same name in an unrelated trait) would fail here, not just slip past `runs`.
    let js = compile(
        r#"
        import std::print;
        trait Greeter { fun greet(self): str; }
        struct Hello { name: str }
        impl Hello with Greeter { fun greet(self): str { "hi " + self.name } }
        struct Wrapper<T: Greeter> { inner: T }
        impl Wrapper<type T> { fun run(self): str { (self.inner).greet() } }
        fun main() { print(Wrapper { inner = Hello { name = "x" } }.run()); }
        "#,
    )
    .expect("compiles");
    assert!(
        !js.contains("async") && !js.contains("await"),
        "a purely-sync generic dispatch must not be made async:\n{js}"
    );
}

#[test]
fn generic_element_serialized_in_a_closure_through_a_bounded_method() {
    // A closure passed to a generic method (`feed.each(|T| ..)`) on a parameterized-bound
    // receiver (`F: Feed<T>`), serializing the element `T` inside the closure. Two gaps
    // used to break this: the closure parameter lost its `T: Json` bound — a compile error
    // ("cannot call method 'to_json' on T") — and `T`, which appears *only* in the bound
    // `F: Feed<T>`, was never derived from the concrete `Nums: Feed<i32>` at the call site,
    // so `to_json` monomorphized to the empty abstract method and yielded `undefined`.
    // Both are fixed (the parameterized-bound substitution in the `Type::Generic` method
    // arm, and the derive-from-bound step in `resolve_call_subject`).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::Json;
        trait Feed<T> { fun each(self, observer: |T| void); }
        struct Nums {}
        impl Nums with Feed<i32> {
            fun each(self, observer: |i32| void) { observer(7); observer(9); }
        }
        fun pump<T: Json, F: Feed<T>>(feed: F, out: |str| void) {
            feed.each(|value| out(value.to_json()))
        }
        fun main() { pump(Nums {}, |s| print(s)); }
        "#,
        "7\n9\n",
    );
}

#[test]
fn generic_source_element_serialized_in_a_sub_closure() {
    // The reactive shape the fix unblocks: forward a `Source<T>`'s values, serialized
    // inside the `sub` closure, where `T` appears only in the `S: Source<T>` bound.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::Json;
        import std::reactive::{ Source, Signal, Subscription };
        fun forward<T: Json, S: Source<T>>(source: S, out: |str| void): Subscription {
            source.sub(|value| out(value.to_json()))
        }
        fun main() {
            let s = Signal::new(7);
            let _ = forward(s, |json| print(json));
            s.set(9);
        }
        "#,
        "7\n9\n",
    );
}

#[test]
fn generic_element_type_derived_from_a_parameterized_bound() {
    // A struct payload `T` (not a scalar) crosses the same paths: the element flows
    // through the closure and a `[derive(Json)]` `to_json`, and `T` is derived from the
    // bound. Pins that the fix threads a concrete *aggregate* type, not just `i32`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::Json;
        trait Feed<T> { fun each(self, observer: |T| void); }
        [derive(Json)]
        struct Point { x: i32, y: i32 }
        struct Points {}
        impl Points with Feed<Point> {
            fun each(self, observer: |Point| void) { observer(Point { x = 1, y = 2 }); }
        }
        fun dump<T: Json, F: Feed<T>>(feed: F) {
            feed.each(|point| print(point.to_json()))
        }
        fun main() { dump(Points {}); }
        "#,
        "{\"x\":1,\"y\":2}\n",
    );
}

#[test]
fn generic_bound_derivation_through_a_method_call() {
    // The same fix on the *method* path (`bind_method_own_generics`): a struct method
    // `<T: Json, S: Source<T>>` whose `T` appears only in the bound, serializing the
    // element in a `sub` closure. Called as `sink.forward(signal, ..)`, `T` is derived
    // from the concrete signal's `Source` impl — the shape `examples/rpc`'s `expose` uses.
    // The source argument is *inferred* (`let s = Signal::new(7)`, no annotation), so its
    // type lands only after the call is first seen; `resolve_method_call` defers while the
    // bound-owner is unresolved and re-derives on a later pass (mirroring the free-function
    // path), so the inferred case works too.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::Json;
        import std::reactive::{ Source, Signal, Subscription };
        struct Sink {}
        impl Sink {
            fun forward<T: Json, S: Source<T>>(self, source: S, out: |str| void): Subscription {
                source.sub(|value| out(value.to_json()))
            }
        }
        fun main() {
            let s = Signal::new(7);
            let _ = Sink {}.forward(s, |json| print(json));
            s.set(9);
        }
        "#,
        "7\n9\n",
    );
}

#[test]
fn owner_take_disposes_a_mapped_and_a_root_subscription() {
    // Pins `vilan/test/reactive.js`'s reachable miscompilation as *observable* runtime
    // behaviour — the golden alone proved an unreliable gate (it drifted stale), so an
    // executed assertion is the stronger pin. `Owner::take<T: Disposable>` (an *unparameterized*
    // bound) stores `|| item.dispose()` in a cleanup closure for later. Two `take` sites are
    // needed to trigger it: `take(mapped.sub(..))` where `mapped = root.map(..)` resolves its
    // element *late* (through `map`'s generic), and `take(root.sub(..))` which resolves early.
    // The pre-fix analyzer bound the *mapped* site's `T` before its argument landed and
    // monomorphized that `take` to the empty abstract `Disposable::dispose` (the *root* site
    // stayed concrete), so disposing the owner never removed the mapped subscriber and it
    // leaked. reactive.js hides it (its owner is never disposed); here we dispose the owner,
    // so a leaked subscription keeps firing: pre-fix this printed a trailing `a=10`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner };
        fun main() {
            let owner = Owner::new();
            let count = Signal::new(0);
            let doubled = count.map(|n| n * 2);
            owner.take(doubled.sub(|n| print(i"a={n}")));   // mapped/late site
            owner.take(count.sub(|n| print(i"b={n}")));     // root/early site
            count.set(1);       // a=2, b=1
            owner.dispose();    // the *real* dispose must remove BOTH subscribers
            count.set(5);       // silent iff both disposed; leaks "a=10" if the mapped take went abstract
        }
        "#,
        "a=0\nb=0\na=2\nb=1\n",
    );
}

// === Reactive batching (proposal/reactive-batching.md) ============================

#[test]
fn lone_set_notifies_synchronously() {
    // Outside a `batch`, `set` notifies inline (eager) — a lone set fires its observers
    // before the next statement, exactly as before batching existed.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal };
        fun main() {
            let a = Signal::new(0);
            let _ = a.sub(|v| print(i"a={v}"));   // immediate: a=0
            a.set(1);                             // eager -> a=1 now
            print("after");
            a.set(2);                             // a=2
        }
        "#,
        "a=0\na=1\nafter\na=2\n",
    );
}

#[test]
fn batch_commits_value_immediately_but_defers_notification() {
    // Inside a `batch`, a root's value is committed at once (`s.get()` is fresh), but a
    // *derived* value recomputes only at the flush boundary — so mid-batch it is stale,
    // then settles. Pins the "defer notification, not the value" divergence.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, batch };
        fun main() {
            let s = Signal::new(0);
            let doubled = s.map(|n| n * 2);
            batch(|| {
                s.set(5);
                print(i"in-batch s={s.get()} doubled={doubled.get()}");   // s=5 fresh, doubled=0 stale
            });
            print(i"after doubled={doubled.get()}");                      // 10 (settled at flush)
        }
        "#,
        "in-batch s=5 doubled=0\nafter doubled=10\n",
    );
}

#[test]
fn batch_coalesces_a_multi_input_observer() {
    // A node fed by two inputs (hand-rolled `d = a + b`, recomputed when either changes)
    // recomputes with both inputs settled inside a `batch` — glitch-free. The `d` observer
    // fires once (11 -> 22), with no intermediate (a-new, b-old) reading.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, batch };
        fun main() {
            let a = Signal::new(1);
            let b = Signal::new(10);
            let d = Signal::new(a.get() + b.get());
            let _ = a.sub(|_| { d.set(a.get() + b.get()); });
            let _ = b.sub(|_| { d.set(a.get() + b.get()); });
            let _ = d.sub(|v| print(i"d={v}"));   // immediate: d=11
            batch(|| {
                a.set(2);
                b.set(20);
            });                                    // coalesced -> d=22 once
        }
        "#,
        "d=11\nd=22\n",
    );
}

#[test]
fn without_a_batch_a_multi_input_observer_glitches() {
    // The same graph without a `batch`: each eager `set` fires the observer, so it sees the
    // intermediate (a=2, b=10) state — the glitch (`d=12`) the batch above elides. Pins that
    // batching is what removes it (the opt-in boundary).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal };
        fun main() {
            let a = Signal::new(1);
            let b = Signal::new(10);
            let d = Signal::new(a.get() + b.get());
            let _ = a.sub(|_| { d.set(a.get() + b.get()); });
            let _ = b.sub(|_| { d.set(a.get() + b.get()); });
            let _ = d.sub(|v| print(i"d={v}"));   // d=11
            a.set(2);                              // d=12 (glitch: b still 10)
            b.set(20);                             // d=22
        }
        "#,
        "d=11\nd=12\nd=22\n",
    );
}

#[test]
fn batch_cascade_settles_in_one_flush() {
    // A linear cascade `a -> map -> map -> observer` settles to its final value in one flush
    // when the root is set inside a `batch` — the observer fires once with the fully-cascaded
    // value (20 -> 60), never an intermediate.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, batch };
        fun main() {
            let a = Signal::new(1);
            let b = a.map(|n| n + 1);      // b = a + 1
            let c = b.map(|n| n * 10);     // c = b * 10
            let _ = c.sub(|v| print(i"c={v}"));   // immediate: c=20
            batch(|| { a.set(5); });               // a=5 -> b=6 -> c=60
        }
        "#,
        "c=20\nc=60\n",
    );
}

#[test]
fn nested_batches_flush_at_the_outer_boundary() {
    // An inner `batch` does not flush (depth stays > 0) — notifications wait for the outermost
    // boundary and coalesce to the final value. `mid` prints before any observer fires.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, batch };
        fun main() {
            let a = Signal::new(0);
            let _ = a.sub(|v| print(i"a={v}"));   // immediate: a=0
            batch(|| {
                a.set(1);
                batch(|| {
                    a.set(2);
                });
                print("mid");        // inner batch did NOT flush -> no a-notify yet
                a.set(3);
            });                       // outer flush -> a=3 (once, final)
        }
        "#,
        "a=0\nmid\na=3\n",
    );
}

#[test]
fn dispose_in_a_batch_scrubs_the_pending_notify() {
    // A subscription disposed *after* its source was set in the same `batch` must not fire:
    // `dispose` scrubs the pending queue, so the enqueued notify is removed before the flush.
    // Pins the "disposed is silent" resolution (no `tick 1` from the batch, no `tick 2` after).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, batch };
        fun main() {
            let counter = Signal::new(0);
            let sub = counter.sub(|n| print(i"tick {n}"));   // immediate: tick 0
            batch(|| {
                counter.set(1);     // enqueues `sub`'s notify
                sub.dispose();      // scrubs it from the pending queue
            });                      // flush -> nothing
            print("done");
            counter.set(2);          // sub disposed -> silent
        }
        "#,
        "tick 0\ndone\n",
    );
}

// === RPC foundation: the generic `call` helper (examples/rpc §4.1) ================

#[test]
fn generic_call_over_a_bounded_transport_decodes() {
    // The RPC foundation's `call<T, Tx: Transport>` shape: a generic function that calls a trait
    // method on a bound-generic transport, `await`s it, and decodes the reply as a generic
    // `T: FromJson` — invoked from a *generic* client that passes its own `Tx`-typed field. Pins
    // that this whole generic-through-generic path monomorphizes (the example isn't auto-run).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ Json, FromJson };
        import std::result::Result::{ self, Ok, Err };
        import std::promise::Promise;
        trait Wire { fun send(self, msg: str): Promise<str>; }
        struct Echo {}
        impl Echo with Wire {
            fun send(self, msg: str): Promise<str> { async { msg } }   // echoes the request
        }
        [derive(Json)]
        struct Pt { x: i32 }
        fun fetch<T: FromJson, Tx: Wire>(transport: Tx, msg: str): Result<T, str> {
            let reply = await transport.send(msg);
            Ok(T::from_json(reply))                       // decode the generic T from the reply
        }
        struct Client<Tx: Wire> { transport: Tx }
        impl Client<type Tx> {
            fun get(self): Result<Pt, str> {
                fetch(self.transport, "{\"x\":42}")        // T=Pt inferred from the return type
            }
        }
        fun main() {
            let c = Client { transport = Echo {} };
            match c.get() {
                Ok(let p) => print(i"x={p.x}"),
                Err(let e) => print(i"err {e}"),
            }
        }
        "#,
        "x=42\n",
    );
}

// === [derive(Wire)] — the data boundary (proposal/transport-rpc.md §3) ============

#[test]
fn wire_derives_the_json_round_trip() {
    // `[derive(Wire)]` reuses the Json round-trip: a Wire struct/enum encodes and decodes,
    // including nested Wire structs, `List<Wire>`, and Wire enums.
    assert_compiles_and_runs(
        r#"
        import std::print;
        [derive(Wire)]
        struct Point { x: i32, y: i32 }
        [derive(Wire)]
        struct Line { from: Point, to: Point, tags: List<str> }
        [derive(Wire)]
        enum Shape { Seg(Line), Empty }
        fun main() {
            let line = Line { from = Point { x = 1, y = 2 }, to = Point { x = 3, y = 4 }, tags = ["a"] };
            let back = Line::from_json(line.to_json());
            print(i"{back.from.x} {back.from.y} {back.to.x} {back.to.y}");   // 1 2 3 4
            match Shape::from_json(Shape::Seg(back).to_json()) {
                Shape::Seg(let l) => print(i"seg {l.from.x}"),               // seg 1
                Shape::Empty => print("empty"),
            }
        }
        "#,
        "1 2 3 4\nseg 1\n",
    );
}

#[test]
fn wire_rejects_a_non_wire_field() {
    // The boundary: a `[derive(Wire)]` type with a non-Wire field (`Password` has no codec)
    // is a compile error — the leak the type system prevents by construction.
    assert_fails(
        r#"
        struct Password { hash: str }
        [derive(Wire)]
        struct User { id: i32, password: Password }
        fun main() {}
        "#,
    );
}

#[test]
fn wire_rejects_a_list_of_non_wire() {
    // The recursive rule: `List<Secret>` is not Wire because `Secret` is not. This pins the
    // Wire check specifically — without it, the conditional `List<T: Json>` impl would let
    // `List<Secret>` slip through the codegen unchecked (the conditional-bound gap).
    assert_fails(
        r#"
        struct Secret { s: str }
        [derive(Wire)]
        struct Bag { items: List<Secret> }
        fun main() {}
        "#,
    );
}

// === [rpc] / [expose] — the service-surface checks (transport-rpc.md §4.2) ========

#[test]
fn rpc_accepts_a_wire_signature() {
    // An `[rpc]` method whose whole signature is Wire compiles: multiple parameters,
    // a container (`List<str>`), a nested `[derive(Wire)]` struct, an `Option` return —
    // and `self` is exempt from the check.
    assert_compiles(
        r#"
        import std::option::Option::{ self, Some, None };
        [derive(Wire)]
        struct Pt { x: i32 }
        struct Service {}
        impl Service {
            [rpc] fun locate(self, id: i32, tags: List<str>, at: Pt): Option<Pt> {
                Some(at)
            }
        }
        fun main() {}
        "#,
    );
}

#[test]
fn rpc_rejects_a_non_wire_parameter() {
    // The exposure rule: an `[rpc]` method cannot take a non-Wire type — the
    // dispatcher would have to decode it off the wire.
    assert_fails(
        r#"
        struct Password { hash: str }
        struct Service {}
        impl Service {
            [rpc] fun store(self, secret: Password) {}
        }
        fun main() {}
        "#,
    );
}

#[test]
fn rpc_rejects_a_non_wire_return() {
    // ...nor return one — the reply crosses the wire.
    assert_fails(
        r#"
        struct Password { hash: str }
        struct Service {}
        impl Service {
            [rpc] fun leak(self): Password {
                Password { hash = "x" }
            }
        }
        fun main() {}
        "#,
    );
}

#[test]
fn expose_accepts_a_signal_of_wire() {
    // An `[expose]`d field must be a `Signal` of a Wire element — a scalar and a
    // `[derive(Wire)]` struct both qualify.
    assert_compiles(
        r#"
        import std::reactive::Signal;
        [derive(Wire)]
        struct Pt { x: i32 }
        struct Session {
            [expose] status: Signal<str>,
            [expose] cursor: Signal<Pt>,
            hidden: i32,
        }
        fun main() {}
        "#,
    );
}

#[test]
fn expose_rejects_a_non_signal_field() {
    // Exposure is observation: a plain value has nothing to subscribe to.
    assert_fails(
        r#"
        struct Session {
            [expose] name: str,
        }
        fun main() {}
        "#,
    );
}

#[test]
fn expose_rejects_a_signal_of_non_wire() {
    // The observed values cross the wire, so the element must be Wire.
    assert_fails(
        r#"
        import std::reactive::Signal;
        struct Password { hash: str }
        struct Session {
            [expose] secret: Signal<Password>,
        }
        fun main() {}
        "#,
    );
}

// === [trait_only] / [doc(hidden)] — namespace hygiene (transport-rpc.md §3.2) =====

#[test]
fn trait_only_method_is_hidden_from_the_concrete_type() {
    // A `[trait_only]` trait method never resolves on the concrete type's own
    // surface — the direct call is an error even though the impl provides it.
    assert_fails(
        r#"
        import std::print;
        trait Marker { [trait_only] fun tag(self): str; }
        struct Pt { x: i32 }
        impl Pt with Marker { fun tag(self): str { "pt" } }
        fun main() { print(Pt { x = 1 }.tag()); }
        "#,
    );
}

#[test]
fn trait_only_method_resolves_through_a_bound() {
    // ...but through a trait bound it resolves and monomorphizes normally.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Marker { [trait_only] fun tag(self): str; }
        struct Pt { x: i32 }
        impl Pt with Marker { fun tag(self): str { "pt" } }
        fun describe<T: Marker>(value: T): str { value.tag() }
        fun main() { print(describe(Pt { x = 1 })); }
        "#,
        "pt\n",
    );
}

#[test]
fn trait_only_static_is_hidden_from_the_concrete_type() {
    // The same exclusion covers statics: `Pt::make()` is an error when `make`
    // is `[trait_only]` — the `from_json`-style surface stays clean.
    assert_fails(
        r#"
        trait Factory { [trait_only] fun make(): i32; }
        struct Pt {}
        impl Pt with Factory { fun make(): i32 { 7 } }
        fun main() { let n = Pt::make(); }
        "#,
    );
}

#[test]
fn trait_only_static_resolves_through_a_bound() {
    // ...while `T::make()` through the bound stays the sanctioned path.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Factory { [trait_only] fun make(): i32; }
        struct Pt {}
        impl Pt with Factory { fun make(): i32 { 7 } }
        fun build<T: Factory>(witness: T): i32 { T::make() }
        fun main() { print(build(Pt {})); }
        "#,
        "7\n",
    );
}

#[test]
fn trait_only_default_method_is_bound_reachable_but_hidden() {
    // A `[trait_only]` *default* method: an empty impl inherits it for the
    // bound path, but it is not promoted onto the concrete surface.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Marker { [trait_only] fun tag(self): str { "default" } }
        struct Pt { x: i32 }
        impl Pt with Marker {}
        fun via_bound<T: Marker>(value: T): str { value.tag() }
        fun main() { print(via_bound(Pt { x = 1 })); }
        "#,
        "default\n",
    );
    assert_fails(
        r#"
        import std::print;
        trait Marker { [trait_only] fun tag(self): str { "default" } }
        struct Pt { x: i32 }
        impl Pt with Marker {}
        fun main() { print(Pt { x = 1 }.tag()); }
        "#,
    );
}

#[test]
fn trait_only_does_not_shadow_an_inherent_method() {
    // The collision-safety point: a type's OWN method with the same name stays
    // reachable on the concrete surface — the `[trait_only]` trait method never
    // shadows it (nor is shadowed by it at the bound).
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Marker { [trait_only] fun tag(self): str { "trait-default" } }
        struct Pt { x: i32 }
        impl Pt { fun tag(self): str { "own" } }
        impl Pt with Marker {}
        fun main() { print(Pt { x = 1 }.tag()); }
        "#,
        "own\n",
    );
}

#[test]
#[ignore = "pre-existing dispatch shadowing (independent of [trait_only], reproduces \
            without it): a bound call's monomorphized dispatch resolves the concrete \
            type's *inherent* same-name method instead of the trait's inherited default \
            — `via_bound(Pt {..})` prints `own` instead of `trait-default`. The analyzer \
            resolves the bound path correctly; the transformer's name-based dispatch \
            lookup does not distinguish the inherent method from the trait member."]
fn bound_dispatch_prefers_the_trait_method_on_a_name_collision() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Marker { fun tag(self): str { "trait-default" } }
        struct Pt { x: i32 }
        impl Pt { fun tag(self): str { "own" } }
        impl Pt with Marker {}
        fun via_bound<T: Marker>(value: T): str { value.tag() }
        fun main() { print(via_bound(Pt { x = 1 })); }
        "#,
        "trait-default\n",
    );
}

// === [service(Client)] generation (transport-rpc.md §4.2) =========================

#[test]
fn service_generates_dispatcher_client_and_mirror() {
    // The whole §4.2 surface, end to end and in-process: `[service(Client)]` generates
    // `Session::dispatcher(self)` (routes both `[rpc]` methods — multi-arg and no-arg),
    // the sibling `Client<T: Transport>` with `Result`-wrapped requestors, and a
    // `RemoteSource` mirror for the `[expose]`d field (whose update arrives in the same
    // wire turn as the mutating call's reply — hence `status = bumped` before `bump -> 5`).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::reactive::Signal;
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ Json, FromJson };
        import std::rpc::{ local_rpc, duplex_pair, ReactiveServer, ReactiveClient };

        [service(Client)]
        struct Session {
            [expose] status: Signal<str>,
            count: Shared<i32>,
        }

        impl Session {
            [rpc]
            fun bump(self, by: i32): i32 {
                self.count.write() = self.count.read() + by;
                self.status.set("bumped");
                self.count.read()
            }

            [rpc]
            fun total(self): i32 {
                self.count.read()
            }
        }

        fun main() {
            let session = Session { status = Signal::new("idle"), count = Shared::new(0) };
            let transport = local_rpc(session.dispatcher().into_protocol());
            let (client_end, server_end) = duplex_pair();
            let channel = ReactiveServer::new(server_end).expose(session.status);
            let client = Client { transport, status = ReactiveClient::new(client_end).source(channel) };
            let watching = client.status.sub(|json| {
                let s: str = str::from_json(json);
                print(i"status = {s}");
            });
            match client.bump(5) {
                Ok(let n) => print(i"bump -> {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            match client.total() {
                Ok(let n) => print(i"total -> {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            let hashes_match = session.contract_hash() == client.contract_hash();
            print(i"hashes match = {hashes_match}");
            watching.dispose();
        }
        "#,
        "status = idle\nstatus = bumped\nbump -> 5\ntotal -> 5\nhashes match = true\n",
    );
}

#[test]
fn service_client_name_defaults_to_struct_client() {
    // Bare `[service]` names the generated client `<Struct>Client`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::result::Result::{ self, Ok, Err };
        import std::json::Json;
        import std::rpc::{ local_rpc };

        [service]
        struct Counter {
            count: Shared<i32>,
        }

        impl Counter {
            [rpc]
            fun get(self): i32 {
                self.count.read()
            }
        }

        fun main() {
            let counter = Counter { count = Shared::new(41) };
            let transport = local_rpc(counter.dispatcher().into_protocol());
            let client = CounterClient { transport };
            match client.get() {
                Ok(let n) => print(i"n = {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
        }
        "#,
        "n = 41\n",
    );
}

#[test]
fn service_contract_verify_matches_and_catches_drift() {
    // The generated `verify()` (Q6 v2): a client fetches the server's contract hash
    // over the built-in `__contract` route and compares. Against its own service:
    // `Ok(true)`. Against a *different* service's dispatcher (a drifted contract —
    // the versioning failure mode): `Ok(false)`, a clean signal instead of decode
    // garbage.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::result::Result::{ self, Ok, Err };
        import std::json::Json;
        import std::rpc::{ local_rpc };

        [service(AClient)]
        struct Alpha { count: Shared<i32> }
        impl Alpha {
            [rpc] fun ping(self): i32 { 1 }
        }

        [service(BClient)]
        struct Beta { count: Shared<i32> }
        impl Beta {
            [rpc] fun rename(self, name: str): str { name }
        }

        fun main() {
            let alpha_transport = local_rpc(Alpha { count = Shared::new(0) }.dispatcher().into_protocol());
            let matching = AClient { transport = alpha_transport };
            match matching.verify() {
                Ok(let same) => print(i"self = {same}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            // A BClient pointed at Alpha's dispatcher — the drift case.
            let drifted = BClient { transport = alpha_transport };
            match drifted.verify() {
                Ok(let same) => print(i"drift = {same}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
        }
        "#,
        "self = true\ndrift = false\n",
    );
}

#[test]
fn rpc_rejects_a_missing_return() {
    // A void `[rpc]` method has no reply payload to encode — the return must be a
    // declared Wire type (fire-and-forget needs its own design).
    assert_fails(
        r#"
        struct Service {}
        impl Service {
            [rpc] fun ping(self) {}
        }
        fun main() {}
        "#,
    );
}

#[test]
fn doc_hidden_method_stays_callable() {
    // `[doc(hidden)]` is tooling-only: completion omits it, resolution doesn't.
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Pt { x: i32 }
        impl Pt {
            [doc(hidden)]
            fun secret(self): i32 { self.x }
        }
        fun main() { print(Pt { x = 9 }.secret()); }
        "#,
        "9\n",
    );
}
