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

// --- Known bugs (tracked; remove `#[ignore]` when fixed) --------------------

#[test]
#[ignore = "Bug B: a closure parameter passed to a generic method types as a \
            fresh abstract generic, not the concrete receiver binding, so a \
            generic call on it (`n.to_string()`) can't dispatch. The impl's `T` \
            and the method-signature's `T` have diverged into different ids — \
            the fresh-generic-identity issue (analyzer-refactor.md item 6). \
            Workaround: annotate (`|n: i32|`) or use an i-string (`i\"{n}\"`)."]
fn generic_call_on_closure_parameter() {
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
