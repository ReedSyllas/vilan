//! Filesystem-backed tests for package-module resolution (P1): a `pkg::` module
//! resolves equivalently whether it's a flat `foo.vl` or a directory `foo/lib.vl`,
//! both existing is an ambiguity error, and the `none` target gates out the
//! platform `std` layers. These need real files on disk (the loader reads them),
//! so each writes a throwaway package directory and analyzes against it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use vilan_core::{Target, analyze_source};

fn std_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std/src")
}

/// Writes `files` (relative path → contents) into a fresh temp package directory,
/// analyzes `entry` (also relative) against it as `pkg_root`, and returns the
/// diagnostic messages. The directory is removed before returning.
fn analyze_package(files: &[(&str, &str)], entry: &str, target: Target) -> Vec<String> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("vilan_modres_{}_{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (relative, contents) in files {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }
    let entry_path = dir.join(entry);
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(leaked, &std_root(), &dir, &entry_path, Some(target));
    let _ = std::fs::remove_dir_all(&dir);
    errors.into_iter().map(|error| error.msg).collect()
}

const ENTRY: &str = "import std::print;\nimport pkg::foo::bar;\nfun main() { print(bar()); }\n";
const MODULE: &str = "fun bar(): i32 { 7 }\n";

#[test]
fn flat_module_resolves() {
    let errors = analyze_package(
        &[("main.vl", ENTRY), ("foo.vl", MODULE)],
        "main.vl",
        Target::Node,
    );
    assert!(
        errors.is_empty(),
        "expected a clean compile, got: {errors:#?}"
    );
}

#[test]
fn lib_module_resolves() {
    // The directory form `foo/lib.vl` resolves identically to the flat `foo.vl`.
    let errors = analyze_package(
        &[("main.vl", ENTRY), ("foo/lib.vl", MODULE)],
        "main.vl",
        Target::Node,
    );
    assert!(
        errors.is_empty(),
        "expected a clean compile, got: {errors:#?}"
    );
}

#[test]
fn both_forms_is_ambiguous() {
    let errors = analyze_package(
        &[
            ("main.vl", ENTRY),
            ("foo.vl", MODULE),
            ("foo/lib.vl", MODULE),
        ],
        "main.vl",
        Target::Node,
    );
    assert!(
        errors.iter().any(|error| error.contains("ambiguous")),
        "expected an ambiguity error, got: {errors:#?}"
    );
}

#[test]
fn none_target_rejects_platform_std() {
    // A `none` (pure-library) target reaches only the core std layer, so importing
    // a Node-layer module is a clear platform diagnostic rather than a build.
    let entry = "import std::http;\nfun main() {}\n";
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", Target::None);
    assert!(
        errors.iter().any(|error| error.contains("not available")),
        "expected a platform-gate error, got: {errors:#?}"
    );
}

#[test]
fn none_target_allows_core_std() {
    // Core std (e.g. `print`) is universal — a `none` target still type-checks it.
    let entry = "import std::print;\nfun main() { print(1); }\n";
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", Target::None);
    assert!(
        errors.is_empty(),
        "expected a clean compile, got: {errors:#?}"
    );
}
