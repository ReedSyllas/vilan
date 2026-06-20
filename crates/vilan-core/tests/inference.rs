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
fn reactive_derive_sub_and_set_with() {
    assert_compiles(
        r#"
        import std::print;
        import std::reactive::Signal;
        fun main() {
            let count = Signal::new(0);
            let doubled = count.derive(|n| n * 2);
            doubled.sub(|n| print(n));
            count.set_with(|n| n + 1);
        }
        "#,
    );
}

#[test]
fn generic_dispatch_to_extern_impl() {
    // A trait method on a generic, dispatching to a primitive's `@extern` impl.
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
    // its own. (Bug B in disguise — `Signal::new(0).derive(|n| ..)` left `n`
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
    // Bug B (fixed): a closure passed to a generic method (`count.derive(|n|
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
            let label = count.derive(|n| n.to_string());
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
    // A chained `derive` (`count.derive(|n| n * 2).derive(|m| format(m))`) used to
    // emit `undefined`: the first `derive<U>` left its result `Source<U>` abstract
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
            let label = count.derive(|n| n * 2).derive(|m| format(m));
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
    // parameter (`count.derive(|n| format(n))`) emitted `undefined`. The call
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
            let label = count.derive(|n| format(n));
            label.sub(|s| print(s));
            count.set(5);
        }
        "#,
        "0\n5\n",
    );
}
