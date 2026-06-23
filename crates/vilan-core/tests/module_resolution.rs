//! Filesystem-backed tests for package-module resolution (P1): a `pkg::` module
//! resolves equivalently whether it's a flat `foo.vl` or a directory `foo/lib.vl`,
//! both existing is an ambiguity error, and the `none` target gates out the
//! platform `std` layers. These need real files on disk (the loader reads them),
//! so each writes a throwaway package directory and analyzes against it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use vilan_core::{Error, PackageSpec, Target, Workspace, analyze_source};

fn std_spec() -> PackageSpec {
    vilan_core::manifest::resolve_std(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"),
    )
}

/// Writes `files` (relative path → contents) into a fresh temp package directory,
/// analyzes `entry` (also relative) against it as `pkg_root`, and returns the raw
/// diagnostics (message + span). The directory is removed before returning.
fn analyze_package_raw(files: &[(&str, &str)], entry: &str, target: Target) -> Vec<Error> {
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
        &std_spec(),
        &dir,
        &entry_path,
        Some(target),
        &Workspace::default(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    errors
}

/// As [`analyze_package_raw`], but just the diagnostic messages.
fn analyze_package(files: &[(&str, &str)], entry: &str, target: Target) -> Vec<String> {
    analyze_package_raw(files, entry, target)
        .into_iter()
        .map(|error| error.msg)
        .collect()
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
            base_root: dep_root,
            target_roots: Vec::new(),
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
        &std_spec(),
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
            base_root: dep_root,
            target_roots: Vec::new(),
            dependencies: Vec::new(),
        }],
        entry_dependencies: vec![("common".to_string(), 0)],
    };
    let entry_path = app_dir.join("main.vl");
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_spec(),
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

// --- Cross-target import error recovery (P3) -------------------------------

#[test]
fn cross_target_std_import_does_not_cascade() {
    // A browser build of a Node program: the two cross-target imports are reported,
    // but `std::http`/`std::fs` still load for typing, so `Server`,
    // `read_file_to_str`, etc. resolve and there's no unresolved-name cascade.
    let entry = concat!(
        "import std::http::{ Server, Response };\n",
        "import std::fs::read_file_to_str;\n",
        "fun main() {\n",
        "    let data = read_file_to_str(\"x.txt\");\n",
        "    let server = Server::builder().port(3000).build();\n",
        "    server.start();\n",
        "}\n",
    );
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", Target::Browser);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("std::http") && e.contains("not available")),
        "missing the std::http cross-target error: {errors:#?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("std::fs") && e.contains("not available")),
        "missing the std::fs cross-target error: {errors:#?}"
    );
    assert!(
        !errors.iter().any(|e| e.contains("cannot find")),
        "expected no cascade, got: {errors:#?}"
    );
}

#[test]
fn cross_target_diagnostic_is_spanned() {
    // The error points at the offending `import` (not EMPTY_SPAN at 0..0).
    let entry = "import std::http::Server;\nfun main() { Server::builder(); }\n";
    let errors = analyze_package_raw(&[("main.vl", entry)], "main.vl", Target::Browser);
    let http = errors
        .iter()
        .find(|e| e.msg.contains("std::http") && e.msg.contains("not available"))
        .expect("a std::http cross-target error");
    let range = http.span.into_range();
    assert!(
        range.start < range.end && range.start > 0,
        "expected a real import span, got {range:?}"
    );
}

#[test]
fn cross_target_transitive_import_not_reported() {
    // Importing `std::http` reports `http` once; the modules it pulls in
    // transitively (std-internal) load but are not separately gated.
    let entry = "import std::http::Server;\nfun main() { Server::builder(); }\n";
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", Target::Browser);
    let cross_target = errors
        .iter()
        .filter(|e| e.contains("not available"))
        .count();
    assert_eq!(
        cross_target, 1,
        "expected exactly one cross-target error (http), got: {errors:#?}"
    );
    assert!(errors.iter().any(|e| e.contains("std::http")));
}

#[test]
fn platform_modules_load_for_typing_under_opposite_target() {
    // Loading a cross-target std module purely to type-check it must not introduce
    // spurious errors beyond the single cross-target diagnostic (P3 Q5 sweep).
    for (module, target) in [
        ("http", Target::Browser),
        ("fs", Target::Browser),
        ("process", Target::Browser),
        ("dom", Target::Node),
        ("ui", Target::Node),
    ] {
        let entry = format!("import std::{module};\nfun main() {{}}\n");
        let errors = analyze_package(&[("main.vl", &entry)], "main.vl", target);
        assert_eq!(
            errors.len(),
            1,
            "`std::{module}` under {target:?} should yield exactly the one cross-target \
             error (loading-for-typing introduced no others): {errors:#?}"
        );
        assert!(
            errors[0].contains(module) && errors[0].contains("not available"),
            "`std::{module}` under {target:?}: unexpected error: {errors:#?}"
        );
    }
}

// --- Library target layers (L1) --------------------------------------------

/// Sets up a library `plat` with layers — a base module `shared`, a `node`-overlay
/// `nodeonly`, and a `clock` present in both overlays (node returns `i32`, browser
/// `str`) — and an empty base `lib.vl`. Analyzes `entry` (which imports from
/// `plat`) for `target`, returning the diagnostics.
fn analyze_layered(entry: &str, target: Target) -> Vec<String> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("vilan_layer_{}_{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    let entry_path = app.join("main.vl");
    std::fs::write(&entry_path, entry).unwrap();

    let plat = root.join("plat");
    let put = |rel: &str, contents: &str| {
        let path = plat.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };
    put("src/lib.vl", "");
    put("src/shared.vl", "fun shared(): i32 { 0 }\n");
    put("src/node/nodeonly.vl", "fun nodeonly(): i32 { 1 }\n");
    put("src/node/clock.vl", "fun clock(): i32 { 1 }\n");
    put("src/browser/clock.vl", "fun clock(): str { \"x\" }\n");

    let workspace = Workspace {
        packages: vec![PackageSpec {
            base_root: plat.join("src"),
            target_roots: vec![
                (Target::Node, plat.join("src/node")),
                (Target::Browser, plat.join("src/browser")),
            ],
            dependencies: Vec::new(),
        }],
        entry_dependencies: vec![("plat".to_string(), 0)],
    };
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_spec(),
        &app,
        &entry_path,
        Some(target),
        &workspace,
    );
    let _ = std::fs::remove_dir_all(&root);
    errors.into_iter().map(|error| error.msg).collect()
}

#[test]
fn base_module_available_for_all_targets() {
    let entry = "import plat::shared::shared;\nfun main() { shared() }\n";
    assert!(analyze_layered(entry, Target::Node).is_empty());
    assert!(analyze_layered(entry, Target::Browser).is_empty());
}

#[test]
fn overlay_module_available_only_for_its_target() {
    let entry = "import plat::nodeonly::nodeonly;\nfun main() { nodeonly() }\n";
    assert!(
        analyze_layered(entry, Target::Node).is_empty(),
        "the node overlay module is available for a node build"
    );
    let browser = analyze_layered(entry, Target::Browser);
    assert!(
        browser.iter().any(|e| e.contains("another target's layer")),
        "expected a cross-target error for browser, got: {browser:#?}"
    );
    assert!(
        !browser.iter().any(|e| e.contains("cannot find")),
        "the module still loads for typing (no cascade): {browser:#?}"
    );
}

#[test]
fn varying_module_resolves_the_target_version() {
    // `clock` is `i32` in the node overlay, `str` in the browser overlay. Passing it
    // to an `i32` parameter type-checks for node and fails for browser — proving the
    // build target's version loaded (the P4 case, structurally).
    let entry = concat!(
        "import plat::clock::clock;\n",
        "fun need_int(n: i32) {}\n",
        "fun main() { need_int(clock()) }\n",
    );
    assert!(
        analyze_layered(entry, Target::Node).is_empty(),
        "node `clock` is i32"
    );
    assert!(
        !analyze_layered(entry, Target::Browser).is_empty(),
        "browser `clock` is str — a type mismatch, proving the browser version loaded"
    );
}

#[test]
fn base_lib_reexporting_an_overlay_module_errors() {
    // A library whose base `lib.vl` re-exports `nodeonly` (a node-overlay module):
    // the public surface must be target-agnostic, so this is a Q4 violation.
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("vilan_q4_{}_{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    let entry_path = app.join("main.vl");
    std::fs::write(
        &entry_path,
        "import plat::shared::shared;\nfun main() { shared() }\n",
    )
    .unwrap();
    let plat = root.join("plat");
    let put = |rel: &str, contents: &str| {
        let path = plat.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };
    put("src/lib.vl", "export import pkg::nodeonly::nodeonly;\n");
    put("src/shared.vl", "fun shared(): i32 { 0 }\n");
    put("src/node/nodeonly.vl", "fun nodeonly(): i32 { 1 }\n");
    let workspace = Workspace {
        packages: vec![PackageSpec {
            base_root: plat.join("src"),
            target_roots: vec![(Target::Node, plat.join("src/node"))],
            dependencies: Vec::new(),
        }],
        entry_dependencies: vec![("plat".to_string(), 0)],
    };
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_spec(),
        &app,
        &entry_path,
        Some(Target::Node),
        &workspace,
    );
    let _ = std::fs::remove_dir_all(&root);
    let errors: Vec<String> = errors.into_iter().map(|error| error.msg).collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("re-exports") && e.contains("nodeonly")),
        "expected a base-lib re-export error, got: {errors:#?}"
    );
}
