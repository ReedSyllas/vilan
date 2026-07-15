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
    compile_on(source, Platform::default())
}

/// `compile` for a browser build — the platform whose layer holds `std::ui` /
/// `std::dom` / `std::router`, none of which the default (node) platform can
/// import.
fn compile_browser(source: &str) -> Result<String, Vec<String>> {
    compile_on(source, Platform::Browser)
}

fn compile_on(source: &str, platform: Platform) -> Result<String, Vec<String>> {
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
                    Some(platform),
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

/// Asserts compilation fails with a diagnostic containing `message_part` — like
/// [`assert_fails`] but pinning *which* error, so a test can't pass on an
/// unrelated failure.
#[track_caller]
fn assert_fails_with(source: &str, message_part: &str) {
    match compile(source) {
        Ok(_) => panic!("expected a compile error, but it compiled cleanly"),
        Err(errors) => assert!(
            errors.iter().any(|error| error.contains(message_part)),
            "no diagnostic contains {message_part:?}; got: {errors:#?}"
        ),
    }
}

#[track_caller]
fn assert_compiles_browser(source: &str) {
    if let Err(errors) = compile_browser(source) {
        panic!("expected a clean browser compile, got: {errors:#?}");
    }
}

/// Asserts a browser compile fails with a diagnostic containing `message_part`.
#[track_caller]
fn assert_fails_browser_with(source: &str, message_part: &str) {
    match compile_browser(source) {
        Ok(_) => panic!("expected a browser compile error, but it compiled cleanly"),
        Err(errors) => assert!(
            errors.iter().any(|error| error.contains(message_part)),
            "no browser diagnostic contains {message_part:?}; got: {errors:#?}"
        ),
    }
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

/// The rendered per-function requirement line (`platform_color::requirements`
/// — the hover's data) for the named function, through the real pipeline on
/// the default platform. `None` = the function is colorless. Panics on
/// analysis errors or an unknown name, so a pin can't pass vacuously.
fn requirement_line_of(source: &str, function_name: &str) -> Option<String> {
    let source = source.to_string();
    let function_name = function_name.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let leaked: &'static str = Box::leak(source.into_boxed_str());
            let (program, errors) = analyze_source(
                leaked,
                &std_spec(),
                Path::new("."),
                Path::new("test.vl"),
                Some(Platform::default()),
                &Workspace::default(),
            );
            let messages: Vec<String> = errors.into_iter().map(|error| error.msg).collect();
            assert!(
                messages.is_empty(),
                "expected a clean analysis, got: {messages:#?}"
            );
            let program = program.expect("analysis should produce a program");
            let function_id = program
                .functions
                .iter()
                .find(|(_, function)| function.name == function_name.as_str())
                .map(|(id, _)| *id)
                .or_else(|| {
                    // A layer function may be a bodiless extern (e.g.
                    // `std::fs::write_file`), seeded exactly like one with a body.
                    program
                        .external_functions
                        .iter()
                        .find(|(_, function)| function.name == function_name.as_str())
                        .map(|(id, _)| *id)
                })
                .or_else(|| {
                    // A module-level binding: its initializer is code, so it
                    // carries a requirement line like a function does.
                    program
                        .variables
                        .iter()
                        .find(|(_, variable)| variable.name == function_name.as_str())
                        .map(|(id, _)| *id)
                })
                .unwrap_or_else(|| panic!("no function or binding named `{function_name}`"));
            vilan_core::platform_color::requirements(&program)
                .get(&function_id)
                .cloned()
        })
        .expect("spawn worker")
        .join()
        .expect("worker panicked")
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
        import std::result::Result::{ self, Ok, Err };
        fun main() {
            let nums: Result<List<i32>, str> = List::from_json("[1,2,3]");
            print(nums is Ok(let ns) && ns.to_json() == "[1,2,3]");
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
fn an_inline_option_view_transient_writes_through() {
    // C5.2: constructing an `Option<&mut T>` inline and immediately matching it —
    // the transient the spec's open question sanctioned. The `Some(&mut a)` never
    // outlives the `match`, so it doesn't escape; the capture binds the view and
    // writes through. Both the direct subject and the conditional form (`match if
    // c { Some(..) } else { None }`, the inline analogue of `Arena::get`).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        fun main() {
            mut a = 5;
            match Some(&mut a) {          // direct scalar transient
                Some(let v) => { v = 99; }
                None => {}
            }
            print(a);                    // 99 — written through

            mut b = 10;
            let take = false;
            match if take { Some(&mut b) } else { None } {   // conditional
                Some(let v) => { v = 1; }
                None => { print("none"); }
            }
            print(b);                    // 10 — None branch, untouched
        }
        "#,
        "99\nnone\n10\n",
    );
}

#[test]
fn an_inline_aggregate_option_view_transient_writes_through() {
    // C5.2, aggregate flavor: the payload is a `&mut struct`, so the capture is
    // the value's own reference and `.field` write-through reaches the original.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        struct Node { value: i32 }
        fun main() {
            mut node = Node { value = 1 };
            match Some(&mut node) {
                Some(let v) => { v.value = 42; }
                None => {}
            }
            print(node.value);           // 42
        }
        "#,
        "42\n",
    );
}

#[test]
fn a_view_parameter_forwarded_into_an_inline_transient_writes_through() {
    // C5.2, forward flavor: a bare `&mut` parameter passed straight into the
    // inline constructor (`Some(p)`) — the capture aliases the same view, so the
    // write reaches the caller's value. Scalar (`(base, key)`) and aggregate.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        struct Node { value: i32 }
        fun bump_scalar(p: &mut i32) {
            match Some(p) { Some(let v) => { v += 1; } None => {} }
        }
        fun bump_field(p: &mut Node) {
            match Some(p) { Some(let v) => { v.value += 1; } None => {} }
        }
        fun main() {
            mut a = 41;
            bump_scalar(&mut a);
            print(a);              // 42

            mut n = Node { value = 41 };
            bump_field(&mut n);
            print(n.value);        // 42
        }
        "#,
        "42\n42\n",
    );
}

#[test]
fn a_forwarded_immutable_view_transient_rejects_a_write() {
    // C5.2 boundary: forwarding a `&` (read-only) view keeps its convention — a
    // write through the capture is still rejected.
    assert_fails(
        r#"
        import std::option::Option::{ self, Some, None };
        fun peek(p: &i32) {
            match Some(p) { Some(let v) => { v = 9; } None => {} }
        }
        fun main() { mut a = 5; peek(&a); }
        "#,
    );
}

#[test]
fn a_stored_inline_option_view_is_rejected() {
    // C5.2 boundary: the sanction is for the *transient* only. Binding the same
    // `Some(&mut a)` to a `let` stores the view in an enum payload that outlives
    // the statement — a real escape, still rejected.
    assert_fails(
        r#"
        import std::option::Option::{ self, Some, None };
        fun main() {
            mut a = 5;
            let stored = Some(&mut a);
            match stored {
                Some(let v) => { v = 9; }
                None => {}
            }
        }
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
                Option::from_json("{\"id\":1,\"name\":\"Ada\"}");
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
        fun decode(text: str): Result<Option<User>, str> { Option::from_json(text) }
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
                "ok" => Option::from_json(json),
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
        import std::result::Result::{ self, Ok, Err };
        fun main() {
            let nums: Result<List<i32>, str> = List::from_json("[1,2,3]");
            match nums {
                Ok(let ns) => print(ns.to_json()),
                Err(let e) => print(e),
            }
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
        import std::result::Result::{ self, Ok, Err };
        fun main() {
            let grid: Result<List<List<i32>>, str> = List::from_json("[[1,2],[3,4]]");
            match grid {
                Ok(let g) => print(g.to_json()),
                Err(let e) => print(e),
            }
            let deep: Result<List<List<List<i32>>>, str> = List::from_json("[[[1]],[[2,3]]]");
            match deep {
                Ok(let d) => print(d.to_json()),
                Err(let e) => print(e),
            }
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
        import std::result::Result::{ self, Ok, Err };
        [derive(Json)] struct P { x: i32 }
        fun main() {
            let a: Result<Option<List<i32>>, str> = Option::from_json("[1,2,3]");
            match a {
                Ok(let av) => print(av.to_json()),
                Err(let e) => print(e),
            }
            let b: Result<List<Option<i32>>, str> = List::from_json("[1,null,3]");
            match b {
                Ok(let bv) => print(bv.to_json()),
                Err(let e) => print(e),
            }
            let c: Result<List<P>, str> = List::from_json("[{\"x\":1},{\"x\":2}]");
            match c {
                Ok(let cv) => print(cv.to_json()),
                Err(let e) => print(e),
            }
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
fn impl_binder_inherits_bound_from_a_later_declared_struct() {
    // The same program as `impl_binder_inherits_struct_bound`, but with the
    // struct declared *after* the impl. The walk registers the binder
    // unbounded and retrofits the struct's bound just before solving, once
    // every declaration exists — declaration order no longer matters.
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
fn impl_binder_inherits_multiple_bounds_from_a_later_declared_struct() {
    // The deferred retrofit carries MULTI-bounds too: `T: Greeter + Counter`
    // declared after the impl, methods from both traits resolving.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Greeter { fun greet(self): str; }
        trait Counter { fun count(self): i32; }
        struct Hello { name: str }
        impl Hello with Greeter { fun greet(self): str { "hi " + self.name } }
        impl Hello with Counter { fun count(self): i32 { self.name.len() } }
        impl Wrapper<type T> {
            fun describe(self): str {
                (self.inner).greet()
            }
            fun tally(self): i32 {
                (self.inner).count()
            }
        }
        struct Wrapper<T: Greeter + Counter> { inner: T }
        fun main() {
            let wrapped = Wrapper { inner = Hello { name = "xy" } };
            print(wrapped.describe());
            print(wrapped.tally());
        }
        "#,
        "hi xy\n2\n",
    );
}

#[test]
fn impl_binder_inherits_bound_from_a_later_declared_enum() {
    // Enum subjects inherit through the same deferred path as structs.
    assert_compiles_and_runs(
        r#"
        import std::print;
        trait Greeter { fun greet(self): str; }
        struct Hello { name: str }
        impl Hello with Greeter { fun greet(self): str { "hi " + self.name } }
        impl Holder<type T> {
            fun open(self): str {
                match self {
                    Holder::Item(let inner) => inner.greet(),
                }
            }
        }
        enum Holder<T: Greeter> {
            Item(T),
        }
        fun main() {
            print(Holder::Item(Hello { name = "e" }).open());
        }
        "#,
        "hi e\n",
    );
}

#[test]
fn a_boundless_trait_argument_binder_inherits_the_traits_bound() {
    // `with DescribeInto<type S>` omits the bound; the TRAIT declares
    // `S: Sink`, so the binder inherits it — the subject-binder rule applied
    // to the with-clause.
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
        trait DescribeInto<S: Sink> {
            fun describe_into(self, sink: S);
        }
        struct Point { x: i32 }
        impl Point with DescribeInto<type S> {
            fun describe_into(self, sink: S) {
                sink.put(self.x);
            }
        }
        fun main() {
            let point = Point { x = 5 };
            let collector = Collector { total = Shared::new(0) };
            point.describe_into(collector);
            print(collector.total.read());
        }
        "#,
        "5\n",
    );
}

#[test]
fn subject_and_trait_argument_binders_compose_on_one_impl() {
    // `impl Box<type T> with DescribeInto<type S: Sink>` — the receiver binds
    // T, the argument binds S, one call resolves both.
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
        trait Sized2 { fun size(self): i32; }
        struct Pair { a: i32, b: i32 }
        impl Pair with Sized2 { fun size(self): i32 { 2 } }
        trait DescribeInto<S> {
            fun describe_into(self, sink: S);
        }
        struct Box2<T: Sized2> { inner: T }
        impl Box2<type T> with DescribeInto<type S: Sink> {
            fun describe_into(self, sink: S) {
                sink.put((self.inner).size());
            }
        }
        fun main() {
            let boxed = Box2 { inner = Pair { a = 1, b = 2 } };
            let collector = Collector { total = Shared::new(40) };
            boxed.describe_into(collector);
            print(collector.total.read());
        }
        "#,
        "42\n",
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
            T::from_json(reply)                           // decode the generic T from the reply
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
        import std::result::Result::{ self, Ok, Err };
        [derive(Wire)]
        struct Point { x: i32, y: i32 }
        [derive(Wire)]
        struct Line { from: Point, to: Point, tags: List<str> }
        [derive(Wire)]
        enum Shape { Seg(Line), Empty }
        fun main() {
            let line = Line { from = Point { x = 1, y = 2 }, to = Point { x = 3, y = 4 }, tags = ["a"] };
            match Line::from_json(line.to_json()) {                          // decoding yields a Result (I3)
                Ok(let back) => {
                    print(i"{back.from.x} {back.from.y} {back.to.x} {back.to.y}");   // 1 2 3 4
                    match Shape::from_json(Shape::Seg(back).to_json()) {
                        Ok(Shape::Seg(let l)) => print(i"seg {l.from.x}"),           // seg 1
                        Ok(Shape::Empty) => print("empty"),
                        Err(let e) => print(e),
                    }
                }
                Err(let e) => print(e),
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

// === Async rpc handlers (the dispatch spine awaits — J2 through the wire) =========

#[test]
fn an_async_rpc_method_replies_after_its_await() {
    // The user-shaped case: a `[rpc]` method that awaits (here `sleep_for`)
    // compiles, and its reply carries the value computed AFTER the suspension.
    // The `[service]` macro wraps each route in `turn_async`, and every seam
    // of the spine (`Dispatcher.handle` → `RpcProtocol.respond` →
    // `LocalTransport.call`) awaits through a re-marked `let` (J2 v1).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ Json, json_codec };
        import std::rpc::{ local_rpc };
        import std::time::{ sleep_for, Duration };

        [service(SlowClient)]
        struct Slow { calls: Shared<i32> }

        impl Slow {
            [rpc]
            fun slow_double(self, by: i32): i32 {
                self.calls.write() = self.calls.read() + 1;
                sleep_for(Duration::millis(10));
                by * 2
            }
        }

        fun main() {
            let service = Slow { calls = Shared::new(0) };
            let transport = local_rpc(service.dispatcher().into_protocol(json_codec()));
            let client = SlowClient { transport, codec = json_codec() };
            match client.slow_double(7) {
                Ok(let n) => print(i"slow_double -> {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            print(i"calls = {service.calls.read()}");
        }
        "#,
        "slow_double -> 14\ncalls = 1\n",
    );
}

#[test]
fn sync_and_async_rpc_methods_coexist_on_one_service() {
    // J2 in both directions through the retyped spine: the sync method rides
    // the same `async |..|`-seamed dispatch (awaiting a plain value just
    // resolves), the async one settles before its reply encodes.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ Json, json_codec };
        import std::rpc::{ local_rpc };
        import std::time::{ sleep_for, Duration };

        [service(MixedClient)]
        struct Mixed { count: Shared<i32> }

        impl Mixed {
            [rpc]
            fun quick(self): i32 { 1 }

            [rpc]
            fun slow(self): i32 {
                sleep_for(Duration::millis(5));
                2
            }
        }

        fun main() {
            let transport = local_rpc(
                Mixed { count = Shared::new(0) }.dispatcher().into_protocol(json_codec()),
            );
            let client = MixedClient { transport, codec = json_codec() };
            match client.quick() {
                Ok(let n) => print(i"quick -> {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            match client.slow() {
                Ok(let n) => print(i"slow -> {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
        }
        "#,
        "quick -> 1\nslow -> 2\n",
    );
}

#[test]
fn an_async_rpc_methods_writes_settle_as_one_wave_with_its_reply() {
    // The wire turn HOLDS across the handler's await (`turn_async`, the true
    // at-end cadence): a write before and a write after the suspension
    // coalesce, so the mirror sees ONE update — the final value — alongside
    // the reply. (Per-segment settling would leak "working" as its own
    // update before the reply.)
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        import std::reactive::Signal;
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ Json, FromJson };
        import std::json::json_codec;
        import std::rpc::{ local_rpc, duplex_pair, ReactiveServer, ReactiveClient, RemoteSource };
        import std::time::{ sleep_for, Duration };

        [service(JobClient)]
        struct Job {
            [expose] status: Signal<str>,
        }

        impl Job {
            [rpc]
            fun run(self): i32 {
                self.status.set("working");
                sleep_for(Duration::millis(10));
                self.status.set("done");
                7
            }
        }

        fun main() {
            let job = Job { status = Signal::new("idle") };
            let transport = local_rpc(job.dispatcher().into_protocol(json_codec()));
            let (client_end, server_end) = duplex_pair();
            let channel = ReactiveServer::new(server_end, json_codec()).expose(job.status);
            let mirror: RemoteSource<str> = ReactiveClient::new(client_end, json_codec()).source(channel);
            let client = JobClient { transport, codec = json_codec(), status = mirror };
            let watching = client.status.sub(|s| {
                print(i"status = {s}");
            });
            match client.run() {
                Ok(let n) => print(i"run -> {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            watching.dispose();
        }
        "#,
        "status = idle\nstatus = done\nrun -> 7\n",
    );
}

#[test]
fn a_no_arg_rpc_methods_writes_coalesce_in_the_wire_turn() {
    // The hole the wave pin uncovered, pinned on its own (no async involved):
    // no-arg methods once took a bare `.on(..)` fast path that skipped the
    // wire turn entirely, so each write leaked as its own update. Every
    // method route now goes through `route_block`'s turn — two writes in a
    // sync no-arg method arrive at the mirror as ONE update, the final value.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::Signal;
        import std::result::Result::{ self, Ok, Err };
        import std::json::{ Json, FromJson };
        import std::json::json_codec;
        import std::rpc::{ local_rpc, duplex_pair, ReactiveServer, ReactiveClient, RemoteSource };

        [service(FlipClient)]
        struct Flip {
            [expose] state: Signal<str>,
        }

        impl Flip {
            [rpc]
            fun flip(self): i32 {
                self.state.set("mid");
                self.state.set("final");
                1
            }
        }

        fun main() {
            let flip = Flip { state = Signal::new("start") };
            let transport = local_rpc(flip.dispatcher().into_protocol(json_codec()));
            let (client_end, server_end) = duplex_pair();
            let channel = ReactiveServer::new(server_end, json_codec()).expose(flip.state);
            let mirror: RemoteSource<str> = ReactiveClient::new(client_end, json_codec()).source(channel);
            let client = FlipClient { transport, codec = json_codec(), state = mirror };
            let watching = client.state.sub(|s| {
                print(i"state = {s}");
            });
            match client.flip() {
                Ok(let n) => print(i"flip -> {n}"),
                Err(let error) => print(i"err {error.to_json()}"),
            }
            watching.dispose();
        }
        "#,
        "state = start\nstate = final\nflip -> 1\n",
    );
}

#[test]
fn a_hand_written_async_route_dispatches_through_respond() {
    // The foundation API without the macro: an async handler registered with
    // `Dispatcher.on` (its `async |..|` parameter), driven through `respond`
    // directly — the reply envelope encodes the settled outcome.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::json_codec;
        import std::wire::Frame;
        import std::rpc::{ Dispatcher, reply, encode_request, RpcOutcome };
        import std::time::{ sleep_for, Duration };

        fun main() {
            let protocol = Dispatcher::new()
                .on("slow", |request| {
                    sleep_for(Duration::millis(5));
                    reply(21)
                })
                .into_protocol(json_codec());
            let answer = protocol.respond(encode_request(json_codec(), "slow", []));
            match answer {
                Frame::Text(let envelope) => print(i"answer: {envelope}"),
                Frame::Binary(let bytes) => print("answer: unexpected binary"),
            }
        }
        "#,
        "answer: {\"Success\":21}\n",
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
fn impl_binder_in_trait_argument_position() {
    // One impl serving every sink: the binder sits in the TRAIT argument,
    // registered like a subject binder (bound-less ones inherit the trait's
    // declared bound for the position) — transport-rpc.md §6.1's other gap,
    // closed.
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
        	T::from_json(text)
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

// B13, FIXED: a direct call on a let-bound closure now fills an unannotated
// parameter's shared type slot from the argument, so the body's uses type.
// (The first call site wins; later calls compare against it.)
#[test]
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
            let e = 5i53;
            let f = 5u53;
            let g = 2.5f32;
            let allowed = 128i8;
            let expected: u8 = 7;
            let fractional: f32 = 1.5;
            print(a + a);
            print(b);
            print(c + c);
            print(d);
            print(e + f.as_i53());
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
fn an_i53_literal_beyond_the_f64_window_errors() {
    assert_fails_spanning(
        "fun main() { let x = 9007199254740993i53; }\nmain();\n",
        "9007199254740993i53",
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
            print(100i53 / 8i53);
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
            print((2.5f32).as_i53());
            print((5i53).as_u53());
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

// --- The ambient owner (proposal/ambient-owner.md, backlog A5) ---

// A covered `effect` registers into the ambient owner and dies with it.
#[test]
fn effect_registers_into_the_ambient_owner_and_dies_with_it() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner, Disposable, owner_scope };

        fun main() {
            let count = Signal::new(1);
            let owner = Owner::new();
            owner_scope.run(owner, || {
                count.effect(|value| print(value));
            });
            count.set(2);
            owner.dispose();
            count.set(3);
            print("done");
        }

        main();
        "#,
        "1\n2\ndone\n",
    );
}

// The static fence: `effect` reachable outside every `owner_scope.run` is a
// compile error, not a runtime absence.
#[test]
fn effect_outside_an_owner_scope_is_a_compile_error() {
    let diagnostics = failure_diagnostics(
        r#"
import std::print;
import std::reactive::Signal;

fun main() {
    let count = Signal::new(1);
    count.effect(|value| print(value));
}
main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("without an enclosing `run`")),
        "expected the coverage fence; got: {diagnostics:#?}"
    );
}

// The dead-reader exemption: a program that imports `std::reactive` without
// ever using the ambient layer must compile — an uncalled reader cannot run,
// so it cannot run uncovered.
#[test]
fn importing_reactive_without_the_ambient_layer_compiles() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Subscription, Disposable };

        fun main() {
            let count = Signal::new(5);
            let seen = count.sub(|value| print(value));
            seen.dispose();
        }

        main();
        "#,
        "5\n",
    );
}

// A DEAD user helper reaching the ambient reader must not poison the
// covered path beside it.
#[test]
fn a_dead_ambient_reader_does_not_poison_covered_paths() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner, Disposable, owner_scope, get_owner };

        // Never called: exempt, and it must not unbind `get_owner` for the
        // covered path below.
        fun forgotten() {
            let owner = get_owner();
            owner.dispose();
        }

        fun main() {
            let count = Signal::new(7);
            let owner = Owner::new();
            owner_scope.run(owner, || {
                count.effect(|value| print(value));
            });
            print("alive");
        }

        main();
        "#,
        "7\nalive\n",
    );
}

// FIXED (backlog B14): the context pass now adds trait-dispatch edges
// locally — a default body reading a context is covered when its dispatch
// sites are, and the hidden value threads through the dispatch call.
#[test]
fn a_trait_default_body_reads_context_through_covered_dispatch() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let current: Context<i32> = Context::new();

        trait Probe {
            fun name(self): str;

            fun report(self) {
                print(i"{self.name()}: {current.get()}");
            }
        }

        struct Widget { tag: str }

        impl Widget with Probe {
            fun name(self): str {
                self.tag
            }
        }

        fun main() {
            current.run(9, || {
                Widget { tag = "w" }.report();
            });
        }

        main();
        "#,
        "w: 9\n",
    );
}

// FIXED with B14's slice: an inherited trait default called on a GENERIC
// subject's concrete instance (`Signal<i32>` inheriting from
// `impl Signal<type T> with Source<T>`) — `resolve_inherited_default`
// matched impl subjects by exact type equality, so generic subjects never
// matched and the call silently bound to the trait's ABSTRACT member (the
// B12 silent-miscompile shape). Now nominal, like `resolve_member_on_type`.
#[test]
fn an_inherited_default_on_a_generic_subject_dispatches() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        trait Doubler<T> {
            fun once(self): T;

            fun twice(self): T {
                self.once() + self.once()
            }
        }

        struct Holder<T> {
            value: T,
        }

        impl Holder<type T> with Doubler<T> {
            fun once(self): T {
                self.value
            }
        }

        fun main() {
            print(Holder { value = 21 }.twice());
        }

        main();
        "#,
        "42\n",
    );
}

// --- Context-typed closure parameters (proposal/ambient-owner.md §5, B15) ---

// The flagship: an injected closure rides a PLAIN function into `run` — the
// literal is born outside the extent and defers its binding to the call.
#[test]
fn an_injected_closure_rides_a_plain_wrapper_into_run() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let current: Context<i32> = Context::new();

        fun run_with(value: i32, body: (|| void) context current) {
            current.run(value, body);
        }

        fun main() {
            run_with(5, || print(current.get()));
            run_with(9, || print(current.get() + 1));
        }

        main();
        "#,
        "5\n10\n",
    );
}

// Injected values forward to parameters with the SAME clause, and calls
// through them thread the deferred argument on.
#[test]
fn injected_closures_forward_and_thread_through_calls() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let current: Context<i32> = Context::new();

        fun call_it(body: (|| void) context current) {
            body();
        }

        fun forward(body: (|| void) context current) {
            call_it(body);
        }

        fun main() {
            current.run(7, || {
                forward(|| print(current.get() + 100));
            });
        }

        main();
        "#,
        "107\n",
    );
}

// A multi-context clause: both deferred arguments supply, in clause order.
#[test]
fn a_multi_context_clause_injects_both_values() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let left: Context<i32> = Context::new();
        let right: Context<i32> = Context::new();

        fun call_it(body: (|| void) context (left, right)) {
            body();
        }

        fun main() {
            left.run(3, || {
                right.run(4, || {
                    call_it(|| print(left.get() * 10 + right.get()));
                });
            });
        }

        main();
        "#,
        "34\n",
    );
}

// Calling an injected closure is a read: an uncovered caller is fenced.
#[test]
fn an_uncovered_injected_call_is_a_compile_error() {
    let diagnostics = failure_diagnostics(
        r#"
import std::print;
import std::context::Context;

let current: Context<i32> = Context::new();

fun call_it(body: (|| void) context current) {
    body();
}

fun main() {
    call_it(|| print(current.get()));
}
main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("injected closure is called here")),
        "expected the injected-call fence; got: {diagnostics:#?}"
    );
}

// The value-flow restriction: an injected closure may be called, forwarded to
// a matching clause, or handed to `run` — nothing else.
#[test]
fn an_injected_closure_cannot_escape() {
    let source = r#"
import std::context::Context;

let current: Context<i32> = Context::new();

fun hold(body: (|| void) context current) {
    let escaped = body;
}

fun main() {}
main();
        "#;
    let diagnostics = failure_diagnostics(source);
    // The error anchors at the escaping USE (the second `body`), not the
    // parameter declaration.
    let use_site = source.rfind("body").unwrap();
    assert!(
        diagnostics.iter().any(|(message, range)| {
            message.contains("can only be called, forwarded") && range.start == use_site
        }),
        "expected the escape error at the use; got: {diagnostics:#?}"
    );
}

// Clause validation: the named value must be a context.
#[test]
fn a_clause_naming_a_non_context_errors() {
    let diagnostics = failure_diagnostics(
        r#"
import std::context::Context;

let unused: Context<i32> = Context::new();
let plain = 5;

fun bad(body: (|| void) context plain) {
    body();
}

fun main() {}
main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("names a value that is not a context")),
        "expected the non-context clause error; got: {diagnostics:#?}"
    );
}

// Clause placement: closure types only.
#[test]
fn a_clause_on_a_non_closure_type_errors() {
    let diagnostics = failure_diagnostics(
        r#"
import std::context::Context;

let current: Context<i32> = Context::new();

fun bad(value: (i32) context current) {}

fun main() {}
main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("only supported on a closure type")),
        "expected the placement error; got: {diagnostics:#?}"
    );
}

// Clause resolution: unknown names error at the name.
#[test]
fn a_clause_naming_an_unknown_value_errors() {
    assert_fails_spanning(
        r#"
fun bad(body: (|| void) context missing_name) {
    body();
}

fun main() {}
main();
        "#,
        "missing_name",
        "cannot find context `missing_name`",
    );
}

// Duplicate contexts in one clause error.
#[test]
fn a_duplicate_context_in_a_clause_errors() {
    let diagnostics = failure_diagnostics(
        r#"
import std::context::Context;

let current: Context<i32> = Context::new();

fun bad(body: (|| void) context (current, current)) {
    body();
}

fun main() {}
main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("duplicate context `current`")),
        "expected the duplicate error; got: {diagnostics:#?}"
    );
}

// `run` accepts an injected value only when its clause is exactly the run's
// context.
#[test]
fn run_rejects_a_mismatched_injected_body() {
    let diagnostics = failure_diagnostics(
        r#"
import std::context::Context;

let current: Context<i32> = Context::new();
let other: Context<i32> = Context::new();

fun mismatch(body: (|| void) context current) {
    other.run(1, body);
}

fun main() {}
main();
        "#,
    );
    assert!(
        diagnostics.iter().any(|(message, _)| {
            message.contains("closure value whose type is `context`-annotated")
        }),
        "expected the run-mismatch error; got: {diagnostics:#?}"
    );
}

// FIXED alongside B15: a context that is created but never read or run no
// longer emits a dangling `Context::new()` call — the news lower on the
// early path too.
#[test]
fn an_unused_context_compiles_and_runs() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let current: Context<i32> = Context::new();

        fun main() {
            print("quiet");
        }

        main();
        "#,
        "quiet\n",
    );
}

// `Context.run` yields its body's value (the `batch` shape): direct,
// expression-position, and void bodies stay compatible.
#[test]
fn run_yields_the_body_value() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let current: Context<i32> = Context::new();

        fun main() {
            let answer = current.run(21, || current.get() * 2);
            print(answer);
            print(current.run(5, || current.get() + 1) + 100);
            current.run(1, || {
                print(current.get());
            });
        }

        main();
        "#,
        "42\n106\n1\n",
    );
}

// `comp` — the component scope: the body's product pairs with the disposal
// handle, and the component's effects die with it.
#[test]
fn comp_returns_the_product_and_the_scope() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner, Disposable, comp };

        fun main() {
            let count = Signal::new(1);
            let (label, scope) = comp(|| {
                count.effect(|value| print(value));
                "built"
            });
            print(label);
            count.set(2);
            scope.dispose();
            count.set(3);
            print("done");
        }

        main();
        "#,
        "1\nbuilt\n2\ndone\n",
    );
}

// `run_with_owner` yields its body's value too.
#[test]
fn run_with_owner_yields_the_body_value() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Owner, run_with_owner };

        fun main() {
            let owner = Owner::new();
            print(run_with_owner(owner, || 40 + 2));
        }

        main();
        "#,
        "42\n",
    );
}

// The clause may name an IMPORTED context (the `std::ui` shape) — resolution
// runs after the import fixpoint, following the import alias to the defining
// binding so identity agrees with the threading pass.
#[test]
fn a_clause_can_name_an_imported_context() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner, Disposable, owner_scope, run_with_owner };

        fun boundary(body: (|| void) context owner_scope) {
            let owner = Owner::new();
            run_with_owner(owner, || body());
        }

        fun main() {
            let count = Signal::new(4);
            boundary(|| count.effect(|value| print(value)));
            print("ok");
        }

        main();
        "#,
        "4\nok\n",
    );
}

// --- B12: a generic bound instantiated at a type LACKING the impl must be a ---
// --- spanned compile error, not a silent dispatch to the abstract member.  ---

// The shared shape: `Dog` implements `Greet`, `Cat` does not. `greet` returns
// void so a miss is the fully SILENT miscompile (no return-type error to trip
// over) — the worst form of the class.
const GREET_PRELUDE: &str = r#"
    trait Greet {
        fun greet(self);
    }
    struct Dog { name: str }
    struct Cat { name: str }
    impl Dog with Greet {
        fun greet(self) {
            let _woof = self.name;
        }
    }
"#;

#[test]
fn a_bound_satisfied_by_an_impl_still_compiles() {
    assert_compiles(&format!(
        r#"{GREET_PRELUDE}
        fun describe<T: Greet>(subject: T) {{
            subject.greet();
        }}
        fun main() {{
            describe(Dog {{ name = "rex" }});
        }}
        main();
        "#
    ));
}

#[test]
fn a_free_function_bound_rejects_a_type_without_the_impl() {
    let source = format!(
        r#"{GREET_PRELUDE}
        fun describe<T: Greet>(subject: T) {{
            subject.greet();
        }}
        fun main() {{
            describe(Cat {{ name = "tom" }});
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        r#"describe(Cat { name = "tom" })"#,
        "does not implement trait 'Greet'",
    );
}

#[test]
fn a_method_own_generic_bound_rejects_a_type_without_the_impl() {
    let source = format!(
        r#"{GREET_PRELUDE}
        struct Kennel {{ size: i32 }}
        impl Kennel {{
            fun admit<T: Greet>(self, guest: T) {{
                guest.greet();
            }}
        }}
        fun main() {{
            let kennel = Kennel {{ size = 3 }};
            kennel.admit(Cat {{ name = "tom" }});
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        r#"kennel.admit(Cat { name = "tom" })"#,
        "does not implement trait 'Greet'",
    );
}

#[test]
fn a_multi_bound_names_the_missing_trait() {
    // `Dog` implements `Greet` but not `Fetch` — the error must name `Fetch`.
    let source = format!(
        r#"{GREET_PRELUDE}
        trait Fetch {{
            fun fetch(self);
        }}
        fun train<T: Greet + Fetch>(subject: T) {{
            subject.greet();
            subject.fetch();
        }}
        fun main() {{
            train(Dog {{ name = "rex" }});
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        r#"train(Dog { name = "rex" })"#,
        "does not implement trait 'Fetch'",
    );
}

#[test]
fn a_static_bound_call_rejects_a_type_without_the_impl() {
    // The `T::member()` channel: an explicit generic argument that fails the bound.
    let source = format!(
        r#"{GREET_PRELUDE}
        trait Fresh {{
            fun fresh(): Self;
        }}
        impl Dog with Fresh {{
            fun fresh(): Self {{
                ret Dog {{ name = "pup" }};
            }}
        }}
        fun spawn<T: Fresh>(): T {{
            ret T::fresh();
        }}
        fun main() {{
            let _cat: Cat = spawn<Cat>();
        }}
        main();
        "#
    );
    assert_fails(&source);
}

#[test]
fn a_rebounded_forward_still_compiles() {
    // A wrapper that re-declares the bound forwards legally.
    assert_compiles(&format!(
        r#"{GREET_PRELUDE}
        fun describe<T: Greet>(subject: T) {{
            subject.greet();
        }}
        fun relay<U: Greet>(subject: U) {{
            describe(subject);
        }}
        fun main() {{
            relay(Dog {{ name = "rex" }});
        }}
        main();
        "#
    ));
}

#[test]
fn a_generic_impl_subject_satisfies_the_bound() {
    // `impl Crate2<type X> with Greet` covers every `Crate2<..>` instantiation.
    assert_compiles(&format!(
        r#"{GREET_PRELUDE}
        struct Crate2<T> {{ inner: T }}
        impl Crate2<type X> with Greet {{
            fun greet(self) {{
                let _hi = 1;
            }}
        }}
        fun describe<T: Greet>(subject: T) {{
            subject.greet();
        }}
        fun main() {{
            describe(Crate2 {{ inner = 5 }});
        }}
        main();
        "#
    ));
}

#[test]
fn a_trait_default_without_an_impl_does_not_satisfy_the_bound() {
    // A default body is inherited THROUGH an impl; with no `impl Cat with
    // Chatty` at all, the bound stays unsatisfied.
    let source = r#"
        trait Chatty {
            fun chat(self) {
                let _hello = 1;
            }
        }
        struct Cat { name: str }
        fun engage<T: Chatty>(subject: T) {
            subject.chat();
        }
        fun main() {
            engage(Cat { name = "tom" });
        }
        main();
        "#;
    assert_fails_spanning(
        source,
        r#"engage(Cat { name = "tom" })"#,
        "does not implement trait 'Chatty'",
    );
}

#[test]
fn an_under_bounded_forward_is_rejected_at_the_inner_call() {
    // Forwarding through a wrapper does NOT launder the requirement: the
    // wrapper's own parameter must re-declare the bound (see
    // `a_rebounded_forward_still_compiles` for the legal spelling).
    let source = format!(
        r#"{GREET_PRELUDE}
        fun describe<T: Greet>(subject: T) {{
            subject.greet();
        }}
        fun outer<U>(x: U) {{
            describe(x);
        }}
        fun main() {{
            outer(Dog {{ name = "rex" }});
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        "describe(x)",
        "generic parameter 'U' is missing the bound ': Greet'",
    );
}

#[test]
fn a_bound_satisfied_through_a_subtrait_impl_compiles() {
    // Implementing a SUBTRAIT satisfies a supertrait bound: `Loud` extends
    // `Greet`, and `impl Dog with Loud` must satisfy `T: Greet`.
    assert_compiles(
        r#"
        trait Greet {
            fun greet(self);
        }
        trait Loud with Greet {
            fun shout(self);
        }
        struct Dog { name: str }
        impl Dog with Loud {
            fun greet(self) {
                let _quiet = 1;
            }
            fun shout(self) {
                let _loud = 2;
            }
        }
        fun describe<T: Greet>(subject: T) {
            subject.greet();
        }
        fun main() {
            describe(Dog { name = "rex" });
        }
        main();
        "#,
    );
}

// --- B12 depth: a CONDITIONAL impl (`impl Box2<type X: Greet> with Greet`) ---
// --- satisfies a bound only when its binder bounds hold at the argument.   ---

const CONDITIONAL_PRELUDE: &str = r#"
    trait Greet {
        fun greet(self);
    }
    struct Dog { name: str }
    struct Cat { name: str }
    impl Dog with Greet {
        fun greet(self) {
            let _woof = self.name;
        }
    }
    struct Box2<T> { inner: T }
    impl Box2<type X: Greet> with Greet {
        fun greet(self) {
            self.inner.greet();
        }
    }
    fun describe<T: Greet>(subject: T) {
        subject.greet();
    }
"#;

#[test]
fn a_conditional_impl_with_a_satisfied_condition_compiles() {
    assert_compiles(&format!(
        r#"{CONDITIONAL_PRELUDE}
        fun main() {{
            describe(Box2 {{ inner = Dog {{ name = "rex" }} }});
        }}
        main();
        "#
    ));
}

#[test]
fn a_conditional_impl_with_a_failed_condition_is_rejected() {
    let source = format!(
        r#"{CONDITIONAL_PRELUDE}
        fun main() {{
            describe(Box2 {{ inner = Cat {{ name = "tom" }} }});
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        r#"describe(Box2 { inner = Cat { name = "tom" } })"#,
        "does not implement trait 'Greet'",
    );
}

#[test]
fn a_conditional_impl_checks_recursively() {
    // The condition applies at every level: a box of boxes of dogs greets,
    // a box of boxes of cats does not.
    assert_compiles(&format!(
        r#"{CONDITIONAL_PRELUDE}
        fun main() {{
            describe(Box2 {{ inner = Box2 {{ inner = Dog {{ name = "rex" }} }} }});
        }}
        main();
        "#
    ));
    let source = format!(
        r#"{CONDITIONAL_PRELUDE}
        fun main() {{
            describe(Box2 {{ inner = Box2 {{ inner = Cat {{ name = "tom" }} }} }});
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        r#"describe(Box2 { inner = Box2 { inner = Cat { name = "tom" } } })"#,
        "does not implement trait 'Greet'",
    );
}

#[test]
fn an_inherited_binder_bound_conditions_the_impl() {
    // The impl binder declares no bound of its own, so it INHERITS the struct
    // declaration's (`struct Kennel2<T: Greet>`); binding through the impl
    // must still enforce it.
    let source = r#"
        trait Greet {
            fun greet(self);
        }
        trait Show {
            fun show(self);
        }
        struct Dog { name: str }
        struct Cat { name: str }
        impl Dog with Greet {
            fun greet(self) {
                let _woof = self.name;
            }
        }
        struct Kennel2<T: Greet> { inner: T }
        impl Kennel2<type T> with Show {
            fun show(self) {
                self.inner.greet();
            }
        }
        fun display<T: Show>(subject: T) {
            subject.show();
        }
        fun main() {
            display(Kennel2 { inner = Cat { name = "tom" } });
        }
        main();
        "#;
    assert_fails_spanning(
        source,
        r#"display(Kennel2 { inner = Cat { name = "tom" } })"#,
        "does not implement trait 'Show'",
    );
}

// --- B12 family: DECLARED bounds check at CONSTRUCTION — a struct literal ---
// --- or enum-variant call binding a declared generic must satisfy it.     ---

#[test]
fn a_struct_literal_satisfying_the_declared_bound_compiles() {
    assert_compiles(&format!(
        r#"{GREET_PRELUDE}
        struct Kennel2<T: Greet> {{ inner: T }}
        fun main() {{
            let _kennel = Kennel2 {{ inner = Dog {{ name = "rex" }} }};
        }}
        main();
        "#
    ));
}

#[test]
fn a_struct_literal_violating_the_declared_bound_is_rejected() {
    let source = format!(
        r#"{GREET_PRELUDE}
        struct Kennel2<T: Greet> {{ inner: T }}
        fun main() {{
            let _kennel = Kennel2 {{ inner = Cat {{ name = "tom" }} }};
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        r#"Kennel2 {{ inner = Cat {{ name = "tom" }} }}"#
            .replace("{{", "{")
            .replace("}}", "}")
            .as_str(),
        "does not implement trait 'Greet'",
    );
}

#[test]
fn an_enum_variant_violating_the_declared_bound_is_rejected() {
    let source = format!(
        r#"{GREET_PRELUDE}
        enum Slot<T: Greet> {{
            Filled(T),
            Empty,
        }}
        fun main() {{
            let _slot = Slot::Filled(Cat {{ name = "tom" }});
        }}
        main();
        "#
    );
    assert_fails(&source);
}

#[test]
fn an_enum_variant_satisfying_the_declared_bound_compiles() {
    assert_compiles(&format!(
        r#"{GREET_PRELUDE}
        enum Slot<T: Greet> {{
            Filled(T),
            Empty,
        }}
        fun main() {{
            let _slot = Slot::Filled(Dog {{ name = "rex" }});
        }}
        main();
        "#
    ));
}

#[test]
fn a_generic_struct_literal_with_a_bounded_forward_compiles() {
    // Construction inside a generic function whose parameter re-declares the
    // bound is legal.
    assert_compiles(&format!(
        r#"{GREET_PRELUDE}
        struct Kennel2<T: Greet> {{ inner: T }}
        fun pack<U: Greet>(value: U) {{
            let _kennel = Kennel2 {{ inner = value }};
        }}
        fun main() {{
            pack(Dog {{ name = "rex" }});
        }}
        main();
        "#
    ));
}

// The unbounded-forward gap's root fix: the initializer's second-chance
// FIELD-first reconcile binds a declared parameter from a generic field
// value, so the argument reads as the caller's `U` (whose missing bound the
// declared-bound check then rejects) instead of the constraint fallback.
#[test]
fn a_generic_struct_literal_with_an_unbounded_forward_is_rejected() {
    let source = format!(
        r#"{GREET_PRELUDE}
        struct Kennel2<T: Greet> {{ inner: T }}
        fun pack<U>(value: U) {{
            let _kennel = Kennel2 {{ inner = value }};
        }}
        fun main() {{
            pack(Dog {{ name = "rex" }});
        }}
        main();
        "#
    );
    assert_fails(&source);
}

#[test]
fn a_partially_binding_variant_still_checks_its_bound_parameter() {
    // `Pair::Left` binds only `A` — the check must still fire on `A` even
    // though `B` stays unbound at this construction.
    let source = format!(
        r#"{GREET_PRELUDE}
        enum Pair<A: Greet, B: Greet> {{
            Left(A),
            Right(B),
        }}
        fun main() {{
            let _left = Pair::Left(Cat {{ name = "tom" }});
        }}
        main();
        "#
    );
    assert_fails(&source);
}

// --- B12 family: bound trait ARGUMENTS must match — an impl providing ---
// --- `Feed<str>` does not satisfy `F: Feed<i32>`.                     ---

const FEED_PRELUDE: &str = r#"
    trait Feed<T> {
        fun feed(self, food: T);
    }
    struct Bird { name: str }
    struct Fish { name: str }
    impl Bird with Feed<str> {
        fun feed(self, food: str) {
            let _crumbs = food;
        }
    }
    impl Fish with Feed<i32> {
        fun feed(self, food: i32) {
            let _flakes = food;
        }
    }
"#;

#[test]
fn a_matching_trait_argument_satisfies_the_bound() {
    assert_compiles(&format!(
        r#"{FEED_PRELUDE}
        fun wants_numbers<F: Feed<i32>>(feeder: F) {{
            feeder.feed(3);
        }}
        fun main() {{
            wants_numbers(Fish {{ name = "bubbles" }});
        }}
        main();
        "#
    ));
}

#[test]
fn a_mismatched_trait_argument_is_rejected() {
    let source = format!(
        r#"{FEED_PRELUDE}
        fun wants_numbers<F: Feed<i32>>(feeder: F) {{
            feeder.feed(3);
        }}
        fun main() {{
            wants_numbers(Bird {{ name = "tweety" }});
        }}
        main();
        "#
    );
    assert_fails_spanning(
        &source,
        r#"wants_numbers(Bird { name = "tweety" })"#,
        "does not implement trait 'Feed<i32>'",
    );
}

#[test]
fn a_bound_argument_flowing_from_another_generic_is_checked() {
    // `F: Feed<T>` with `T` bound by a sibling argument: eat(bird, 5) needs
    // Feed<i32>, and Bird only provides Feed<str>.
    assert_compiles(&format!(
        r#"{FEED_PRELUDE}
        fun eat<T, F: Feed<T>>(feeder: F, seed: T) {{
            feeder.feed(seed);
        }}
        fun main() {{
            eat(Bird {{ name = "tweety" }}, "worm");
        }}
        main();
        "#
    ));
    let source = format!(
        r#"{FEED_PRELUDE}
        fun eat<T, F: Feed<T>>(feeder: F, seed: T) {{
            feeder.feed(seed);
        }}
        fun main() {{
            eat(Bird {{ name = "tweety" }}, 5);
        }}
        main();
        "#
    );
    assert_fails(&source);
}

#[test]
fn a_declared_bound_trait_argument_is_checked_at_construction() {
    let source = format!(
        r#"{FEED_PRELUDE}
        struct Aviary<F: Feed<i32>> {{ feeder: F }}
        fun main() {{
            let _aviary = Aviary {{ feeder = Bird {{ name = "tweety" }} }};
        }}
        main();
        "#
    );
    assert_fails(&source);
}

#[test]
fn a_conditional_impl_binder_trait_argument_is_checked() {
    // The binder bound carries arguments too: a box is only numeric-feedable
    // when its content feeds on numbers.
    let source = format!(
        r#"{FEED_PRELUDE}
        struct Box3<T> {{ inner: T }}
        impl Box3<type X: Feed<i32>> with Feed<i32> {{
            fun feed(self, food: i32) {{
                self.inner.feed(food);
            }}
        }}
        fun wants_numbers<F: Feed<i32>>(feeder: F) {{
            feeder.feed(3);
        }}
        fun main() {{
            wants_numbers(Box3 {{ inner = Bird {{ name = "tweety" }} }});
        }}
        main();
        "#
    );
    assert_fails(&source);
}

#[test]
fn a_generic_enum_variant_with_an_unbounded_forward_is_rejected() {
    // The enum analogue of the struct forward: the checker derives the
    // variant's bindings by reconciling payload types against argument
    // types, so the caller's unbounded `U` surfaces and fails the bound.
    let source = format!(
        r#"{GREET_PRELUDE}
        enum Slot<T: Greet> {{
            Filled(T),
            Empty,
        }}
        fun pack<U>(value: U) {{
            let _slot = Slot::Filled(value);
        }}
        fun main() {{
            pack(Dog {{ name = "rex" }});
        }}
        main();
        "#
    );
    assert_fails(&source);
}

#[test]
fn a_generic_enum_variant_with_a_bounded_forward_compiles() {
    assert_compiles(&format!(
        r#"{GREET_PRELUDE}
        enum Slot<T: Greet> {{
            Filled(T),
            Empty,
        }}
        fun pack<U: Greet>(value: U) {{
            let _slot = Slot::Filled(value);
        }}
        fun main() {{
            pack(Dog {{ name = "rex" }});
        }}
        main();
        "#
    ));
}

// --- view-invalidation.md E2: a mutating call on the viewed root is an ---
// --- invalidating event, like reassignment (rule 4).                   ---

#[test]
fn a_mutating_method_under_a_live_element_view_is_rejected() {
    // The proposal's P3: pop() may drop the viewed element.
    let source = r#"
        fun main() {
            mut a = [ 0 ];
            let b = &mut a[0];
            a.pop();
            b = 99;
        }
        main();
        "#;
    assert_fails_spanning(
        source,
        "a.pop()",
        "cannot mutate 'a' with '.pop(..)' while a view into it is live",
    );
}

#[test]
fn a_push_under_a_live_element_view_is_rejected() {
    // push is included deliberately: harmless on JS, reallocates on native.
    let source = r#"
        fun main() {
            mut a = [ 0 ];
            let b = &mut a[0];
            a.push(1);
            b = 99;
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn passing_the_viewed_root_by_mut_ref_is_rejected() {
    // The proposal's P4: the callee may resize the container.
    let source = r#"
        fun grow(list: &mut List<i32>) {
            list.push(7);
        }
        fun main() {
            mut a = [ 0 ];
            let b = &mut a[0];
            grow(&mut a);
            b = 99;
        }
        main();
        "#;
    assert_fails_spanning(
        source,
        "grow(&mut a)",
        "cannot pass '&mut a' to 'grow' while a view into it is live",
    );
}

#[test]
fn a_user_mut_self_method_under_a_live_view_is_rejected() {
    let source = r#"
        struct Basket { items: List<i32> }
        impl Basket {
            fun clear_items(&mut self) {
                self.items = [];
            }
        }
        fun main() {
            mut basket = Basket { items = [ 1 ] };
            let held = &mut basket.items;
            basket.clear_items();
            held.push(2);
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn a_read_only_method_under_a_live_view_compiles() {
    // &self methods do not invalidate.
    assert_compiles(
        r#"
        import std::print;
        fun main() {
            mut a = [ 5 ];
            let b = &mut a[0];
            print(a.len());
            b = 99;
        }
        main();
        "#,
    );
}

#[test]
fn writing_through_the_view_itself_compiles() {
    // The view's whole purpose; not an invalidating event.
    assert_compiles(
        r#"
        fun main() {
            mut a = [ 5 ];
            let b = &mut a[0];
            b = 99;
            b = 100;
        }
        main();
        "#,
    );
}

#[test]
fn mutating_an_unrelated_container_compiles() {
    assert_compiles(
        r#"
        fun main() {
            mut a = [ 5 ];
            mut other = [ 1 ];
            let b = &mut a[0];
            other.pop();
            b = 99;
        }
        main();
        "#,
    );
}

#[test]
fn a_mutating_call_before_the_view_exists_compiles() {
    // Scan order: the view is not yet live at the call.
    assert_compiles(
        r#"
        fun main() {
            mut a = [ 5 ];
            a.pop();
            a.push(6);
            let b = &mut a[0];
            b = 99;
        }
        main();
        "#,
    );
}

#[test]
fn a_mutating_call_in_a_nested_block_under_an_outer_view_is_rejected() {
    // Lexical liveness carries into inner blocks.
    let source = r#"
        fun main() {
            mut a = [ 0 ];
            let b = &mut a[0];
            {
                a.pop();
            }
            b = 99;
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn mutating_the_container_inside_a_for_mut_loop_is_rejected() {
    // The loop binding is a view into the container for the body's extent.
    let source = r#"
        fun main() {
            mut a = [ 1, 2, 3 ];
            for e in &mut a {
                a.pop();
            }
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn reassigning_the_container_inside_a_for_mut_loop_is_rejected() {
    // The same loop-binding origin feeds the shipped E1 (reassignment) check.
    let source = r#"
        fun main() {
            mut a = [ 1, 2, 3 ];
            for e in &mut a {
                a = [];
            }
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn a_mut_call_on_a_viewed_scalar_root_compiles() {
    // The transparent-references demo's shape: a scalar's boxed cell has no
    // geometry — a callee can only write the slot, which is the aliasing the
    // model permits. E2 exempts scalar roots.
    assert_compiles(
        r#"
        import std::print;
        fun add_ten(value: &mut i32) {
            value += 10;
        }
        fun main() {
            mut a: i32 = 10;
            let b: &mut i32 = &mut a;
            add_ten(&mut a);
            print(*b);
        }
        main();
        "#,
    );
}

// --- view-invalidation.md E3: a view may not live across `await` — the ---
// --- writer set during a suspension is the whole program.              ---

#[test]
fn a_view_across_await_is_rejected() {
    // The proposal's probe program (compiled silently before E3).
    let source = r#"
        struct Point { x: i32 }
        async fun tick() {
            let _beat = 1;
        }
        async fun mutate_across_await() {
            mut point = Point { x = 1 };
            let view = &mut point;
            await tick();
            view.x = 99;
        }
        fun main() {
            mutate_across_await();
        }
        main();
        "#;
    assert_fails_spanning(source, "await tick()", "cannot hold a view across 'await'");
}

#[test]
fn a_view_created_after_the_await_compiles() {
    assert_compiles(
        r#"
        struct Point { x: i32 }
        async fun tick() {
            let _beat = 1;
        }
        async fun late_view() {
            mut point = Point { x = 1 };
            await tick();
            let view = &mut point;
            view.x = 99;
        }
        fun main() {
            late_view();
        }
        main();
        "#,
    );
}

#[test]
fn an_await_in_one_branch_under_a_live_view_is_rejected() {
    // Lexical liveness: an await on ANY path while the view is live counts.
    let source = r#"
        struct Point { x: i32 }
        async fun tick() {
            let _beat = 1;
        }
        async fun branchy(flag: bool) {
            mut point = Point { x = 1 };
            let view = &mut point;
            if flag {
                await tick();
            }
            view.x = 99;
        }
        fun main() {
            branchy(true);
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn an_await_inside_a_for_mut_loop_is_rejected() {
    // The loop binding is a view live across every iteration.
    let source = r#"
        async fun tick() {
            let _beat = 1;
        }
        async fun stream() {
            mut items = [ 1, 2, 3 ];
            for e in &mut items {
                await tick();
            }
        }
        fun main() {
            stream();
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn a_shared_write_view_across_await_is_rejected() {
    // The settled sub-question: Shared is NOT exempt — the handle pins the
    // cell (memory-safe), but another turn's write still reseats elements
    // under the held view. Re-acquire after the await. (`read()` returns a
    // COPY by design, so only `write()`'s view is at stake — see the guard
    // below.)
    let source = r#"
        import std::shared::Shared;
        async fun tick() {
            let _beat = 1;
        }
        async fun stale_view() {
            let shared = Shared::new([ 1, 2, 3 ]);
            let list = shared.write();
            await tick();
            list.push(4);
        }
        fun main() {
            stale_view();
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn a_shared_read_copy_across_await_compiles() {
    // `read()` returns a copy (value semantics) — nothing to invalidate.
    assert_compiles(
        r#"
        import std::shared::Shared;
        import std::print;
        async fun tick() {
            let _beat = 1;
        }
        async fun fresh_copy() {
            let shared = Shared::new([ 1, 2, 3 ]);
            let list = shared.read();
            await tick();
            print(list.len());
        }
        fun main() {
            fresh_copy();
        }
        main();
        "#,
    );
}

#[test]
fn an_async_function_with_a_view_parameter_is_rejected() {
    // The signature rule: the caller's view would be held inside the
    // suspended callee across its awaits.
    let source = r#"
        async fun tick() {
            let _beat = 1;
        }
        async fun stash(value: &mut i32) {
            await tick();
            value += 1;
        }
        fun main() {
            mut a = 5;
            stash(&mut a);
        }
        main();
        "#;
    assert_fails_spanning(source, "value", "cannot take '&mut' parameters");
}

#[test]
fn a_sync_function_with_view_parameters_called_from_async_compiles() {
    // Sync callees cannot suspend — views pass freely.
    assert_compiles(
        r#"
        async fun tick() {
            let _beat = 1;
        }
        fun bump(value: &mut i32) {
            value += 1;
        }
        async fun flow() {
            mut a = 5;
            bump(&mut a);
            await tick();
            bump(&mut a);
        }
        fun main() {
            flow();
        }
        main();
        "#,
    );
}

#[test]
fn an_async_closure_capturing_a_view_is_rejected() {
    let source = r#"
        async fun tick() {
            let _beat = 1;
        }
        fun main() {
            mut a = 5;
            let view = &mut a;
            let task = async {
                await tick();
                view += 1;
            };
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn an_await_with_no_live_views_compiles() {
    assert_compiles(
        r#"
        async fun tick() {
            let _beat = 1;
        }
        async fun clean() {
            mut a = [ 1 ];
            a.push(2);
            await tick();
            a.push(3);
        }
        fun main() {
            clean();
        }
        main();
        "#,
    );
}

// --- K2: the std math surface (proposal: backlog K2) ---

#[test]
fn math_constants_and_moved_free_functions_import() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::math::{ PI, TAU, E, EPSILON, min, max, minmax };

        fun main() {
            print(PI);
            print(TAU == PI * 2f);
            print(E > 2.7f && E < 2.8f);
            print(EPSILON > 0f);
            print(min(3, 9));
            print(max(3, 9));
            let (low, high) = minmax(9, 3);
            print(low);
            print(high);
        }
        main();
        "#,
        "3.141592653589793\ntrue\ntrue\ntrue\n3\n9\n3\n9\n",
    );
}

#[test]
fn f64_float_classification_predicates() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::math::{ NAN, INFINITY };

        fun main() {
            print(NAN.is_nan());
            print(1.5f.is_nan());
            print(1.5f.is_finite());
            print(INFINITY.is_finite());
            print(INFINITY.is_infinite());
            print(NAN.is_infinite());
        }
        main();
        "#,
        "true\nfalse\ntrue\nfalse\ntrue\nfalse\n",
    );
}

#[test]
fn rem_is_truncated_remainder_across_the_families() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            print(7.rem(3));
            print((0 - 7).rem(3));
            print(7.5f.rem(2f));
            print(250u8.rem(7u8));
            print(9i53.rem(4i53));
        }
        main();
        "#,
        "1\n-1\n1.5\n5\n1\n",
    );
}

#[test]
fn sized_types_carry_the_applicable_math_family() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            print((0i8 - 5i8).abs());
            print(3i16.pow(2i16));
            print(200u16.min(90u16));
            print(7u53.max(9u53));
            print(2f32.pow(3f32));
            print(2.25f32.sqrt());
        }
        main();
        "#,
        "5\n9\n90\n9\n8\n1.5\n",
    );
}

// --- K2 side-fix: conformance credits a SEPARATE impl of the declaring ---
// --- supertrait (impl X with Eq {} need not restate PartialEq's eq).   ---

#[test]
fn a_marker_impl_rides_a_separate_supertrait_impl() {
    assert_compiles(
        r#"
        trait Alike<B = Self> {
            fun same(self, b: B): bool;
        }
        trait Settled with Alike {}
        struct Coin { face: i32 }
        impl Coin with Alike {
            fun same(self, b: Coin): bool {
                self.face == b.face
            }
        }
        impl Coin with Settled {}
        fun main() {
            let _ok = Coin { face = 1 }.same(Coin { face = 1 });
        }
        main();
        "#,
    );
}

#[test]
fn a_missing_supertrait_member_still_errors() {
    let source = r#"
        trait Alike<B = Self> {
            fun same(self, b: B): bool;
        }
        trait Settled with Alike {}
        struct Coin { face: i32 }
        impl Coin with Settled {}
        fun main() {
            let _coin = Coin { face = 1 };
        }
        main();
        "#;
    let diagnostics = failure_diagnostics(source);
    assert!(
        diagnostics.iter().any(|(message, _)| message
            .contains("'Coin' does not implement trait 'Settled': missing 'same'")),
        "got: {diagnostics:#?}"
    );
}

#[test]
fn a_same_named_member_from_an_unrelated_trait_does_not_satisfy() {
    // `same` provided via an UNRELATED trait's impl must not satisfy
    // `Settled`'s inherited requirement.
    let source = r#"
        trait Alike<B = Self> {
            fun same(self, b: B): bool;
        }
        trait Settled with Alike {}
        trait Lookalike {
            fun same(self, b: Self): bool;
        }
        struct Coin { face: i32 }
        impl Coin with Lookalike {
            fun same(self, b: Coin): bool {
                self.face == b.face
            }
        }
        impl Coin with Settled {}
        fun main() {
            let _coin = Coin { face = 1 };
        }
        main();
        "#;
    assert_fails(source);
}

// --- reactive-turns §5.1: `get_safe` — the possibly-established context ---
// --- read (ambient-owner.md §2.1's sketch; turn_scope's prerequisite).  ---

#[test]
fn get_safe_yields_none_outside_and_some_inside_a_run() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;
        import std::option::Option::{ Some, None };

        let current: Context<i32> = Context::new();

        fun describe(): str {
            match current.get_safe() {
                Some(let value) => i"some {value}",
                None => "none",
            }
        }

        fun main() {
            print(describe());
            current.run(7, || {
                print(describe());
            });
            print(describe());
        }
        main();
        "#,
        "none\nsome 7\nnone\n",
    );
}

#[test]
fn get_safe_wraps_inside_a_strict_covered_region() {
    // A strict (get-reading) function calls a safe-only one: the boundary
    // Some-wraps the bare value.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;
        import std::option::Option::{ Some, None };

        let current: Context<i32> = Context::new();

        fun peek(): str {
            match current.get_safe() {
                Some(let value) => i"peeked {value}",
                None => "nothing",
            }
        }

        fun strict_report() {
            let value = current.get();
            print(i"strict {value}");
            print(peek());
        }

        fun main() {
            current.run(9, || {
                strict_report();
            });
        }
        main();
        "#,
        "strict 9\npeeked 9\n",
    );
}

#[test]
fn get_safe_threads_through_a_transitive_chain() {
    // The middle function neither reads nor runs — the Option threads
    // through it, Some on the covered path and None from the top level.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;
        import std::option::Option::{ Some, None };

        let current: Context<i32> = Context::new();

        fun leaf(): str {
            match current.get_safe() {
                Some(let value) => i"leaf {value}",
                None => "leaf none",
            }
        }

        fun middle(): str {
            leaf()
        }

        fun main() {
            print(middle());
            current.run(3, || {
                print(middle());
            });
        }
        main();
        "#,
        "leaf none\nleaf 3\n",
    );
}

#[test]
fn get_safe_survives_await_and_stored_closures() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;
        import std::option::Option::{ Some, None };

        let current: Context<i32> = Context::new();

        fun label(): str {
            match current.get_safe() {
                Some(let value) => i"got {value}",
                None => "got none",
            }
        }

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            mut stored: List<|| void> = [];
            current.run(5, || {
                let task = async {
                    await tick();
                    print(label());
                };
                stored.push(|| print(label()));
            });
            print(label());
            for callback in stored {
                callback();
            }
        }
        main();
        "#,
        "got none\ngot 5\ngot 5\n",
    );
}

#[test]
fn the_strict_fence_is_unchanged_by_get_safe() {
    // A strict `get` on an uncovered path still errors, even in a program
    // that also uses `get_safe`; and a get_safe-only function pulled onto a
    // strict chain is fenced like any strict code.
    let source = r#"
        import std::print;
        import std::context::Context;
        import std::option::Option::{ Some, None };

        let current: Context<i32> = Context::new();

        fun sneaky(): i32 {
            current.get()
        }

        fun probe(): str {
            match current.get_safe() {
                Some(let value) => i"some {value}",
                None => "none",
            }
        }

        fun main() {
            print(probe());
            print(sneaky());
        }
        main();
        "#;
    assert_fails_spanning(
        source,
        "current.get()",
        "can be reached without an enclosing `run`",
    );
}

// --- reactive-turns §5.2: turn-scoped flush — the isolation model. ---

#[test]
fn a_turn_flush_cannot_drain_another_turns_queue() {
    // The two-requests scenario, distilled: B's flush must not fire A's
    // pending notification.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Turn, turn_scope, flush };

        fun main() {
            let a = Signal::new(0);
            let _watch = a.sub(|value| print(i"a {value}"));
            let turn_a = Turn::new();
            let turn_b = Turn::new();
            turn_scope.run(turn_a, || {
                a.set(1);
            });
            turn_scope.run(turn_b, || flush());
            print("b flushed");
            turn_scope.run(turn_a, || flush());
        }
        main();
        "#,
        "a 0\nb flushed\na 1\n",
    );
}

#[test]
fn a_batch_body_defers_even_at_the_top_level() {
    // The batch body is INJECTED (created before the extent exists), so its
    // writes defer to batch's own fresh turn — the shipped batch semantics,
    // now per-extent instead of a global depth counter.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, batch };

        fun main() {
            let count = Signal::new(0);
            let _watch = count.sub(|value| print(i"seen {value}"));
            batch(|| {
                count.set(1);
                count.set(2);
                print("settling");
            });
        }
        main();
        "#,
        "seen 0\nsettling\nseen 2\n",
    );
}

#[test]
fn a_turn_follows_its_extents_continuation_across_await() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Turn, turn_scope, flush };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let count = Signal::new(0);
            let _watch = count.sub(|value| print(i"seen {value}"));
            let mine = Turn::new();
            turn_scope.run(mine, || {
                let task = async {
                    await tick();
                    count.set(7);
                    flush();
                };
            });
            print("sync done");
        }
        main();
        "#,
        "seen 0\nsync done\nseen 7\n",
    );
}

// --- reactive-turns §2: the UI event boundary mechanism — a host-invoked ---
// --- plain ADAPTER wraps each dispatch in a fresh turn; the clause-typed ---
// --- handler (a user literal, deferred) receives it at the call.        ---

#[test]
fn a_host_invoked_adapter_gives_each_dispatch_its_own_turn() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, FlushPolicy, turn, turn_scope };

        fun simulate_events(handler: (|| void) context turn_scope) {
            // The DOM stores only this plain closure; each invocation is a
            // boundary dispatch.
            let adapter = || turn(FlushPolicy::AtSuspension, || handler());
            adapter();
            adapter();
        }

        fun main() {
            let count = Signal::new(0);
            let _watch = count.sub(|value| print(i"seen {value}"));
            simulate_events(|| {
                count.set(count.get() + 1);
                count.set(count.get() + 1);
                print("handler done");
            });
        }
        main();
        "#,
        "seen 0\nhandler done\nseen 2\nhandler done\nseen 4\n",
    );
}

#[test]
fn a_named_handler_binding_adopts_the_clause() {
    // `let add = || ..; take(add)` — the unannotated closure binding passed
    // into a clause position adopts it: the literal defers (receiving each
    // dispatch's turn), and DIRECT calls of the binding thread like any
    // injected call.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, FlushPolicy, turn, turn_scope };

        fun dispatch(handler: (|| void) context turn_scope) {
            turn(FlushPolicy::AtEnd, || handler());
        }

        fun main() {
            let count = Signal::new(0);
            let _watch = count.sub(|value| print(i"seen {value}"));
            let add = || {
                count.set(count.get() + 1);
                count.set(count.get() + 1);
            };
            dispatch(add);
            print("mid");
            turn(FlushPolicy::AtEnd, || add());
        }
        main();
        "#,
        "seen 0\nseen 2\nmid\nseen 4\n",
    );
}

#[test]
fn an_annotated_clause_binding_defers_and_forwards() {
    // The explicit spelling: a clause on the LET annotation. The binding
    // forwards into same-clause parameters and works as `run`'s body.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let current: Context<i32> = Context::new();

        fun invoke(body: (|| void) context current) {
            current.run(9, body);
        }

        fun main() {
            let report: (|| void) context current = || print(current.get());
            invoke(report);
            current.run(5, report);
        }
        main();
        "#,
        "9\n5\n",
    );
}

#[test]
fn a_non_closure_binding_in_a_clause_position_is_rejected() {
    let source = r#"
        import std::reactive::{ FlushPolicy, turn, turn_scope };

        fun dispatch(handler: (|| void) context turn_scope) {
            turn(FlushPolicy::AtEnd, || handler());
        }

        fun main() {
            let not_a_closure = 5;
            dispatch(not_a_closure);
        }
        main();
        "#;
    assert_fails(source);
}

#[test]
fn an_annotated_binding_with_a_non_literal_initializer_is_rejected() {
    let source = r#"
        import std::context::Context;

        let current: Context<i32> = Context::new();

        fun main() {
            let value = 5;
            let bad: (|| void) context current = value;
        }
        main();
        "#;
    assert_fails(source);
}

// --- reactive-turns: the suspension hook. A turn's async continuations ---
// --- must settle without manual flushes, and AtSuspension pre-flushes  ---
// --- at each await (the optimistic-paint cadence).                     ---

#[test]
fn a_continuation_set_settles_without_a_manual_flush() {
    // The silent-loss fix: after the extent's first suspension the turn is
    // SETTLED; a late enqueue drains itself instead of waiting forever.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, FlushPolicy, turn, turn_scope };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let count = Signal::new(0);
            let _watch = count.sub(|value| print(i"seen {value}"));
            turn(FlushPolicy::AtEnd, || {
                let task = async {
                    await tick();
                    count.set(7);
                };
            });
            print("sync done");
        }
        main();
        "#,
        "seen 0\nsync done\nseen 7\n",
    );
}

#[test]
fn at_suspension_flushes_before_each_await() {
    // The optimistic-paint cadence: writes made BEFORE an await are settled
    // at the suspension point (compiler-inserted, policy-gated), so the
    // first paint happens before the slow work.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, FlushPolicy, turn, turn_scope };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let status = Signal::new("idle");
            let _watch = status.sub(|value| print(i"status {value}"));
            turn(FlushPolicy::AtSuspension, || {
                let task = async {
                    status.set("saving");
                    await tick();
                    status.set("saved");
                };
            });
            print("sync done");
        }
        main();
        "#,
        "status idle\nstatus saving\nsync done\nstatus saved\n",
    );
}

#[test]
fn at_end_holds_writes_across_the_await_inside_the_extent() {
    // The transactional cadence: an AtEnd turn does NOT pre-flush at the
    // suspension — the pre-await write settles with the extent (here, the
    // sync drain at the body's first suspension boundary), not before it.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, FlushPolicy, turn, turn_scope };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let status = Signal::new("idle");
            let _watch = status.sub(|value| print(i"status {value}"));
            turn(FlushPolicy::AtEnd, || {
                let task = async {
                    status.set("working");
                    await tick();
                    status.set("done");
                };
                status.set("queued");
            });
            print("sync done");
        }
        main();
        "#,
        "status idle\nstatus queued\nsync done\nstatus done\n",
    );
}

// --- reactive-turns follow-ons: `turn_async` (the true held-across-await ---
// --- transaction) and the optimistic-write → reconcile lifecycle.        ---

#[test]
fn turn_async_holds_writes_until_the_body_completes() {
    // The transactional extent: NOTHING publishes during the body — not
    // before the await, not in continuations — and the single settle
    // coalesces same-signal writes to the final value ("working" never
    // fires).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, turn_async, turn_scope };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let status = Signal::new("idle");
            let _watch = status.sub(|value| print(i"status {value}"));
            turn_async(|| {
                status.set("working");
                tick();
                status.set("done");
            });
            print("after turn");
        }
        main();
        "#,
        "status idle\nstatus done\nafter turn\n",
    );
}

#[test]
fn turn_async_returns_the_body_value() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ turn_async, turn_scope };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let answer = turn_async(|| {
                tick();
                42
            });
            print(answer);
        }
        main();
        "#,
        "42\n",
    );
}

#[test]
fn optimistic_paints_then_reconciles_to_the_confirmed_value() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, optimistic };
        import std::result::Result::{ self, Ok, Err };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let label = Signal::new("saved v1");
            let _watch = label.sub(|value| print(i"label {value}"));
            let outcome = optimistic(label, "saving v2", || {
                tick();
                Ok("saved v2")
            });
            match outcome {
                Ok(let value) => print(i"ok {value}"),
                Err(let _e) => print("failed"),
            }
        }
        main();
        "#,
        "label saved v1\nlabel saving v2\nlabel saved v2\nok saved v2\n",
    );
}

#[test]
fn optimistic_rolls_back_on_failure() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, optimistic };
        import std::result::Result::{ self, Ok, Err };

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let label = Signal::new("saved v1");
            let _watch = label.sub(|value| print(i"label {value}"));
            let outcome: Result<str, str> = optimistic(label, "saving v2", || {
                tick();
                Err("offline")
            });
            match outcome {
                Ok(let _value) => print("ok"),
                Err(let error) => print(i"failed: {error}"),
            }
        }
        main();
        "#,
        "label saved v1\nlabel saving v2\nlabel saved v1\nfailed: offline\n",
    );
}

// --- backlog J2: `async || T` closure types — asyncness as a type-level ---
// --- contract, so indirect calls await implicitly like direct ones.     ---

#[test]
fn a_call_through_an_async_typed_parameter_awaits() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        async fun tick() {
            let _beat = 1;
        }

        fun run_job(job: async || i32): i32 {
            let value = job();
            print(i"got {value}");
            value
        }

        fun main() {
            let result = run_job(|| {
                tick();
                7
            });
            print(i"result {result}");
        }
        main();
        "#,
        "got 7\nresult 7\n",
    );
}

#[test]
fn a_sync_closure_into_an_async_parameter_is_fine() {
    // The safe direction: awaiting a plain value just resolves.
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun run_job(job: async || i32): i32 {
            job()
        }

        fun main() {
            print(run_job(|| 5));
        }
        main();
        "#,
        "5\n",
    );
}

#[test]
fn an_async_closure_into_a_plain_void_parameter_is_spawn_semantics() {
    // Fire-and-forget through a plain `|| void` parameter stays legal — the
    // UI handler / turn-body shape (continuations settle via the turn
    // machinery; no value is lied about).
    assert_compiles_and_runs(
        r#"
        import std::print;

        async fun tick() {
            let _beat = 1;
        }

        fun fire(callback: || void) {
            callback();
            print("fired");
        }

        fun main() {
            fire(|| {
                tick();
                print("later");
            });
            print("sync end");
        }
        main();
        "#,
        "fired\nsync end\nlater\n",
    );
}

#[test]
fn an_async_closure_into_a_plain_valued_parameter_is_rejected() {
    // The J2 divergence, killed: the result would be a promise typing as T.
    let source = r#"
        async fun tick() {
            let _beat = 1;
        }

        fun compute(producer: || i32): i32 {
            producer()
        }

        fun main() {
            let _x = compute(|| {
                tick();
                7
            });
        }
        main();
        "#;
    assert_fails_spanning(source, "producer", "async closure");
}

#[test]
fn an_async_closure_type_composes_with_a_context_clause() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::context::Context;

        let current: Context<i32> = Context::new();

        async fun tick() {
            let _beat = 1;
        }

        fun stage(body: (async || i32) context current): i32 {
            current.run(3, body)
        }

        fun main() {
            let doubled = stage(|| {
                tick();
                current.get() * 2
            });
            print(doubled);
        }
        main();
        "#,
        "6\n",
    );
}

#[test]
fn an_async_annotated_let_awaits_at_its_calls() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        async fun tick() {
            let _beat = 1;
        }

        fun main() {
            let job: async || i32 = || {
                tick();
                11
            };
            print(job());
        }
        main();
        "#,
        "11\n",
    );
}

// --- I4: subscript absence panics (checked subscripts) -----------------------
// `a[i]` — read, write, or `&mut a[i]` view mint — requires `0 <= i < a.len()`;
// a violation panics. Writes never create slots (growth is `push`); `get(i)`
// stays the total `Option` form. The check happens at use / at mint; a deref
// through an already-minted view is the dynamic rule-4 remainder (C2), not
// this item.

/// Compiles and runs `source`, asserting the run FAILS and its stderr mentions
/// `expected_in_stderr` — the shape of a runtime panic. (A compile failure also
/// arrives as `Err`, but its messages won't contain a panic string, so the
/// substring assert distinguishes the two.)
#[track_caller]
fn assert_run_panics(source: &str, expected_in_stderr: &str) {
    match compile_and_run(source) {
        Ok(stdout) => panic!(
            "expected a runtime panic mentioning {expected_in_stderr:?}, got a clean run: {stdout:?}"
        ),
        Err(errors) => {
            let combined = errors.join("\n");
            assert!(
                combined.contains(expected_in_stderr),
                "the failure does not mention {expected_in_stderr:?}:\n{combined}"
            );
        }
    }
}

#[test]
fn an_out_of_bounds_read_panics() {
    assert_run_panics(
        r#"
        import std::print;
        fun main() {
            mut xs: List<i32> = List::new();
            xs.push(10);
            xs.push(20);
            print(xs[5]);
        }
        main();
        "#,
        "index out of bounds: the length is 2 but the index is 5",
    );
}

#[test]
fn an_out_of_bounds_write_panics_rather_than_growing() {
    assert_run_panics(
        r#"
        fun main() {
            mut xs: List<i32> = List::new();
            xs.push(10);
            xs[3] = 9;
        }
        main();
        "#,
        "index out of bounds: the length is 1 but the index is 3",
    );
}

#[test]
fn a_negative_index_panics() {
    assert_run_panics(
        r#"
        import std::print;
        fun main() {
            mut xs: List<i32> = List::new();
            xs.push(10);
            let i = 0 - 1;
            print(xs[i]);
        }
        main();
        "#,
        "index out of bounds: the length is 1 but the index is -1",
    );
}

#[test]
fn an_out_of_bounds_view_mint_panics() {
    // The view never comes to exist: the panic fires at `&mut xs[4]`, before
    // `bump` is entered.
    assert_run_panics(
        r#"
        fun bump(slot: &mut i32) {
            slot = *slot + 1;
        }
        fun main() {
            mut xs: List<i32> = List::new();
            xs.push(10);
            bump(&mut xs[4]);
        }
        main();
        "#,
        "index out of bounds: the length is 1 but the index is 4",
    );
}

#[test]
fn an_empty_list_subscript_panics() {
    // view-invalidation.md §1's P1 case: the empty list, subscripted.
    assert_run_panics(
        r#"
        import std::print;
        fun main() {
            mut xs: List<i32> = List::new();
            print(xs[0]);
        }
        main();
        "#,
        "index out of bounds: the length is 0 but the index is 0",
    );
}

#[test]
fn in_bounds_subscripts_are_unchanged() {
    // Read, in-place write, and a scalar element view — the subscript.vl
    // shapes, asserted here so the checked emission can't regress them.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun bump(slot: &mut i32) {
            slot = *slot + 100;
        }
        fun main() {
            mut xs: List<i32> = List::new();
            xs.push(10);
            xs.push(20);
            print(xs[0] + xs[1]);
            xs[1] = 99;
            print(xs[1]);
            bump(&mut xs[0]);
            print(xs[0]);
        }
        main();
        "#,
        "30\n99\n110\n",
    );
}

#[test]
fn an_unused_binding_with_an_indexing_initializer_still_panics() {
    // An indexing expression is effectful (it can throw), so dropping the
    // unused binding must not drop the check.
    assert_run_panics(
        r#"
        import std::print;
        fun main() {
            mut xs: List<i32> = List::new();
            let _probe = xs[0];
            print("reached");
        }
        main();
        "#,
        "index out of bounds: the length is 0 but the index is 0",
    );
}

#[test]
fn list_get_stays_the_option_form() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        fun main() {
            mut xs: List<i32> = List::new();
            xs.push(10);
            match xs.get(5) {
                Some(let value) => print(value),
                None => print("none"),
            }
            match xs.get(0) {
                Some(let value) => print(value),
                None => print("none"),
            }
        }
        main();
        "#,
        "none\n10\n",
    );
}

#[test]
fn a_macro_time_out_of_bounds_subscript_fails_expansion() {
    // The macro interpreter enforces the same bounds; OOB at expansion time is
    // an expansion failure at the invocation, carrying the panic message.
    assert_fails_spanning(
        r#"
        [probe]
        struct Point {
            x: i32,
        }

        macro fun probe(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            let xs = [1, 2];
            let y = xs[5];
            source("")
        }

        fun main() {}

        main();
        "#,
        "probe",
        "index out of bounds",
    );
}

#[test]
fn an_ungrounded_element_type_gets_a_direct_message() {
    // `mut a = []; a[0]` — the element type never grounds. The old message was
    // circular ("cannot index List (only a `List` is indexable)"); it must say
    // what is actually missing.
    assert_fails_spanning(
        r#"
        fun main() {
            mut a = [];
            let x = a[0];
        }
        main();
        "#,
        "a[0]",
        "element type is never determined",
    );
}

// --- H4: triple-quoted strings ------------------------------------------------
// `"""` ... `"""` is a RAW multi-line string literal: the whitespace before
// the closing delimiter is the indentation prefix stripped from every line,
// the newlines adjoining the delimiters belong to the syntax, and no escape
// processing happens at all (util::trim_multiline_string pins the rules at
// unit level; these pin the pipeline).

#[test]
fn a_triple_quoted_string_trims_to_the_closing_indentation() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let text = """
                    line 1
                line 2

                  line 3
                    
                """;
            print(text);
        }
        main();
        "#,
        "    line 1\nline 2\n\n  line 3\n    \n",
    );
}

#[test]
fn a_triple_quoted_string_is_raw() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let text = """
                escapes \n and \t stay raw, {braces} too
                """;
            print(text);
        }
        main();
        "#,
        "escapes \\n and \\t stay raw, {braces} too\n",
    );
}

#[test]
fn an_empty_triple_quoted_string_is_empty() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let text = """
                """;
            print(text);
            print("after");
        }
        main();
        "#,
        "\nafter\n",
    );
}

#[test]
fn content_after_the_opening_quotes_is_an_error() {
    assert_fails_spanning(
        r#"
        fun main() {
            let x = """oops
                """;
        }
        main();
        "#,
        "oops",
        "nothing may follow the opening",
    );
}

#[test]
fn the_closing_quotes_must_be_alone_on_their_line() {
    assert_fails_spanning(
        r#"
        fun main() {
            let x = """
                alpha
                beta """;
        }
        main();
        "#,
        "                beta ",
        "alone on its line",
    );
}

#[test]
fn insufficient_indentation_is_an_error_naming_the_line() {
    assert_fails_spanning(
        r#"
        fun main() {
            let x = """
                properly_indented
              shallow
                """;
        }
        main();
        "#,
        "              shallow",
        "line 2 of the triple-quoted string is not indented",
    );
}

#[test]
fn a_macro_emits_source_from_a_triple_quoted_string() {
    // The worlds path: the macro interpreter receives the trimmed VALUE (the
    // transformer trims before emission), so generated source needs no
    // concatenation ceremony for its static skeleton.
    assert_compiles_and_runs(
        r#"
        import std::print;

        macro fun gen(item: Item): Source {
            import macro_std::source;
            import macro_std::meta::{ Item, Source };
            source("""
                fun answer(): i32 {
                    42
                }
                """)
        }

        [gen]
        struct Marker {}

        fun main() {
            print(answer());
        }
        main();
        "#,
        "42\n",
    );
}

// --- H5: the `%` remainder operator -------------------------------------------
// Truncated remainder (the dividend's sign), like Rust and JS agree on. Exact
// for every integer type (unlike `/`, `%` needs no trunc wrap: an integer
// remainder is always representable); BigInt for i53/u53; overloadable through
// `std::operators::Rem` like the arithmetic four.

#[test]
fn remainder_on_i32_follows_the_dividend_sign() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(7 % 3);
            print((0 - 7) % 3);
            print(7 % (0 - 3));
        }
        main();
        "#,
        "1\n-1\n1\n",
    );
}

#[test]
fn remainder_on_floats() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(7.5 % 2f);
        }
        main();
        "#,
        "1.5\n",
    );
}

#[test]
fn remainder_on_i53_is_exact() {
    // i53 is f64-repped (F2 profiled trunc over BigInt); `%` of two in-range
    // integers is exact with no wrap needed.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(9000000000000000i53 % 7i53);
        }
        main();
        "#,
        "5\n",
    );
}

#[test]
fn remainder_on_bigint_values() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(9007199254740993n % 4n);
        }
        main();
        "#,
        "1n\n",
    );
}

#[test]
fn u32_remainder_stays_unsigned() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(4000000000u32 % 7u32);
        }
        main();
        "#,
        "3\n",
    );
}

#[test]
fn remainder_binds_with_product() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            print(1 + 7 % 3);
            print(2 * 7 % 3);
            print(7 % 3 * 2);
        }
        main();
        "#,
        "2\n2\n2\n",
    );
}

#[test]
fn a_compound_remainder_assignment_works() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut x = 17;
            x %= 5;
            print(x);
        }
        main();
        "#,
        "2\n",
    );
}

#[test]
fn a_user_type_dispatches_through_the_rem_trait() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::operators::Rem;

        struct Meters {
            v: i32,
        }

        impl Meters with Rem {
            fun rem(self, b: Self): Self {
                Meters { v = self.v % b.v }
            }
        }

        fun main() {
            let left = Meters { v = 17 };
            let right = Meters { v = 5 };
            print((left % right).v);
        }
        main();
        "#,
        "2\n",
    );
}

// --- B16: methods on generic receivers actually check their arguments ---------
// The hole: `resolve_method_arg_check` reconciled arguments against the RAW
// parameter type — `Type::Generic(T)` reconciles with anything — never applying
// the call's receiver substitution. And an empty `[]` literal erased its
// element (zero-argument `List`), so pushes had no slot to ground. Every case
// below pins one shape of the class.

#[test]
fn an_annotated_lists_push_checks_its_argument() {
    assert_fails_spanning(
        r#"
        fun main() {
            mut a: List<i32> = List::new();
            a.push("text");
        }
        main();
        "#,
        "\"text\"",
        "Expected i32, but got str instead.",
    );
}

#[test]
fn a_second_push_conflicting_with_the_first_is_an_error() {
    assert_fails_spanning(
        r#"
        fun main() {
            mut a = List::new();
            a.push(10);
            a.push("text");
        }
        main();
        "#,
        "\"text\"",
        "Expected i32, but got str instead.",
    );
}

#[test]
fn an_empty_literal_pushed_two_incompatible_types_is_an_error() {
    // The motivating repro (examples/playground).
    assert_fails_spanning(
        r#"
        fun main() {
            mut a = [];
            a.push(10);
            a.push("some text");
        }
        main();
        "#,
        "\"some text\"",
        "Expected i32, but got str instead.",
    );
}

#[test]
fn an_empty_literals_element_grounds_from_a_push() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut a = [];
            a.push(10);
            print(a[0] + 1);
        }
        main();
        "#,
        "11\n",
    );
}

#[test]
fn a_push_grounds_reads_earlier_in_the_source() {
    // Inference is a fixpoint over the whole function, not a statement walk: a
    // later push types an earlier subscript. (The early read sits behind a
    // length guard — reading before pushing would be a correct I4 panic at
    // runtime; this pins TYPING order-independence.)
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut a = [];
            if a.len() > 0 {
                print(a[0] + 1);
            }
            a.push(10);
            print(a[0] + 1);
        }
        main();
        "#,
        "11\n",
    );
}

#[test]
fn a_generic_structs_method_checks_its_argument() {
    assert_fails_spanning(
        r#"
        struct Holder<T> {
            item: T,
        }

        impl Holder<type T> {
            fun replace(&mut self, value: T): void {
                self.item = value;
            }
        }

        fun main() {
            mut h = Holder { item = 1 };
            h.replace("text");
        }
        main();
        "#,
        "\"text\"",
        "Expected i32, but got str instead.",
    );
}

#[test]
fn a_maps_insert_checks_its_value() {
    assert_fails_spanning(
        r#"
        import std::map::Map;
        fun main() {
            mut m: Map<str, i32> = Map::new();
            m.insert("k", "not an int");
        }
        main();
        "#,
        "\"not an int\"",
        "Expected i32, but got str instead.",
    );
}

#[test]
fn a_never_grounded_list_new_subscript_errors() {
    // Same rule as the empty literal (the I4 diagnostic): reading an element
    // whose type never grounds is an error, not a silent `Unknown`.
    assert_fails_spanning(
        r#"
        fun main() {
            mut a = List::new();
            let first = a[0];
        }
        main();
        "#,
        "a[0]",
        "element type is never determined",
    );
}

#[test]
fn a_never_pushed_lists_len_stays_legal() {
    // The tolerance that must survive: methods that don't touch the element
    // type work on a never-grounded list.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut a = [];
            print(a.len());
        }
        main();
        "#,
        "0\n",
    );
}

#[test]
fn a_for_loop_over_a_grounded_literal_types_its_item() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut a = [];
            a.push(10);
            a.push(20);
            for item in a {
                print(item + 1);
            }
        }
        main();
        "#,
        "11\n21\n",
    );
}

#[test]
fn a_nonempty_literals_push_checks_its_argument() {
    assert_fails_spanning(
        r#"
        fun main() {
            mut a = [1, 2];
            a.push("text");
        }
        main();
        "#,
        "\"text\"",
        "Expected i32, but got str instead.",
    );
}

// --- G2: `const` — compile-time evaluation -------------------------------------
// `const` is a weak-precedence expression prefix: it captures the largest
// expression to its right within the bracket/comma context and evaluates it at
// compile time with the macro interpreter, serializing the plain-data result
// IN PLACE (proposal/const-eval.md). Free variables must be const-known;
// failures are spanned diagnostics; the LSP evaluates explicit consts and
// `vilan check` evaluates as `build` does.

/// Compiles `source` and asserts the emitted JS contains `needle` — the
/// serialized-literal check for const results.
#[track_caller]
fn assert_emits_containing(source: &str, needle: &str) {
    match compile(source) {
        Ok(js) => assert!(
            js.contains(needle),
            "emitted JS does not contain {needle:?}:\n{js}"
        ),
        Err(errors) => panic!("expected a clean compile, got: {errors:#?}"),
    }
}

#[test]
fn a_const_expression_folds_to_a_literal() {
    let source = r#"
        import std::print;
        fun main() {
            let a = const 1 + 2;
            print(a);
        }
        main();
        "#;
    assert_emits_containing(source, "= 3;");
    assert_compiles_and_runs(source, "3\n");
}

#[test]
fn const_captures_weakly_to_the_expression_end() {
    let source = r#"
        import std::print;
        fun main() {
            let a = const 1 + 2 * 3;
            print(a);
        }
        main();
        "#;
    assert_emits_containing(source, "= 7;");
    assert_compiles_and_runs(source, "7\n");
}

#[test]
fn parens_narrow_the_capture() {
    let source = r#"
        import std::print;
        fun runtime_part(): i32 {
            5
        }
        fun main() {
            let a = (const 2 * 3) + runtime_part();
            print(a);
        }
        main();
        "#;
    assert_emits_containing(source, "6 + ");
    assert_compiles_and_runs(source, "11\n");
}

#[test]
fn a_const_call_evaluates_through_functions() {
    let source = r#"
        import std::print;
        fun square(n: i32): i32 {
            n * n
        }
        fun main() {
            let a = const square(7);
            print(a);
        }
        main();
        "#;
    assert_emits_containing(source, "= 49;");
    assert_compiles_and_runs(source, "49\n");
}

#[test]
fn const_chains_through_const_known_bindings() {
    let source = r#"
        import std::print;
        fun main() {
            let x = const 5;
            let y = const x * 2;
            print(y);
        }
        main();
        "#;
    assert_emits_containing(source, "= 10;");
    assert_compiles_and_runs(source, "10\n");
}

#[test]
fn a_literal_initialized_binding_is_const_known() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            let x = 5;
            let y = const x + 1;
            print(y);
        }
        main();
        "#,
        "6\n",
    );
}

#[test]
fn a_module_level_const_serves_functions() {
    let source = r#"
        import std::print;
        fun doubled(): List<i32> {
            mut result: List<i32> = List::new();
            result.push(2);
            result.push(4);
            result
        }
        let TABLE = const doubled();
        fun main() {
            print(TABLE[0] + TABLE[1]);
        }
        main();
        "#;
    assert_emits_containing(source, "[ 2, 4 ]");
    assert_compiles_and_runs(source, "6\n");
}

#[test]
fn a_const_argument_stops_at_the_comma() {
    let source = r#"
        import std::print;
        fun show(a: i32, b: i32) {
            print(a + b);
        }
        fun main() {
            show(const 3 * 4, 1);
        }
        main();
        "#;
    assert_emits_containing(source, "(12,");
    assert_compiles_and_runs(source, "13\n");
}

#[test]
fn a_const_block_runs_statements_at_compile_time() {
    let source = r#"
        import std::print;
        fun main() {
            let a = const {
                let left = 2;
                let right = 3;
                left * right
            };
            print(a);
        }
        main();
        "#;
    assert_emits_containing(source, "= 6;");
    assert_compiles_and_runs(source, "6\n");
}

#[test]
fn mut_initialized_by_const_stays_runtime_mutable() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut cache = const 1 + 2;
            cache = cache + 1;
            print(cache);
        }
        main();
        "#,
        "4\n",
    );
}

#[test]
fn a_runtime_parameter_is_rejected_as_a_free_variable() {
    // The diagnostic spans the REFERENCE itself (the last `w` — the first is
    // the declaration).
    let source = r#"
        fun f(w: i32): i32 {
            const w + 1
        }
        fun main() {
            let _x = f(1);
        }
        main();
        "#;
    let reference = source.rfind('w').unwrap();
    let diagnostics = failure_diagnostics(source);
    assert!(
        diagnostics
            .iter()
            .any(|(message, range)| message.contains("runtime value")
                && *range == (reference..reference + 1)),
        "no precise-span diagnostic: {diagnostics:#?}"
    );
}

#[test]
fn a_mut_binding_is_not_const_known() {
    let source = r#"
        fun main() {
            mut q = 5;
            let y = const q + 1;
        }
        main();
        "#;
    let reference = source.rfind('q').unwrap();
    let diagnostics = failure_diagnostics(source);
    assert!(
        diagnostics
            .iter()
            .any(|(message, range)| message.contains("runtime value")
                && *range == (reference..reference + 1)),
        "no precise-span diagnostic: {diagnostics:#?}"
    );
}

#[test]
fn a_call_initialized_binding_is_not_const_known() {
    let source = r#"
        fun mk(): i32 {
            5
        }
        fun main() {
            let z = mk();
            let y = const z + 1;
        }
        main();
        "#;
    let reference = source.rfind('z').unwrap();
    let diagnostics = failure_diagnostics(source);
    assert!(
        diagnostics
            .iter()
            .any(|(message, range)| message.contains("runtime value")
                && *range == (reference..reference + 1)),
        "no precise-span diagnostic: {diagnostics:#?}"
    );
}

#[test]
fn a_panic_at_const_time_is_a_compile_error() {
    // The diagnostic spans the whole const expression (deep spans into the
    // failing subexpression are the recorded refinement).
    let diagnostics = failure_diagnostics(
        r#"
        fun main() {
            let a = const {
                mut xs: List<i32> = List::new();
                xs.push(1);
                xs[5]
            };
        }
        main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("const evaluation failed")
                && message.contains("index out of bounds")),
        "no const-panic diagnostic: {diagnostics:#?}"
    );
}

#[test]
fn a_capability_is_rejected_at_const_time() {
    assert_fails_spanning(
        r#"
        import std::random::range;
        fun main() {
            let a = const range(1, 6);
        }
        main();
        "#,
        "range(1, 6)",
        "not available",
    );
}

#[test]
fn a_closure_result_is_not_plain_data() {
    assert_fails_spanning(
        r#"
        fun main() {
            let f = const || 1;
        }
        main();
        "#,
        "|| 1",
        "plain data",
    );
}

#[test]
fn the_js_refugee_hint_names_the_idiom() {
    assert_fails_spanning(
        r#"
        fun main() {
            const x = 3;
        }
        main();
        "#,
        "const x = 3",
        "vilan has no const declarations",
    );
}

#[test]
fn bigint_and_float_results_serialize_faithfully() {
    let source = r#"
        import std::print;
        fun main() {
            let big = const 2n * 3n;
            let precise = const 0.1 + 0.2;
            print(big);
            print(precise);
        }
        main();
        "#;
    assert_emits_containing(source, "6n");
    assert_compiles_and_runs(source, "6n\n0.30000000000000004\n");
}

#[test]
fn struct_and_enum_results_serialize() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        struct Point {
            x: i32,
            y: i32,
        }
        fun main() {
            let p = const Point { x = 1, y = 2 };
            print(p.x + p.y);
            let o = const Some(5);
            match o {
                Some(let value) => print(value),
                None => print("none"),
            }
        }
        main();
        "#,
        "3\n5\n",
    );
}

#[test]
fn a_const_dependency_cycle_is_an_error() {
    assert_fails(
        r#"
        let a: i32 = const b + 1;
        let b: i32 = const a + 1;
        fun main() {}
        main();
        "#,
    );
}

#[test]
fn const_chains_through_computed_bindings() {
    // The dependency is itself a COMPUTED const (not a literal): `y`'s
    // mini-program declares `x` from the stored result, keyed by its
    // initializer expression.
    let source = r#"
        import std::print;
        fun square(n: i32): i32 {
            n * n
        }
        fun main() {
            let x = const square(3);
            let y = const x + 1;
            print(y);
        }
        main();
        "#;
    assert_emits_containing(source, "= 10;");
    assert_compiles_and_runs(source, "10\n");
}

// --- G2 slice 5: the asset channel + the const-only bit -----------------------
// `std::asset::emit(kind, line)` accumulates build assets during const
// evaluation (const-eval.md §3); the channel dedups by line and orders
// lexically. `emit` is const-ONLY (§2): a runtime call path errors at the
// boundary call site — the crossing from runtime code into emit-reaching
// territory.

/// The `(kind, line)` assets a program's const evaluation emitted.
fn collected_assets(source: &str) -> Vec<(String, String)> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let leaked: &'static str = Box::leak(source.into_boxed_str());
            let (program, errors) = analyze_source(
                leaked,
                &std_spec(),
                Path::new("."),
                Path::new("test.vl"),
                Some(Platform::default()),
                &Workspace::default(),
            );
            assert!(errors.is_empty(), "expected a clean analysis: {errors:#?}");
            program.map(|p| p.const_assets).unwrap_or_default()
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn a_const_emit_collects_assets() {
    let assets = collected_assets(
        r#"
        import std::asset::emit;
        fun rule(): i32 {
            emit("css", ".a{color:red}");
            emit("css", ".b{color:blue}");
            1
        }
        let _style = const rule();
        fun main() {}
        main();
        "#,
    );
    assert!(
        assets.contains(&("css".to_string(), ".a{color:red}".to_string())),
        "{assets:?}"
    );
    assert!(
        assets.contains(&("css".to_string(), ".b{color:blue}".to_string())),
        "{assets:?}"
    );
}

#[test]
fn assets_deduplicate_and_sort_lexically() {
    // Two consts emit overlapping lines and a media block; the assembled file
    // dedups and sorts — '.' < '@', so media rules take the LATER cascade
    // position they need (the CSS-soundness argument in assemble_assets).
    let assets = collected_assets(
        r#"
        import std::asset::emit;
        fun base(): i32 {
            emit("css", ".pA3{padding:1rem}");
            emit("css", "@media (min-width: 768px){.mX{padding:2rem}}");
            1
        }
        fun accent(): i32 {
            emit("css", ".pA3{padding:1rem}");
            emit("css", ".bC7{background:blue}");
            2
        }
        let _a = const base();
        let _b = const accent();
        fun main() {}
        main();
        "#,
    );
    let assembled = vilan_core::const_eval::assemble_assets(&assets);
    let css = assembled.get("css").expect("a css asset");
    assert_eq!(
        css,
        ".bC7{background:blue}\n.pA3{padding:1rem}\n@media (min-width: 768px){.mX{padding:2rem}}\n"
    );
}

#[test]
fn asset_kinds_stay_separate() {
    let assets = collected_assets(
        r#"
        import std::asset::emit;
        fun both(): i32 {
            emit("css", ".a{}");
            emit("txt", "hello");
            1
        }
        let _x = const both();
        fun main() {}
        main();
        "#,
    );
    let assembled = vilan_core::const_eval::assemble_assets(&assets);
    assert_eq!(assembled.get("css").map(String::as_str), Some(".a{}\n"));
    assert_eq!(assembled.get("txt").map(String::as_str), Some("hello\n"));
}

#[test]
fn a_runtime_emit_is_rejected() {
    assert_fails_spanning(
        r#"
        import std::asset::emit;
        fun main() {
            emit("css", ".a{}");
        }
        main();
        "#,
        r#"emit("css", ".a{}")"#,
        "compile-time-only",
    );
}

#[test]
fn a_runtime_call_reaching_emit_is_rejected_at_the_boundary() {
    // The error sits at main's CALL into emit-reaching territory — the
    // outermost runtime crossing — not at the emit inside `rule`. (rfind:
    // the declaration `fun rule():` also contains the snippet.)
    let source = r#"
        import std::asset::emit;
        fun rule(): i32 {
            emit("css", ".a{}");
            1
        }
        fun main() {
            let _x = rule();
        }
        main();
        "#;
    let call = source.rfind("rule()").unwrap();
    let diagnostics = failure_diagnostics(source);
    assert!(
        diagnostics
            .iter()
            .any(|(message, range)| message.contains("compile-time-only")
                && *range == (call..call + "rule()".len())),
        "no boundary diagnostic at the call: {diagnostics:#?}"
    );
}

#[test]
fn a_top_level_runtime_call_reaching_emit_is_rejected() {
    let source = r#"
        import std::asset::emit;
        fun rule(): i32 {
            emit("css", ".a{}");
            1
        }
        let _style = rule();
        fun main() {}
        main();
        "#;
    let call = source.rfind("rule()").unwrap();
    let diagnostics = failure_diagnostics(source);
    assert!(
        diagnostics
            .iter()
            .any(|(message, range)| message.contains("compile-time-only")
                && *range == (call..call + "rule()".len())),
        "no top-level boundary diagnostic: {diagnostics:#?}"
    );
}

#[test]
fn reaching_functions_inside_const_are_fine() {
    // The styling shape: property functions bottom out in emit, called from
    // const chains — legal, and the assets flow.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::asset::emit;
        fun padding(): i32 {
            emit("css", ".pA3{padding:1rem}");
            4
        }
        fun main() {
            let width = const padding() * 2;
            print(width);
        }
        main();
        "#,
        "8\n",
    );
}

// --- A8: std::style — typed atomic styles, compiled ---------------------------
// The styling system riding const evaluation and the asset channel
// (proposal/ui-styling.md): builder-chain construction inside `const`, atomic
// rules with content-hashed class names, per-property last-wins merge,
// var-carried theme tokens, condition combinators.

#[test]
fn a_style_emits_atomic_rules_and_theme_vars() {
    let assets = collected_assets(
        r#"
        import std::style::{ style, space, Style };
        fun card(): Style {
            style().padding(space(4))
        }
        let _card = const card();
        fun main() {}
        main();
        "#,
    );
    assert!(
        assets.contains(&(
            "css".to_string(),
            ".s1ufvr2{padding:var(--space-4)}".to_string()
        )),
        "{assets:?}"
    );
    assert!(
        assets.contains(&("css".to_string(), ":root{--space-4:1rem}".to_string())),
        "{assets:?}"
    );
}

#[test]
fn last_wins_within_a_chain() {
    // Two paddings, one slot: the class list carries exactly one class — the
    // later one's.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::style::{ style, space, Style };
        fun padded(): Style {
            style().padding(space(4)).padding(space(6))
        }
        fun main() {
            let card = const padded();
            let classes = card.class_list();
            print(classes.contains(" "));
            let six = const style().padding(space(6));
            print(classes == six.class_list());
        }
        main();
        "#,
        "false\ntrue\n",
    );
}

#[test]
fn add_merges_per_property_right_wins() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::style::{ style, space, Style, Color };
        fun base(): Style {
            style().padding(space(4)).background(Color::gray(50))
        }
        fun accent(): Style {
            style().padding(space(6))
        }
        fun main() {
            let merged = const base() + accent();
            let expected = const style().padding(space(6)).background(Color::gray(50));
            print(merged.class_list().len() > 0);
            print(merged.class_list() == expected.class_list());
        }
        main();
        "#,
        "true\ntrue\n",
    );
}

#[test]
fn extend_with_override_is_a_property_method_on_a_style() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::style::{ style, space, Style };
        fun main() {
            let base = const style().padding(space(4));
            let bigger = const base.padding(space(6));
            let six = const style().padding(space(6));
            print(bigger.class_list() == six.class_list());
        }
        main();
        "#,
        "true\n",
    );
}

#[test]
fn hover_emits_a_pseudo_rule() {
    let assets = collected_assets(
        r#"
        import std::style::{ style, Style, Color };
        fun s(): Style {
            style().hover(style().background(Color::gray(100)))
        }
        let _s = const s();
        fun main() {}
        main();
        "#,
    );
    assert!(
        assets
            .iter()
            .any(|(_, line)| line.contains(":hover{background-color:var(--gray-100)}")),
        "{assets:?}"
    );
}

#[test]
fn breakpoints_wrap_media_and_stack_with_pseudo() {
    let assets = collected_assets(
        r#"
        import std::style::{ style, space, Style };
        fun s(): Style {
            style().md(style().hover(style().padding(space(6))))
        }
        let _s = const s();
        fun main() {}
        main();
        "#,
    );
    assert!(
        assets
            .iter()
            .any(|(_, line)| line.starts_with("@media (min-width: 768px){.")
                && line.contains(":hover{padding:var(--space-6)}")),
        "{assets:?}"
    );
}

#[test]
fn dark_prefixes_the_theme_selector() {
    let assets = collected_assets(
        r#"
        import std::style::{ style, Style, Color };
        fun s(): Style {
            style().dark(style().background(Color::gray(900)))
        }
        let _s = const s();
        fun main() {}
        main();
        "#,
    );
    assert!(
        assets
            .iter()
            .any(|(_, line)| line.starts_with(":root[data-theme=\"dark\"] .")),
        "{assets:?}"
    );
}

#[test]
fn an_unknown_scale_step_fails_the_build() {
    let diagnostics = failure_diagnostics(
        r#"
        import std::style::{ style, space, Style };
        fun s(): Style {
            style().padding(space(37))
        }
        let _s = const s();
        fun main() {}
        main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("unknown spacing step 37")),
        "{diagnostics:#?}"
    );
}

#[test]
fn an_unknown_ramp_step_fails_the_build() {
    let diagnostics = failure_diagnostics(
        r#"
        import std::style::{ style, Style, Color };
        fun s(): Style {
            style().background(Color::gray(55))
        }
        let _s = const s();
        fun main() {}
        main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("unknown gray step 55")),
        "{diagnostics:#?}"
    );
}

#[test]
fn runtime_style_construction_is_rejected() {
    let diagnostics = failure_diagnostics(
        r#"
        import std::style::{ style, space, Style };
        fun main() {
            let card = style().padding(space(4));
        }
        main();
        "#,
    );
    assert!(
        diagnostics
            .iter()
            .any(|(message, _)| message.contains("compile-time-only")),
        "{diagnostics:#?}"
    );
}

#[test]
fn length_units_render_their_css() {
    let assets = collected_assets(
        r#"
        import std::style::{ style, Style, Length };
        fun s(): Style {
            style()
                .width(Length::px(37))
                .height(Length::pct(50))
                .margin(Length::auto())
                .max_width(Length::var("--w"))
        }
        let _s = const s();
        fun main() {}
        main();
        "#,
    );
    let lines: Vec<&str> = assets.iter().map(|(_, line)| line.as_str()).collect();
    assert!(
        lines.iter().any(|l| l.contains("{width:37px}")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("{height:50%}")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("{margin:auto}")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("{max-width:var(--w)}")),
        "{lines:?}"
    );
}

#[test]
fn identical_rules_deduplicate_across_styles() {
    let assets = collected_assets(
        r#"
        import std::style::{ style, space, Style };
        fun a(): Style {
            style().padding(space(4))
        }
        fun b(): Style {
            style().padding(space(4))
        }
        let _a = const a();
        let _b = const b();
        fun main() {}
        main();
        "#,
    );
    let assembled = vilan_core::const_eval::assemble_assets(&assets);
    let css = assembled.get("css").expect("css");
    assert_eq!(
        css.matches(".s1ufvr2{padding:var(--space-4)}").count(),
        1,
        "{css}"
    );
}

// --- K3: std::crypto / std::jwt / std::base64 (Kolt migration) ---------------
// WebCrypto-backed auth primitives. HMAC/PBKDF2 run against the host
// crypto.subtle (present in node), so these are assert_compiles_and_runs; the
// vectors are RFC-checked (HMAC-SHA-512 = RFC 4231 #2). base64url and
// constant-time compare are pure vilan.

#[test]
fn base64url_round_trips_every_tail_length() {
    // 0, 1, 2 leftover bytes each exercise a distinct decode tail.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::base64::{ encode_url, decode_url };
        import std::bytes::{ encode_utf8, decode_utf8 };
        import std::option::Option::{ self, Some, None };
        fun show(text: str) {
            let encoded = encode_url(encode_utf8(text));
            match decode_url(encoded) {
                Some(let bytes) => print(decode_utf8(bytes)),
                None => print("decode failed"),
            }
        }
        fun main() {
            show("abc");
            show("ab");
            show("a");
            show("hello, world");
        }
        main();
        "#,
        "abc\nab\na\nhello, world\n",
    );
}

#[test]
fn hmac_sha512_matches_the_rfc_vector() {
    // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?".
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::crypto::hmac_sha512;
        import std::bytes::encode_utf8;
        async fun main() {
            let mac = hmac_sha512(encode_utf8("Jefe"), encode_utf8("what do ya want for nothing?"));
            print(mac.to_hex());
        }
        main();
        "#,
        "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737\n",
    );
}

#[test]
fn a_jwt_round_trips_signs_and_verifies() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::jwt::{ sign_hs512, verify_hs512 };
        import std::bytes::encode_utf8;
        import std::option::Option::{ self, Some, None };
        import std::wire::Wire;

        [derive(Wire)]
        struct Claims {
            sub: str,
            admin: bool,
        }

        async fun main() {
            let secret = encode_utf8("top-secret");
            let token = sign_hs512(secret, Claims { sub = "user-42", admin = true });
            print(token.split(".").len());
            let ok: Option<Claims> = verify_hs512(secret, token);
            match ok {
                Some(let claims) => print(i"{claims.sub} {claims.admin}"),
                None => print("verify failed"),
            }
        }
        main();
        "#,
        "3\nuser-42 true\n",
    );
}

#[test]
fn a_tampered_or_wrong_key_jwt_is_rejected() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::jwt::{ sign_hs512, verify_hs512 };
        import std::bytes::encode_utf8;
        import std::option::Option::{ self, Some, None };
        import std::wire::Wire;

        [derive(Wire)]
        struct Claims {
            sub: str,
        }

        fun outcome(label: str, result: Option<Claims>) {
            match result {
                Some(let _c) => print(i"{label}: ACCEPTED"),
                None => print(i"{label}: rejected"),
            }
        }

        async fun main() {
            let secret = encode_utf8("top-secret");
            let token = sign_hs512(secret, Claims { sub = "user-42" });
            let tampered: Option<Claims> = verify_hs512(secret, token + "x");
            outcome("tampered", tampered);
            let wrong: Option<Claims> = verify_hs512(encode_utf8("other-key"), token);
            outcome("wrong-key", wrong);
        }
        main();
        "#,
        "tampered: rejected\nwrong-key: rejected\n",
    );
}

#[test]
fn constant_time_equality_is_correct() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::crypto::equals_constant_time;
        import std::bytes::encode_utf8;
        fun main() {
            print(equals_constant_time(encode_utf8("abcd"), encode_utf8("abcd")));
            print(equals_constant_time(encode_utf8("abcd"), encode_utf8("abce")));
            print(equals_constant_time(encode_utf8("abcd"), encode_utf8("abc")));
        }
        main();
        "#,
        "true\nfalse\nfalse\n",
    );
}

#[test]
fn a_generic_call_in_an_else_branch_binds_its_type_argument() {
    // B17 (FIXED): the root cause was structural, not async — the `if`
    // inference arm propagated the expected-type constraint only into the
    // `then` branch, so a generic call reached only through an `else`
    // (here `dec<C>` in a nested-then inside the outer `else`) never received
    // `Option<C>` and left `C` unbound, miscompiling the `Wire` deserialize
    // to its abstract body. The await in the discovering case was incidental.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ encode_json, decode_json };
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::wire::Wire;

        [derive(Wire)]
        struct P { v: str }

        fun dec<C: Wire>(json: str): Option<C> {
            let decoded: Result<C, str> = decode_json(json);
            match decoded {
                Ok(let c) => Some(c),
                Err(let _e) => None,
            }
        }

        fun f<C: Wire>(json: str): Option<C> {
            if json.len() == 0 {
                None
            } else {
                if json.len() > 0 { dec(json) } else { None }
            }
        }

        fun main() {
            let json = encode_json(P { v = "hi" });
            let back: Option<P> = f(json);
            match back {
                Some(let c) => print(c.v),
                None => print("none"),
            }
        }
        main();
        "#,
        "hi\n",
    );
}

#[test]
fn a_generic_call_in_a_match_arm_binds_its_type_argument() {
    // The second half of B17: a `match` reads its expectation from the
    // `expected_types` channel, which the constraint parameter alone doesn't
    // feed — so a generic call in a match arm reached through a branch needs
    // the expectation seeded there too. This is the exact std::jwt shape:
    // if -> else -> match Some-arm -> if then -> generic decode.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ encode_json, decode_json };
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::wire::Wire;

        [derive(Wire)]
        struct P { v: str }

        fun dec<C: Wire>(json: str): Option<C> {
            let decoded: Result<C, str> = decode_json(json);
            match decoded {
                Ok(let c) => Some(c),
                Err(let _e) => None,
            }
        }

        fun f<C: Wire>(json: str): Option<C> {
            if json.len() == 0 {
                None
            } else {
                match Some(json) {
                    Some(let inner) => {
                        if inner.len() > 0 { dec(inner) } else { None }
                    },
                    None => None,
                }
            }
        }

        fun main() {
            let json = encode_json(P { v = "hi" });
            let back: Option<P> = f(json);
            match back {
                Some(let c) => print(c.v),
                None => print("none"),
            }
        }
        main();
        "#,
        "hi\n",
    );
}

#[test]
fn a_generic_call_after_a_branch_nested_await_monomorphizes() {
    // The exact shape jwt.vl had to be restructured around (the async form of
    // the same B17 else-branch bug).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::{ encode_json, decode_json };
        import std::crypto::hmac_sha512;
        import std::bytes::{ Bytes, encode_utf8 };
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };
        import std::wire::Wire;

        [derive(Wire)]
        struct P { v: str }

        fun dec<C: Wire>(json: str): Option<C> {
            let decoded: Result<C, str> = decode_json(json);
            match decoded {
                Ok(let c) => Some(c),
                Err(let _e) => None,
            }
        }

        async fun f<C: Wire>(secret: Bytes, json: str): Option<C> {
            if json.len() == 0 {
                None
            } else {
                let _mac = hmac_sha512(secret, encode_utf8(json));
                if json.len() > 0 { dec(json) } else { None }
            }
        }

        async fun main() {
            let json = encode_json(P { v = "hi" });
            let back: Option<P> = f(encode_utf8("k"), json);
            match back {
                Some(let c) => print(c.v),
                None => print("none"),
            }
        }
        main();
        "#,
        "hi\n",
    );
}

// --- K4: std::db — SQLite over node:sqlite (Kolt migration) ------------------
// The server-only storage seam: `node:sqlite`'s DatabaseSync through the new
// module-qualified `[extern(new, "module", "Class")]` binding form, with
// `__db_*` helpers for parameter spreads and column reads. Runs against the
// real host database (node ships it built in).

#[test]
fn a_database_round_trips_inserts_and_queries() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::db::{ Database, Statement, Row };
        import std::option::Option::{ self, Some, None };
        fun main() {
            let db = Database::open(":memory:");
            db.exec("CREATE TABLE account (id INTEGER PRIMARY KEY, username TEXT, age INTEGER)");
            let insert = db.prepare("INSERT INTO account (username, age) VALUES (?, ?)");
            print(insert.run(["reed", 30]));
            print(insert.run(["ada", 36]));
            let by_name = db.prepare("SELECT id, username, age FROM account WHERE username = ?");
            match by_name.first(["ada"]) {
                Some(let row) => print(i"{row.text("username")} is {row.integer("age")}"),
                None => print("not found"),
            }
            match by_name.first(["nobody"]) {
                Some(let _row) => print("ghost"),
                None => print("none"),
            }
            let names = db.prepare("SELECT username FROM account ORDER BY id").all([]);
            for row in names {
                print(row.text("username"));
            }
        }
        main();
        "#,
        "1\n2\nada is 36\nnone\nreed\nada\n",
    );
}

#[test]
fn null_columns_are_detectable() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::db::{ Database, Row };
        import std::option::Option::{ self, Some, None };
        fun main() {
            let db = Database::open(":memory:");
            db.exec("CREATE TABLE t (name TEXT, note TEXT)");
            db.prepare("INSERT INTO t (name, note) VALUES (?, NULL)").run(["only-name"]);
            match db.prepare("SELECT name, note FROM t").first([]) {
                Some(let row) => {
                    print(row.is_null("note"));
                    print(row.is_null("name"));
                },
                None => print("empty"),
            }
        }
        main();
        "#,
        "true\nfalse\n",
    );
}

// --- A11 / pilot: web storage + the method-call-result-call parse gap --------

#[test]
fn calling_a_method_call_result_binds_first() {
    // The pilot's KoltStore stored server hooks as `Shared<|..| R>` and called
    // them; `self.hook.read()(args)` — calling a METHOD-call result directly —
    // does not parse (B-note), but binding the result first does. This pins the
    // working shape; the direct form is the ignored pin below.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;
        struct Holder { hook: Shared<|str| i32> }
        impl Holder {
            fun call_it(self, a: str): i32 {
                let hook = self.hook.read();
                hook(a)
            }
        }
        fun main() {
            let h = Holder { hook = Shared::new(|a: str| a.len()) };
            print(h.call_it("abcd"));
        }
        main();
        "#,
        "4\n",
    );
}

#[test]
fn calling_a_method_call_result_directly_parses() {
    // Fixed with the direct-call postfix (backlog §H.18): a member fuses at
    // most one call, so a second `(args)` calls the RESULT.
    assert_compiles(
        r#"
        import std::shared::Shared;
        struct Holder { hook: Shared<|str| i32> }
        impl Holder {
            fun call_it(self, a: str): i32 {
                self.hook.read()(a)
            }
        }
        fun main() {
            let holder = Holder { hook = Shared::new(|text: str| text.len()) };
            let _n = holder.call_it("hi");
        }
        "#,
    );
}

// --- A10: `std::router` + `View.swap` (proposal/router.md) -------------------
//
// The runtime semantics (interception, pushState/popstate, dedupe, disposal)
// are pinned end-to-end in `crates/vilan-cli/tests/router.rs` under a DOM
// stub; these pin the compile-level surface.

#[test]
fn swap_renders_a_dynamic_subtree_per_route_value() {
    // The canonical router shape: nested route enums, a hand-written
    // parse/href pair, `link` through the app's `Routable` impl, and a `swap`
    // whose render closure matches the (unannotated) route value.
    assert_compiles_browser(
        r#"
        import std::ui::{ View, view, mount_root };
        import std::reactive::Signal;
        import std::router::{ current_path, navigate, segments, link, Routable };

        [derive(PartialEq)]
        enum Route {
            Home,
            Workspace(str, WorkspaceRoute),
        }

        [derive(PartialEq)]
        enum WorkspaceRoute {
            Overview,
            Task(i32),
        }

        fun parse(path: str): Route {
            let parts = segments(path);
            if parts.len() == 0 {
                Route::Home
            } else {
                Route::Workspace(parts[0], WorkspaceRoute::Overview)
            }
        }

        fun href(route: Route): str {
            match route {
                Route::Home => "/",
                Route::Workspace(let org, let _inner) => i"/w/{org}",
            }
        }

        impl Route with Routable {
            fun to_path(self): str {
                href(self)
            }
        }

        fun workspace_layout(org: str, inner: WorkspaceRoute): View {
            view("section").child(view("aside").text(org)).child(match inner {
                WorkspaceRoute::Overview => view("div").text("overview"),
                WorkspaceRoute::Task(let id) => view("div").text(i"task {id}"),
            })
        }

        fun main() {
            let route = current_path().map(parse);
            let _root = mount_root("app", || view("main")
                .child(link("Home", Route::Home))
                .child(view("button").on("click", || navigate(href(Route::Home))))
                .swap(route, |current| match current {
                    Route::Home => view("section").text("home"),
                    Route::Workspace(let org, let inner) => workspace_layout(org, inner),
                }));
        }
        "#,
    );
}

// --- B6: closure-return element inference (CLOSED) ---------------------------
//
// `xs.map(|p| p.x)` once typed as `List<unknown>`: `map` bound its result
// generic `U` from the closure's return while the body's field accessor was
// still in-flight. A first general fix deadlocked the slot case and was
// reverted; the B19 defer machinery (plus this window's binder work) closed
// the family for real. These pins hold every recorded shape — this area has
// regressed before, so each case stands on its own.

#[test]
fn a_field_mapped_element_types_without_annotation() {
    // The headline case: `U` comes only from the closure's `p.name`, and the
    // element must be concrete enough to dispatch `len()`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Point { x: i32, name: str }
        fun main() {
            let points = [Point { x = 1, name = "ab" }];
            let names = points.map(|p| p.name);
            print(names[0].len());
        }
        "#,
        "2\n",
    );
}

#[test]
fn a_field_mapped_element_meets_an_annotated_expectation() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Point { x: i32, name: str }
        fun main() {
            let points = [Point { x = 1, name = "abc" }];
            let names: List<str> = points.map(|p| p.name);
            print(names[0].len());
        }
        "#,
        "3\n",
    );
}

#[test]
fn a_field_mapped_result_chains_immediately() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Point { x: i32, name: str }
        fun main() {
            let points = [Point { x = 1, name = "ab" }];
            print(points.map(|p| p.name)[0].len());
        }
        "#,
        "2\n",
    );
}

#[test]
fn mapped_maps_thread_the_element_type() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Point { x: i32, name: str }
        fun main() {
            let points = [Point { x = 1, name = "abc" }];
            let lens = points.map(|p| p.name).map(|s| s.len());
            print(lens[0]);
        }
        "#,
        "3\n",
    );
}

#[test]
fn a_nested_accessor_closure_return_grounds() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Inner { v: i32 }
        struct Point { inner: Inner }
        fun main() {
            let points = [Point { inner = Inner { v = 41 } }];
            let vs = points.map(|p| p.inner.v);
            print(vs[0] + 1);
        }
        "#,
        "42\n",
    );
}

#[test]
fn a_struct_element_map_dispatches_members_downstream() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Point { x: i32, name: str }
        fun main() {
            let points = [Point { x = 1, name = "ab" }];
            let same = points.map(|p| p);
            print(same.map(|q| q.name)[0].len());
        }
        "#,
        "2\n",
    );
}

#[test]
fn a_slot_grounded_list_maps_a_field_closure() {
    // The combination the reverted general fix deadlocked on: the element
    // type comes from a `push`-grounded slot AND the map's `U` comes from a
    // field-access closure return. Both resolutions must be observable to
    // the constraint wake.
    assert_compiles_and_runs(
        r#"
        import std::print;
        struct Point { x: i32, name: str }
        fun main() {
            mut ps = List::new();
            ps.push(Point { x = 1, name = "abcd" });
            let names = ps.map(|p| p.name);
            print(names[0].len());
        }
        "#,
        "4\n",
    );
}

#[test]
fn a_slot_grounded_list_maps_and_sums() {
    // The exact deadlock reproducer from the reverted attempt.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut xs = List::new();
            xs.push(1);
            let s = xs.map(|n| n + 1).sum();
            print(s);
        }
        "#,
        "2\n",
    );
}

#[test]
fn a_mapped_signal_meets_a_bound_without_annotation() {
    // B19 (FIXED): `current_path().map(..)` yields `Signal<U = Route>`;
    // passing it to `swap<T: PartialEq>` without annotating the intermediate
    // binding must check the bound against the RESOLVED `Route`, not demand
    // `U: PartialEq`. The method resolution now DEFERS while a closure
    // argument's body is untyped, so `U` binds from the closure's return on
    // the retry instead of freezing abstract.
    assert_compiles_browser(
        r#"
        import std::ui::{ View, view, mount_root };
        import std::reactive::Signal;
        import std::router::{ current_path, segments };

        [derive(PartialEq)]
        enum Route {
            Home,
            Other,
        }

        fun parse(path: str): Route {
            if segments(path).len() == 0 { Route::Home } else { Route::Other }
        }

        fun main() {
            let route = current_path().map(|path| parse(path));
            let _root = mount_root("app", || view("main")
                .swap(route, |current| match current {
                    Route::Home => view("section").text("home"),
                    Route::Other => view("section").text("other"),
                }));
        }
        "#,
    );
}

#[test]
fn swap_requires_a_comparable_value() {
    // `swap<T: PartialEq>` — the dedupe needs `==`, so a source over a struct
    // without the impl is rejected at the call.
    assert_fails_browser_with(
        r#"
        import std::ui::{ View, view, mount_root };
        import std::reactive::Signal;

        struct Opaque {
            tag: str,
        }

        fun main() {
            let source: Signal<Opaque> = Signal::new(Opaque { tag = "a" });
            let _root = mount_root("app", || view("main")
                .swap(source, |current| view("p").text(current.tag)));
        }
        "#,
        "does not implement trait 'PartialEq'",
    );
}

#[test]
fn swap_boundaries_nest() {
    // A swap inside another swap's render closure — each level is its own
    // disposal boundary, and the inner render's owner registration must
    // resolve under the outer's injected extent.
    assert_compiles_browser(
        r#"
        import std::ui::{ View, view, mount_root };
        import std::reactive::Signal;

        fun main() {
            let outer: Signal<i32> = Signal::new(0);
            let inner: Signal<str> = Signal::new("a");
            let _root = mount_root("app", || view("main")
                .swap(outer, |level| view("section")
                    .child(view("h1").text(i"level {level}"))
                    .swap(inner, |name| view("p").text(name))));
        }
        "#,
    );
}

#[test]
fn swap_composes_with_sibling_bindings() {
    // `swap` alongside `bind_each` and `show` on one element tree — the mixed
    // form: three boundary kinds registering into the same enclosing owner.
    assert_compiles_browser(
        r#"
        import std::ui::{ View, view, mount_root };
        import std::reactive::Signal;

        fun main() {
            let page: Signal<i32> = Signal::new(0);
            let items: Signal<List<str>> = Signal::new(["a", "b"]);
            let visible: Signal<bool> = Signal::new(true);
            let _root = mount_root("app", || view("main")
                .child(view("ul").bind_each(items, |item| item, |item| view("li").text(item)))
                .child(view("aside").show(visible))
                .swap(page, |current| view("section").text(i"page {current}")));
        }
        "#,
    );
}

#[test]
fn on_event_hands_the_handler_the_dom_event() {
    // `View.on_event` — the handler receives a typed `Event` and can consult
    // modifier/key state and cancel the default action.
    assert_compiles_browser(
        r#"
        import std::ui::{ View, view, mount_root };
        import std::dom::Event;

        fun main() {
            let _root = mount_root("app", || view("input")
                .on_event("keydown", |event| {
                    if event.key() == "Enter" && !event.shift_key() && event.button() == 0 {
                        event.prevent_default();
                    }
                }));
        }
        "#,
    );
}

#[test]
fn link_accepts_any_routable_and_chains() {
    // `link<R: Routable>` dispatches `to_path` through the bound, and the
    // returned `View` chains like any other.
    assert_compiles_browser(
        r#"
        import std::ui::{ View, view, mount_root };
        import std::router::{ link, Routable };

        [derive(PartialEq)]
        enum Route {
            Home,
            Item(i32),
        }

        impl Route with Routable {
            fun to_path(self): str {
                match self {
                    Route::Home => "/",
                    Route::Item(let id) => i"/item/{id}",
                }
            }
        }

        fun main() {
            let _root = mount_root("app", || view("nav")
                .child(link("Home", Route::Home).class("nav-item"))
                .child(link("First", Route::Item(1))));
        }
        "#,
    );
}

#[test]
fn platform_requirement_flows_through_trait_dispatch() {
    // A bounded method call can't name one callee pre-monomorphization, so the
    // walk descends into every CANDIDATE (async_infer's rule): a browser build
    // reaching `save_it` is charged for the @process impl.
    assert_fails_browser_with(
        r#"
        import std::fs::write_file;

        trait Save {
            fun save(self): bool;
        }

        struct DiskStore { path: str }

        impl DiskStore with Save {
            fun save(self): bool {
                write_file(self.path, "state");
                true
            }
        }

        fun save_it<T: Save>(store: T): bool {
            store.save()
        }

        fun main() {
            save_it(DiskStore { path = "s.txt" });
        }
        "#,
        "requires the `process` layer of `std`",
    );
}

#[test]
fn a_closures_platform_charges_its_creator() {
    // The v1 creator rule: making the closure is the colored act — the body
    // is charged where the literal is created, whether or not it is called.
    assert_fails_browser_with(
        r#"
        import std::fs::write_file;

        fun make_saver(path: str): |str| void {
            |content: str| {
                write_file(path, content);
            }
        }

        fun main() {
            let _saver = make_saver("s.txt");
        }
        "#,
        "requires the `process` layer of `std`",
    );
}

#[test]
fn a_neutral_instantiation_is_admitted_despite_a_colored_impl() {
    // §3.2's refinement, landed: the walk threads each call's recorded
    // bindings, so `save_it(MemStore { .. })` descends only into
    // `MemStore`'s impl — `DiskStore`'s `@process` body no longer charges
    // an instantiation that never selects it.
    assert_compiles_browser(
        r#"
        import std::fs::write_file;

        trait Save {
            fun save(self): bool;
        }

        struct MemStore { last: str }
        struct DiskStore { path: str }

        impl MemStore with Save {
            fun save(self): bool { true }
        }

        impl DiskStore with Save {
            fun save(self): bool {
                write_file(self.path, "state");
                true
            }
        }

        fun save_it<T: Save>(store: T): bool {
            store.save()
        }

        fun main() {
            // Only the neutral impl is instantiated; the disk impl exists but
            // is never reached on this build.
            save_it(MemStore { last = "" });
        }
        "#,
    );
}

// --- §3.7: declared platform fences ------------------------------------------
//
// `[platform("…")]` declares the platforms a function promises to run on;
// the inferred requirement is checked against every matching host on EVERY
// compile — no entry needed, independent of the build target. Violations
// hang their chain from the fence.

#[test]
fn a_platform_fence_rejects_an_off_platform_reach() {
    // Checked on a NODE build (which itself admits `exists`) and with main
    // never calling the fenced function — the fence alone carries the check.
    assert_fails_spanning(
        r#"
        import std::fs::exists;

        [platform("browser")]
        fun probe_cache(): bool {
            exists("cache")
        }

        fun main() {}
        "#,
        r#"exists("cache")"#,
        "reachable from `probe_cache`, fenced `[platform(\"browser\")]`",
    );
}

#[test]
fn a_satisfied_fence_compiles_on_every_build_target() {
    let source = r#"
        import std::fs::exists;

        [platform("@process")]
        fun probe_cache(): bool {
            exists("cache")
        }

        fun main() {}
        "#;
    assert_compiles(source);
    assert_compiles_browser(source);
}

#[test]
fn a_neutral_fence_spanning_families_holds_for_base_code() {
    assert_compiles(
        r#"
        import std::print;

        [platform("@process", "browser")]
        fun shared_label(): str {
            "everywhere"
        }

        fun main() {
            print(shared_label());
        }
        "#,
    );
}

#[test]
fn an_unknown_fence_pattern_errors() {
    assert_fails(
        r#"
        [platform("wat")]
        fun probe(): i32 { 1 }

        fun main() {}
        "#,
    );
}

#[test]
fn a_fence_on_a_generic_promises_every_instantiation() {
    // Fences walk unbound, so dispatch considers every candidate: the
    // colored impl's existence alone breaks a browser fence on the generic —
    // deliberate conservatism (the fence promises for every possible T).
    assert_fails_browser_with(
        r#"
        import std::fs::exists;

        trait Check {
            fun check(self): bool;
        }

        struct DiskProbe { path: str }

        impl DiskProbe with Check {
            fun check(self): bool {
                exists(self.path)
            }
        }

        [platform("browser")]
        fun run_check<T: Check>(subject: T): bool {
            subject.check()
        }

        fun main() {}
        "#,
        "reachable from `run_check`, fenced `[platform(\"browser\")]`",
    );
}

#[test]
fn a_fence_on_a_method_checks_like_a_functions() {
    assert_fails_browser_with(
        r#"
        import std::fs::exists;

        struct Store { path: str }

        impl Store {
            [platform("browser")]
            fun probe(self): bool {
                exists(self.path)
            }
        }

        fun main() {}
        "#,
        "reachable from `probe`, fenced `[platform(\"browser\")]`",
    );
}

#[test]
fn a_colored_instantiation_still_rejects_beside_a_neutral_one() {
    // The refinement is not a hole: when the SAME generic is instantiated
    // both ways, the colored instantiation's path still rejects — chained
    // through the impl that instantiation actually selects.
    assert_fails_browser_with(
        r#"
        import std::fs::write_file;

        trait Save {
            fun save(self): bool;
        }

        struct MemStore { last: str }
        struct DiskStore { path: str }

        impl MemStore with Save {
            fun save(self): bool { true }
        }

        impl DiskStore with Save {
            fun save(self): bool {
                write_file(self.path, "state");
                true
            }
        }

        fun save_it<T: Save>(store: T): bool {
            store.save()
        }

        fun main() {
            save_it(MemStore { last = "" });
            save_it(DiskStore { path = "s.txt" });
        }
        "#,
        "reachable from the entry: main → save_it → save → write_file (std::fs)",
    );
}

#[test]
fn instantiation_bindings_compose_through_nested_generics() {
    // `route<T>` forwards to `commit<U>` — the binding threads two frames
    // deep, so the neutral instantiation stays admitted even though the
    // dispatch happens in the inner generic.
    assert_compiles_browser(
        r#"
        import std::fs::write_file;

        trait Save {
            fun save(self): bool;
        }

        struct MemStore { last: str }
        struct DiskStore { path: str }

        impl MemStore with Save {
            fun save(self): bool { true }
        }

        impl DiskStore with Save {
            fun save(self): bool {
                write_file(self.path, "state");
                true
            }
        }

        fun commit<U: Save>(store: U): bool {
            store.save()
        }

        fun route<T: Save>(store: T): bool {
            commit(store)
        }

        fun main() {
            route(MemStore { last = "" });
        }
        "#,
    );
}

#[test]
fn a_never_instantiated_impls_globals_leave_no_residue() {
    // The emission side moves with the refinement (emitted ⊆ admitted): a
    // binding referenced only by the impl no instantiation selects is
    // dropped, its callees — and their `node:` imports — with it.
    let source = r#"
        import std::fs::exists;

        trait Save {
            fun save(self): bool;
        }

        struct MemStore { last: str }
        struct DiskStore { path: str }

        let disk_ready = exists("state");

        impl MemStore with Save {
            fun save(self): bool { true }
        }

        impl DiskStore with Save {
            fun save(self): bool { disk_ready }
        }

        fun save_it<T: Save>(store: T): bool {
            store.save()
        }

        fun main() {
            save_it(MemStore { last = "" });
        }
        "#;
    let browser = compile_browser(source).expect("the neutral instantiation compiles");
    assert!(
        !browser.contains("node:") && !browser.contains("\"state\""),
        "the unselected impl's binding leaked into the bundle:\n{browser}"
    );
}

#[test]
fn the_router_is_browser_only() {
    // `std::router` lives in the browser layer. Under platform coloring the
    // import is fine — REACHING `navigate` from a node build's entry is the
    // violation, anchored at the user call site with the chain
    // (proposal/platform-coloring.md §3.6).
    assert_fails_spanning(
        r#"
        import std::router::navigate;

        fun main() {
            navigate("/home");
        }
        "#,
        r#"navigate("/home")"#,
        "requires the `browser` layer of `std` and cannot run on `node",
    );
}

// --- platform coloring: per-function requirement lines (hover's data) --------
//
// `platform_color::requirements` renders what the admission walk knows into an
// entry-independent per-function map — the language server appends these lines
// to hover (proposal/platform-coloring.md phase 2). The pins fix the exact
// vocabulary: the layer label, a SHORTEST via-chain, library frames labeled
// with their module, user frames bare.

#[test]
fn a_requirement_line_names_the_layer_and_the_via_chain() {
    let line = requirement_line_of(
        r#"
        import std::fs;

        fun save() {
            fs::write_file("state", "data");
        }

        fun main() {
            save();
        }
        "#,
        "save",
    )
    .expect("`save` reaches `std::fs` and should carry a requirement");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `write_file (std::fs)`)"
    );
}

#[test]
fn a_requirement_line_propagates_to_callers_growing_the_chain() {
    // `main` acquires the same label one hop later; its own frame is implicit,
    // the user frame `save` renders bare, the library frame keeps its module.
    let line = requirement_line_of(
        r#"
        import std::fs;

        fun save() {
            fs::write_file("state", "data");
        }

        fun main() {
            save();
        }
        "#,
        "main",
    )
    .expect("`main` reaches `std::fs` through `save`");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `save → write_file (std::fs)`)"
    );
}

#[test]
fn a_seeded_library_functions_line_has_no_chain() {
    // The std function itself is seeded at its definition site — its line is
    // the bare requirement, no `via`.
    let line = requirement_line_of(
        r#"
        import std::fs;

        fun main() {
            fs::write_file("state", "data");
        }
        "#,
        "write_file",
    )
    .expect("`write_file` is defined in the layer");
    assert_eq!(line, "requires the `process` layer of `std`");
}

#[test]
fn the_via_chain_is_a_shortest_path_to_the_layer() {
    // `main` reaches the layer both through `relay → save` and through `save`
    // directly; the witness chain takes the short way.
    let line = requirement_line_of(
        r#"
        import std::fs;

        fun save() {
            fs::write_file("state", "data");
        }

        fun relay() {
            save();
        }

        fun main() {
            relay();
            save();
        }
        "#,
        "main",
    )
    .expect("`main` reaches the layer");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `save → write_file (std::fs)`)"
    );
}

#[test]
fn a_created_closures_requirement_lands_on_its_creator_line() {
    // The v1 creator rule, rendered: the closure's body charges its creator,
    // and the chain shows the closure frame it traveled through.
    let line = requirement_line_of(
        r#"
        import std::fs::write_file;

        fun make_saver(path: str): |str| void {
            |content: str| {
                write_file(path, content);
            }
        }

        fun main() {
            let _saver = make_saver("s.txt");
        }
        "#,
        "make_saver",
    )
    .expect("`make_saver` creates the colored closure");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `closure → write_file (std::fs)`)"
    );
}

#[test]
fn a_dispatch_candidates_requirement_reaches_the_bounded_caller_line() {
    // Candidate descent (async_infer's rule): the bounded call charges the
    // colored impl's method, and the line says which one — even though this
    // node build ADMITS the layer (the map is platform-independent).
    let line = requirement_line_of(
        r#"
        import std::fs::write_file;

        trait Save {
            fun save(self): bool;
        }

        struct DiskStore { path: str }

        impl DiskStore with Save {
            fun save(self): bool {
                write_file(self.path, "state");
                true
            }
        }

        fun save_it<T: Save>(store: T): bool {
            store.save()
        }

        fun main() {
            save_it(DiskStore { path = "s.txt" });
        }
        "#,
        "save_it",
    )
    .expect("`save_it`'s bound admits the colored impl");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `save → write_file (std::fs)`)"
    );
}

#[test]
fn a_base_only_function_is_colorless() {
    assert_eq!(
        requirement_line_of(
            r#"
        import std::print;

        fun greet() {
            print("hi");
        }

        fun main() {
            greet();
        }
        "#,
            "greet",
        ),
        None
    );
}

#[test]
fn an_unreached_function_still_knows_its_requirement() {
    // Entry-independence: nothing calls `orphan`, but its line exists — the
    // fixpoint serves the editor, not just the entry walk.
    let line = requirement_line_of(
        r#"
        import std::fs;

        fun orphan() {
            fs::write_file("state", "data");
        }

        fun main() {}
        "#,
        "orphan",
    )
    .expect("`orphan` should be colored without being reachable");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `write_file (std::fs)`)"
    );
}

// --- platform coloring: module-level initializers ----------------------------
//
// A module-level binding's initializer runs iff something reachable
// references it (F6 — emission's rule), so a REFERENCE is an edge and the
// initializer's calls color like any body. Previously initializers were not
// graph nodes at all: a browser build could reference a binding whose
// initializer called `std::fs` and compile clean, shipping a load-time crash.

#[test]
fn a_module_initializers_call_colors_the_referencing_entry() {
    assert_fails_browser_with(
        r#"
        import std::fs::exists;

        let cache = exists("cache.txt");

        fun main() {
            let content = cache;
        }
        "#,
        "`exists` requires the `process` layer of `std` and cannot run on `browser`\n  reachable from the entry: main → cache → exists (std::fs)",
    );
}

#[test]
fn an_initializer_violation_anchors_at_the_initializer_call() {
    // The deepest user-code call site on the path is the initializer's own
    // call — the squiggle lands on the code that would run off-platform.
    // (Span-pinned on the node build via a browser-layer binding, the
    // `navigate` precedent.)
    assert_fails_spanning(
        r#"
        import std::storage::get;

        let token = get("notes-token");

        fun main() {
            let t = token;
        }
        "#,
        r#"get("notes-token")"#,
        "requires the `browser` layer of `std` and cannot run on `node",
    );
}

#[test]
fn an_initializer_reaching_a_user_function_colors_through_it() {
    assert_fails_browser_with(
        r#"
        import std::fs::exists;

        fun boot_check(): bool {
            exists("state")
        }

        let ready = boot_check();

        fun main() {
            let r = ready;
        }
        "#,
        "reachable from the entry: main → ready → boot_check → exists (std::fs)",
    );
}

#[test]
fn a_global_referencing_a_colored_global_chains_through_both() {
    assert_fails_browser_with(
        r#"
        import std::fs::exists;

        let raw = exists("data.txt");
        let copy = raw;

        fun main() {
            let c = copy;
        }
        "#,
        "reachable from the entry: main → copy → raw → exists (std::fs)",
    );
}

#[test]
fn a_global_closures_body_charges_the_binding_that_creates_it() {
    // The creator rule, at module level: the initializer creates the closure,
    // so referencing the binding is what admits (or rejects) the body.
    assert_fails_browser_with(
        r#"
        import std::fs::write_file;

        let saver = |content: str| write_file("state", content);

        fun main() {
            let s = saver;
        }
        "#,
        "reachable from the entry: main → saver → closure → write_file (std::fs)",
    );
}

#[test]
fn calling_a_global_closure_colors_via_its_binding() {
    // Before initializer edges, a global closure's body was charged to
    // NOBODY: the call is value-indirect (skipped) and it has no lexical
    // parent. The call's subject is a reference to the binding, so the
    // reference edge now carries the charge.
    assert_fails_browser_with(
        r#"
        import std::fs::write_file;

        let saver = |content: str| write_file("state", content);

        fun main() {
            saver("boot");
        }
        "#,
        "requires the `process` layer of `std` and cannot run on `browser`",
    );
}

#[test]
fn an_unreferenced_colored_global_is_elided_not_rejected() {
    // F6: a dropped binding's initializer does not run — referencing it only
    // from unreached code keeps the browser build clean.
    assert_compiles_browser(
        r#"
        import std::fs::read_file_to_str;

        let cache = read_file_to_str("cache.txt");

        fun server_only(): str {
            cache
        }

        fun main() {}
        "#,
    );
}

#[test]
fn a_neutral_global_is_colorless_everywhere() {
    assert_compiles_browser(
        r#"
        import std::print;

        let greeting = "hello";

        fun main() {
            print(greeting);
        }
        "#,
    );
}

#[test]
fn a_const_bindings_initializer_is_compile_time_data() {
    // `const` initializers run in the compile-time interpreter and ship as
    // serialized values — nothing runs on the build platform, so the binding
    // seeds nothing and carries no requirement line.
    assert_compiles_browser(
        r#"
        import std::print;

        let width = const 2 + 2;

        fun main() {
            print(width);
        }
        "#,
    );
    assert_eq!(
        requirement_line_of(
            r#"
        import std::print;

        let width = const 2 + 2;

        fun main() {
            print(width);
        }
        "#,
            "width",
        ),
        None
    );
}

#[test]
fn a_coerced_functions_body_charges_the_reference_site() {
    // fn-to-closure coercion (proposal/fn-coercion.md): a named function
    // passed as a value has no closure-creation event for the creator rule,
    // so the REFERENCE is the charge — every later call through the value is
    // deliberately uncharged (`Indirect(Value)`).
    assert_fails_browser_with(
        r#"
        import std::fs::write_file;

        fun save(content: str) {
            write_file("state", content);
        }

        fun apply(action: |str| void) {
            action("x");
        }

        fun main() {
            apply(save);
        }
        "#,
        "reachable from the entry: main → save → write_file (std::fs)",
    );
}

#[test]
fn an_index_expressions_subject_reference_colors() {
    // The `Index` collector blind spot: `cache[0]` never walked its subject,
    // so the reference — and the initializer behind it — went unseen (it also
    // dropped load-bearing bindings from emission; `const.vl`'s golden pins
    // that side).
    assert_fails_browser_with(
        r#"
        import std::print;
        import std::fs::read_file_to_str;

        let cache = [read_file_to_str("cache.txt")];

        fun main() {
            print(cache[0]);
        }
        "#,
        "requires the `process` layer of `std` and cannot run on `browser`",
    );
}

#[test]
fn an_iterator_protocols_next_call_colors_the_loop() {
    // `for x in iterable` calls the resolved protocol `next()` every pass —
    // an edge anchored at the loop (previously invisible: the desugar happened
    // at emission, after the graph was built).
    assert_fails_browser_with(
        r#"
        import std::option::Option::{ self, Some, None };
        import std::iterator::Iterator;
        import std::fs::write_file;

        mut produced = 0;

        struct Audited { limit: i32 }

        impl Audited with Iterator<i32> {
            fun next(self): Option<i32> {
                write_file("audit.log", "tick");
                produced = produced + 1;
                if produced <= self.limit {
                    Some(produced)
                } else {
                    None
                }
            }
        }

        fun main() {
            // The struct-literal iterable is parenthesized: a `for .. in`
            // iterable is a condition position, which excludes bare struct
            // literals (§H.1).
            for n in (Audited { limit = 3 }) {
                let _n = n;
            }
        }
        "#,
        "requires the `process` layer of `std` and cannot run on `browser`",
    );
}

#[test]
fn a_dropped_bindings_initializer_leaves_no_residue_in_the_bundle() {
    // Emission's half of F6 (the phantom-retention fix): a binding referenced
    // only by unreached code must not drag its callees — nor their host
    // `import ... from "node:..."` lines — into the bundle. A browser bundle
    // with a `node:` import fails at module parse, before any code runs.
    let source = r#"
        import std::fs::read_file_to_str;

        let cache = read_file_to_str("cache.txt");

        fun server_only(): str {
            cache
        }

        fun main() {}
        "#;
    let browser = compile_browser(source).expect("the elided reach compiles for the browser");
    assert!(
        !browser.contains("node:"),
        "phantom host import in the browser bundle:\n{browser}"
    );
    assert!(
        !browser.contains("cache.txt"),
        "dropped initializer emitted:\n{browser}"
    );
    // The same binding still emits where the reference is load-bearing. (A
    // reference inside an ELIDED unused local doesn't count as running the
    // initializer — emission drops both, and admission merely
    // over-approximates in the safe direction by still checking it.)
    let node = compile(
        r#"
        import std::print;
        import std::fs::exists;

        let cache = exists("cache.txt");

        fun main() {
            print(cache);
        }
        "#,
    )
    .expect("the node build admits the reach");
    assert!(node.contains("cache.txt"), "reached initializer must emit");
}

#[test]
fn a_globals_requirement_line_serves_hover_like_a_functions() {
    let line = requirement_line_of(
        r#"
        import std::fs::read_file_to_str;

        let cache = read_file_to_str("cache.txt");

        fun main() {}
        "#,
        "cache",
    )
    .expect("`cache`'s initializer reaches the layer");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `read_file_to_str (std::fs)`)"
    );
}

#[test]
fn a_function_referencing_a_colored_global_inherits_its_line() {
    let line = requirement_line_of(
        r#"
        import std::fs::read_file_to_str;

        let cache = read_file_to_str("cache.txt");

        fun peek(): str {
            cache
        }

        fun main() {}
        "#,
        "peek",
    )
    .expect("`peek` runs the initializer by referencing the binding");
    assert_eq!(
        line,
        "requires the `process` layer of `std` (via `cache → read_file_to_str (std::fs)`)"
    );
}

#[test]
fn a_function_requiring_two_layers_renders_one_line_each_in_label_order() {
    // The mixed form: one function reaching two different layers gets one
    // line per label, label-sorted. (`torn` is unreached, so the node build
    // stays admissible while the browser requirement is still computed.)
    let line = requirement_line_of(
        r#"
        import std::fs;
        import std::router::navigate;

        fun torn() {
            fs::write_file("state", "data");
            navigate("/home");
        }

        fun main() {}
        "#,
        "torn",
    )
    .expect("`torn` requires both layers");
    assert_eq!(
        line,
        "requires the `browser` layer of `std` (via `navigate (std::router)`)\n\
         requires the `process` layer of `std` (via `write_file (std::fs)`)"
    );
}

// --- B19: closure-return-grounded method generics (backlog.md §B.19) ---------
//
// A method's own generic fixed ONLY by a closure argument's return
// (`map<U>(self, transform: |V| U)`) used to freeze abstract when the call
// resolved before the closure's body typed: the substitution — and the call's
// return type — kept `Generic(U)`, so a later bounded call rejected 'U', and
// monomorphization through the value dispatched abstractly. The resolution now
// defers (the same retry the non-closure path always had) until the closure's
// type lands. The browser-side shape is pinned above
// (`a_mapped_signal_meets_a_bound_without_annotation`).

#[test]
fn a_closure_grounded_generic_dispatches_through_its_bound() {
    // The runtime half: the grounded `U` must reach monomorphization, so the
    // consumer's `==` dispatches to the REAL PartialEq — both outcomes, so an
    // empty abstract method (undefined ~ falsy) cannot pass.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialEq;

        struct Wrap<V> {
            value: V,
        }

        impl Wrap<type V> {
            fun map<U>(self, transform: |V| U): Wrap<U> {
                Wrap { value = transform(self.value) }
            }
        }

        [derive(PartialEq)]
        struct Label {
            text: str,
        }

        fun same<T: PartialEq>(a: T, b: T): bool {
            a == b
        }

        fun tag(n: i32): Label {
            Label { text = i"tag-{n}" }
        }

        fun main() {
            let a = Wrap { value = 3 }.map(|n| tag(n));
            let b = Wrap { value = 3 }.map(|n| tag(n));
            let c = Wrap { value = 4 }.map(|n| tag(n));
            print(same(a.value, b.value));
            print(same(a.value, c.value));
        }
        main();
        "#,
        "true\nfalse\n",
    );
}

#[test]
fn a_closure_grounded_generic_still_fails_an_unmet_bound() {
    // The other direction: once `U` grounds to a type WITHOUT the impl, the
    // bound check must reject it — deferral must not soften the gate.
    assert_fails_spanning(
        r#"
        import std::print;
        import std::compare::PartialEq;

        struct Wrap<V> {
            value: V,
        }

        impl Wrap<type V> {
            fun map<U>(self, transform: |V| U): Wrap<U> {
                Wrap { value = transform(self.value) }
            }
        }

        struct Opaque {
            tag: str,
        }

        fun needs_eq<T: PartialEq>(wrapped: Wrap<T>): bool {
            wrapped.value == wrapped.value
        }

        fun cloak(n: i32): Opaque {
            Opaque { tag = i"{n}" }
        }

        fun main() {
            let wrapped = Wrap { value = 3 }.map(|n| cloak(n));
            print(needs_eq(wrapped));
        }
        "#,
        "needs_eq(wrapped)",
        "does not implement trait 'PartialEq'",
    );
}

#[test]
fn chained_maps_ground_each_link() {
    // Two chained closure-grounded links: the outer receiver is itself a
    // deferred call result, so the retries must converge inside-out.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialEq;

        struct Wrap<V> {
            value: V,
        }

        impl Wrap<type V> {
            fun map<U>(self, transform: |V| U): Wrap<U> {
                Wrap { value = transform(self.value) }
            }
        }

        fun same<T: PartialEq>(a: T, b: T): bool {
            a == b
        }

        fun stringify(n: i32): str {
            i"{n}"
        }

        fun measure(text: str): i32 {
            text.len()
        }

        fun main() {
            let wrapped = Wrap { value = 41 }.map(|n| stringify(n)).map(|text| measure(text));
            print(same(wrapped.value, 2));
            print(wrapped.value);
        }
        main();
        "#,
        "true\n2\n",
    );
}

#[test]
fn a_closure_grounded_generic_meets_a_method_bound() {
    // The consumer as a METHOD with its own bounded generic (the `swap` shape)
    // rather than a free function.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialEq;

        struct Wrap<V> {
            value: V,
        }

        impl Wrap<type V> {
            fun map<U>(self, transform: |V| U): Wrap<U> {
                Wrap { value = transform(self.value) }
            }
        }

        struct Gate {
            open: bool,
        }

        impl Gate {
            fun admits<T: PartialEq>(self, wrapped: Wrap<T>): bool {
                self.open && wrapped.value == wrapped.value
            }
        }

        fun parse(text: str): i32 {
            text.len()
        }

        fun main() {
            let gate = Gate { open = true };
            let wrapped = Wrap { value = "hi" }.map(|text| parse(text));
            print(gate.admits(wrapped));
        }
        main();
        "#,
        "true\n",
    );
}

// --- B20: named functions as closure values (proposal/fn-coercion.md) --------
//
// A reference to a plain (non-generic, non-method, non-async, non-extern)
// named function coerces to a matching closure type — `map(parse)` instead of
// `map(|path| parse(path))`. On JS the named function IS the value, so the
// whole feature is type-layer.

#[test]
fn a_named_function_passes_as_a_method_closure_argument() {
    // The motivating shape: a method's closure parameter whose return binds
    // the method's own generic (`map<U>`'s `U = Route`) from the FUNCTION's
    // declared return.
    assert_compiles_and_runs(
        r#"
        import std::print;

        struct Wrap<V> {
            value: V,
        }

        impl Wrap<type V> {
            fun map<U>(self, transform: |V| U): Wrap<U> {
                Wrap { value = transform(self.value) }
            }
        }

        fun measure(text: str): i32 {
            text.len()
        }

        fun main() {
            let wrapped = Wrap { value = "abcd" }.map(measure);
            print(wrapped.value);
        }
        main();
        "#,
        "4\n",
    );
}

#[test]
fn a_named_function_passes_as_a_free_closure_argument() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun apply(seed: i32, transform: |i32| i32): i32 {
            transform(seed)
        }

        fun double(n: i32): i32 {
            n * 2
        }

        fun main() {
            print(apply(21, double));
        }
        main();
        "#,
        "42\n",
    );
}

#[test]
fn a_named_function_binds_to_an_annotated_let_and_field() {
    // The two storage positions: a closure-annotated binding, and a
    // closure-typed struct field (the Kolt server-hook shape).
    assert_compiles_and_runs(
        r#"
        import std::print;

        struct Holder {
            hook: |str| i32,
        }

        fun measure(text: str): i32 {
            text.len()
        }

        fun main() {
            let bound: |str| i32 = measure;
            print(bound("abc"));
            let holder = Holder { hook = measure };
            let hook = holder.hook;
            print(hook("abcde"));
        }
        main();
        "#,
        "3\n5\n",
    );
}

#[test]
fn a_named_function_returns_as_a_closure() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun double(n: i32): i32 {
            n * 2
        }

        fun pick(): |i32| i32 {
            double
        }

        fun main() {
            let f = pick();
            print(f(8));
        }
        main();
        "#,
        "16\n",
    );
}

#[test]
fn a_void_function_without_annotation_coerces() {
    // An unannotated-return (void) function into a `|| void` slot — the
    // handler shape; the return type comes from the body's inferred type.
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun run_twice(action: || void) {
            action();
            action();
        }

        fun say_hi() {
            print("hi");
        }

        fun main() {
            run_twice(say_hi);
        }
        main();
        "#,
        "hi\nhi\n",
    );
}

#[test]
fn a_stored_function_value_survives_shared_storage() {
    // Through `Shared<|str| i32>` — stored as a value, read back, called
    // indirectly (the pilot's hook pattern, without the eta-expansion).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;

        fun measure(text: str): i32 {
            text.len()
        }

        fun main() {
            let hook: Shared<|str| i32> = Shared::new(measure);
            let stored = hook.read();
            print(stored("abcd"));
        }
        main();
        "#,
        "4\n",
    );
}

#[test]
fn a_mismatched_function_still_fails_closure_positions() {
    // Wrong parameter type: no coercion, the mismatch error stays.
    assert_fails(
        r#"
        fun apply(seed: i32, transform: |i32| i32): i32 {
            transform(seed)
        }

        fun shout(text: str): str {
            text + "!"
        }

        fun main() {
            apply(3, shout);
        }
        "#,
    );
}

#[test]
fn a_generic_function_does_not_coerce() {
    // Rule 2: no single value exists for a generic function (which
    // instantiation?) — deferred, still the mismatch error.
    assert_fails(
        r#"
        fun apply(seed: i32, transform: |i32| i32): i32 {
            transform(seed)
        }

        fun identity<T>(value: T): T {
            value
        }

        fun main() {
            apply(3, identity);
        }
        "#,
    );
}

#[test]
fn an_async_function_does_not_coerce() {
    // Rule 4: a call through a plain closure value is not awaited, so the
    // coerced value would leak a raw promise — rejected.
    assert_fails(
        r#"
        fun apply(seed: i32, transform: |i32| i32): i32 {
            transform(seed)
        }

        async fun slow_double(n: i32): i32 {
            n * 2
        }

        fun main() {
            apply(3, slow_double);
        }
        "#,
    );
}

#[test]
fn a_context_reading_function_still_cannot_be_a_value() {
    // Rule 5: coercion doesn't bypass the context pass — a needs-context
    // function used as a value keeps its value-use rejection (its hidden
    // parameter can't thread through an indirect call).
    let source = r#"
        import std::context::Context;

        let scope: Context<i32> = Context::new();

        fun reads_scope(): i32 {
            scope.get()
        }

        fun apply(transform: || i32): i32 {
            transform()
        }

        fun main() {
            let result = scope.run(7, || apply(reads_scope));
        }
        main();
        "#;
    match compile(source) {
        Ok(_) => panic!("expected the context value-use rejection, but it compiled"),
        Err(errors) => assert!(
            errors
                .iter()
                .any(|error| error.contains("can't be used as a value")),
            "no diagnostic mentions the value-use rule; got: {errors:#?}"
        ),
    }
}

#[test]
fn an_imported_function_coerces_across_modules() {
    // The reference resolves through an import binding (browser layer:
    // `std::router::segments` is a plain vilan fn) — the coercion and the
    // emitted value must both follow the alias to the defining function.
    assert_compiles_browser(
        r#"
        import std::router::segments;

        fun apply(path: str, transform: |str| List<str>): List<str> {
            transform(path)
        }

        fun main() {
            let parts = apply("/a/b", segments);
        }
        "#,
    );
}

// --- K5: `std::time` + i53 on the wire (kolt-migration.md §2.5) --------------
//
// The runtime surface (arithmetic, describe, ISO, codec round-trips, sleep) is
// pinned by the corpus (`vilan/test/time.vl`, node-run; interpreter-excluded —
// host clock). These pin the compile-level rules.

#[test]
fn the_clock_is_not_const_evaluable() {
    // `now()` reads the host clock — an impure capability. A `const` forcing
    // it must fail at compile time, not fold a build-machine timestamp into
    // the program.
    let source = r#"
        import std::time::now;
        import std::print;

        fun main() {
            let moment = const now();
            print(moment.millis);
        }
        main();
        "#;
    match compile(source) {
        Ok(_) => panic!("expected `const now()` to be rejected, but it compiled"),
        Err(errors) => assert!(
            errors
                .iter()
                .any(|error| error.contains("unknown host call `Date.now`")),
            "no diagnostic rejects the host clock under const; got: {errors:#?}"
        ),
    }
}

#[test]
fn time_is_platform_neutral() {
    // `Date.now`/`Date`/`setTimeout` exist on every host, so the module lives
    // in the base layer: the same program compiles for node AND browser.
    let source = r#"
        import std::time::{ now, sleep_for, Instant, Duration };

        async fun main() {
            let anchor = Instant { millis = 0i53 };
            let age = now().since(anchor) + Duration::minutes(1);
            let _rendered = age.describe();
            let _shifted = now() - Duration::hours(1) + Duration::seconds(30);
            sleep_for(Duration::millis(1i53));
        }
        "#;
    assert_compiles(source);
    assert_compiles_browser(source);
}

#[test]
fn i53_fields_are_wire() {
    // The K5 blocker, closed: `i53` is a Wire scalar (its own serializer
    // channel), so timestamps and row ids ride derives directly — including
    // nested through `Instant` and `List`/`Option`.
    assert_compiles(
        r#"
        import std::time::Instant;
        import std::option::Option;

        [derive(Wire)]
        struct Task {
            id: i53,
            created_at: Instant,
            due: Option<i53>,
            checkpoints: List<i53>,
        }

        fun main() {
            let _task = Task {
                id = 9007199254740991i53,
                created_at = Instant { millis = 0i53 },
                due = Option::None,
                checkpoints = [1i53, 2i53],
            };
        }
        "#,
    );
}

#[test]
fn i53_signatures_are_rpc_legal() {
    // The `[rpc]` Wire-signature rule shares the scalar set: i53 parameters
    // and returns are legal.
    assert_compiles(
        r#"
        import std::reactive::Signal;

        [service(TickClient)]
        struct Ticker {
            [expose] latest: Signal<i53>,
        }

        impl Ticker {
            [rpc]
            fun record(self, at: i53): i53 {
                at
            }
        }

        fun main() {
            let _ticker = Ticker { latest = Signal::new(0i53) };
        }
        "#,
    );
}

#[test]
fn non_wire_fields_still_fail() {
    // The gate holds around the new scalar: a closure-typed field is still
    // rejected by the Wire boundary.
    assert_fails_spanning(
        r#"
        [derive(Wire)]
        struct Holder {
            callback: |i53| i53,
        }
        "#,
        "|i53| i53",
        "which is not Wire",
    );
}

// --- B22: return-expectation inference bound to the caller's generics --------
//
// A call's return-type-only generic inference (the `let n: Cell<i32> =
// Cell::fresh()` gap-filler) must bind only the CALLEE's own generics. When an
// abstract argument already bound the callee's `T` to the caller's `T`, the
// substituted return type's generics are the caller's — unifying THOSE against
// the expectation wrote a caller-keyed entry into the call's substitution map,
// and the bound check then demanded the caller generic's bounds of whatever it
// unified with (a raw unbounded struct binder), rejecting valid code.

#[test]
fn a_bounded_caller_constructs_an_unbounded_struct_via_a_generic_static_new() {
    // The motivating shape (std::reactive's `draft()`): `fun draft<T:
    // PartialEq>` building a struct whose field is made by an UNBOUNDED
    // generic container's static `new`. The field expectation mentions the
    // struct's raw binder; the call's return mentions the caller's `T` — the
    // poison unification paired the two and demanded `PartialEq` of the
    // struct binder.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialEq;

        struct Cell<T> {
            value: T,
        }

        impl Cell<type T> {
            fun new(value: T): Cell<T> {
                Cell { value }
            }
        }

        struct Box<T> {
            inner: Cell<T>,
        }

        fun boxed<T: PartialEq>(initial: T): Box<T> {
            Box {
                inner = Cell::new(initial),
            }
        }

        fun main() {
            let held = boxed(3);
            print(held.inner.value);
        }
        main();
        "#,
        "3\n",
    );
}

#[test]
fn two_bounded_generics_construct_two_unbounded_fields() {
    // Multi-parameter form: each field's constructor call must stay keyed to
    // its own binding — before the fix BOTH `A` and `B` were rejected.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialEq;

        struct Cell<T> {
            value: T,
        }

        impl Cell<type T> {
            fun new(value: T): Cell<T> {
                Cell { value }
            }
        }

        struct Duo<A, B> {
            left: Cell<A>,
            right: Cell<B>,
        }

        fun paired<A: PartialEq, B: PartialEq>(first: A, second: B): Duo<A, B> {
            Duo {
                left = Cell::new(first),
                right = Cell::new(second),
            }
        }

        fun main() {
            let held = paired(1, "two");
            print(held.left.value);
            print(held.right.value);
        }
        main();
        "#,
        "1\ntwo\n",
    );
}

#[test]
fn a_nested_generic_argument_still_binds_through_the_expectation() {
    // Nested form: the caller's `T` sits INSIDE the callee's binding
    // (`Cell::new([initial])` binds the callee's `T` to `List<T_caller>`).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialEq;

        struct Cell<T> {
            value: T,
        }

        impl Cell<type T> {
            fun new(value: T): Cell<T> {
                Cell { value }
            }
        }

        struct Box<T> {
            inner: Cell<List<T>>,
        }

        fun boxed<T: PartialEq>(initial: T): Box<T> {
            Box {
                inner = Cell::new([initial]),
            }
        }

        fun main() {
            let held = boxed(7);
            print(held.inner.value[0]);
        }
        main();
        "#,
        "7\n",
    );
}

#[test]
fn return_type_only_inference_still_binds_a_static_generic() {
    // The feature the merge exists for keeps working: no argument mentions
    // `T`, so the expectation is the only thing that can bind it — the
    // callee's own return-type generic must still be inferred.
    assert_compiles_and_runs(
        r#"
        import std::print;

        struct Cell<T> {
            value: List<T>,
        }

        impl Cell<type T> {
            fun fresh(): Cell<T> {
                Cell { value = [] }
            }
        }

        fun main() {
            let cell: Cell<i32> = Cell::fresh();
            print(cell.value.len());
        }
        main();
        "#,
        "0\n",
    );
}

// --- Draft<T>: local-first cells (std::reactive, kolt-migration §3) ----------
//
// `draft(initial, commit)` is a local-first cell: edits land in `local`
// FIRST (`push` spawns the commit, never awaits it), `adopt` folds in remote
// changes without fighting in-flight edits, and failure KEEPS the local value
// (unlike `optimistic`'s rollback — right for one-shot actions, hostile
// mid-typing). Conflicts are last-write-wins.

#[test]
fn draft_push_is_local_first_and_settles_synced() {
    // `push` returns with `local` set and the state Dirty while the commit
    // is still on the wire; the settle lands afterwards.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ draft, Draft, DraftState };
        import std::option::Option::{ self, Some, None };
        import std::shared::Shared;
        import std::time::{ sleep_for, Duration };

        fun main() {
            let committed: Shared<List<str>> = Shared::new([]);
            let name = draft("seed", |value: str| {
                sleep_for(Duration::millis(5));
                committed.write().push(value);
                None
            });
            print(name.state.get() == DraftState::Synced);
            name.push("edit");
            print(name.local.get());
            print(name.state.get() == DraftState::Dirty);
            sleep_for(Duration::millis(20));
            print(name.state.get() == DraftState::Synced);
            print(committed.read().len());
        }
        main();
        "#,
        "true\nedit\ntrue\ntrue\n1\n",
    );
}

#[test]
fn draft_adopt_echo_is_a_no_op() {
    // A pushed value reflected back by the remote (the mirror echo) changes
    // nothing — state stays Synced, `local` untouched.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ draft, Draft, DraftState };
        import std::option::Option::{ self, Some, None };
        import std::time::{ sleep_for, Duration };

        fun main() {
            let name = draft("seed", |value: str| {
                let _sent = value;
                None
            });
            name.push("edit");
            sleep_for(Duration::millis(10));
            name.adopt("edit");
            print(name.local.get());
            print(name.state.get() == DraftState::Synced);
        }
        main();
        "#,
        "edit\ntrue\n",
    );
}

#[test]
fn draft_adopt_takes_remote_when_local_is_clean() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ draft, Draft, DraftState };
        import std::option::Option::{ self, Some, None };

        fun main() {
            let name = draft("seed", |value: str| {
                let _sent = value;
                None
            });
            name.adopt("remote");
            print(name.local.get());
            print(name.synced.read());
            print(name.state.get() == DraftState::Synced);
        }
        main();
        "#,
        "remote\nremote\ntrue\n",
    );
}

#[test]
fn draft_failure_keeps_the_local_value() {
    // Unlike `optimistic`, no rollback: the user's text survives the failed
    // commit, and the state carries the reason.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ draft, Draft, DraftState };
        import std::option::Option::{ self, Some, None };
        import std::time::{ sleep_for, Duration };

        fun main() {
            let sour = draft("base", |value: str| {
                let _sent = value;
                Some("boom")
            });
            sour.push("mine");
            sleep_for(Duration::millis(10));
            print(sour.state.get() == DraftState::Failed("boom"));
            print(sour.local.get());
            print(sour.synced.read());
        }
        main();
        "#,
        "true\nmine\nbase\n",
    );
}

#[test]
fn draft_dirty_local_survives_adoption() {
    // Last-write-wins: a dirty local ignores the remote value in `local`
    // (the user's text wins for now) while `synced` records it, so the
    // eventual push knowingly overwrites.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ draft, Draft, DraftState };
        import std::option::Option::{ self, Some, None };
        import std::time::{ sleep_for, Duration };

        fun main() {
            let sour = draft("base", |value: str| {
                let _sent = value;
                Some("boom")
            });
            sour.push("mine");
            sleep_for(Duration::millis(10));
            sour.adopt("theirs");
            print(sour.local.get());
            print(sour.synced.read());
        }
        main();
        "#,
        "mine\ntheirs\n",
    );
}

#[test]
fn draft_generation_guard_discards_superseded_pushes() {
    // Fast typing over a slow wire: the first push's commit lands LAST, but
    // only the newest push settles the state — the stale completion is
    // discarded.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ draft, Draft, DraftState };
        import std::option::Option::{ self, Some, None };
        import std::time::{ sleep_for, Duration };

        fun main() {
            let raced = draft("start", |value: str| {
                if value == "slow" {
                    sleep_for(Duration::millis(30));
                } else {
                    sleep_for(Duration::millis(5));
                }
                None
            });
            raced.push("slow");
            raced.push("fast");
            sleep_for(Duration::millis(60));
            print(raced.local.get());
            print(raced.synced.read());
            print(raced.state.get() == DraftState::Synced);
        }
        main();
        "#,
        "fast\nfast\ntrue\n",
    );
}

#[test]
fn bind_draft_compiles_for_the_browser() {
    // The ui seam: an input two-way bound to a draft (user input pushes;
    // adoption writes `local` and bypasses the push path).
    assert_compiles_browser(
        r#"
        import std::ui::{ view, View, mount_root };
        import std::reactive::{ draft, Draft, DraftState };
        import std::option::Option::{ self, Some, None };

        fun main() {
            let name = draft("seed", |value: str| {
                let _sent = value;
                None
            });
            let _root = mount_root("app", || view("input").bind_draft(name));
        }
        main();
        "#,
    );
}

// --- B23: effect-closure parameter grounding (backlog.md §B.23) --------------

#[test]
fn an_effect_closures_unannotated_parameter_grounds_from_the_signal() {
    // B23, FIXED: the inherited-trait-default path now records the trait's
    // receiver bindings (so `effect`'s `|T| void` types concretely), and
    // `resolve_match` defers on a not-yet-filled closure parameter instead
    // of binding pattern captures against the enum's raw declaration.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner, run_with_owner };
        import std::option::Option::{ self, Some, None };

        struct Task {
            name: str,
        }

        fun main() {
            let entry: Signal<Option<Task>> = Signal::new(Some(Task { name = "a" }));
            let owner = Owner::new();
            run_with_owner(owner, || {
                entry.effect(|current| {
                    match current {
                        Some(let task) => print(task.name),
                        None => {},
                    }
                });
            });
        }
        main();
        "#,
        "a\n",
    );
}

#[test]
fn an_annotated_effect_parameter_destructures_the_signals_payload() {
    // The pinned workaround (and the kolt draft editor's shipped shape):
    // annotating the parameter grounds everything downstream.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::reactive::{ Signal, Owner, run_with_owner };
        import std::option::Option::{ self, Some, None };

        struct Task {
            name: str,
        }

        fun main() {
            let entry: Signal<Option<Task>> = Signal::new(Some(Task { name = "a" }));
            let owner = Owner::new();
            run_with_owner(owner, || {
                entry.effect(|current: Option<Task>| {
                    match current {
                        Some(let task) => print(task.name),
                        None => {},
                    }
                });
            });
        }
        main();
        "#,
        "a\n",
    );
}

// --- B24: primitive comparisons skip operand-type checking (FIXED) ----------
//
// Found writing the spec (§5.7): comparison operators between PRIMITIVES
// bypassed the PartialEq/PartialOrd model, so ill-typed mixes compiled and
// emitted raw JS comparisons (with JS coercion semantics). The rule now
// checked on the native fast path: the right operand types as `B = Self`
// with no implicit conversions (§5.8), `bool` has no ordering, and `&&`/`||`
// take `bool`. The right side is inferred WITH the left's type as its
// expectation, so an unsuffixed literal adapts exactly as it does in a
// `let` — `1i53 < 3` is `i53 < i53` — while genuinely typed operands must
// match.

#[test]
fn a_bool_compared_to_an_integer_is_rejected() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _x = true < 3;
        }
        "#,
        "true < 3",
        "`bool` has no ordering",
    );
}

#[test]
fn an_integer_compared_to_a_string_is_rejected() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _x = 1 == "a";
        }
        "#,
        r#"1 == "a""#,
        "`==` compares two values of the same type",
    );
}

#[test]
fn mixed_width_typed_comparison_is_rejected() {
    // TYPED operands of different widths reject — no implicit conversions.
    assert_fails_spanning(
        r#"
        fun main() {
            let a: i53 = 1;
            let b: i32 = 3;
            let _x = a < b;
        }
        "#,
        "a < b",
        "`<` compares two values of the same type",
    );
}

#[test]
fn an_unsuffixed_literal_adapts_to_the_comparisons_peer() {
    // The literal rule (numeric-types.md §3): an unsuffixed integer takes
    // the expected type — the peer operand here — so this is `i53 < i53`.
    assert_compiles(
        r#"
        fun main() {
            let _x = 1i53 < 3;
        }
        "#,
    );
}

#[test]
fn equality_between_mismatched_natives_is_rejected_for_typed_operands() {
    assert_fails(
        r#"
        fun main() {
            let n: u32 = 5;
            let s = "five";
            let _x = n == s;
        }
        "#,
    );
}

#[test]
fn logical_operators_take_bool_operands() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _x = 1 && true;
        }
        "#,
        "1 && true",
        "`&&` takes `bool` operands; the left operand is `i32`",
    );
}

#[test]
fn ordering_dispatches_through_a_partial_ord_impl() {
    // B25, fixed: the ordering operators resolve `PartialOrd`'s comparison
    // methods — usually the trait DEFAULTS over the impl's `partial_compare`,
    // re-dispatched to the concrete receiver like any inherited method.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::time::{ now, Duration };

        fun main() {
            let started = now();
            let deadline = started + Duration::hours(2i53);
            if started < deadline {
                print("dispatches");
            }
        }
        "#,
        "dispatches\n",
    );
}

#[test]
fn all_four_orderings_dispatch_on_a_user_type() {
    // lt / le / gt / ge, each through the trait default over one
    // `partial_compare` — both truth values exercised.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::{ PartialEq, PartialOrd, Ordering };
        import std::option::Option::{ self, Some };

        struct Level { rank: i32 }

        impl Level with PartialEq {
            fun eq(self, b: Level): bool { self.rank == b.rank }
        }

        impl Level with PartialOrd {
            fun partial_compare(self, b: Level): Option<Ordering> {
                self.rank.partial_compare(b.rank)
            }
        }

        fun main() {
            let low = Level { rank = 1 };
            let high = Level { rank = 9 };
            if low < high { print("lt"); }
            if low <= low { print("le"); }
            if high > low { print("gt"); }
            if high >= high { print("ge"); }
            if high < low { print("wrong-lt"); }
            if low > high { print("wrong-gt"); }
        }
        "#,
        "lt\nle\ngt\nge\n",
    );
}

#[test]
fn a_declared_lt_override_wins_over_the_default() {
    // An impl may declare the operator method itself (the `binary_op_dispatch`
    // path) — reversed ordering proves the OVERRIDE ran, not the default.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::{ PartialEq, PartialOrd, Ordering };
        import std::option::Option::{ self, Some };

        struct Upside { value: i32 }

        impl Upside with PartialEq {
            fun eq(self, b: Upside): bool { self.value == b.value }
        }

        impl Upside with PartialOrd {
            fun partial_compare(self, b: Upside): Option<Ordering> {
                self.value.partial_compare(b.value)
            }

            fun lt(self, b: Upside): bool {
                self.value > b.value
            }
        }

        fun main() {
            let small = Upside { value = 1 };
            let big = Upside { value = 9 };
            if big < small { print("override"); }
            if small < big { print("default"); }
        }
        "#,
        "override\n",
    );
}

#[test]
fn a_partial_ord_bound_dispatches_orderings_generically() {
    // `T: PartialOrd` — the `OnConstraint` path, re-resolved per
    // monomorphization; exercised with std's `Duration` impl.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::compare::PartialOrd;
        import std::time::Duration;

        fun smallest<T: PartialOrd>(a: T, b: T): T {
            if a < b { a } else { b }
        }

        fun main() {
            let short = Duration::seconds(5i53);
            let long = Duration::minutes(2i53);
            print(smallest(long, short).describe());
            print(smallest(3, 11));
        }
        "#,
        "5s\n3\n",
    );
}

#[test]
fn ordering_a_struct_is_rejected_not_js_compared() {
    // No `PartialOrd` dispatch for user types yet — a silent raw-JS `<`
    // (object coercion) would be a miscompile, so it errors instead.
    assert_fails_spanning(
        r#"
        struct Point { x: i32 }

        fun main() {
            let a = Point { x = 1 };
            let b = Point { x = 2 };
            let _x = a < b;
        }
        "#,
        "a < b",
        "does not implement the `PartialOrd` operator; add `impl Point with PartialOrd` providing `partial_compare`",
    );
}

#[test]
fn same_type_native_comparisons_still_compile_and_run() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let a: u32 = 5;
            let b: u32 = 9;
            if a < b && "a" < "b" && "x" == "x" && 1.5 < 2.5 && true == false || 3 <= 3 {
                print("ok");
            }
        }
        "#,
        "ok\n",
    );
}

// --- §J.3: module-level initializers cannot await ----------------------------
//
// Initializers run at module load — no enclosing function to become async,
// no top-level await in the emission model. An async call there used to
// type-check as `T` while holding a live promise at runtime (`state + 1`
// was garbage); it is now refused cleanly. Creating async closures stays
// legal: nothing awaits at load.

#[test]
fn an_async_call_in_a_module_initializer_is_rejected() {
    assert_fails_spanning(
        r#"
        import std::print;
        import std::time::{ sleep_for, Duration };

        async fun ready(tag: str): i32 {
            sleep_for(Duration::millis(1));
            42
        }

        let state = ready("boot");

        fun main() {
            print(state + 1);
        }
        "#,
        r#"ready("boot")"#,
        "a module-level binding cannot await",
    );
}

#[test]
fn an_initializer_calling_an_inferred_async_function_is_rejected() {
    // `warm` never says `async`; it is inferred (it calls `sleep_for`), and
    // the initializer's call to it is refused all the same.
    assert_fails_spanning(
        r#"
        import std::time::{ sleep_for, Duration };

        fun warm(tag: str): i32 {
            sleep_for(Duration::millis(1));
            7
        }

        let state = warm("boot");

        fun main() {
            let _s = state;
        }
        "#,
        r#"warm("boot")"#,
        "calls `warm`, which is async",
    );
}

#[test]
fn creating_an_async_closure_in_an_initializer_stays_legal() {
    // The charge is on AWAITING at load, not on holding async machinery:
    // a closure created in an initializer awaits nothing until called.
    assert_compiles(
        r#"
        import std::time::{ sleep_for, Duration };

        let warm = || sleep_for(Duration::millis(1));

        fun main() {
            let _w = warm;
        }
        "#,
    );
}

// --- The i53/u53 rename (numeric-types.md §8) --------------------------------
//
// The f64-backed wide integers are named for the precision they deliver
// (±2^53), and unknown numeric suffixes are ERRORS rather than silently
// typing as unsuffixed (`5q` once compiled as an i32).

#[test]
fn an_unknown_numeric_suffix_errors() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _x = 5q;
        }
        "#,
        "5q",
        "unknown numeric suffix `q`",
    );
}

#[test]
fn a_fractional_literal_with_an_unknown_suffix_errors() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _x = 2.5q;
        }
        "#,
        "2.5q",
        "unknown numeric suffix `q`",
    );
}

#[test]
fn the_old_i64_suffix_errors_with_a_rename_hint() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _stamp = 1000i64;
        }
        "#,
        "1000i64",
        "`i64` was renamed to `i53`",
    );
}

#[test]
fn the_old_u64_suffix_errors_with_a_rename_hint() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _wide = 1000u64;
        }
        "#,
        "1000u64",
        "`u64` was renamed to `u53`",
    );
}

#[test]
fn i53_suffixed_literals_compile_and_run() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let wide = 9007199254740992i53;
            print(wide);
            print((3.9).as_i53());
            print((5i53).as_u53());
        }
        "#,
        "9007199254740992\n3\n5\n",
    );
}

// --- Bare-namespace paths in expression position (found by the walkthrough) --
//
// `std::math::min(1, 2)` inline used to PANIC the compiler: the failed
// resolution of the path head left its type id unmapped, and the static-
// accessor pass crashed on the first `get_type`. The namespace root is not
// a binding by design — qualified access goes through an imported module
// name — so the shape is a clean, guiding error now.

#[test]
fn a_bare_std_function_path_errors_cleanly() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _x = std::math::min(1, 2);
        }
        "#,
        "std",
        "`std` is a namespace, not a value",
    );
}

#[test]
fn a_bare_std_variant_path_errors_cleanly() {
    assert_fails_spanning(
        r#"
        fun main() {
            let _x = std::compare::Ordering::Less;
        }
        "#,
        "std",
        "`std` is a namespace, not a value",
    );
}

#[test]
fn an_imported_module_alias_qualifies_statics() {
    // The supported spelling: import the module, qualify through its name.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::math;

        fun main() {
            print(math::min(1, 2));
        }
        "#,
        "1\n",
    );
}

// --- Direct calls on postfix results (backlog §H.18, fixed) ------------------
//
// `self.hook.read()(a, b)` used to fail to parse ("expected a method name
// after `.`"): the member grammar greedily folded the second `(args)` into
// the member. A member now fuses at most ONE call; further `(args)` are
// direct-call postfixes on the chain (calling a closure-typed value).

#[test]
fn a_method_call_result_is_directly_callable() {
    // The service-hook shape that carried the bind-first workaround.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;

        struct Holder {
            hook: Shared<|i32, i32| i32>,
        }

        fun main() {
            let holder = Holder { hook = Shared::new(|a: i32, b: i32| a + b) };
            print(holder.hook.read()(20, 22));
        }
        "#,
        "42\n",
    );
}

#[test]
fn an_index_result_is_directly_callable() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let handlers: List<|i32| i32> = [|n: i32| n * 2, |n: i32| n + 1];
            print(handlers[0](21));
            print(handlers[1](41));
        }
        "#,
        "42\n42\n",
    );
}

#[test]
fn a_direct_call_chains_into_further_postfixes() {
    // The direct call's result re-enters the chain (here: indexed).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::shared::Shared;

        struct Factory {
            make: Shared<|i32| List<i32>>,
        }

        fun main() {
            let factory = Factory { make = Shared::new(|seed: i32| [seed, seed * 2]) };
            print(factory.make.read()(21)[1]);
        }
        "#,
        "42\n",
    );
}

#[test]
fn tuple_member_access_grounds() {
    // §I.19, fixed: `.0` resolves positionally against the tuple's elements
    // (spec §5.9) — the field path grew its Tuple arm. Destructuring remains
    // the multi-element form; `.0` is the point access.
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let pair: (i32, i32) = (41, 1);
            print(pair.0 + pair.1);
        }
        "#,
        "42\n",
    );
}

#[test]
fn tuple_member_access_infers_without_an_annotation() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let pair = (40, 2);
            print(pair.0 + pair.1);
        }
        "#,
        "42\n",
    );
}

#[test]
fn tuple_elements_carry_their_own_types() {
    // `.1` on `(i32, str)` is a str — methods dispatch on the element type.
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let entry = (7, "vilan");
            print(entry.1.len());
        }
        "#,
        "5\n",
    );
}

#[test]
fn nested_tuple_access_chains() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let nested = ((1, 2), 3);
            print(nested.0.1);
        }
        "#,
        "2\n",
    );
}

#[test]
fn a_tuple_typed_element_reads_as_a_value() {
    // Flat storage: `.0` on a nested tuple reslices its region, and the
    // result behaves as a full tuple value (destructure, re-access).
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let nested = ((1, 2), 3);
            let inner = nested.0;
            let (x, y) = inner;
            print(inner.1 + x + y);
        }
        "#,
        "5\n",
    );
}

#[test]
fn a_tuple_typed_element_assignment_writes_its_region() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            mut nested = ((1, 2), 3);
            nested.0 = (40, 2);
            print(nested.0.0 + nested.0.1 + nested.1);
        }
        "#,
        "45\n",
    );
}

#[test]
fn a_nested_tuple_write_hits_the_storage_not_a_copy() {
    // Chained positional accesses FOLD to one flat offset on the root, so a
    // write through a nested path mutates the tuple — never a resliced copy.
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            mut deep = ((1, 2), 3);
            deep.0.1 = 41;
            print(deep.0.1 + deep.0.0);
        }
        "#,
        "42\n",
    );
}

#[test]
fn a_tuple_element_out_of_range_is_rejected() {
    assert_fails_spanning(
        r#"
        fun main() {
            let pair = (41, 1);
            let _x = pair.2;
        }
        "#,
        "pair.2",
        "has no element 2 — its arity is 2",
    );
}

#[test]
fn a_named_member_on_a_tuple_is_rejected() {
    assert_fails_spanning(
        r#"
        fun main() {
            let pair = (41, 1);
            let _x = pair.first;
        }
        "#,
        "pair.first",
        "a tuple's members are its positions",
    );
}

#[test]
fn a_tuple_element_assigns_through_a_mut_binding() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            mut pair: (i32, i32) = (41, 1);
            pair.0 = 40;
            pair.1 = 2;
            print(pair.0 + pair.1);
        }
        "#,
        "42\n",
    );
}

#[test]
fn a_tuple_element_assignment_needs_a_mut_binding() {
    assert_fails(
        r#"
        fun main() {
            let pair: (i32, i32) = (41, 1);
            pair.0 = 5;
        }
        "#,
    );
}

// --- Never-typed divergence (two gotchas closed) ------------------------------
//
// `panic(..)`, `ret ..`, and `jump break/continue` now type as `Never`,
// which YIELDS in unification: a diverging match leg or if branch no longer
// constrains (panic's old `Any` absorbed the whole match; `ret` legs typed
// void and mismatched). The transformer emits diverging leg results as
// statements (`return e`, not `x = return e`).

#[test]
fn a_ret_leg_no_longer_poisons_the_match_type() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };

        fun first_or_bail(items: List<i32>): i32 {
            mut copy = items;
            let head = match copy.pop() {
                Some(let value) => value,
                None => ret 0 - 1,
            };
            head * 2
        }

        fun main() {
            print(first_or_bail([21]));
            let empty: List<i32> = [];
            print(first_or_bail(empty));
        }
        "#,
        "42\n-1\n",
    );
}

#[test]
fn a_panic_leg_no_longer_absorbs_the_match_type() {
    // The binding is UNANNOTATED — the value leg's type wins.
    assert_compiles_and_runs(
        r#"
        import std::{ print, panic };
        import std::option::Option::{ self, Some, None };

        fun unwrap_or_panic(slot: Option<str>): str {
            let value = match slot {
                Some(let text) => text,
                None => panic("missing"),
            };
            value + "!"
        }

        fun main() {
            print(unwrap_or_panic(Some("hi")));
        }
        "#,
        "hi!\n",
    );
}

#[test]
fn a_panicking_if_branch_yields_to_the_other() {
    assert_compiles_and_runs(
        r#"
        import std::{ print, panic };

        fun main() {
            let flag = true;
            let picked = if flag { 42 } else { panic("no") };
            print(picked);
        }
        "#,
        "42\n",
    );
}

#[test]
fn a_jump_leg_diverges_inside_a_loop() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            mut total = 0;
            for step in [1, 0, 2, 0, 3] {
                let value = match step {
                    0 => jump continue,
                    let n => n,
                };
                total += value;
            }
            print(total);
        }
        "#,
        "6\n",
    );
}

#[test]
fn all_diverging_legs_still_satisfy_an_annotation() {
    // Never fits any expected type; nothing runs past the match.
    assert_compiles(
        r#"
        import std::panic;

        fun choose(flag: bool): i32 {
            let value: i32 = match flag {
                true => panic("a"),
                false => ret 0,
            };
            value
        }

        fun main() {
            let _n = choose(false);
        }
        "#,
    );
}

#[test]
fn a_direct_call_types_several_unannotated_parameters() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let add = |a, b| a + b;
            print(add(20, 22));
        }
        "#,
        "42\n",
    );
}

#[test]
fn a_direct_call_respects_annotated_parameters() {
    // Mixed: the annotation stays authoritative; only the Unknown fills.
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun main() {
            let scale = |a: i32, b| a * b;
            print(scale(6, 7));
        }
        "#,
        "42\n",
    );
}

// --- H.1: struct literals as operator operands ----------------------------------
// The operator/postfix chain admits struct literals as operands in ordinary
// expression positions; condition positions (`if`/`for` conditions, `for .. in`
// iterables, `match` subjects) exclude them so `if Foo { .. }` keeps the brace
// for the block. Parenthesize a literal to use it in a condition.

#[test]
fn a_struct_literal_is_a_left_operand() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        [derive(PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            let p = Point { x = 1, y = 2 };
            print(Point { x = 1, y = 2 } == p);
        }
        "#,
        "true\n",
    );
}

#[test]
fn a_struct_literal_is_a_right_operand() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        [derive(PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            let p = Point { x = 1, y = 2 };
            print(p != Point { x = 3, y = 4 });
        }
        "#,
        "true\n",
    );
}

#[test]
fn a_struct_literal_folds_a_field_access() {
    // The old dedicated literal member-fold, now the general postfix chain.
    assert_compiles_and_runs(
        r#"
        import std::print;

        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            print(Point { x = 3, y = 4 }.x);
        }
        "#,
        "3\n",
    );
}

#[test]
fn a_struct_literal_folds_a_method_call() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        struct Point {
            x: i32,
            y: i32,
        }

        impl Point {
            fun sum(self): i32 {
                self.x + self.y
            }
        }

        fun main() {
            print(Point { x = 3, y = 4 }.sum());
        }
        "#,
        "7\n",
    );
}

#[test]
fn a_struct_literal_operand_composes_with_logical_operators() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        [derive(PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            let p = Point { x = 1, y = 2 };
            print(Point { x = 1, y = 2 } == p && 1 < 2);
        }
        "#,
        "true\n",
    );
}

#[test]
fn a_generic_struct_literal_is_an_operand() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        [derive(PartialEq)]
        struct Holder<T> {
            value: T,
        }

        fun main() {
            let h = Holder { value = 3 };
            print(Holder<i32> { value = 3 } == h);
        }
        "#,
        "true\n",
    );
}

#[test]
fn a_parenthesized_struct_literal_serves_in_a_condition() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        [derive(PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            let p = Point { x = 1, y = 2 };
            if p == (Point { x = 1, y = 2 }) {
                print("equal");
            }
        }
        "#,
        "equal\n",
    );
}

#[test]
fn a_bare_struct_literal_statement_still_parses() {
    assert_compiles(
        r#"
        struct Point {
            x: i32,
        }

        fun main() {
            Point { x = 1 };
        }
        "#,
    );
}

#[test]
fn a_match_subject_does_not_take_a_struct_literal() {
    // Condition positions stay struct-free: the `{` after the subject is the
    // arms block, so a literal there is a parse error (parenthesize instead).
    assert_fails(
        r#"
        struct Point {
            x: i32,
        }

        fun main() {
            match Point { x = 1 } {
                _ => 0,
            }
        }
        "#,
    );
}

#[test]
fn a_for_iterable_does_not_take_a_struct_literal() {
    assert_fails(
        r#"
        struct Wrapper {
            items: i32,
        }

        fun main() {
            for e in Wrapper { items = 1 } { }
        }
        "#,
    );
}

// --- B.27: a bare type name is not a value --------------------------------------
// A bare name that resolves to a non-value entity — a type (struct/enum,
// primitives included), a trait, a type parameter, or a module — is rejected in
// value position (it used to compile, `let q = Point;` binding the constructor
// object). This is also what disarmed the condition-position misparse: with H.1
// keeping struct literals out of conditions, `if p == Point { .. } { .. }`
// parses `p == Point` as the condition, which now errors on `Point` instead of
// running against the type object and trapping at runtime.

#[test]
fn a_bare_struct_name_is_not_a_value() {
    assert_fails_with(
        r#"
        struct Point {
            x: i32,
        }

        fun main() {
            let q = Point;
        }
        "#,
        "`Point` is a type, not a value",
    );
}

#[test]
fn a_bare_enum_name_is_not_a_value() {
    assert_fails_with(
        r#"
        enum Color {
            Red,
            Green,
        }

        fun main() {
            let q = Color;
        }
        "#,
        "`Color` is a type, not a value",
    );
}

#[test]
fn a_bare_trait_name_is_not_a_value() {
    assert_fails_with(
        r#"
        trait Show {
        }

        fun main() {
            let q = Show;
        }
        "#,
        "`Show` is a trait, not a value",
    );
}

#[test]
fn a_bare_type_parameter_is_not_a_value() {
    // Inside an instantiated generic, `T` names a type, not a runtime value.
    assert_fails_with(
        r#"
        import std::print;

        fun identity<T>(x: T): T {
            let q = T;
            x
        }

        fun main() {
            print(identity(5));
        }
        "#,
        "`T` is a type parameter, not a value",
    );
}

#[test]
fn a_bare_primitive_name_is_not_a_value() {
    // Primitives are source `external struct`s, so they take the same path.
    assert_fails_with(
        r#"
        fun main() {
            let q = i32;
        }
        "#,
        "`i32` is a type, not a value",
    );
}

#[test]
fn a_bare_module_name_is_not_a_value() {
    assert_fails_with(
        r#"
        import std::math;

        fun main() {
            let q = math;
        }
        "#,
        "`math` is a module, not a value",
    );
}

#[test]
fn an_unparenthesized_struct_literal_condition_is_rejected_not_misparsed() {
    // The realistic shape: a user writes a struct-literal comparison in a
    // condition. H.1 parses `p == Point` (struct-free condition); B.27 then
    // rejects `Point` as a value, so it's a clear error, not a runtime trap.
    assert_fails_with(
        r#"
        import std::print;

        [derive(PartialEq)]
        struct Point {
            x: i32,
        }

        fun main() {
            let p = Point { x = 1 };
            if p == Point {
                print("y");
            }
        }
        "#,
        "`Point` is a type, not a value",
    );
}

// --- B.27 regression guards: these value forms must still compile --------------

#[test]
fn an_enum_variant_and_struct_literal_stay_values() {
    assert_compiles_and_runs(
        r#"
        import std::print;

        enum Color {
            Red,
            Green,
        }

        [derive(PartialEq)]
        struct Point {
            x: i32,
        }

        fun main() {
            let c = Color::Red;
            print(c is Color::Red);
            let p = Point { x = 1 };
            print(p == Point { x = 1 });
        }
        "#,
        "true\ntrue\n",
    );
}

#[test]
fn a_bare_function_name_stays_a_value() {
    // B20 fn→closure coercion: a function used as a value (here coerced to a
    // closure parameter) is not rejected — only type-like names are.
    assert_compiles_and_runs(
        r#"
        import std::print;

        fun apply(f: |i32| i32, x: i32): i32 {
            f(x)
        }

        fun double(x: i32): i32 {
            x * 2
        }

        fun main() {
            print(apply(double, 21));
        }
        "#,
        "42\n",
    );
}

// --- I3: validating per-type `from_json` -----------------------------------------
// Decoding is fallible and never crashes: a missing field, a wrong-shaped value,
// or text that is not JSON is a `Result` decode error rather than `undefined`
// garbage or a thrown `JSON.parse`. Both `FromJson` methods return
// `Result<Self, str>`; the `!` operator threads a leaf failure.

#[test]
fn from_json_decodes_a_valid_scalar() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::result::Result::{ self, Ok, Err };

        fun main() {
            print(i32::from_json("7") is Ok(let n) && n == 7);
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_rejects_a_wrong_typed_scalar() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::result::Result::{ self, Ok, Err };

        fun main() {
            print(i32::from_json("\"x\"") is Err(let e));
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_rejects_malformed_text() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::result::Result::{ self, Ok, Err };

        fun main() {
            print(i32::from_json("not json") is Err(let e) && e == "not valid JSON");
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_names_a_missing_struct_field() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::FromJson;
        import std::result::Result::{ self, Ok, Err };

        [derive(Json)]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            match Point::from_json("{\"x\":1}") {
                Ok(_) => print("?"),
                Err(let reason) => print(reason),
            }
        }
        "#,
        "missing field y\n",
    );
}

#[test]
fn from_json_rejects_a_wrong_typed_struct_field() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::FromJson;
        import std::result::Result::{ self, Ok, Err };

        [derive(Json)]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            print(Point::from_json("{\"x\":1,\"y\":\"z\"}") is Err(let e));
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_ignores_extra_struct_fields() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::FromJson;
        import std::result::Result::{ self, Ok, Err };

        [derive(Json)]
        struct Point {
            x: i32,
            y: i32,
        }

        fun main() {
            print(Point::from_json("{\"x\":1,\"y\":2,\"z\":3}") is Ok(let p) && p.x == 1);
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_recurses_into_a_nested_struct() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::FromJson;
        import std::result::Result::{ self, Ok, Err };

        [derive(Json)]
        struct Point {
            x: i32,
        }

        [derive(Json)]
        struct Line {
            from: Point,
            to: Point,
        }

        fun main() {
            // The inner `Point` is missing its field — the failure propagates.
            print(Line::from_json("{\"from\":{\"x\":1},\"to\":{}}") is Err(let e));
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_reads_option_null_and_value() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::option::Option::{ self, Some, None };
        import std::result::Result::{ self, Ok, Err };

        fun main() {
            let empty: Result<Option<i32>, str> = Option::from_json("null");
            print(empty is Ok(let a) && a is None);
            let some: Result<Option<i32>, str> = Option::from_json("7");
            print(some is Ok(let b) && b is Some(let v) && v == 7);
        }
        "#,
        "true\ntrue\n",
    );
}

#[test]
fn from_json_rejects_a_non_array_for_a_list() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::result::Result::{ self, Ok, Err };

        fun main() {
            let bad: Result<List<i32>, str> = List::from_json("5");
            print(bad is Err(let e) && e == "expected an array");
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_short_circuits_on_a_bad_list_element() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::result::Result::{ self, Ok, Err };

        fun main() {
            let good: Result<List<i32>, str> = List::from_json("[1,2,3]");
            print(good is Ok(let xs) && xs.len() == 3);
            let bad: Result<List<i32>, str> = List::from_json("[1,\"x\",3]");
            print(bad is Err(let e));
        }
        "#,
        "true\ntrue\n",
    );
}

#[test]
fn from_json_rejects_an_unknown_enum_variant() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::FromJson;
        import std::result::Result::{ self, Ok, Err };

        [derive(Json)]
        enum Shape {
            Circle(i32),
            Empty,
        }

        fun main() {
            print(Shape::from_json("\"Triangle\"") is Err(let e));
        }
        "#,
        "true\n",
    );
}

#[test]
fn from_json_round_trips_a_derived_enum() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::json::FromJson;
        import std::result::Result::{ self, Ok, Err };

        [derive(Json, PartialEq)]
        enum Shape {
            Circle(i32),
            Rect(i32, i32),
            Empty,
        }

        fun main() {
            let r = Shape::Rect(2, 3);
            print(Shape::from_json(r.to_json()) is Ok(let back) && back == r);
        }
        "#,
        "true\n",
    );
}

// --- I1: value-keyed Map/Set via Hashable ---------------------------------------
// Map/Set key by value: a struct/enum/List key works (via `[derive(Hashable)]`
// or a hand-written impl), a fresh equal key hits, and `key.hash()` is dispatched
// so a custom impl is honored inside std collections.

#[test]
fn a_derived_struct_key_maps_by_value() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::map::Map;
        import std::hash::Hashable;
        import std::option::Option::{ self, Some, None };

        [derive(Hashable)]
        struct Point { x: i32, y: i32 }

        fun main() {
            mut m: Map<Point, str> = Map::new();
            m.insert(Point { x = 1, y = 2 }, "here");
            // A FRESH, distinct-but-equal Point hits.
            match m.get(Point { x = 1, y = 2 }) {
                Some(let v) => print(v),
                None => print("miss"),
            }
            print(m.contains_key(Point { x = 9, y = 9 }));
        }
        "#,
        "here\nfalse\n",
    );
}

#[test]
fn a_set_dedups_struct_elements_by_value() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::set::Set;
        import std::hash::Hashable;

        [derive(Hashable)]
        struct Point { x: i32, y: i32 }

        fun main() {
            mut s: Set<Point> = Set::new();
            s.insert(Point { x = 1, y = 2 });
            s.insert(Point { x = 1, y = 2 });   // dup by value
            s.insert(Point { x = 3, y = 4 });
            print(s.len());                      // 2
            print(s.contains(Point { x = 1, y = 2 }));
        }
        "#,
        "2\ntrue\n",
    );
}

#[test]
fn a_derived_enum_is_a_valid_key() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::set::Set;
        import std::hash::Hashable;

        [derive(Hashable)]
        enum Shape { Circle(i32), Rect(i32, i32), Empty }

        fun main() {
            mut s: Set<Shape> = Set::new();
            s.insert(Shape::Circle(5));
            s.insert(Shape::Circle(5));   // dup by value
            s.insert(Shape::Empty);
            print(s.len());               // 2
            print(s.contains(Shape::Circle(5)));
        }
        "#,
        "2\ntrue\n",
    );
}

#[test]
fn a_custom_hashable_impl_is_honored_by_map() {
    // Genuine per-call dispatch: a hand-written `hash()` (by one field) is used
    // inside the std Map, so two values that hash equal collide.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::map::Map;
        import std::hash::{ Hashable, Hash };

        struct User { id: i32, name: str }
        impl User with Hashable {
            fun hash(self): Hash {
                self.id.hash()
            }
        }

        fun main() {
            mut m: Map<User, str> = Map::new();
            m.insert(User { id = 1, name = "Ada" }, "a");
            m.insert(User { id = 1, name = "Bob" }, "b");   // same id -> overwrites
            print(m.len());                                  // 1
        }
        "#,
        "1\n",
    );
}

#[test]
fn a_list_is_a_valid_key() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::map::Map;
        import std::hash::Hashable;
        import std::option::Option::{ self, Some, None };

        fun main() {
            mut m: Map<List<i32>, str> = Map::new();
            m.insert([1, 2, 3], "here");
            match m.get([1, 2, 3]) {
                Some(let v) => print(v),
                None => print("miss"),
            }
        }
        "#,
        "here\n",
    );
}

#[test]
fn map_keys_and_set_iteration_return_real_values() {
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::map::Map;
        import std::set::Set;
        import std::hash::Hashable;

        [derive(Hashable, Debug)]
        struct Point { x: i32, y: i32 }

        fun main() {
            mut m: Map<Point, i32> = Map::new();
            m.insert(Point { x = 1, y = 2 }, 10);
            for key in m.keys() { print(key.debug()); }   // Point { x = 1, y = 2 }
            mut s: Set<i32> = Set::new();
            s.insert(7);
            s.insert(8);
            for x in s { print(x); }                       // 7, 8
        }
        "#,
        "Point { x = 1, y = 2 }\n7\n8\n",
    );
}

#[test]
fn a_non_hashable_field_is_rejected_by_the_derive() {
    // The all-fields check: a closure field can't be canonically hashed.
    assert_fails(
        r#"
        import std::hash::Hashable;

        [derive(Hashable)]
        struct Handler { name: str, callback: || void }

        fun main() {}
        "#,
    );
}

#[test]
fn an_aggregate_key_is_snapshot_on_insert() {
    // Value semantics: the key is copied into the map, so mutating the original
    // afterward can't desync it (§3.6).
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::map::Map;
        import std::hash::Hashable;
        import std::option::Option::{ self, Some, None };

        fun main() {
            mut xs: List<i32> = [1, 2];
            mut m: Map<List<i32>, str> = Map::new();
            m.insert(xs, "here");
            xs.push(3);                        // mutate the original AFTER insert
            print(m.contains_key([1, 2]));     // true  — snapshot held
            print(m.contains_key([1, 2, 3]));  // false — the mutation didn't leak
        }
        "#,
        "true\nfalse\n",
    );
}

#[test]
fn hashable_builds_a_reusable_container() {
    // The point of a trait-with-a-value (not a marker): a user bounds their own
    // container on `K: Hashable`, calls `key.hash()`, and keys a `Map<Hash, ..>`.
    assert_compiles_and_runs(
        r#"
        import std::print;
        import std::map::Map;
        import std::hash::{ Hashable, Hash };
        import std::option::Option::{ self, Some, None };

        struct Counter<K: Hashable> {
            counts: Map<Hash, i32>,
        }

        impl Counter<type K: Hashable> {
            fun new(): Counter<K> {
                let counts: Map<Hash, i32> = Map::new();
                Counter { counts = counts }
            }
            fun bump(&mut self, key: K) {
                let h = key.hash();
                let current = match self.counts.get(h) {
                    Some(let n) => n,
                    None => 0,
                };
                self.counts.insert(h, current + 1);
            }
            fun count(self, key: K): i32 {
                match self.counts.get(key.hash()) {
                    Some(let n) => n,
                    None => 0,
                }
            }
        }

        [derive(Hashable)]
        struct Word { text: str }

        fun main() {
            mut c: Counter<Word> = Counter::new();
            c.bump(Word { text = "hi" });
            c.bump(Word { text = "hi" });
            c.bump(Word { text = "bye" });
            print(c.count(Word { text = "hi" }));   // 2
            print(c.count(Word { text = "bye" }));  // 1
        }
        "#,
        "2\n1\n",
    );
}

// --- C5.1: a scalar view read as a value requires `*` -----------------------------
// `transparent-references.md`: `*v` is the only way to cross from view to value —
// the language never silently converts. A bare scalar view (whose runtime form is
// the `(base, key)` pair) in a value position used to leak that pair; now it's an
// error, mirroring the let-binding rule (R1).

#[test]
fn a_scalar_view_read_as_a_value_is_rejected() {
    // `print(b)` for `let b = &mut a[0]` would leak `[[99],0]`.
    assert_fails(
        r#"
        import std::print;
        fun main() {
            mut a = [99];
            let b = &mut a[0];
            print(b);
        }
        "#,
    );
}

#[test]
fn a_scalar_view_as_a_value_parameter_is_rejected() {
    assert_fails(
        r#"
        fun take_value(x: i32): i32 { x }
        fun main() {
            mut a = [99];
            let b = &mut a[0];
            let _ = take_value(b);
        }
        "#,
    );
}

#[test]
fn a_scalar_view_as_a_binary_operand_is_rejected() {
    assert_fails(
        r#"
        import std::print;
        fun main() {
            mut a = [99];
            let b = &mut a[0];
            print(b + 1);
        }
        "#,
    );
}

#[test]
fn an_explicit_deref_reads_the_scalar_view() {
    // The fix steers to `*b`, which reads the element.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun main() {
            mut a = [99];
            let b = &mut a[0];
            print(*b);       // 99
            print(*b + 1);   // 100
        }
        "#,
        "99\n100\n",
    );
}

#[test]
fn a_scalar_view_passes_to_a_view_parameter() {
    // A view binding is still allowed as a view argument (aliasing) and for a
    // compound write-through — neither is a value read.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun bump(v: &mut i32) { v = *v + 1; }
        fun main() {
            mut a = [99];
            let b = &mut a[0];
            bump(b);      // aliasing — not a value read
            b += 5;       // compound write-through — sanctioned
            print(*b);    // 105
        }
        "#,
        "105\n",
    );
}

#[test]
fn a_mut_bool_view_writes_through() {
    // C5.3: `bool` is a numeric enum, so it used to take the aggregate view path
    // (`Object.assign`) — a no-op write. It's a scalar `(base, key)` view now.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun set_true(v: &mut bool) { v = true; }
        fun main() {
            mut flags = [false, false];
            let b = &mut flags[0];
            set_true(b);          // writes through
            print(*b);            // true
            print(flags[0]);      // true — the write reached the list
            print(flags[1]);      // false — untouched
        }
        "#,
        "true\ntrue\nfalse\n",
    );
}

#[test]
fn a_mut_bool_view_toggles_through_a_negated_deref() {
    // C5.3 + the operator-lexer fix: the natural thing to do with a `&mut bool`
    // view is toggle it, `v = !*v`. That failed to *parse* before — the lexer
    // fused `!*` into one bogus token — so the scalar-bool view shipped without
    // an ergonomic toggle. Now it reads through (`*v`), negates, and writes back.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun toggle(v: &mut bool) { v = !*v; }
        fun main() {
            mut flags = [true, false];
            toggle(&mut flags[0]);   // transient views — none outlive its call
            toggle(&mut flags[1]);
            print(flags[0]);   // false
            print(flags[1]);   // true
        }
        "#,
        "false\ntrue\n",
    );
}

#[test]
fn a_mut_bool_view_of_a_scalar_local_writes_through() {
    // C5.3 gap (found verifying the v0.6.0 release): a view of a scalar *local*
    // must box the local to `[value]` so the `(base, key)` pair has a real cell.
    // `bool` is a numeric enum, so `compute_boxed_locals` (keyed on
    // `is_scalar_primitive`, structs only) skipped it — `&mut b` lowered to
    // `[b, 0]` over the raw value and the write-through no-oped. The earlier bool
    // pins used list elements (base already an object), so they missed it.
    assert_compiles_and_runs(
        r#"
        import std::print;
        fun toggle(v: &mut bool) { v = !*v; }
        fun main() {
            mut b = true;
            toggle(&mut b);      // through a call
            print(b);            // false
            let w = &mut b;      // direct local view
            w = true;
            print(b);            // true
        }
        "#,
        "false\ntrue\n",
    );
}
