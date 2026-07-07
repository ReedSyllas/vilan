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

/// The analyzer's diagnostics as `(message, span range)` pairs — the E7 span
/// harness's raw material (`compile` keeps only the messages).
fn failure_diagnostics(source: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let leaked: &'static str = Box::leak(source.into_boxed_str());
            let (_program, errors) = analyze_source(
                leaked,
                &std_spec(),
                Path::new("."),
                Path::new("test.vl"),
                Some(Platform::default()),
                &Workspace::default(),
            );
            errors
                .into_iter()
                .map(|error| (error.msg, error.span.into_range()))
                .collect()
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Asserts compilation fails with a diagnostic whose message contains
/// `message_part` and whose span covers exactly the first occurrence of
/// `spanning` in the source — spans pin like messages (backlog E7). The
/// distinct `spanning` snippet locates the *pertinent* expression, so a
/// diagnostic that regresses to an enclosing aggregate span fails here.
#[track_caller]
fn assert_fails_spanning(source: &str, spanning: &str, message_part: &str) {
    let expected_start = source
        .find(spanning)
        .expect("the `spanning` snippet must occur in the source");
    let expected = expected_start..expected_start + spanning.len();
    let diagnostics = failure_diagnostics(source);
    let matching: Vec<_> = diagnostics
        .iter()
        .filter(|(message, _)| message.contains(message_part))
        .collect();
    assert!(
        !matching.is_empty(),
        "no diagnostic contains {message_part:?}; got: {diagnostics:#?}"
    );
    assert!(
        matching.iter().any(|(_, range)| *range == expected),
        "no {message_part:?} diagnostic spans {expected:?} ({spanning:?}); spans: {:#?}",
        matching
            .iter()
            .map(|(message, range)| (message.as_str(), range.clone(), &source[range.clone()]))
            .collect::<Vec<_>>()
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
fn bound_dispatch_prefers_the_trait_method_on_a_name_collision() {
    // FIXED: the analyzer resolved `value.tag()` through the `Marker` bound,
    // but the transformer's name-based re-dispatch found the concrete type's
    // INHERENT `tag` first. The resolved trait is now recorded per call
    // (bound_dispatch_traits) and emission dispatches on that trait's surface
    // — override, else default — so an inherent name collision can't shadow it.
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
        import std::json::json_codec;
        import std::rpc::{ local_rpc, duplex_pair, ReactiveServer, ReactiveClient, RemoteSource };

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
            let transport = local_rpc(session.dispatcher().into_protocol(json_codec()));
            let (client_end, server_end) = duplex_pair();
            let channel = ReactiveServer::new(server_end, json_codec()).expose(session.status);
            let mirror: RemoteSource<str> = ReactiveClient::new(client_end, json_codec()).source(channel);
            let client = Client { transport, codec = json_codec(), status = mirror };
            let watching = client.status.sub(|s| {
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
        import std::json::{ Json, json_codec };
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
            let transport = local_rpc(counter.dispatcher().into_protocol(json_codec()));
            let client = CounterClient { transport, codec = json_codec() };
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
        import std::json::{ Json, json_codec };
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
            let alpha_transport = local_rpc(Alpha { count = Shared::new(0) }.dispatcher().into_protocol(json_codec()));
            let matching = AClient { transport = alpha_transport, codec = json_codec() };
            match matching.verify() {
                Ok(let same) => print(i"self = {same}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            // A BClient pointed at Alpha's dispatcher — the drift case.
            let drifted = BClient { transport = alpha_transport, codec = json_codec() };
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
fn a_discarded_async_block_still_runs() {
    // `async { .. }` is an *invoked* async arrow: its body starts executing
    // immediately (up to the first await), so it is effectful even when the
    // promise is discarded. The transformer's side-effect analysis used to
    // classify it as pure and elide the whole statement — `let _ = async { pump
    // loop }` silently vanished from codegen (found via SplitDuplex's pump).
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let _ = async {
                print("ran");
            };
            print("after");
        }
        "#,
        "ran\nafter\n",
    );
}

#[test]
fn a_parenthesized_type_is_grouping_not_a_tuple() {
    // `(T)` in type position is grouping, not a one-tuple — required to write a
    // closure-typed closure parameter (`|(|| void)| void`, the host-Promise
    // executor shape `std::time::sleep` uses). The inner closure is passed AND
    // called through the parenthesized annotation.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun run_with(callback: |(|| void)| void) {
            callback(|| print("called"));
        }
        fun main() {
            run_with(|done: || void| {
                done();
            });
        }
        "#,
        "called\n",
    );
}

#[test]
fn calling_an_unannotated_closure_parameter_defers() {
    // FIXED: a free call whose SUBJECT is an unannotated closure parameter
    // (`|done| { done(); }`) now defers until bidirectional inference lands
    // the parameter's type — the same rule the method-receiver and argument
    // paths already had (Bug C′'s family).
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun run_with(callback: |(|| void)| void) {
            callback(|| print("called"));
        }
        fun main() {
            run_with(|done| {
                done();
            });
        }
        "#,
        "called\n",
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

#[test]
fn emitted_js_preserves_grouping_across_precedence() {
    // A latent emitter miscompile (found by the bits-and-bytes probe,
    // proposal/bits-and-bytes.md §0): the JS printer rendered binary operands
    // flat, so `(1 + 2) * 3` emitted as `1 + 2 * 3` and printed 7. Operands are
    // now parenthesized by JS precedence.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print((1 + 2) * 3);
            let a = 1;
            let b = 2;
            let c = 3;
            print((a + b) * c);
            print(0 - (a - b));
            print(a - (b - c));
            print((a + b) / (b + c) + 1);
            print((1.0 + 2.0) / (2.0 + 3.0) + 1.0);
        }
        "#,
        "9\n9\n1\n2\n1\n1.6\n",
    );
}

#[test]
fn emitted_js_parenthesizes_right_nested_string_concat() {
    // `+` is left-associative but not insensitive to grouping once strings mix
    // in: `1 + (2 + "x")` is "12x", while flat `1 + 2 + "x"` would be "3x".
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let suffix = "x";
            print(1 + (2 + suffix));
        }
        "#,
        "12x\n",
    );
}

#[test]
fn hex_literals_type_and_evaluate_like_decimal() {
    // `0x` is a spelling, not a type: suffix, context, and the i32 default all
    // apply, and the literal reaches JS verbatim (proposal/bits-and-bytes.md §1).
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(0xFF);
            print(0x10 + 1);
            let big = 0xDEADn;
            print(big);
            print(i"masked = {0xF0 & 0x1F}");
        }
        "#,
        "255\n17\n57005n\nmasked = 16\n",
    );
}

#[test]
fn bitwise_operators_on_i32_use_signed_js_semantics() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(12 & 10);
            print(12 | 3);
            print(12 ^ 10);
            print(1 << 5);
            print(0 - 8 >> 1);
        }
        "#,
        "8\n15\n6\n32\n-4\n",
    );
}

#[test]
fn bitwise_operators_on_u32_stay_unsigned() {
    // JS bitwise is signed; `u32` results re-wrap with `>>> 0` and `>>` is the
    // logical `>>>` — a set high bit must come back as a large unsigned value
    // (proposal/bits-and-bytes.md §2).
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let high: u32 = 0x80000000;
            print(high | 0);
            print(high >> 4);
            print(0xFFFFFFFFu32 >> 28);
            let one: u32 = 1;
            print(one << 31);
            print(0xF0F0F0F0u32 ^ 0xFFFFFFFFu32);
        }
        "#,
        "2147483648\n134217728\n15\n2147483648\n252645135\n",
    );
}

#[test]
fn bitwise_operators_on_bigint_do_not_wrap() {
    // BigInt is arbitrary-precision: the native JS operators apply and the u32
    // `>>> 0` normalization must NOT — `1n << 64n` exceeds 64 bits intact.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(0xFFn & 0x0Fn);
            print(1n << 64n);
        }
        "#,
        "15n\n18446744073709551616n\n",
    );
}

#[test]
fn bitwise_precedence_is_rust_order_not_c_order() {
    // `<< >>` over `&` over `^` over `|`, all over comparisons — so
    // `1 << 2 == 4` is `(1 << 2) == 4` and `1 | 2 ^ 2 & 3` is `1 | (2 ^ (2 & 3))`.
    // Emission must survive JS's DIFFERENT (C-style) order via parentheses.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(1 << 2 == 4);
            print(1 | 2 ^ 2 & 3);
            print((1 | 2) & 3 == 3);
            let masked = 0xFF & 0x0F;
            print(masked == 15);
        }
        "#,
        "true\n1\ntrue\ntrue\n",
    );
}

#[test]
fn shifts_coexist_with_nested_generics() {
    // `<<`/`>>` are two ADJACENT control tokens in expression position;
    // `List<List<i32>>` (type position) and comparisons are untouched.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let nested: List<List<i32>> = [[1, 2], [3]];
            let shifted = nested.len() << 2;
            print(shifted);
            print(1 < 2);
        }
        "#,
        "8\ntrue\n",
    );
}

#[test]
fn split_shift_stays_a_parse_error() {
    // Adjacency is load-bearing: `a < < b` must not silently become a shift.
    assert_fails(
        r#"
        fun main() {
            let a = 1;
            let b = 2;
            let c = a < < b;
        }
        "#,
    );
}

#[test]
fn bitand_dispatches_to_the_operator_trait() {
    // `&` on a struct routes through `std::operators::BitAnd::bit_and`,
    // mirroring `+`/`Add`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::operators::BitAnd;
        struct Flags { bits: i32 }
        impl Flags with BitAnd {
            fun bit_and(self, other: Flags): Flags {
                Flags { bits = self.bits & other.bits }
            }
        }
        fun main() {
            let a = Flags { bits = 12 };
            let b = Flags { bits = 10 };
            print((a & b).bits);
        }
        "#,
        "8\n",
    );
}

#[test]
fn missing_bitwise_impl_names_the_trait() {
    // A non-native type without the impl gets the operator diagnostic naming
    // the trait, mirroring `Add`.
    assert_fails(
        r#"
        struct Flags { bits: i32 }
        fun main() {
            let a = Flags { bits = 1 };
            let b = Flags { bits = 2 };
            let c = a ^ b;
        }
        "#,
    );
}

#[test]
fn bytes_buffers_round_trip() {
    // `std::bytes` (proposal/bits-and-bytes.md §3): alloc/len/get/set with the
    // host's `& 0xFF` store semantics, slice, concat, and a multibyte UTF-8
    // round-trip. The codec substrate.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::bytes::{ Bytes, encode_utf8, decode_utf8 };
        fun main() {
            let buffer = Bytes::alloc(4);
            print(buffer.len());
            buffer.set(0, 0xDE);
            buffer.set(1, 0x1FF);
            print(buffer.get(0));
            print(buffer.get(1));
            print(buffer.get(2));
            let joined = Bytes::concat(buffer.slice(0, 2), buffer);
            print(joined.len());
            let text = "héllo 🎉";
            let encoded = encode_utf8(text);
            print(encoded.len());
            print(decode_utf8(encoded) == text);
        }
        "#,
        "4\n222\n255\n0\n6\n11\ntrue\n",
    );
}

#[test]
fn generic_trait_method_dispatches_through_a_bound() {
    // FIXED: a trait method with its OWN generic parameters (describe<S: Sink>)
    // used to no-op silently through `T: Describable` — the OnConstraint
    // emission re-targeted the concrete impl's method without the call's
    // own-generic bindings (whose ids belong to the TRAIT member), so the
    // instance emitted with S unbound. The bindings now cross the re-dispatch
    // as ordered values, zipped onto the target's own generics.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        trait Sink { fun put(self, value: i32); }
        struct Collector { total: Shared<i32> }
        impl Collector with Sink {
            fun put(self, value: i32) {
                self.total.write() = self.total.read() + value;
            }
        }
        trait Describable {
            fun describe<S: Sink>(self, sink: S);
        }
        struct Point { x: i32, y: i32 }
        impl Point with Describable {
            fun describe<S: Sink>(self, sink: S) {
                sink.put(self.x);
                sink.put(self.y);
            }
        }
        fun encode<T: Describable, S: Sink>(value: T, sink: S) {
            value.describe(sink);
        }
        fun main() {
            let collector = Collector { total = Shared::new(0) };
            let point = Point { x = 3, y = 4 };
            point.describe(collector);
            print(collector.total.read());
            encode(point, collector);
            print(collector.total.read());
        }
        "#,
        "7\n14\n",
    );
}

#[test]
#[ignore = "an impl cannot bind a trait's generic argument (`impl T with Trait<type S: Bound>`)"]
fn impl_binder_in_trait_argument_position() {
    // One impl serving every sink: the binder sits in the TRAIT argument. The
    // analyzer reports "cannot find type 'S'" — unsupported, so a per-serializer
    // generic impl can't be written (transport-rpc.md §6.1's other gap).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        trait Sink { fun put(self, value: i32); }
        struct Collector { total: Shared<i32> }
        impl Collector with Sink {
            fun put(self, value: i32) {
                self.total.write() = self.total.read() + value;
            }
        }
        trait DescribeInto<S> {
            fun describe_into(self, sink: S);
        }
        struct Point { x: i32 }
        impl Point with DescribeInto<type S: Sink> {
            fun describe_into(self, sink: S) {
                sink.put(self.x);
            }
        }
        fun main() {
            let point = Point { x = 3 };
            let collector = Collector { total = Shared::new(0) };
            point.describe_into(collector);
            print(collector.total.read());
        }
        "#,
        "3\n",
    );
}

#[test]
fn hand_written_wire_impls_round_trip_through_json() {
    // The §6.1 visitor, proven hand-written before the derive targets it: a
    // struct (scalar/list/option/nested-enum fields) and an enum (0/1/2-arity
    // variants) describe to `JsonWriter` and rebuild from `JsonReader`. The
    // encoded text must match the established `to_json` wire format exactly
    // (externally-tagged variants, arity>1 payload arrays, bare `Some`,
    // `null` for `None`), and structural failures are sticky decode errors —
    // backlog I3's validating decode.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::wire::{ Wire, Serialize, Deserialize };
        import std::json::{ encode_json, decode_json };

        enum Status {
            Offline,
            Away(str),
            Busy(str, i32),
        }

        impl Status with Wire {
            fun describe<S: Serialize>(self, serializer: S) {
                match self {
                    Status::Offline => {
                        serializer.begin_variant("Offline", 0);
                        serializer.end_variant();
                    },
                    Status::Away(let reason) => {
                        serializer.begin_variant("Away", 1);
                        reason.describe(serializer);
                        serializer.end_variant();
                    },
                    Status::Busy(let task, let minutes) => {
                        serializer.begin_variant("Busy", 2);
                        task.describe(serializer);
                        minutes.describe(serializer);
                        serializer.end_variant();
                    },
                }
            }

            fun rebuild<D: Deserialize>(deserializer: D): Status {
                let tag = deserializer.variant_tag();
                match tag {
                    "Offline" => {
                        deserializer.begin_variant("Offline", 0);
                        deserializer.end_variant();
                        Status::Offline
                    },
                    "Away" => {
                        deserializer.begin_variant("Away", 1);
                        let reason = str::rebuild(deserializer);
                        deserializer.end_variant();
                        Status::Away(reason)
                    },
                    "Busy" => {
                        deserializer.begin_variant("Busy", 2);
                        let task = str::rebuild(deserializer);
                        let minutes = i32::rebuild(deserializer);
                        deserializer.end_variant();
                        Status::Busy(task, minutes)
                    },
                    _ => {
                        deserializer.fail(i"unknown variant '{tag}'");
                        Status::Offline
                    },
                }
            }
        }

        struct Profile {
            id: i32,
            name: str,
            scores: List<i32>,
            nickname: Option<str>,
            status: Status,
        }

        impl Profile with Wire {
            fun describe<S: Serialize>(self, serializer: S) {
                serializer.begin_struct(5);
                serializer.field("id");
                self.id.describe(serializer);
                serializer.field("name");
                self.name.describe(serializer);
                serializer.field("scores");
                self.scores.describe(serializer);
                serializer.field("nickname");
                self.nickname.describe(serializer);
                serializer.field("status");
                self.status.describe(serializer);
                serializer.end_struct();
            }

            fun rebuild<D: Deserialize>(deserializer: D): Profile {
                deserializer.begin_struct();
                deserializer.field("id");
                let id = i32::rebuild(deserializer);
                deserializer.field("name");
                let name = str::rebuild(deserializer);
                deserializer.field("scores");
                let scores: List<i32> = List::rebuild(deserializer);
                deserializer.field("nickname");
                let nickname: Option<str> = Option::rebuild(deserializer);
                deserializer.field("status");
                let status = Status::rebuild(deserializer);
                deserializer.end_struct();
                Profile { id = id, name = name, scores = scores, nickname = nickname, status = status }
            }
        }

        fun main() {
            let profile = Profile {
                id = 7,
                name = "ada \"the\" first",
                scores = [3, 1, 4],
                nickname = None,
                status = Status::Busy("proofs", 45),
            };
            let encoded = encode_json(profile);
            print(encoded);
            let decoded: Result<Profile, str> = decode_json(encoded);
            match decoded {
                Ok(let back) => {
                    print(back.id);
                    print(back.scores.len());
                    match back.status {
                        Status::Busy(let task, let minutes) => print(i"busy {task} {minutes}"),
                        _ => print("wrong status"),
                    }
                },
                Err(let reason) => print(i"decode failed: {reason}"),
            }
            print(encode_json(Profile { id = 1, name = "bob", scores = [], nickname = Some("bo"), status = Status::Away("lunch") }));
            let missing: Result<Profile, str> = decode_json("{\"id\":1,\"name\":\"x\",\"scores\":[]}");
            match missing {
                Ok(let value) => print("should have failed"),
                Err(let reason) => print(i"err: {reason}"),
            }
            let unknown: Result<Status, str> = decode_json("{\"Vanished\":1}");
            match unknown {
                Ok(let value) => print("should have failed"),
                Err(let reason) => print(i"err: {reason}"),
            }
        }
        "#,
        "{\"id\":7,\"name\":\"ada \\\"the\\\" first\",\"scores\":[3,1,4],\"nickname\":null,\"status\":{\"Busy\":[\"proofs\",45]}}\n7\n3\nbusy proofs 45\n{\"id\":1,\"name\":\"bob\",\"scores\":[],\"nickname\":\"bo\",\"status\":{\"Away\":\"lunch\"}}\nerr: missing field 'nickname'\nerr: unknown variant 'Vanished'\n",
    );
}

#[test]
fn qualified_generic_static_resolves_inner_trait_statics() {
    // FIXED: `List<i32>::rebuild(d)` (the qualified-generic spelling) used to
    // emit the inner `T::rebuild` as an EMPTY function — the accessor resolution
    // discarded the subject's type args entirely. A qualified subject now seeds
    // the matched impl's binder bindings into ITS call's substitution.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Build {
            fun build(seed: i32): Build;
        }
        impl i32 with Build {
            fun build(seed: i32): i32 { seed + 1 }
        }
        struct Boxy<T> { value: T }
        impl Boxy<type T: Build> {
            fun make(seed: i32): Boxy<T> {
                Boxy { value = T::build(seed) }
            }
        }
        fun main() {
            let via_annotation: Boxy<i32> = Boxy::make(1);
            print(via_annotation.value);
            let via_qualified = Boxy<i32>::make(1);
            print(via_qualified.value);
        }
        "#,
        "2\n2\n",
    );
}

#[test]
fn derived_wire_visitor_matches_to_json_and_round_trips() {
    // `[derive(Wire)]` now also emits the §6.1 visitor impls: the described
    // output must equal the derived `to_json` byte-for-byte, rebuild must
    // round-trip (scalars, List, Option, a nested derived enum), and
    // structural failures surface as sticky decode errors through the
    // GENERATED rebuilds.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ Json, encode_json, decode_json };

        [derive(Wire)]
        enum Status {
            Offline,
            Away(str),
            Busy(str, i32),
        }

        [derive(Wire)]
        struct Profile {
            id: i32,
            name: str,
            scores: List<i32>,
            nickname: Option<str>,
            status: Status,
        }

        fun main() {
            let profile = Profile {
                id = 7,
                name = "ada",
                scores = [3, 1, 4],
                nickname = None,
                status = Status::Busy("proofs", 45),
            };
            let via_visitor = encode_json(profile);
            print(via_visitor == profile.to_json());
            let decoded: Result<Profile, str> = decode_json(via_visitor);
            match decoded {
                Ok(let back) => {
                    print(back.id);
                    match back.status {
                        Status::Busy(let task, let minutes) => print(i"busy {task} {minutes}"),
                        _ => print("wrong"),
                    }
                },
                Err(let reason) => print(i"failed: {reason}"),
            }
            let missing: Result<Profile, str> = decode_json("{\"id\":1}");
            match missing {
                Ok(let value) => print("should fail"),
                Err(let reason) => print(i"err: {reason}"),
            }
            let unknown: Result<Status, str> = decode_json("\"Vanished\"");
            match unknown {
                Ok(let value) => print("should fail"),
                Err(let reason) => print(i"err: {reason}"),
            }
        }
        "#,
        "true\n7\nbusy proofs 45\nerr: missing field 'name'\nerr: unknown variant 'Vanished'\n",
    );
}

#[test]
fn derived_struct_with_two_differently_typed_options() {
    // FIXED (same root as the qualified-static gap): with the subject's type
    // args discarded, `Option<str>::from_json_value(..)` and
    // `Option<i32>::from_json_value(..)` in one generated function fought over
    // one shared binder — use sites failed with "Expected Option<i32>, but got
    // Option<str>". Per-call subject bindings keep the two instantiations apart.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        [derive(Json)]
        struct OnlyOptions {
            nick: Option<str>,
            zero: Option<i32>,
        }
        fun main() {
            let value = OnlyOptions { nick = Some("bo"), zero = Some(0) };
            match value.nick {
                Some(let nick) => print(i"nick {nick}"),
                None => print("no nick"),
            }
            match value.zero {
                Some(let zero) => print(i"zero {zero}"),
                None => print("no zero"),
            }
        }
        "#,
        "nick bo\nzero 0\n",
    );
}

#[test]
fn both_codecs_round_trip_derived_wire_values() {
    // §6.2 end-to-end: one derived value through `json_codec()` and
    // `binary_codec()` — negative i32, high-bit u32, f64, multibyte str,
    // List, BOTH Option marker paths (Some(0) is exactly what the binary
    // `0x01` marker disambiguates from None's `0x00`), and a 2-arity enum.
    // Plus the failure modes: a frame of the wrong kind arrives pre-poisoned,
    // and a truncated binary frame fails sticky instead of crashing.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::wire::{ Wire, Frame, Codec, encode, decode };
        import std::json::{ Json, json_codec };
        import std::binary::binary_codec;

        [derive(Wire)]
        enum Status {
            Offline,
            Busy(str, i32),
        }

        [derive(Wire)]
        struct Probe {
            id: i32,
            big: u32,
            ratio: f64,
            label: str,
            flags: List<bool>,
            zero: Option<i32>,
            status: Status,
        }

        fun sample(zero: Option<i32>): Probe {
            Probe {
                id = 0 - 42,
                big = 0xDEADBEEF,
                ratio = 0.5,
                label = "héllo 🎉",
                flags = [true, false, true],
                zero = zero,
                status = Status::Busy("proofs", 45),
            }
        }

        fun check(name: str, back: Result<Probe, str>) {
            match back {
                Ok(let value) => {
                    let intact =
                        value.id == 0 - 42 && value.big == 0xDEADBEEFu32
                        && value.ratio == 0.5 && value.label == "héllo 🎉"
                        && value.flags.len() == 3;
                    print(i"{name} intact = {intact}");
                    match value.zero {
                        Some(let n) => print(i"{name} zero = {n}"),
                        None => print(i"{name} zero = none"),
                    }
                },
                Err(let reason) => print(i"{name} failed: {reason}"),
            }
        }

        fun main() {
            let json = json_codec();
            let binary = binary_codec();
            check("json", decode(json, encode(json, sample(Some(0)))));
            check("binary", decode(binary, encode(binary, sample(Some(0)))));
            check("binary-none", decode(binary, encode(binary, sample(None))));
            let crossed: Result<Probe, str> = decode(binary, encode(json, sample(Some(0))));
            match crossed {
                Ok(let value) => print("should fail"),
                Err(let reason) => print(i"err: {reason}"),
            }
            match encode(binary, sample(Some(0))) {
                Frame::Binary(let whole) => {
                    let cut: Result<Probe, str> = decode(binary, Frame::Binary(whole.slice(0, 9)));
                    match cut {
                        Ok(let value) => print("should fail"),
                        Err(let reason) => print(i"err: {reason}"),
                    }
                },
                Frame::Text(let text) => print("unexpected"),
            }
        }
        "#,
        "json intact = true\njson zero = 0\nbinary intact = true\nbinary zero = 0\nbinary-none intact = true\nbinary-none zero = none\nerr: binary codec: received a text frame\nerr: unexpected end of frame\n",
    );
}

#[test]
fn generated_decode_gate_rejects_a_garbled_request() {
    // The §4.1 validating decode, end to end through GENERATED code: a raw
    // envelope calling `add` with no arguments makes the handler's arg pull
    // fail (binary: out of bounds), and the generated `decode_failed` gate
    // returns `RpcError::Decode` instead of running the impl on zero values —
    // the server's counter must still be 0 afterwards.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::result::Result::{ self, Ok, Err };
        import std::json::Json;
        import std::binary::binary_codec;
        import std::rpc::{ local_rpc, RpcError, call };

        [service(Client)]
        struct Counter {
            count: Shared<i32>,
        }

        impl Counter {
            [rpc]
            fun add(self, by: i32): i32 {
                self.count.write() = self.count.read() + by;
                self.count.read()
            }
        }

        fun main() {
            let counter = Counter { count = Shared::new(0) };
            let transport = local_rpc(counter.dispatcher().into_protocol(binary_codec()));
            // A hand-built envelope with ZERO args for a one-arg method.
            let garbled: Result<i32, RpcError> = call(transport, binary_codec(), "add", []);
            match garbled {
                Ok(let value) => print("should have failed"),
                Err(let error) => print(i"err: {error.to_json()}"),
            }
            let untouched = counter.count.read();
            print(i"count still {untouched}");
        }
        "#,
        "err: {\"Decode\":\"unexpected end of frame\"}\ncount still 0\n",
    );
}

#[test]
fn ws_parser_handles_the_rfc_vectors() {
    // std::ws (transport-rpc.md §5): the RFC 6455 masked "Hello" vector, the
    // same frame split across two feeds, our own encoder round-tripped, the
    // 16-bit length ladder, fragmentation reassembly, ping surfacing, and
    // close ending the stream (later frames ignored).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::bytes::{ Bytes, encode_utf8 };
        import std::ws::{ WsParser, WsEvent, text_frame, encode_frame, close_frame };

        fun show(events: List<WsEvent>) {
            for event in events {
                match event {
                    WsEvent::Text(let text) => print(i"text: {text}"),
                    WsEvent::Binary(let bytes) => print(i"binary: {bytes.len()} bytes"),
                    WsEvent::Ping(let payload) => print(i"ping: {payload.len()} bytes"),
                    WsEvent::Closed => print("closed"),
                }
            }
        }

        fun masked_hello(): Bytes {
            let masked = Bytes::alloc(11);
            masked.set(0, 0x81);
            masked.set(1, 0x85);
            masked.set(2, 0x37);
            masked.set(3, 0xFA);
            masked.set(4, 0x21);
            masked.set(5, 0x3D);
            masked.set(6, 0x7F);
            masked.set(7, 0x9F);
            masked.set(8, 0x4D);
            masked.set(9, 0x51);
            masked.set(10, 0x58);
            masked
        }

        fun main() {
            let parser = WsParser::new();
            show(parser.feed(masked_hello()));
            let splitter = WsParser::new();
            show(splitter.feed(masked_hello().slice(0, 5)));
            print("(partial fed)");
            show(splitter.feed(masked_hello().slice(5, 11)));
            let echo = WsParser::new();
            show(echo.feed(text_frame("server says hi")));
            let big = encode_frame(0x2, Bytes::alloc(200));
            print(i"200B frame = {big.len()} bytes on the wire");
            show(echo.feed(big));
            let part1 = text_frame("Hel");
            part1.set(0, 0x01);
            let part2 = text_frame("lo");
            part2.set(0, 0x80);
            let fragmented = WsParser::new();
            show(fragmented.feed(Bytes::concat(part1, part2)));
            let control = WsParser::new();
            show(control.feed(encode_frame(0x9, encode_utf8("hb"))));
            show(control.feed(close_frame()));
            show(control.feed(text_frame("after close")));
            print("done");
        }
        "#,
        "text: Hello\n(partial fed)\ntext: Hello\ntext: server says hi\n200B frame = 204 bytes on the wire\nbinary: 200 bytes\ntext: Hello\nping: 2 bytes\nclosed\ndone\n",
    );
}

#[test]
fn client_connect_enforces_the_contract_and_wires_mirrors() {
    // §4.2's Client::connect, end to end over a real WebSocket: one generated
    // call opens the socket, VERIFIES the contract hash (Q6 enforcement — the
    // drift case below refuses with Err(Contract) before any decode), calls
    // the generated __attach against the runtime session registry
    // (serve_service), and wires one RemoteSource mirror per [expose]d field
    // in declaration order — both mirrors deliver.
    assert_compiles_and_runs(
        r#"
import std::print;
        import std::process::exit;
        import std::time::sleep;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ Json, FromJson, json_codec };
        import std::reactive::Signal;
        import std::shared::Shared;
        import std::rpc_server::serve_service;
        import std::http::Response;
        
        // The whole paradigm, zero manual wiring: [expose]d state + [rpc] methods,
        // serve_service on the server, Client::connect on the client.
        [service(Client)]
        struct Board {
        	[expose] count: Signal<i32>,
        	[expose] label: Signal<str>,
        	total: Shared<i32>,
        }
        
        impl Board {
        	[rpc]
        	fun add(self, by: i32): i32 {
        		self.count.set(self.count.get() + by);
        		self.total.write() = self.total.read() + by;
        		self.label.set(i"sum {self.count.get()}");
        		self.count.get()
        	}
        }
        
        // A second, DIFFERENT service on another port — the drift case.
        [service(OtherClient)]
        struct Other {
        	value: Shared<i32>,
        }
        
        impl Other {
        	[rpc]
        	fun ping(self): i32 { 1 }
        }
        
        fun main() {
        	let board = Board { count = Signal::new(0), label = Signal::new(""), total = Shared::new(0) };
        	serve_service(
        		48411,
        		board.dispatcher().into_protocol(json_codec()),
        		|request| Response::builder().code(404).body("probe").build(),
        		|| {
        			let other = Other { value = Shared::new(0) };
        			serve_service(
        				48412,
        				other.dispatcher().into_protocol(json_codec()),
        				|request| Response::builder().code(404).body("probe").build(),
        				|| drive(),
        			);
        		},
        	);
        }
        
        fun drive() {
        	// One call: socket + contract enforcement + attach + mirrors.
        	match Client::connect("ws://localhost:48411", json_codec()) {
        		Ok(let client) => {
        			// Typed mirrors: values arrive decoded at each field's type.
        			let counting = client.count.sub(|n| {
        				print(i"count = {n}");
        			});
        			let labeling = client.label.sub(|s| {
        				if s != "" {
        					print(i"label = {s}");
        				}
        			});
        			match client.add(7) {
        				Ok(let n) => print(i"add -> {n}"),
        				Err(let error) => print(i"add err {error.to_json()}"),
        			}
        			sleep(300);
        			// Drift: a Board client pointed at Other's server refuses cleanly.
        			match Client::connect("ws://localhost:48412", json_codec()) {
        				Ok(let wrong) => print("drift NOT caught"),
        				Err(let error) => print(i"drift: {error.to_json()}"),
        			}
        			sleep(100);
        			exit(0);
        		},
        		Err(let error) => {
        			print(i"connect failed: {error.to_json()}");
        			exit(1);
        		},
        	}
        }

        "#,
        "count = 0\ncount = 7\nlabel = sum 7\nadd -> 7\ndrift: {\"Contract\":\"the server reports a different service surface\"}\n",
    );
}

// --- Bare `ret` (return void) -------------------------------------------------

// `ret` with no value is a void early-return: the guard exits before the print,
// and the non-guarded call falls through to it.
#[test]
fn bare_ret_returns_void_early() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun guard(flag: bool) {
        	if flag {
        		ret;
        	}
        	print("passed");
        }

        fun main() {
        	guard(true);
        	guard(false);
        }
        "#,
        "passed\n",
    );
}

// A `ret` value must match the declared return type (proposal/ret-checking.md:
// `ret` joins the tail's `ReturnType` constraint, which now verifies via
// `reconcile_type` instead of only directing inference).
#[test]
fn ret_value_is_checked_against_the_declared_return_type() {
    assert_fails(
        r#"
        fun bad(): i32 {
        	ret "nope";
        	1
        }

        fun main() {
        	let _ = bad();
        }
        "#,
    );
}

// The void case: a bare `ret` is `ret <void>` — legal exactly when the
// declared return type is void, rejected in a value-returning function.
#[test]
fn bare_ret_in_a_value_returning_function_is_rejected() {
    assert_fails(
        r#"
        fun bad(flag: bool): i32 {
        	if flag {
        		ret;
        	}
        	1
        }

        fun main() {
        	let _ = bad(true);
        }
        "#,
    );
}

// --- Malformed frames are decode errors, never crashes -------------------------

// The JSON codec's reader must arrive PRE-POISONED on text that is not JSON at
// all (wire frames are untrusted input): `decode` returns `Err`, and an RPC
// protocol answers a garbage request with `Failure(Decode)` — it used to throw
// out of `JSON.parse`, letting one malformed request kill a server process.
#[test]
fn malformed_json_frames_fail_sticky_instead_of_crashing() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ json_codec, decode_json };
        import std::wire::{ decode, Frame };
        import std::rpc::{ Dispatcher, reply, RpcOutcome, RpcError };

        fun main() {
        	// The decode seam: garbage text and a garbage binary frame both Err.
        	let direct: Result<i32, str> = decode_json("garbage{{{");
        	match direct {
        		Ok(let value) => print("direct: unexpected Ok"),
        		Err(let reason) => print(i"direct: {reason}"),
        	}
        	let framed: Result<i32, str> = decode(json_codec(), Frame::Text("also not json"));
        	match framed {
        		Ok(let value) => print("framed: unexpected Ok"),
        		Err(let reason) => print(i"framed: {reason}"),
        	}
        	// The RPC seam: a protocol ANSWERS a garbage request (Failure
        	// envelope), it does not throw.
        	let protocol = Dispatcher::new().on("ping", |request| reply(1)).into_protocol(json_codec());
        	let answer = protocol.respond(Frame::Text("garbage{{{"));
        	match answer {
        		Frame::Text(let envelope) => print(i"rpc answers: {envelope}"),
        		Frame::Binary(let bytes) => print("rpc: unexpected binary"),
        	}
        }
        "#,
        "direct: malformed JSON\nframed: malformed JSON\nrpc answers: {\"Failure\":{\"Decode\":\"malformed JSON\"}}\n",
    );
}

// The wider half of the same gap (proposal/ret-checking.md): the TAIL was not
// checked either — `Constraint::ReturnType` directed inference but never
// verified. `fun f(): i32 { "nope" }` used to compile clean.
#[test]
fn function_tail_is_checked_against_the_declared_return_type() {
    assert_fails(
        r#"
        fun bad(): i32 {
        	"nope"
        }

        fun main() {
        	let _ = bad();
        }
        "#,
    );
}

// A void CALL is not a value: caught in tail position...
#[test]
fn a_void_call_tail_is_not_a_value_return() {
    assert_fails(
        r#"
        import std::print;

        fun bad(): i32 {
        	print("side effect")
        }

        fun main() {
        	let _ = bad();
        }
        "#,
    );
}

// ...and in `ret` position.
#[test]
fn a_void_call_ret_is_not_a_value_return() {
    assert_fails(
        r#"
        import std::print;

        fun bad(): i32 {
        	ret print("side effect");
        	1
        }

        fun main() {
        	let _ = bad();
        }
        "#,
    );
}

// One bad `ret` among good ones is flagged — the check is per return site,
// not per function.
#[test]
fn one_bad_ret_among_good_ones_is_flagged() {
    assert_fails(
        r#"
        fun bad(a: bool, b: bool): i32 {
        	if a {
        		ret 1;
        	}
        	if b {
        		ret "two";
        	}
        	3
        }

        fun main() {
        	let _ = bad(true, false);
        }
        "#,
    );
}

// In a function with NO declared return type, `ret <value>` is unchecked and
// the value is discarded — the same rule as the (unchecked) tail of a void
// function. Consistency with the tail is the deliberate semantic
// (proposal/ret-checking.md rule 3).
#[test]
fn ret_with_a_value_in_an_undeclared_void_function_is_allowed() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun loud(flag: bool) {
        	if flag {
        		ret print("early");
        	}
        	print("late");
        }

        fun main() {
        	loud(true);
        	loud(false);
        }
        "#,
        "early\nlate\n",
    );
}

// A generic return type checks `ret` by unification, exactly like the tail.
#[test]
fn generic_return_rets_bind_like_the_tail() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;

        fun pick<T>(flag: bool, a: T, b: T): T {
        	if flag {
        		ret a;
        	}
        	b
        }

        fun main() {
        	print(format(pick(true, 1, 2)));
        	print(pick(false, "x", "y"));
        }
        "#,
        "1\ny\n",
    );
}

// `ret` is a first-class return position: a return-position generic call binds
// its type parameters from the declared type through `ret`, like the tail.
#[test]
fn ret_directs_return_position_generics() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;

        fun fresh(flag: bool): List<i32> {
        	if flag {
        		ret List::new();
        	}
        	[7]
        }

        fun main() {
        	print(format(fresh(true).len()));
        	print(format(fresh(false).len()));
        }
        "#,
        "0\n1\n",
    );
}

// An `async` function's `ret` checks against its declared return type.
#[test]
fn async_function_rets_check_against_the_declared_type() {
    assert_fails(
        r#"
        async fun bad(flag: bool): i32 {
        	if flag {
        		ret "nope";
        	}
        	1
        }

        async fun main() {
        	let _ = await bad(true);
        }
        "#,
    );
}

// `ret` returns from the NEAREST callable: a closure (or `async` block) is its
// own boundary — at runtime `ret` exits the closure, not the function, and an
// agreeing early-exit ret checks cleanly against the body's tail type.
#[test]
fn ret_inside_a_closure_exits_the_closure() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;

        fun apply(f: |i32| i32): i32 {
        	f(10)
        }

        fun main() {
        	let result = apply(|x| {
        		if x > 5 {
        			ret 99;
        		}
        		x + 1
        	});
        	print(format(result));
        	print("after");
        }
        "#,
        "99\nafter\n",
    );
}

// A closure's `ret` PARTICIPATES in its return typing: a ret disagreeing with
// the body's tail type is rejected (the collected-rets constraint —
// proposal/ret-checking.md rule 4's follow-up, now shipped).
#[test]
fn ret_participates_in_closure_return_inference() {
    assert_fails(
        r#"
        fun apply(f: |i32| i32): i32 {
        	f(10)
        }

        fun main() {
        	let _ = apply(|x| {
        		if x > 5 {
        			ret "mismatched";
        		}
        		x + 1
        	});
        }
        "#,
    );
}

// A trait-typed `self` returns through a trait-typed signature (the
// `impl Iterator<type T> with Iterable<T> { fun iter(self): Iterator<T> { self } }`
// shape) — pins the `(Trait, Trait)` reconcile arm the return check surfaced.
#[test]
fn a_trait_typed_self_returns_through_a_trait_typed_signature() {
    assert_compiles(
        r#"
        import std::option::Option::{ self, Some, None };

        trait Walk<T> {
        	fun step(self): Option<T>;
        }

        trait AsWalk<T> {
        	fun as_walk(self): Walk<T>;
        }

        impl Walk<type T> with AsWalk<T> {
        	fun as_walk(self): Walk<T> {
        		self
        	}
        }

        fun main() {}
        "#,
    );
}

// --- Diagnostic span precision (backlog E7) ------------------------------------
// Each pins that the error's span covers exactly the PERTINENT expression, not
// an enclosing aggregate — a regression back to the coarse span fails the
// exact-range assertion.

// A match-leg mismatch points at the offending leg's body, not the whole match.
#[test]
fn match_leg_mismatch_spans_the_offending_leg() {
    assert_fails_spanning(
        r#"
        fun pick(flag: bool): i32 {
        	match flag {
        		true => 1,
        		false => "oops",
        	}
        }

        fun main() {
        	let _ = pick(true);
        }
        "#,
        "\"oops\"",
        "match legs have mismatched types",
    );
}

// A struct-initializer field mismatch points at that field's value, not the
// whole `{ .. }` block.
#[test]
fn struct_field_mismatch_spans_the_field_value() {
    assert_fails_spanning(
        r#"
        struct Point {
        	x: i32,
        	y: i32,
        }

        fun main() {
        	let _ = Point { x = 1, y = "two" };
        }
        "#,
        "\"two\"",
        "Expected i32, but got str",
    );
}

// An unknown struct name anchors at the initializer (which includes the name),
// not the field block alone.
#[test]
fn unknown_struct_spans_the_initializer() {
    assert_fails_spanning(
        r#"
        fun main() {
        	let _ = Pointt { x = 1 };
        }
        "#,
        "Pointt { x = 1 }",
        "unknown struct",
    );
}

// A missing import segment points at that segment, not the whole statement.
#[test]
fn import_segment_error_spans_the_segment() {
    assert_fails_spanning(
        r#"
        import std::option::Optionn;

        fun main() {}
        "#,
        "Optionn",
        "cannot find 'Optionn' in the imported path",
    );
}

// An unknown import ROOT points at the root segment.
#[test]
fn import_root_error_spans_the_root() {
    assert_fails_spanning(
        r#"
        import nowhere::thing;

        fun main() {}
        "#,
        "nowhere",
        "cannot find module 'nowhere' to import",
    );
}

// A missing `use` segment points at that segment.
#[test]
fn use_segment_error_spans_the_segment() {
    assert_fails_spanning(
        r#"
        import std::option::Option;

        fun main() {
        	use Option::Somme;
        	let _ = 1;
        }
        "#,
        "Somme",
        "cannot find 'Somme' in the `use` path",
    );
}

// --- `expr!` — assert-or-return (proposal/try-and-lift.md, slice 1) -------------

// The happy and early paths, on both std types, with the early return proven
// by an unreached side effect.
#[test]
fn bang_unwraps_good_and_returns_bad() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };

        fun lookup(key: str): Option<i32> {
        	if key == "hit" {
        		Some(21)
        	} else {
        		None
        	}
        }

        fun doubled(key: str): Option<i32> {
        	let value = lookup(key)!;
        	print("unwrapped");
        	Some(value * 2)
        }

        fun to_number(text: str): Result<i32, str> {
        	match text.parse_i32() {
        		Some(let value) => Ok(value),
        		None => Err(i"not a number: {text}"),
        	}
        }

        fun sum(a: str, b: str): Result<i32, str> {
        	let left = to_number(a)!;
        	let right = to_number(b)!;
        	Ok(left + right)
        }

        fun main() {
        	match doubled("hit") {
        		Some(let v) => print(i"some {format(v)}"),
        		None => print("none"),
        	}
        	match doubled("miss") {
        		Some(let v) => print(i"some {format(v)}"),
        		None => print("none"),
        	}
        	match sum("2", "40") {
        		Ok(let v) => print(i"ok {format(v)}"),
        		Err(let e) => print(i"err {e}"),
        	}
        	match sum("2", "forty") {
        		Ok(let v) => print(i"ok {format(v)}"),
        		Err(let e) => print(i"err {e}"),
        	}
        }
        "#,
        "unwrapped\nsome 42\nnone\nok 42\nerr not a number: forty\n",
    );
}

// A user `Try` type behaves exactly like the std pair — the §8.3 equivalence
// pin: real trait dispatch through `verdict`/`from_bad`.
#[test]
fn a_user_try_type_behaves_like_the_std_pair() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::operators::{ Try, Verdict };

        enum Lint {
        	Clean(i32),
        	Dirty(str),
        }

        impl Lint with Try<i32, str> {
        	fun verdict(self): Verdict<i32, str> {
        		match self {
        			Lint::Clean(let score) => Verdict::Good(score),
        			Lint::Dirty(let complaint) => Verdict::Bad(complaint),
        		}
        	}

        	fun from_bad(bad: str): Lint {
        		Lint::Dirty(bad)
        	}
        }

        fun check(source: str): Lint {
        	if source == "tidy" {
        		Lint::Clean(95)
        	} else {
        		Lint::Dirty(i"messy: {source}")
        	}
        }

        fun grade(source: str): Lint {
        	let score = check(source)!;
        	print("scored");
        	Lint::Clean(score + 5)
        }

        fun main() {
        	match grade("tidy") {
        		Lint::Clean(let score) => print(i"clean {format(score)}"),
        		Lint::Dirty(let complaint) => print(complaint),
        	}
        	match grade("sloppy") {
        		Lint::Clean(let score) => print(i"clean {format(score)}"),
        		Lint::Dirty(let complaint) => print(complaint),
        	}
        }
        "#,
        "scored\nclean 100\nmessy: sloppy\n",
    );
}

// `!` works in async functions (the declared return type is the frame).
#[test]
fn bang_works_in_async_functions() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::result::Result::{ self, Ok, Err };

        async fun fetch_number(flag: bool): Result<i32, str> {
        	if flag {
        		Ok(7)
        	} else {
        		Err("offline")
        	}
        }

        async fun doubled(flag: bool): Result<i32, str> {
        	let value = (await fetch_number(flag))!;
        	Ok(value * 2)
        }

        async fun main() {
        	match await doubled(true) {
        		Ok(let v) => print(i"ok {format(v)}"),
        		Err(let e) => print(e),
        	}
        	match await doubled(false) {
        		Ok(let v) => print(i"ok {format(v)}"),
        		Err(let e) => print(e),
        	}
        }
        "#,
        "ok 14\noffline\n",
    );
}

// `!` binds tighter than comparison, and `a!=b` stays a comparison (the lex
// rule: `!=` wins; the postfix form needs the space).
#[test]
fn bang_spacing_against_not_equals() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };

        fun pick(): Option<i32> {
        	Some(3)
        }

        fun compare(): Option<bool> {
        	let a = 3;
        	let b = 4;
        	// `a!=b` is not-equals on plain values...
        	if a!=b {
        		print("a != b");
        	}
        	// ...while `pick()! == a` unwraps then compares.
        	Some(pick()! == a)
        }

        fun main() {
        	match compare() {
        		Some(let equal) => print(if equal { "equal" } else { "not equal" }),
        		None => print("none"),
        	}
        }
        "#,
        "a != b\nequal\n",
    );
}

// The error cases, each pinned at the pertinent span (E7 harness).
#[test]
fn bang_on_option_requires_an_option_function() {
    assert_fails_spanning(
        r#"
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };

        fun lookup(): Option<i32> {
        	Some(1)
        }

        fun bad(): Result<i32, str> {
        	let value = lookup()!;
        	Ok(value)
        }

        fun main() {
        	let _ = bad();
        }
        "#,
        "lookup()!",
        "must return `Option`",
    );
}

#[test]
fn bang_result_error_types_must_match() {
    assert_fails_spanning(
        r#"
        import std::result::Result::{ self, Ok, Err };

        fun inner(): Result<i32, str> {
        	Ok(1)
        }

        fun bad(): Result<i32, i32> {
        	let value = inner()!;
        	Ok(value)
        }

        fun main() {
        	let _ = bad();
        }
        "#,
        "inner()!",
        "error types must match",
    );
}

#[test]
fn bang_in_a_bare_void_function_is_rejected() {
    assert_fails_spanning(
        r#"
        import std::option::Option::{ self, Some, None };

        fun lookup(): Option<i32> {
        	Some(1)
        }

        fun bad() {
        	let _ = lookup()!;
        }

        fun main() {
        	bad();
        }
        "#,
        "lookup()!",
        "requires the nearest enclosing function",
    );
}

#[test]
fn bang_in_a_closure_is_rejected_v1() {
    assert_fails_spanning(
        r#"
        import std::option::Option::{ self, Some, None };

        fun lookup(): Option<i32> {
        	Some(1)
        }

        fun outer(): Option<i32> {
        	let helper = |x: i32| {
        		let value = lookup()!;
        		value + x
        	};
        	Some(helper(1))
        }

        fun main() {
        	let _ = outer();
        }
        "#,
        "lookup()!",
        "closures and `async` blocks are not yet supported",
    );
}

#[test]
fn bang_on_a_non_try_type_is_rejected() {
    assert_fails_spanning(
        r#"
        import std::option::Option::{ self, Some, None };

        fun bad(): Option<i32> {
        	let n = 5;
        	let value = n!;
        	Some(value)
        }

        fun main() {
        	let _ = bad();
        }
        "#,
        "n!",
        "needs a value implementing `Try`",
    );
}

// A user `Try` type's enclosing return must equal the receiver exactly (v1).
#[test]
fn user_try_requires_the_exact_return_type() {
    assert_fails_spanning(
        r#"
        import std::option::Option::{ self, Some, None };
        import std::operators::{ Try, Verdict };

        enum Lint {
        	Clean(i32),
        	Dirty(str),
        }

        impl Lint with Try<i32, str> {
        	fun verdict(self): Verdict<i32, str> {
        		match self {
        			Lint::Clean(let score) => Verdict::Good(score),
        			Lint::Dirty(let complaint) => Verdict::Bad(complaint),
        		}
        	}

        	fun from_bad(bad: str): Lint {
        		Lint::Dirty(bad)
        	}
        }

        fun check(): Lint {
        	Lint::Clean(1)
        }

        fun bad(): Option<i32> {
        	let score = check()!;
        	Some(score)
        }

        fun main() {
        	let _ = bad();
        }
        "#,
        "check()!",
        "must match exactly",
    );
}

// `void` is the unit expression — the unit type's one value, usable wherever a
// void-typed value is (generic arguments included).
#[test]
fn void_is_the_unit_expression() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };

        fun consume(value: void): i32 {
        	7
        }

        fun confirm(flag: bool): Result<void, str> {
        	if flag {
        		Ok(void)
        	} else {
        		Err("refused")
        	}
        }

        fun main() {
        	print(consume(void));
        	let unit: Option<void> = Some(void);
        	match unit {
        		Some(let _v) => print("some unit"),
        		None => print("none"),
        	}
        	match confirm(true) {
        		Ok(let _v) => print("confirmed"),
        		Err(let e) => print(e),
        	}
        }
        "#,
        "7\nsome unit\nconfirmed\n",
    );
}

// --- `a?.b` — lifted member chains (proposal/try-and-lift.md, slice 2) ----------

// Map and flatten, typed and run: a plain-valued continuation wraps back into
// the container; a container-valued one flattens (single Option, not nested).
// The None subject short-circuits — proven by an unreached side effect.
#[test]
fn lift_maps_flattens_and_short_circuits() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::option::Option::{ self, Some, None };

        struct Profile {
        	name: str,
        }

        impl Profile {
        	fun loud_name(self): str {
        		print("computed");
        		self.name
        	}

        	fun nickname(self): Option<str> {
        		if self.name == "ada" {
        			Some("the countess")
        		} else {
        			None
        		}
        	}
        }

        fun user(key: str): Option<Profile> {
        	if key == "hit" {
        		Some(Profile { name = "ada" })
        	} else {
        		None
        	}
        }

        fun main() {
        	// map — the annotation pins the type: Option<str>, not nested.
        	let mapped: Option<str> = user("hit")?.loud_name();
        	print(mapped.unwrap_or("?"));
        	// short-circuit: the continuation must not run.
        	let skipped: Option<str> = user("miss")?.loud_name();
        	print(skipped.unwrap_or("?"));
        	// flatten — the annotation pins Option<str> (not Option<Option<str>>).
        	let flat: Option<str> = user("hit")?.nickname();
        	print(flat.unwrap_or("?"));
        	let flat_none: Option<str> = user("miss")?.nickname();
        	print(flat_none.unwrap_or("?"));
        	// multi-link with args, escaped by parens.
        	print(format((user("hit")?.nickname()?.len()).unwrap_or(0 - 1)));
        }
        "#,
        "computed\nada\n?\nthe countess\n?\n12\n",
    );
}

// Result lifts: map wraps Ok, flatten passes the chain's own Result through,
// and Err short-circuits as-is.
#[test]
fn lift_works_on_results() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };

        fun to_number(text: str): Result<i32, str> {
        	match text.parse_i32() {
        		Some(let value) => Ok(value),
        		None => Err(i"bad: {text}"),
        	}
        }

        fun halve(value: i32): Result<i32, str> {
        	if value == value / 2 * 2 {
        		Ok(value / 2)
        	} else {
        		Err("odd")
        	}
        }

        fun show(value: Result<i32, str>) {
        	match value {
        		Ok(let v) => print(i"ok {format(v)}"),
        		Err(let e) => print(e),
        	}
        }

        fun main() {
        	let mapped: Result<i32, str> = to_number("21")?.max(0);
        	show(mapped);
        	let flat: Result<i32, str> = to_number("42")?.abs()?.max(0);
        	show(flat);
        	show(to_number("nope")?.max(0));
        }
        "#,
        "ok 21\nok 42\nbad: nope\n",
    );
}

// `?.` composes with `!`: the bang applies to the LIFTED result (it closes the
// group), not inside the continuation.
#[test]
fn lift_composes_with_bang() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };

        struct Wrap {
        	label: str,
        }

        fun boxed(key: str): Option<Wrap> {
        	if key == "hit" {
        		Some(Wrap { label = "inside" })
        	} else {
        		None
        	}
        }

        fun read(key: str): Option<str> {
        	let label = boxed(key)?.label!;
        	Some(label)
        }

        fun main() {
        	match read("hit") {
        		Some(let v) => print(v),
        		None => print("none"),
        	}
        	match read("miss") {
        		Some(let v) => print(v),
        		None => print("none"),
        	}
        }
        "#,
        "inside\nnone\n",
    );
}

// `?.` on a non-Lift subject is rejected at the chain's span.
#[test]
fn lift_on_a_non_lift_type_is_rejected() {
    assert_fails_spanning(
        r#"
        fun main() {
        	let n = 5;
        	let _ = n?.max(1);
        }
        "#,
        "n?.max(1)",
        "`?.` lifts an `Option`, a `Result`, or a type opting in",
    );
}

// A flattened Result chain must keep the same error type.
#[test]
fn lift_flatten_requires_matching_result_errors() {
    assert_fails_spanning(
        r#"
        import std::result::Result::{ self, Ok, Err };

        fun start(): Result<i32, str> {
        	Ok(1)
        }

        struct Helper {}

        impl i32 {
        	fun widen(self): Result<i32, i32> {
        		Ok(self)
        	}
        }

        fun main() {
        	let _ = start()?.widen();
        }
        "#,
        "start()?.widen()",
        "error types must match",
    );
}

// A bare `?` (no following member) does not parse.
#[test]
fn bare_question_mark_is_rejected() {
    assert_fails(
        r#"
        import std::option::Option::{ self, Some, None };

        fun main() {
        	let a = Some(1);
        	let _ = a?;
        }
        "#,
    );
}

// A lifted chain is not an assignment target.
#[test]
fn lift_is_not_an_assignment_target() {
    assert_fails(
        r#"
        import std::option::Option::{ self, Some, None };

        struct Point {
        	x: i32,
        }

        fun main() {
        	let p = Some(Point { x = 1 });
        	p?.x = 5;
        }
        "#,
    );
}

// A RETURN-position generic binds THROUGH `!`: the let's annotation directs
// the receiver's type parameter (`resolve_try_assert` re-infers the receiver
// as `Container<expected, ..>` once the container is known, riding the same
// reconcile-and-record channel as an annotated let).
#[test]
fn bang_directs_return_position_generics_into_its_receiver() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::result::Result::{ self, Ok, Err };
        import std::json::FromJson;

        fun decode_as<T: FromJson>(text: str): Result<T, str> {
        	Ok(T::from_json(text))
        }

        fun run(): Result<i32, str> {
        	let n: i32 = decode_as("42")!;
        	Ok(n)
        }

        fun main() {
        	match run() {
        		Ok(let v) => print(format(v)),
        		Err(let e) => print(e),
        	}
        }
        "#,
        "42\n",
    );
}

// The bare-`ret` half of closure participation: fine in a void-tailed closure,
// rejected in a value-yielding one...
#[test]
fn bare_ret_in_a_value_yielding_closure_is_rejected() {
    assert_fails_spanning(
        r#"
        fun apply(f: |i32| i32): i32 {
        	f(10)
        }

        fun main() {
        	let _ = apply(|x| {
        		if x > 5 {
        			ret;
        		}
        		x + 1
        	});
        }
        "#,
        "ret",
        "a bare `ret` exits a closure whose body yields",
    );
}

// ...and the mirror: a value-`ret` in a closure whose body ends without one.
#[test]
fn value_ret_in_a_void_closure_is_rejected() {
    assert_fails_spanning(
        r#"
        import std::print;

        fun main() {
        	let helper = |x: i32| {
        		if x > 5 {
        			ret 99;
        		}
        		print("small");
        	};
        	helper(1);
        }
        "#,
        "ret 99",
        "make the ret'd value the body's tail",
    );
}

// A bare-ret early exit in a void closure stays legal (the guard pattern).
#[test]
fn bare_ret_in_a_void_closure_is_allowed() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
        	let helper = |x: i32| {
        		if x > 5 {
        			ret;
        		}
        		print("small");
        	};
        	helper(10);
        	helper(1);
        }
        "#,
        "small\n",
    );
}

// `async` blocks get the same participation: an agreeing ret passes, and the
// existing early-return semantics hold.
#[test]
fn async_block_rets_check_against_the_tail() {
    assert_fails_spanning(
        r#"
        fun main() {
        	let flag = true;
        	let pending = async {
        		if flag {
        			ret "mismatched";
        		}
        		2
        	};
        }
        "#,
        "ret \"mismatched\"",
        "but the closure's body yields",
    );
}

// A user `Lift` container: `?.` dispatches to ITS `map`/`and_then` (the tag
// concatenation proves the user's and_then body ran on the flatten path).
#[test]
fn a_user_lift_container_dispatches_to_its_own_map_and_and_then() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::operators::Lift;

        struct Boxy<T> {
        	value: T,
        	tag: str,
        }

        impl Boxy<type T> with Lift {}

        impl Boxy<type T> {
        	fun map<U>(self, fn: |T| U): Boxy<U> {
        		Boxy { value = fn(self.value), tag = self.tag }
        	}

        	fun and_then<U>(self, fn: |T| Boxy<U>): Boxy<U> {
        		let inner = fn(self.value);
        		Boxy { value = inner.value, tag = self.tag + "+" + inner.tag }
        	}
        }

        struct Profile {
        	name: str,
        }

        impl Profile {
        	fun boxed_name(self): Boxy<str> {
        		Boxy { value = self.name, tag = "inner" }
        	}
        }

        fun main() {
        	let boxed = Boxy { value = Profile { name = "ada" }, tag = "outer" };
        	let mapped: Boxy<str> = boxed?.name;
        	print(i"{mapped.value} [{mapped.tag}]");
        	let lengths: Boxy<i32> = boxed?.name.len();
        	print(format(lengths.value));
        	let flat: Boxy<str> = boxed?.boxed_name();
        	print(i"{flat.value} [{flat.tag}]");
        }
        "#,
        "ada [outer]\n3\nada [outer+inner]\n",
    );
}

// The marker is the gate: a mappable type WITHOUT `impl .. with Lift` refuses.
#[test]
fn a_mappable_type_without_the_lift_marker_is_rejected() {
    assert_fails_spanning(
        r#"
        struct Sneaky<T> {
        	value: T,
        }

        impl Sneaky<type T> {
        	fun map<U>(self, fn: |T| U): Sneaky<U> {
        		Sneaky { value = fn(self.value) }
        	}
        }

        fun main() {
        	let s = Sneaky { value = 1 };
        	let _ = s?.max(2);
        }
        "#,
        "s?.max(2)",
        "opting in with `impl .. with Lift`",
    );
}

// The primitive operator/equality impls: generic `T: Add`/`T: BitAnd` code
// dispatches to the numeric primitives (and `str` for Add), and the bodies
// lower to the native operators — including u32's `>>> 0` correction.
#[test]
fn primitive_operator_impls_dispatch_generically() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;
        import std::operators::{ Add, BitAnd };

        fun sum<T: Add>(a: T, b: T): T {
        	a.add(b)
        }

        fun low_bit<T: BitAnd>(value: T, one: T): T {
        	value.bit_and(one)
        }

        fun main() {
        	print(format(sum(40, 2)));
        	print(sum("con", "cat"));
        	print(format(sum(1.5, 2.25)));
        	print(sum(20n, 22n));
        	print(format(low_bit(7, 1)));
        	print(format(low_bit(8u32, 1u32)));
        }
        "#,
        "42\nconcat\n3.75\n42n\n1\n0\n",
    );
}

// `format` covers every displayable primitive — u32 and BigInt were silently
// missing (the bound dispatch emitted the abstract to_string → undefined).
#[test]
fn format_covers_u32_and_bigint() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::format;

        fun main() {
        	print(format(7u32));
        	print(format(42n));
        }
        "#,
        "7\n42\n",
    );
}

// --- Block-scoped imports (backlog H2) ---
// `import`/`use` are statements, legal in any block; a binding is visible
// throughout its enclosing block (like a `let`), shadows outer scopes, and
// compiles to nothing. The loader finds module references at any depth.

// The loader half: `std::io` is referenced ONLY inside the body, so the module
// must still enter the reachable set (collect_module_refs recurses).
#[test]
fn an_import_in_a_function_body_binds_and_loads_its_module() {
    assert_compiles_and_runs(
        r#"
        fun main() {
            import std::io;
            io::print("from the body");
        }

        main();
        "#,
        "from the body\n",
    );
}

// Flat block scope, like a `let`: the binding is visible before its statement
// (imports have no runtime effect, so there is no TDZ hazard either).
#[test]
fn a_body_import_binds_throughout_its_block_like_a_let() {
    assert_compiles_and_runs(
        r#"
        fun main() {
            io::print("early");
            import std::io;
        }

        main();
        "#,
        "early\n",
    );
}

// Confinement: a block's import is invisible outside the block. `outer` comes
// first so the failing `io` is the source's first occurrence (the span pin).
#[test]
fn a_body_import_is_confined_to_its_function() {
    assert_fails_spanning(
        r#"
        fun outer() {
            io::print("outer");
        }

        fun inner() {
            import std::io;
            io::print("inner");
        }

        fun main() {
            inner();
            outer();
        }

        main();
        "#,
        "io",
        "cannot find",
    );
}

#[test]
fn an_inner_block_import_is_confined_to_the_block() {
    assert_fails_spanning(
        r#"
        import std::print;

        fun escaped() {
            io::print("outside");
        }

        fun main() {
            {
                import std::io;
                io::print("inner");
            }
            print("separator");
            escaped();
        }

        main();
        "#,
        "io",
        "cannot find",
    );
}

#[test]
fn an_import_inside_an_if_arm_works() {
    assert_compiles_and_runs(
        r#"
        fun main() {
            if true {
                import std::io;
                io::print("then");
            } else {
                import std::io;
                io::print("else");
            }
        }

        main();
        "#,
        "then\n",
    );
}

#[test]
fn an_import_inside_a_match_arm_works() {
    assert_compiles_and_runs(
        r#"
        fun main() {
            match 2 {
                2 => {
                    import std::io;
                    io::print("two");
                }
                _ => {}
            }
        }

        main();
        "#,
        "two\n",
    );
}

#[test]
fn an_import_inside_a_closure_body_works() {
    assert_compiles_and_runs(
        r#"
        fun main() {
            let show = || {
                import std::io;
                io::print("from closure");
            };
            show();
        }

        main();
        "#,
        "from closure\n",
    );
}

// A function declared in the block resolves the block's import through the
// ordinary scope chain.
#[test]
fn a_nested_function_sees_its_blocks_import() {
    assert_compiles_and_runs(
        r#"
        fun main() {
            import std::io;
            fun emit() {
                io::print("nested");
            }
            emit();
        }

        main();
        "#,
        "nested\n",
    );
}

// An impl body is a statement list too: an import there serves every method.
#[test]
fn an_import_inside_an_impl_body_serves_its_methods() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        struct Greeter {
            name: str,
        }

        impl Greeter {
            import std::display::format;

            fun greet(self) {
                print(format(self.name));
            }
        }

        fun main() {
            let greeter = Greeter { name = "vi" };
            greeter.greet();
        }

        main();
        "#,
        "vi\n",
    );
}

// Scoped `use` rides the same machinery: an inner `use` shadows the outer
// binding for its block, and the outer one is restored after.
#[test]
fn a_scoped_use_shadows_and_restores() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        mod alpha {
            export fun tag(): str {
                "alpha"
            }
        }

        mod beta {
            export fun tag(): str {
                "beta"
            }
        }

        use alpha::tag;

        fun main() {
            print(tag());
            {
                use beta::tag;
                print(tag());
            }
            print(tag());
        }

        main();
        "#,
        "alpha\nbeta\nalpha\n",
    );
}

// A block-scoped binding is deliberately not exportable — and no other
// `export` means anything inside a body.
#[test]
fn an_export_inside_a_body_is_rejected() {
    assert_fails_spanning(
        r#"
        fun main() {
            export import std::io;
        }

        main();
        "#,
        "export import std::io;",
        "`export` is a module-level item",
    );
}

// A body import of a module that does not exist fails at the import itself,
// not with a panic or a cascade at the use sites.
#[test]
fn a_body_import_of_a_missing_module_errors_cleanly() {
    assert_fails_spanning(
        r#"
        fun main() {
            import std::nonexistent;
        }

        main();
        "#,
        "nonexistent",
        "cannot find 'nonexistent' in the imported path",
    );
}

// --- The macro engine, Phase 1 (macro-engine.md §3-§4) ---
// `macro fun` definitions compile hermetically per file and run in the
// expansion interpreter; `[name(args)]` and `[derive(Name)]` splice their
// returned Source before analysis.

// The whole pipeline: hermetic world compile, attribute dispatch, reflection,
// interpreter run, splice, and dispatch INTO the generated impl.
#[test]
fn a_macro_attribute_expands_and_the_generated_impl_dispatches() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::display::{ Display, format };

        macro fun derive_display(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source, StructItem };
            import macro_std::option::Option::{ self, Some, None };

            let target = match item.as_struct() {
                Some(let found) => found,
                None => StructItem { name = "?", fields = [] },
            };
            mut arms = "";
            mut first = true;
            for field in target.fields {
                if first {
                    first = false;
                } else {
                    arms = arms + " + \", \" + ";
                }
                arms = arms + "\"" + field.name + "=\" + format(self." + field.name + ")";
            }
            source(
                "impl " + target.name + " with Display {\n"
                    + "fun to_string(self): str {\n"
                    + "import std::display::format;\n"
                    + arms + "\n}\n}\n",
            )
        }

        [derive_display]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            print(format(Point { x = 1, y = 2 }));
        }

        main();
        "#,
        "x=1, y=2\n",
    );
}

// `[derive(Name)]` dispatches to a registered macro named `Name`; built-in
// derive names keep their Rust generators.
#[test]
fn a_derive_name_dispatches_to_a_registered_macro() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun Tagged(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source, StructItem };
            import macro_std::option::Option::{ self, Some, None };

            let target = match item.as_struct() {
                Some(let found) => found,
                None => StructItem { name = "?", fields = [] },
            };
            source("impl " + target.name + " {\nfun tag(self): str {\n\"" + target.name + "\"\n}\n}\n")
        }

        [derive(Tagged)]
        struct Widget {
            size: i32,
        }

        fun main() {
            print(Widget { size = 3 }.tag());
        }

        main();
        "#,
        "Widget\n",
    );
}

// A two-parameter macro receives the invocation's argument SOURCE TEXTS.
#[test]
fn a_macro_receives_its_arguments_as_source_text() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun labelled(item: Item, arguments: Arguments): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source, Arguments, StructItem };
            import macro_std::option::Option::{ self, Some, None };

            let target = match item.as_struct() {
                Some(let found) => found,
                None => StructItem { name = "?", fields = [] },
            };
            mut body = "";
            mut first = true;
            for value in arguments.values {
                if first {
                    first = false;
                    // A string argument arrives with its quotes — a ready
                    // expression to splice.
                    body = value;
                } else {
                    body = body + " + format(" + value + ")";
                }
            }
            source(
                "impl " + target.name + " {\nfun label(self): str {\n"
                    + "import std::display::format;\n" + body + "\n}\n}\n",
            )
        }

        [labelled("alpha-", 42)]
        struct Thing {
            n: i32,
        }

        fun main() {
            print(Thing { n = 1 }.label());
        }

        main();
        "#,
        "alpha-42\n",
    );
}

// A macro's output can itself carry a built-in derive — the expansion fixpoint.
#[test]
fn a_macros_output_can_carry_a_builtin_derive() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun make_pair(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            import macro_std::meta::{ Item, Source };

            source("[derive(PartialEq)]\nstruct Pair {\na: i32,\nb: i32,\n}\n")
        }

        [make_pair]
        struct Seed {
            unused: i32,
        }

        fun main() {
            let left = Pair { a = 1, b = 2 };
            let same = Pair { a = 1, b = 2 };
            let different = Pair { a = 9, b = 2 };
            print(left == same);
            print(left == different);
        }

        main();
        "#,
        "true\nfalse\n",
    );
}

#[test]
fn an_unknown_macro_attribute_errors_cleanly() {
    assert_fails_spanning(
        r#"
        [no_such_macro]
        struct Point {
            x: i32,
        }

        fun main() {}

        main();
        "#,
        "no_such_macro",
        "no macro named `no_such_macro` is in scope",
    );
}

// Hermeticity (§4): a macro body may import only from `macro_std`.
#[test]
fn a_macro_body_importing_std_is_rejected() {
    assert_fails_spanning(
        r#"
        macro fun bad(item: Item): Source {
            import std::io;
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            source("")
        }

        fun main() {}

        main();
        "#,
        "import std::io",
        "a macro body may import only from `macro_std`",
    );
}

// A panic inside a macro surfaces as a spanned failure at the invocation.
#[test]
fn a_macro_panic_surfaces_at_the_invocation() {
    assert_fails_spanning(
        r#"
        [explode]
        struct Point {
            x: i32,
        }

        macro fun explode(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            import macro_std::panic;
            panic("unsupported item shape");
            source("")
        }

        fun main() {}

        main();
        "#,
        "explode",
        "failed at expansion time",
    );
}

#[test]
fn a_macro_generating_invalid_vilan_errors_at_the_site() {
    assert_fails_spanning(
        r#"
        [broken]
        struct Point {
            x: i32,
        }

        macro fun broken(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            source("fun {")
        }

        fun main() {}

        main();
        "#,
        "broken",
        "generated invalid vilan",
    );
}

#[test]
fn a_macro_generating_a_macro_is_rejected() {
    assert_fails_spanning(
        r#"
        [sneaky]
        struct Point {
            x: i32,
        }

        macro fun sneaky(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            source("macro fun nested(item: Item): Source {\nimport macro_std::source;\nsource(\"\")\n}\n")
        }

        fun main() {}

        main();
        "#,
        "sneaky",
        "macros cannot define macros",
    );
}

#[test]
fn duplicate_macro_names_error() {
    assert_fails(
        r#"
        macro fun twice(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            source("")
        }

        macro fun twice(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            source("")
        }

        fun main() {}

        main();
        "#,
    );
}

// The fuel budget bounds a runaway macro (§5): the failure names the macro at
// its invocation instead of hanging the compiler.
#[test]
fn an_infinite_macro_is_stopped_by_fuel() {
    assert_fails_spanning(
        r#"
        [forever]
        struct Point {
            x: i32,
        }

        macro fun forever(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            mut n = 0;
            for {
                n = n + 1;
            }
            source("")
        }

        fun main() {}

        main();
        "#,
        "forever",
        "failed at expansion time",
    );
}

#[test]
fn a_macro_fun_inside_a_body_is_rejected() {
    assert_fails_spanning(
        r#"
        fun main() {
            macro fun inner(item: Item): Source {
                import macro_std::source;
                import macro_std::meta::{ Item, Source };
                source("")
            }
        }

        main();
        "#,
        "macro fun inner(item: Item): Source {
                import macro_std::source;
                import macro_std::meta::{ Item, Source };
                source(\"\")
            }",
        "must be a top-level item",
    );
}

// --- The macro engine, Phase 2: `macro name(args)` invocations ---

#[test]
fn an_item_invocation_stamps_out_declarations() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun constants(arguments: Arguments): Source {
            import macro_std::source;
            import macro_std::meta::{ Arguments, Source };

            mut body = "";
            mut index = 0;
            for name in arguments.values {
                body = body + i"fun {name}(): i32 \{ {index} \}\n";
                index = index + 1;
            }
            source(body)
        }

        macro constants(zero, one, two);

        fun main() {
            print(two());
            print(zero());
        }

        main();
        "#,
        "2\n0\n",
    );
}

#[test]
fn an_expression_invocation_splices_in_place() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun double_of(arguments: Arguments): Source {
            import macro_std::source;
            import macro_std::meta::{ Arguments, Source };
            import macro_std::option::Option::{ self, Some, None };

            let text = match arguments.values.get(0) {
                Some(let value) => value,
                None => "0",
            };
            source(i"(({text}) * 2)")
        }

        fun main() {
            print(macro double_of(21));
            print(1 + macro double_of(3 + 4));
        }

        main();
        "#,
        "42\n15\n",
    );
}

// A zero-parameter macro is invocable with empty parens.
#[test]
fn a_unit_macro_invokes_with_no_arguments() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun answer(): Source {
            import macro_std::source;
            import macro_std::meta::Source;

            source("42")
        }

        fun main() {
            print(macro answer());
        }

        main();
        "#,
        "42\n",
    );
}

// Gensym hygiene (§7): `fresh()` placeholders stamp unique per splice site, so
// one macro's output cannot capture a binder another site introduced.
#[test]
fn gensyms_do_not_capture_across_splice_sites() {
    assert_fails(
        r#"
        macro fun binds(arguments: Arguments): Source {
            import macro_std::source;
            import macro_std::fresh;
            import macro_std::meta::{ Arguments, Source };

            let binder = fresh();
            source(i"\{ let {binder} = 1; {binder} + macro leaks() \}")
        }

        macro fun leaks(): Source {
            import macro_std::source;
            import macro_std::fresh;
            import macro_std::meta::Source;

            // Emits a REFERENCE to its own fresh placeholder without binding
            // it: if stamping were per-program instead of per-site, this would
            // silently capture `binds`'s binder.
            source(i"{fresh()}")
        }

        fun main() {
            let x = macro binds();
        }

        main();
        "#,
    );
}

// Shape mismatches are clean errors in both directions.
#[test]
fn an_attribute_shaped_macro_cannot_be_invoked() {
    assert_fails_spanning(
        r#"
        fun main() {
            let x = macro takes_item();
        }

        macro fun takes_item(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            source("")
        }

        main();
        "#,
        "takes_item",
        "attribute-shaped",
    );
}

#[test]
fn an_invocation_shaped_macro_cannot_be_an_attribute() {
    assert_fails_spanning(
        r#"
        [takes_arguments]
        struct Point {
            x: i32,
        }

        macro fun takes_arguments(arguments: Arguments): Source {
            import macro_std::source;
            import macro_std::meta::{ Arguments, Source };
            source("")
        }

        fun main() {}

        main();
        "#,
        "takes_arguments",
        "invocation-shaped",
    );
}

// An expression splice must be exactly one expression.
#[test]
fn an_expression_macro_must_generate_one_expression() {
    assert_fails_spanning(
        r#"
        fun main() {
            let x = macro two_statements();
        }

        macro fun two_statements(): Source {
            import macro_std::source;
            import macro_std::meta::Source;
            source("1; 2;")
        }

        main();
        "#,
        "two_statements",
        "generated invalid vilan",
    );
}

// A closure bound to a local and called directly doesn't type its unannotated
// parameter (backlog B13) — the call-site reconciliation channel covers method
// calls and deferred call subjects, but a direct call on a closure-typed local
// never feeds the parameter back. Surfaced writing `macro unroll(..)` callbacks.
#[test]
#[ignore]
fn a_direct_call_types_an_unannotated_closure_parameter() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun accumulate(i: i32): i32 {
            i * 10
        }

        fun main() {
            let f = |i| accumulate(i);
            print(f(3));
        }

        main();
        "#,
        "30\n",
    );
}

// `str.code_at` — the UTF-16 code-unit accessor (added for the service
// macro's djb2 contract hash; charCodeAt under the hood).
#[test]
fn code_at_reads_utf16_units() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            print("A".code_at(0));
            print("ab".code_at(1));
        }

        main();
        "#,
        "65\n98\n",
    );
}

// --- Scoped macro names (macro-engine.md §3 — the flat namespace is gone) ---

// A macro in another module needs a leaf import; unimported = a clean error.
#[test]
fn an_unimported_macro_from_another_module_is_not_in_scope() {
    assert_fails_spanning(
        r#"
        [tag]
        struct Point {
            x: i32,
        }

        mod helpers {
            macro fun tag(item: Item): Source {
                import macro_std::source;
                import macro_std::meta::{ Item, Source };
                source("")
            }
        }

        fun main() {}

        main();
        "#,
        "tag",
        "no macro named `tag` is in scope",
    );
}

// A user macro may now SHADOW a prelude derive for its own file — the
// reserved-name rule died with the flat namespace.
#[test]
fn a_user_macro_shadows_a_prelude_derive_in_its_file() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun PartialEq(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source, StructItem };
            import macro_std::option::Option::{ self, Some, None };

            let target = match item.as_struct() {
                Some(let found) => found,
                None => StructItem { name = "?", fields = [] },
            };
            source(i"impl {target.name} \{\nfun shadowed(self): str \{\n\"local\"\n\}\n\}\n")
        }

        [derive(PartialEq)]
        struct Point {
            x: i32,
        }

        fun main() {
            print(Point { x = 1 }.shadowed());
        }

        main();
        "#,
        "local\n",
    );
}

// The prelude: `[derive(PartialEq)]` still needs no import — the derive
// macros live in always-loaded std modules now, not in a special file.
#[test]
fn prelude_derives_need_no_import() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        [derive(PartialEq)]
        struct Point {
            x: i32,
        }

        fun main() {
            let a = Point { x = 1 };
            let b = Point { x = 1 };
            print(a == b);
        }

        main();
        "#,
        "true\n",
    );
}

// The macro world's AMBIENT meta prelude (macro-engine.md §3/§10): the
// reflection vocabulary — the meta types, `source`, `fresh` — is in scope in
// every macro body with no imports at all. Libraries (`option`, `build`)
// stay explicit.
#[test]
fn the_meta_vocabulary_is_ambient_in_macro_bodies() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun tag(item: Item): Source {
            import macro_std::option::Option::{ self, Some, None };

            let name = match item.as_struct() {
                Some(let found) => found.name,
                None => "?",
            };
            source(i"fun tag_of(): str \{\n\"{name}\"\n\}\n")
        }

        [tag]
        struct Widget {
            size: i32,
        }

        fun main() {
            print(tag_of());
        }

        main();
        "#,
        "Widget\n",
    );
}

// `fresh()` is part of the ambient vocabulary too — a zero-import invocation
// macro gensyms and splices.
#[test]
fn fresh_is_ambient_in_macro_bodies() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun doubled(arguments: Arguments): Source {
            let slot = fresh();
            source(i"let {slot} = 21;\nlet answer = {slot} + {slot};")
        }

        macro doubled()

        fun main() {
            print(answer);
        }

        main();
        "#,
        "42\n",
    );
}

// An explicit same-name definition SHADOWS the ambient prelude — the prelude
// binds first, ordinary resolution order.
#[test]
fn a_macro_fun_shadows_the_ambient_prelude() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun fresh(): str {
            "__custom"
        }

        macro fun emit(arguments: Arguments): Source {
            let slot = fresh();
            source(i"fun generated(): str \{\n\"{slot}\"\n\}\n")
        }

        macro emit()

        fun main() {
            print(generated());
        }

        main();
        "#,
        "__custom\n",
    );
}

// --- `macro { .. }` blocks (macro-engine.md Phase 4) ---

// ITEM position: the block's emissions splice as items.
#[test]
fn an_item_position_macro_block_splices_items() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro {
            source("fun answer(): i32 {\n42\n}\n")
        }

        fun main() {
            print(answer());
        }

        main();
        "#,
        "42\n",
    );
}

// EXPRESSION position: the block folds at compile time and splices one
// expression.
#[test]
fn an_expression_position_macro_block_splices_an_expression() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let folded = macro {
                mut total = 0;
                mut index = 1;
                for index <= 4 {
                    total = total + index;
                    index = index + 1;
                }
                source(i"{total}")
            };
            print(folded);
        }

        main();
        "#,
        "10\n",
    );
}

// A block calls the file's `macro fun` helpers as plain in-world functions.
#[test]
fn a_macro_block_calls_a_same_file_helper() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun doubled(value: i32): str {
            i"{value * 2}"
        }

        fun main() {
            print(macro { source(doubled(21)) });
        }

        main();
        "#,
        "42\n",
    );
}

// The synthetic entry declares `: Source`, so a non-Source tail is a world
// type error at the block's true position.
#[test]
fn a_macro_block_must_yield_source() {
    assert_fails_spanning(
        r#"
fun main() {
    let x = macro { 42 };
}

main();
        "#,
        "macro { 42 }",
        "definition did not compile",
    );
}

// Output that doesn't parse is the ordinary invalid-vilan error, with the
// block's own label.
#[test]
fn a_macro_block_with_invalid_output_errors() {
    assert_fails_spanning(
        r#"
fun main() {
    let x = macro { source("+++ nope") };
}

main();
        "#,
        r#"macro { source("+++ nope") }"#,
        "generated invalid vilan",
    );
}

// Inside a `macro fun` body there is nothing to splice into — the body
// already runs at expansion time.
#[test]
fn a_macro_block_inside_a_macro_fun_is_rejected() {
    assert_fails_spanning(
        r#"
macro fun bad(item: Item): Source {
    macro { source("1") }
}

fun main() {}

main();
        "#,
        r#"macro { source("1") }"#,
        "cannot appear inside macro code",
    );
}

// Same rule one level down: blocks cannot nest.
#[test]
fn a_macro_block_inside_a_macro_block_is_rejected() {
    assert_fails_spanning(
        r#"
fun main() {
    let x = macro { macro { source("1") } };
}

main();
        "#,
        r#"macro { source("1") }"#,
        "cannot appear inside macro code",
    );
}

// Block bodies are hermetic like every macro body: imports root at
// `macro_std` only.
#[test]
fn a_macro_block_body_is_hermetic() {
    assert_fails_spanning(
        r#"
fun main() {
    let x = macro {
        import std::io::print;
        source("1")
    };
}

main();
        "#,
        "import std::io::print",
        "hermetic",
    );
}

// A macro's output cannot carry a `macro { .. }` block (mirrors the
// macro-generating-macro rejection).
#[test]
fn generated_code_cannot_carry_a_macro_block() {
    let source = r#"
macro fun emit_block(arguments: Arguments): Source {
    source("fun answer(): i32 {\nmacro { source(\"1\") }\n}\n")
}

macro emit_block()

fun main() {}

main();
        "#;
    let diagnostics = failure_diagnostics(source);
    // The error anchors at the GENERATING invocation's name (a file span),
    // never into the generated text.
    let invocation_name = source.rfind("emit_block").unwrap();
    assert!(
        diagnostics.iter().any(|(message, range)| {
            message.contains("generated a `macro { .. }` block") && range.start == invocation_name
        }),
        "expected the generated-block rejection at the invocation; got: {diagnostics:#?}"
    );
}

// --- Sized numeric types (proposal/numeric-types.md) ---

// Every new suffix types its literal; `128i8` is admitted (the minimum is
// written as unary minus over the literal); unsuffixed literals adopt an
// expected sized type.
#[test]
fn sized_numeric_literals_type_and_run() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let a = 5i8;
            let b = 200u8;
            let c = 5i16;
            let d = 60000u16;
            let e = 5i64;
            let f = 5u64;
            let g = 2.5f32;
            let allowed = 128i8;
            let expected: u8 = 7;
            let fractional: f32 = 1.5;
            print(a + a);
            print(b);
            print(c + c);
            print(d);
            print(e + f.as_i64());
            print(g);
            print(allowed);
            print(expected);
            print(fractional);
        }

        main();
        "#,
        "10\n200\n10\n60000\n10\n2.5\n128\n7\n1.5\n",
    );
}

#[test]
fn a_u8_literal_out_of_range_errors() {
    assert_fails_spanning(
        "fun main() { let x = 300u8; }\nmain();\n",
        "300u8",
        "out of range for `u8` (0 ..= 255)",
    );
}

#[test]
fn an_i8_literal_out_of_range_errors() {
    assert_fails_spanning(
        "fun main() { let x = 129i8; }\nmain();\n",
        "129i8",
        "out of range for `i8` (-128 ..= 127)",
    );
}

#[test]
fn a_u16_literal_out_of_range_errors() {
    assert_fails_spanning(
        "fun main() { let x = 70000u16; }\nmain();\n",
        "70000u16",
        "out of range for `u16`",
    );
}

#[test]
fn an_i16_literal_out_of_range_errors() {
    assert_fails_spanning(
        "fun main() { let x = 40000i16; }\nmain();\n",
        "40000i16",
        "out of range for `i16`",
    );
}

#[test]
fn a_u32_literal_out_of_range_errors() {
    assert_fails_spanning(
        "fun main() { let x = 5000000000u32; }\nmain();\n",
        "5000000000u32",
        "out of range for `u32`",
    );
}

#[test]
fn an_i32_literal_out_of_range_errors() {
    assert_fails_spanning(
        "fun main() { let x = 3000000000i32; }\nmain();\n",
        "3000000000i32",
        "out of range for `i32`",
    );
}

#[test]
fn an_i64_literal_beyond_the_f64_window_errors() {
    assert_fails_spanning(
        "fun main() { let x = 9007199254740993i64; }\nmain();\n",
        "9007199254740993i64",
        "use `BigInt` for larger values",
    );
}

#[test]
fn a_hex_literal_is_range_checked() {
    assert_fails_spanning(
        "fun main() { let x = 0x100u8; }\nmain();\n",
        "0x100u8",
        "out of range for `u8`",
    );
}

// An unsuffixed literal adopting an expected sized type is range-checked
// against that type.
#[test]
fn an_expected_type_literal_is_range_checked() {
    assert_fails_spanning(
        "fun main() { let x: u8 = 300; }\nmain();\n",
        "300",
        "out of range for `u8`",
    );
}

// Integer division truncates toward zero (numeric-types.md §2) — both signs,
// every width, the compound form, and generic `T: Div` dispatch; float and
// BigInt division are untouched.
#[test]
fn integer_division_truncates_toward_zero() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::operators::Div;

        fun halve<T: Div>(value: T, divisor: T): T {
            value / divisor
        }

        fun main() {
            print(7 / 2);
            print(-7 / 2);
            print(7u32 / 2u32);
            print(100u8 / 3u8);
            print(100i64 / 8i64);
            mut compound = 9;
            compound /= 2;
            print(compound);
            print(halve(100i16, 8i16));
            print(7.0 / 2.0);
            print(7n / 2n);
        }

        main();
        "#,
        "3\n-3\n3\n33\n12\n4\n12\n3.5\n3n\n",
    );
}

// Conversions carry Rust-`as` semantics: truncate toward zero, then fold
// two's-complement into the target's width.
#[test]
fn numeric_conversions_fold_into_the_target_width() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            print((300).as_u8());
            print((-1).as_u8());
            print((130).as_i8());
            print((70000).as_u16());
            print((3.9).as_i32());
            print((-3.9).as_i32());
            print((200u8).as_f64() + 0.5);
            print((2.5f32).as_i64());
            print((5i64).as_u64());
        }

        main();
        "#,
        "44\n255\n-126\n4464\n3\n-3\n200.5\n2\n5\n",
    );
}

// The macro-engine flagship (macro-engine.md §2) realized: one macro stamps
// the operator family for several types at once. (The std family itself is
// generated-and-checked-in because `number.vl` loads inside macro worlds,
// which expand with an empty macro scope — world files must not dispatch.)
#[test]
fn a_macro_stamps_a_numeric_family() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::operators::Add;

        macro fun arithmetic_family(arguments: Arguments): Source {
            import macro_std::option::Option::{ self, Some, None };
            import macro_std::build::{ impl_of, fun_of };

            mut generated = "import std::operators::Add;\n";
            mut index = 0;
            for index < arguments.len() {
                let name = match arguments.as_identifier(index) {
                    Some(let found) => found,
                    None => "?",
                };
                let add = fun_of("add")
                    .parameter("self")
                    .parameter(i"b: {name}")
                    .returns(name)
                    .expr(i"{name} \{ value = self.value + b.value \}");
                generated = generated + impl_of(name).implements("Add").method(add).render();
                index = index + 1;
            }
            source(generated)
        }

        struct Meters { value: i32 }
        struct Seconds { value: i32 }

        macro arithmetic_family(Meters, Seconds)

        fun total<T: Add>(a: T, b: T): T {
            a + b
        }

        fun main() {
            print(total(Meters { value = 2 }, Meters { value = 3 }).value);
            print(total(Seconds { value = 40 }, Seconds { value = 5 }).value);
        }

        main();
        "#,
        "5\n45\n",
    );
}

// --- `flatten` + keyed reconciliation (backlog A4/A3) ---

// The join follows the CURRENT inner: switching detaches the replaced inner
// (its later sets must not leak through) and adopts the new one's value.
#[test]
fn flatten_follows_the_current_inner_and_detaches_the_old() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::Signal;

        fun main() {
            let first = Signal::new(1);
            let second = Signal::new(10);
            let outer = Signal::new(first);
            let joined = outer.flatten();
            first.set(2);
            print(joined.get());
            outer.set(second);
            first.set(99);
            print(joined.get());
            second.set(11);
            print(joined.get());
        }

        main();
        "#,
        "2\n10\n11\n",
    );
}

// Reconcile distinguishes keep/refresh/fresh per new position and reports
// removed old indices — including the duplicate-key claim rule.
#[test]
fn reconcile_plans_keep_refresh_fresh_and_removals() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ reconcile, RowStep };

        fun main() {
            let plan = reconcile([1, 2], [10, 20], [20, 11, 35, 20], |item| item / 10);
            for step in plan.steps {
                let rendered = match step {
                    RowStep::Keep(let index) => i"keep {index}",
                    RowStep::Refresh(let index) => i"refresh {index}",
                    RowStep::Fresh => "fresh",
                };
                print(rendered);
            }
            for index in plan.removed {
                print(i"removed {index}");
            }
        }

        main();
        "#,
        "keep 1\nrefresh 0\nfresh\nfresh\n",
    );
}

// `Owner.defer` runs plain cleanups at disposal, alongside taken disposables.
#[test]
fn owner_defer_runs_cleanups_on_dispose() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Owner, Disposable };

        fun main() {
            let owner = Owner::new();
            owner.defer(|| print("first"));
            owner.defer(|| print("second"));
            owner.dispose();
            print("done");
        }

        main();
        "#,
        "first\nsecond\ndone\n",
    );
}
