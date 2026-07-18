//! The macro engine's conformance gate (proposal/macro-engine.md §5): the
//! fueled `js::Node` interpreter must agree with a real JS engine on every
//! corpus program inside its subset. Each admitted program is compiled once,
//! then executed BOTH ways — formatted and run under node, and evaluated by
//! `interpreter::run_program` — and the (stdout, exit code) pairs must match
//! exactly. Programs outside the subset (async, host capabilities) are listed
//! with the reason; everything else MUST pass, so a pure program regressing
//! into "unsupported" fails the suite rather than silently skipping.

use std::path::{Path, PathBuf};

use vilan_core::interpreter::{self, FailureKind, Limits};
use vilan_core::{
    BuildOptions, PackageSpec, Platform, Workspace, analyze_source, transform, transform_to_ast,
};

fn std_spec() -> PackageSpec {
    vilan_core::manifest::resolve_std(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"),
    )
}

/// Corpus files outside the interpreter's subset, with the capability that
/// excludes them. Everything not listed here must pass the equivalence check.
const EXCLUDED: &[(&str, &str)] = &[
    ("adapt.vl", "async (adapted instances await)"),
    ("async-await.vl", "async"),
    ("async-promise-all.vl", "async"),
    (
        "reactive-turns.vl",
        "async (the turn-follows-continuation section)",
    ),
    ("process-env.vl", "host environment (`__env`, `__args`)"),
    ("crypto.vl", "async + host WebCrypto (`crypto.subtle`)"),
    ("db.vl", "host database (`node:sqlite`)"),
    (
        "time.vl",
        "host clock + timers (`Date.now`, `Date#toISOString`, `setTimeout`)",
    ),
];

/// Runs `source` through the pipeline once, then both execution paths.
/// Returns `(node stdout, node exit code, interpreter result)`.
#[allow(clippy::type_complexity)]
fn both_ways(
    source: String,
    fuel: u64,
) -> Result<(String, i32, Result<(String, i32), (FailureKind, String)>), String> {
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
            let program = match program {
                Some(program) if errors.is_empty() => program,
                _ => return Err(format!("compile failed: {errors:?}")),
            };
            let options = BuildOptions::default();

            // Path 1: the formatter + a real JS engine.
            let text = transform(&program, &options).map_err(|error| error.msg)?;
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("vilan_equiv_{}_{unique}.js", std::process::id()));
            std::fs::write(&path, text).map_err(|error| error.to_string())?;
            let output = std::process::Command::new("timeout")
                .arg("30")
                .arg("node")
                .arg(&path)
                .output()
                .map_err(|error| format!("could not run node: {error}"))?;
            let _ = std::fs::remove_file(&path);
            let node_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let node_exit = output.status.code().unwrap_or(-1);

            // Path 2: the interpreter over the transformer's own AST.
            let ast = transform_to_ast(&program, &options).map_err(|error| error.msg)?;
            let interpreted = match interpreter::run_program(
                &ast,
                Limits {
                    fuel,
                    call_depth: 2048,
                },
            ) {
                Ok(run) => Ok((run.stdout, run.exit_code)),
                Err(failure) => Err((failure.kind, failure.message)),
            };
            Ok((node_stdout, node_exit, interpreted))
        })
        .expect("spawn worker")
        .join()
        .unwrap_or_else(|_| Err("worker thread aborted".to_string()))
}

#[test]
fn every_admitted_corpus_program_is_equivalent_interpreted() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/test");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("corpus directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "vl").then_some(path)
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no corpus programs found");

    let mut failures = Vec::new();
    let mut checked = 0;
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if EXCLUDED.iter().any(|(excluded, _)| *excluded == name) {
            continue;
        }
        let source = std::fs::read_to_string(path).expect("read corpus file");
        match both_ways(source, 50_000_000) {
            Ok((node_stdout, node_exit, Ok((interp_stdout, interp_exit)))) => {
                checked += 1;
                if node_stdout != interp_stdout || node_exit != interp_exit {
                    let first_diff = node_stdout
                        .lines()
                        .zip(interp_stdout.lines())
                        .enumerate()
                        .find(|(_, (a, b))| a != b)
                        .map(|(line, (a, b))| format!("line {}: node {a:?} vs interp {b:?}", line + 1))
                        .unwrap_or_else(|| {
                            format!(
                                "lengths/exits differ (node {} lines exit {node_exit}, interp {} lines exit {interp_exit})",
                                node_stdout.lines().count(),
                                interp_stdout.lines().count()
                            )
                        });
                    failures.push(format!("{name}: {first_diff}"));
                }
            }
            Ok((_, _, Err((kind, message)))) => {
                failures.push(format!("{name}: interpreter failed ({kind:?}): {message}"));
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus programs diverged:\n{}",
        failures.len(),
        checked + failures.len(),
        failures.join("\n")
    );
    assert!(checked > 60, "suspiciously few programs checked: {checked}");
}

// --- Failure-mode pins -------------------------------------------------------

/// Compiles and runs the INTERPRETER ONLY — no node. The failure-mode pins
/// exercise programs a real JS engine would run forever (that's the point of
/// fuel), so they must never reach `both_ways`'s node half.
fn interpret(source: &str, fuel: u64) -> Result<(String, i32), (FailureKind, String)> {
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
            let program = match program {
                Some(program) if errors.is_empty() => program,
                _ => panic!("compile failed: {errors:?}"),
            };
            let ast = transform_to_ast(&program, &BuildOptions::default())
                .unwrap_or_else(|error| panic!("transform failed: {}", error.msg));
            match interpreter::run_program(
                &ast,
                Limits {
                    fuel,
                    call_depth: 2048,
                },
            ) {
                Ok(run) => Ok((run.stdout, run.exit_code)),
                Err(failure) => Err((failure.kind, failure.message)),
            }
        })
        .expect("spawn worker")
        .join()
        .expect("worker thread aborted")
}

#[test]
fn fuel_exhaustion_is_a_clean_error() {
    let (kind, message) = interpret(
        r#"
        fun main() {
            mut n = 0;
            for {
                n = n + 1;
            }
        }

        main();
        "#,
        10_000,
    )
    .expect_err("an infinite loop must exhaust fuel");
    assert_eq!(kind, FailureKind::Fuel);
    assert!(message.contains("fuel"), "unexpected message: {message}");
}

#[test]
fn runaway_recursion_hits_the_depth_cap() {
    let (kind, _) = interpret(
        r#"
        fun forever(n: i32): i32 {
            forever(n + 1)
        }

        fun main() {
            forever(0);
        }

        main();
        "#,
        50_000_000,
    )
    .expect_err("unbounded recursion must hit the depth cap");
    assert_eq!(kind, FailureKind::Depth);
}

#[test]
fn an_impure_capability_is_a_clean_unsupported_error() {
    let (kind, message) = interpret(
        r#"
        import std::random;
        import std::print;

        fun main() {
            print(random::range_i32(1, 6));
        }

        main();
        "#,
        1_000_000,
    )
    .expect_err("randomness must be unavailable at expansion time");
    assert_eq!(kind, FailureKind::Unsupported);
    assert!(
        message.contains("not available at expansion time"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_panic_surfaces_as_thrown_with_its_message() {
    let (kind, message) = interpret(
        r#"
        import std::io::panic;

        fun main() {
            panic("boom at expansion time");
        }

        main();
        "#,
        1_000_000,
    )
    .expect_err("panic must surface as a thrown failure");
    assert_eq!(kind, FailureKind::Thrown);
    assert!(
        message.contains("boom at expansion time"),
        "unexpected message: {message}"
    );
}
