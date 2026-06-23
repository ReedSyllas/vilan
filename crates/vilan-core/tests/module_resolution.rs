//! Filesystem-backed tests for package-module resolution (P1): a `pkg::` module
//! resolves equivalently whether it's a flat `foo.vl` or a directory `foo/lib.vl`,
//! both existing is an ambiguity error, and the `none` target gates out the
//! platform `std` layers. These need real files on disk (the loader reads them),
//! so each writes a throwaway package directory and analyzes against it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use vilan_core::{PackageSpec, Target, Workspace, analyze_source};

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
    let (_program, errors) = analyze_source(
        leaked,
        &std_root(),
        &dir,
        &entry_path,
        Some(target),
        &Workspace::default(),
    );
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

// --- Multi-package workspaces (P2) -----------------------------------------

/// A dependency package for [`analyze_workspace`]: how the entry imports it
/// (`import_name`) and its files (relative path → contents, including a `lib.vl`).
struct Dep {
    import_name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

/// Analyzes an entry program against a set of dependency packages (P2). The entry
/// lives in its own `app/` directory; each dependency in `<import_name>/`. Builds
/// the `Workspace` (entry depends on every dep, each a `none` pure library) and
/// returns the diagnostics. Dependencies are not interdependent here (the loader's
/// transitive edges are exercised through `lib.vl` seeding within a dep).
fn analyze_workspace(entry: &str, deps: &[Dep], target: Target) -> Vec<String> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("vilan_ws_{}_{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let app_dir = root.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let entry_path = app_dir.join("main.vl");
    std::fs::write(&entry_path, entry).unwrap();

    let mut packages = Vec::new();
    let mut entry_dependencies = Vec::new();
    for (index, dep) in deps.iter().enumerate() {
        let dep_root = root.join(dep.import_name);
        for (relative, contents) in dep.files {
            let path = dep_root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
        }
        packages.push(PackageSpec {
            root: dep_root,
            target: Target::None,
            dependencies: Vec::new(),
        });
        entry_dependencies.push((dep.import_name.to_string(), index));
    }
    let workspace = Workspace {
        packages,
        entry_dependencies,
    };

    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_root(),
        &app_dir,
        &entry_path,
        Some(target),
        &workspace,
    );
    let _ = std::fs::remove_dir_all(&root);
    errors.into_iter().map(|error| error.msg).collect()
}

#[test]
fn cross_package_import_resolves() {
    let entry = "import std::print;\nimport common::greeting;\nfun main() { print(greeting()); }\n";
    let common = Dep {
        import_name: "common",
        files: &[("lib.vl", "fun greeting(): str { \"hi\" }\n")],
    };
    let errors = analyze_workspace(entry, &[common], Target::Node);
    assert!(
        errors.is_empty(),
        "expected a clean compile, got: {errors:#?}"
    );
}

#[test]
fn cross_package_submodule_resolves() {
    // `common::shape::area` descends into a submodule of the dependency, whose own
    // `pkg::` self-reference (from `lib.vl`) stays within `common`.
    let entry = "import std::print;\nimport common::shape::area;\nfun main() { print(area(2)); }\n";
    let common = Dep {
        import_name: "common",
        files: &[
            ("lib.vl", "import pkg::shape::area;\n"),
            ("shape.vl", "fun area(side: i32): i32 { side * side }\n"),
        ],
    };
    let errors = analyze_workspace(entry, &[common], Target::Node);
    assert!(
        errors.is_empty(),
        "expected a clean compile, got: {errors:#?}"
    );
}

#[test]
fn dependency_pkg_self_reference_is_isolated() {
    // The dependency's `pkg::helper` must resolve to ITS OWN `helper`, not the
    // entry's same-named module. The entry also has a `helper` with a different
    // signature; if `pkg::` leaked across packages, one side would mistype.
    let entry = concat!(
        "import std::print;\n",
        "import pkg::helper::entry_value;\n",
        "import common::greeting;\n",
        "fun main() { print(entry_value()); print(greeting()); }\n",
    );
    let common = Dep {
        import_name: "common",
        files: &[
            (
                "lib.vl",
                "import pkg::helper::dep_value;\nfun greeting(): i32 { dep_value() }\n",
            ),
            ("helper.vl", "fun dep_value(): i32 { 1 }\n"),
        ],
    };
    // The entry's own `pkg::helper` sibling lives next to the entry.
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("vilan_wsiso_{}_{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let app_dir = root.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(app_dir.join("main.vl"), entry).unwrap();
    std::fs::write(app_dir.join("helper.vl"), "fun entry_value(): i32 { 9 }\n").unwrap();
    let dep_root = root.join("common");
    for (relative, contents) in common.files {
        let path = dep_root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }
    let workspace = Workspace {
        packages: vec![PackageSpec {
            root: dep_root,
            target: Target::None,
            dependencies: Vec::new(),
        }],
        entry_dependencies: vec![("common".to_string(), 0)],
    };
    let entry_path = app_dir.join("main.vl");
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_root(),
        &app_dir,
        &entry_path,
        Some(Target::Node),
        &workspace,
    );
    let _ = std::fs::remove_dir_all(&root);
    let errors: Vec<String> = errors.into_iter().map(|error| error.msg).collect();
    assert!(
        errors.is_empty(),
        "expected a clean compile, got: {errors:#?}"
    );
}

#[test]
fn unknown_dependency_name_errors() {
    // The entry imports a package it doesn't declare — resolution finds no such
    // root and reports it (rather than silently resolving against another package).
    let entry = "import other::thing;\nfun main() {}\n";
    let common = Dep {
        import_name: "common",
        files: &[("lib.vl", "fun greeting(): str { \"hi\" }\n")],
    };
    let errors = analyze_workspace(entry, &[common], Target::Node);
    assert!(
        !errors.is_empty(),
        "expected an unresolved-import error for `other`"
    );
}
