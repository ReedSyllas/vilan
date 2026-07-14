//! Filesystem-backed tests for package-module resolution (P1): a `pkg::` module
//! resolves equivalently whether it's a flat `foo.vl` or a directory `foo/lib.vl`,
//! both existing is an ambiguity error, and the `none` platform gates out the
//! platform `std` layers. These need real files on disk (the loader reads them),
//! so each writes a throwaway package directory and analyzes against it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use vilan_core::{
    Error, Layer, MacroLimits, PackageSpec, Platform, PlatformPattern, Workspace, analyze_source,
};

fn std_spec() -> PackageSpec {
    vilan_core::manifest::resolve_std(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"),
    )
}

/// Writes `files` (relative path → contents) into a fresh temp package directory,
/// analyzes `entry` (also relative) against it as `pkg_root`, and returns the raw
/// diagnostics (message + span). The directory is removed before returning.
fn analyze_package_raw(files: &[(&str, &str)], entry: &str, platform: Platform) -> Vec<Error> {
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
        Some(platform),
        &Workspace::default(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    errors
}

/// As [`analyze_package_raw`], but just the diagnostic messages.
fn analyze_package(files: &[(&str, &str)], entry: &str, platform: Platform) -> Vec<String> {
    analyze_package_raw(files, entry, platform)
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
        Platform::default(),
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
        Platform::default(),
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
        Platform::default(),
    );
    assert!(
        errors.iter().any(|error| error.contains("ambiguous")),
        "expected an ambiguity error, got: {errors:#?}"
    );
}

#[test]
fn none_platform_rejects_reaching_platform_std() {
    // A `none` (pure-library) platform admits only base-layer code. Importing a
    // process-layer module is fine (coloring: imports are not the checkpoint);
    // REACHING it from the entry is the violation.
    let import_only = "import std::http;\nfun main() {}\n";
    let errors = analyze_package(&[("main.vl", import_only)], "main.vl", Platform::None);
    assert!(errors.is_empty(), "import alone is legal: {errors:#?}");

    let reaching = "import std::http::Server;\nfun main() { Server::builder(); }\n";
    let errors = analyze_package(&[("main.vl", reaching)], "main.vl", Platform::None);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("requires the `process` layer of `std`")),
        "expected a platform-coloring violation, got: {errors:#?}"
    );
}

#[test]
fn none_platform_allows_base_std() {
    // Base std (e.g. `print`) is universal — a `none` platform still type-checks it.
    let entry = "import std::print;\nfun main() { print(1); }\n";
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", Platform::None);
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
/// the `Workspace` (entry depends on every dep, each a base-only pure library) and
/// returns the diagnostics. Dependencies are not interdependent here (the loader's
/// transitive edges are exercised through `lib.vl` seeding within a dep).
fn analyze_workspace(entry: &str, deps: &[Dep], platform: Platform) -> Vec<String> {
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
            layers: Vec::new(),
            dependencies: Vec::new(),
            surface: true,
        });
        entry_dependencies.push((dep.import_name.to_string(), index));
    }
    let workspace = Workspace {
        packages,
        entry_dependencies,
        macro_limits: MacroLimits::default(),
    };

    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_spec(),
        &app_dir,
        &entry_path,
        Some(platform),
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
    let errors = analyze_workspace(entry, &[common], Platform::default());
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
    let errors = analyze_workspace(entry, &[common], Platform::default());
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
            layers: Vec::new(),
            dependencies: Vec::new(),
            surface: true,
        }],
        entry_dependencies: vec![("common".to_string(), 0)],
        macro_limits: MacroLimits::default(),
    };
    let entry_path = app_dir.join("main.vl");
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_spec(),
        &app_dir,
        &entry_path,
        Some(Platform::default()),
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
    let errors = analyze_workspace(entry, &[common], Platform::default());
    assert!(
        !errors.is_empty(),
        "expected an unresolved-import error for `other`"
    );
}

// --- Cross-platform import error recovery (P3) -----------------------------

#[test]
fn cross_platform_std_import_does_not_cascade() {
    // A browser build of a Node program: the two cross-platform imports are reported,
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
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", Platform::Browser);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`read_file_to_str` requires the `process` layer of `std`")),
        "missing the fs boundary violation: {errors:#?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("requires the `process` layer of `std`")
                && e.contains("main → builder")),
        "missing the http boundary violation: {errors:#?}"
    );
    assert!(
        !errors.iter().any(|e| e.contains("cannot find")),
        "expected no cascade, got: {errors:#?}"
    );
}

#[test]
fn cross_platform_diagnostic_is_spanned() {
    // The violation anchors at the user CALL SITE (not EMPTY_SPAN at 0..0, and
    // not the import — proposal/platform-coloring.md §3.6).
    let entry = "import std::http::Server;\nfun main() { Server::builder(); }\n";
    let errors = analyze_package_raw(&[("main.vl", entry)], "main.vl", Platform::Browser);
    let http = errors
        .iter()
        .find(|e| e.msg.contains("requires the `process` layer of `std`"))
        .expect("a platform-coloring violation");
    let range = http.span.into_range();
    assert!(
        range.start < range.end && range.start > 0,
        "expected a real call span, got {range:?}"
    );
}

#[test]
fn cross_platform_transitive_import_not_reported() {
    // Importing `std::http` reports `http` once; the modules it pulls in
    // transitively (std-internal) load but are not separately gated.
    let entry = "import std::http::Server;\nfun main() { Server::builder(); }\n";
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", Platform::Browser);
    let violations = errors
        .iter()
        .filter(|e| e.contains("cannot run on"))
        .count();
    assert_eq!(
        violations, 1,
        "one violation at the boundary, not one per function inside the layer: {errors:#?}"
    );
    assert!(errors.iter().any(|e| e.contains("main → builder")));
}

#[test]
fn platform_modules_load_for_typing_under_opposite_platform() {
    // Loading a cross-platform std module purely to type-check it must not introduce
    // spurious errors beyond the single cross-platform diagnostic (P3 Q5 sweep).
    for (module, platform) in [
        ("http", Platform::Browser),
        ("fs", Platform::Browser),
        ("process", Platform::Browser),
        ("dom", Platform::default()),
        ("ui", Platform::default()),
    ] {
        let entry = format!("import std::{module};\nfun main() {{}}\n");
        let errors = analyze_package(&[("main.vl", &entry)], "main.vl", platform);
        assert!(
            errors.is_empty(),
            "`std::{module}` under {platform:?}: importing without reaching is legal \
             (elision), and loading-for-typing introduces no errors: {errors:#?}"
        );
    }
}

// --- Library platform layers (L1) ------------------------------------------

/// Sets up a library `plat` with layers — a base module `shared`, a `process`-layer
/// `nodeonly`, and a `clock` present in both layers (process returns `i32`, browser
/// `str`) — and an empty base `lib.vl`. Analyzes `entry` (which imports from `plat`)
/// for `platform`, returning the diagnostics.
fn analyze_layered(entry: &str, platform: Platform) -> Vec<String> {
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
    put("src/process/nodeonly.vl", "fun nodeonly(): i32 { 1 }\n");
    put("src/process/clock.vl", "fun clock(): i32 { 1 }\n");
    put("src/browser/clock.vl", "fun clock(): str { \"x\" }\n");

    let workspace = Workspace {
        packages: vec![PackageSpec {
            base_root: plat.join("src"),
            layers: vec![
                Layer {
                    name: "process".to_string(),
                    // The `@process` family (node + deno), like real `std`.
                    patterns: PlatformPattern::parse("@process").unwrap(),
                    root: plat.join("src/process"),
                },
                Layer {
                    name: "browser".to_string(),
                    patterns: vec![PlatformPattern::Browser],
                    root: plat.join("src/browser"),
                },
            ],
            dependencies: Vec::new(),
            surface: true,
        }],
        entry_dependencies: vec![("plat".to_string(), 0)],
        macro_limits: MacroLimits::default(),
    };
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_spec(),
        &app,
        &entry_path,
        Some(platform),
        &workspace,
    );
    let _ = std::fs::remove_dir_all(&root);
    errors.into_iter().map(|error| error.msg).collect()
}

#[test]
fn base_module_available_for_all_platforms() {
    let entry = "import plat::shared::shared;\nfun main() { shared() }\n";
    assert!(analyze_layered(entry, Platform::default()).is_empty());
    assert!(analyze_layered(entry, Platform::Browser).is_empty());
}

#[test]
fn layer_module_available_only_for_its_platform() {
    let entry = "import plat::nodeonly::nodeonly;\nfun main() { nodeonly() }\n";
    assert!(
        analyze_layered(entry, Platform::default()).is_empty(),
        "the process-layer module is available for a node build"
    );
    let browser = analyze_layered(entry, Platform::Browser);
    assert!(
        browser
            .iter()
            .any(|e| e.contains("requires the `process` layer of `plat`")
                && e.contains("cannot run on `browser`")),
        "expected a platform-coloring violation for browser, got: {browser:#?}"
    );
    assert!(
        !browser.iter().any(|e| e.contains("cannot find")),
        "the module still loads for typing (no cascade): {browser:#?}"
    );
}

#[test]
fn varying_module_resolves_the_platform_version() {
    // `clock` is `i32` in the process layer, `str` in the browser layer. Passing it
    // to an `i32` parameter type-checks for node and fails for browser — proving the
    // build platform's version loaded (the P4 case, structurally).
    let entry = concat!(
        "import plat::clock::clock;\n",
        "fun need_int(n: i32) {}\n",
        "fun main() { need_int(clock()) }\n",
    );
    assert!(
        analyze_layered(entry, Platform::default()).is_empty(),
        "node `clock` is i32"
    );
    assert!(
        !analyze_layered(entry, Platform::Browser).is_empty(),
        "browser `clock` is str — a type mismatch, proving the browser version loaded"
    );
}

#[test]
fn base_lib_reexporting_a_layer_module_errors() {
    // A library whose base `lib.vl` re-exports `nodeonly` (a process-layer module):
    // the public surface must be platform-agnostic, so this is a Q4 violation.
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
    put("src/process/nodeonly.vl", "fun nodeonly(): i32 { 1 }\n");
    let workspace = Workspace {
        packages: vec![PackageSpec {
            base_root: plat.join("src"),
            layers: vec![Layer {
                name: "process".to_string(),
                patterns: vec![PlatformPattern::Node { version: None }],
                root: plat.join("src/process"),
            }],
            dependencies: Vec::new(),
            surface: true,
        }],
        entry_dependencies: vec![("plat".to_string(), 0)],
        macro_limits: MacroLimits::default(),
    };
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (_program, errors) = analyze_source(
        leaked,
        &std_spec(),
        &app,
        &entry_path,
        Some(Platform::default()),
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

// --- Deno joins `@process` (a second process runtime) ----------------------

/// The current `deno` platform (parsed so a test needn't name the version).
fn deno() -> Platform {
    Platform::parse("deno").expect("deno is a supported platform")
}

#[test]
fn process_layer_std_is_reachable_for_deno() {
    // `std::http` lives in the `process` layer (serves `@process`). Deno is in
    // `@process`, so the import resolves with no cross-platform error — its
    // `node:`-compat bindings are portable across the family (proposal §5).
    let entry = "import std::http::Server;\nfun main() { Server::builder(); }\n";
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", deno());
    assert!(
        errors.is_empty(),
        "std::http should be reachable for deno: {errors:#?}"
    );
}

#[test]
fn browser_layer_std_is_cross_platform_for_deno() {
    // The browser layer doesn't serve deno: reaching a browser-layer function
    // from a deno build is a coloring violation (pattern matching, not names).
    let entry = "import std::router::navigate;\nfun main() { navigate(\"/x\"); }\n";
    let errors = analyze_package(&[("main.vl", entry)], "main.vl", deno());
    assert!(
        errors
            .iter()
            .any(|e| e.contains("requires the `browser` layer of `std`")),
        "reaching the browser layer should violate for deno: {errors:#?}"
    );
}

#[test]
fn layered_process_module_serves_deno() {
    // The `plat` fixture's `process` layer declares `@process`, so `nodeonly` is
    // available for a deno build and `clock` resolves to the process version (i32),
    // exactly as for node — one layer, the whole family.
    assert!(
        analyze_layered(
            "import plat::nodeonly::nodeonly;\nfun main() { nodeonly() }\n",
            deno()
        )
        .is_empty(),
        "the process-layer module should be available for deno"
    );
    let clock = concat!(
        "import plat::clock::clock;\n",
        "fun need_int(n: i32) {}\n",
        "fun main() { need_int(clock()) }\n",
    );
    assert!(
        analyze_layered(clock, deno()).is_empty(),
        "deno resolves the process `clock` (i32), like node"
    );
}

// --- Platform contract check (§4.2 completeness) ----------------------------

/// Writes a library tree under a fresh temp dir — `base` files in `src/`, `process`
/// files in `src/process` (a layer serving `@process`), `browser` files in
/// `src/browser` — then runs the structural platform contract check over it and
/// returns the violation messages.
fn contract_violations(
    base: &[(&str, &str)],
    process: &[(&str, &str)],
    browser: &[(&str, &str)],
) -> Vec<String> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("vilan_contract_{}_{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let put = |dir: &std::path::Path, files: &[(&str, &str)]| {
        std::fs::create_dir_all(dir).unwrap();
        for (name, contents) in files {
            std::fs::write(dir.join(name), contents).unwrap();
        }
    };
    let src = root.join("src");
    put(&src, base);
    put(&src.join("process"), process);
    put(&src.join("browser"), browser);
    let spec = PackageSpec {
        base_root: src.clone(),
        layers: vec![
            Layer {
                name: "process".to_string(),
                patterns: PlatformPattern::parse("@process").unwrap(),
                root: src.join("process"),
            },
            Layer {
                name: "browser".to_string(),
                patterns: vec![PlatformPattern::Browser],
                root: src.join("browser"),
            },
        ],
        dependencies: Vec::new(),
        surface: true,
    };
    let violations = vilan_core::analyzer::check_library_contract(&spec)
        .into_iter()
        .map(|error| error.msg)
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    violations
}

#[test]
fn contract_ok_when_each_module_stays_within_its_served_set() {
    // A base module importing a base sibling (available everywhere) and a process
    // module importing a process sibling (available across `@process`) — both within
    // the platforms their own layer serves.
    let violations = contract_violations(
        &[
            ("lib.vl", ""),
            ("util.vl", "fun util(): i32 { 1 }\n"),
            (
                "core.vl",
                "import pkg::util::util;\nfun core(): i32 { util() }\n",
            ),
        ],
        &[
            ("feature.vl", "fun feature(): i32 { 1 }\n"),
            (
                "service.vl",
                "import pkg::feature::feature;\nfun service(): i32 { feature() }\n",
            ),
        ],
        &[],
    );
    assert!(
        violations.is_empty(),
        "expected no contract violations, got: {violations:#?}"
    );
}

#[test]
fn contract_flags_base_module_reaching_into_a_layer() {
    // A base module serves every host, so importing a process-only module breaks the
    // contract for the platforms the process layer doesn't serve (the browser).
    let violations = contract_violations(
        &[
            ("lib.vl", ""),
            (
                "core.vl",
                "import pkg::feature::feature;\nfun core(): i32 { feature() }\n",
            ),
        ],
        &[("feature.vl", "fun feature(): i32 { 1 }\n")],
        &[],
    );
    assert!(
        violations
            .iter()
            .any(|m| m.contains("core") && m.contains("feature") && m.contains("browser")),
        "expected a completeness violation naming `browser`, got: {violations:#?}"
    );
}

#[test]
fn contract_flags_process_module_reaching_into_the_browser_layer() {
    // A process module serves `@process` (node/deno/bun), so importing a browser-only
    // module isn't available for any of them — a violation, even though neither
    // module is in the base.
    let violations = contract_violations(
        &[("lib.vl", "")],
        &[(
            "service.vl",
            "import pkg::widget::widget;\nfun service(): i32 { widget() }\n",
        )],
        &[("widget.vl", "fun widget(): i32 { 1 }\n")],
    );
    assert!(
        violations
            .iter()
            .any(|m| m.contains("service") && m.contains("widget")),
        "expected a violation for the process→browser import, got: {violations:#?}"
    );
}

#[test]
fn contract_ignores_item_reexports_and_typos() {
    // `pkg::helper` here names an item re-exported through resolution, not a module
    // file — the contract check leaves it to ordinary name resolution.
    let violations = contract_violations(
        &[("lib.vl", "export import pkg::missing::thing;\n")],
        &[],
        &[],
    );
    assert!(
        violations.is_empty(),
        "a non-module `pkg::` ref isn't a contract concern: {violations:#?}"
    );
}

// --- Derives in imported modules (bug #1) -----------------------------------

#[test]
fn derive_in_an_imported_module_resolves() {
    // `[derive(Json)]` in an imported `pkg::` module synthesizes `to_json`/`from_json`
    // there, visible to the importer — derive expansion is no longer entry-file-only.
    let entry = concat!(
        "import std::json::{ Json, FromJson };\n",
        "import pkg::contract::User;\n",
        "fun main() {\n",
        "    let user = User { id = 1, name = \"Ada\" };\n",
        "    let back: User = User::from_json(user.to_json());\n",
        "    back.name\n",
        "}\n",
    );
    let contract = "[derive(Json)]\nstruct User {\n    id: i32,\n    name: str,\n}\n";
    let errors = analyze_package(
        &[("main.vl", entry), ("contract.vl", contract)],
        "main.vl",
        Platform::default(),
    );
    assert!(
        errors.is_empty(),
        "a derived type from an imported module should round-trip, got: {errors:#?}"
    );
}

#[test]
fn derive_in_a_dependency_library_resolves() {
    // The contract-library pattern: a `[derive(Json)]` type in a dependency library's
    // `lib.vl`, used by the app — the derive expands in the dependency too.
    let entry = concat!(
        "import std::json::{ Json, FromJson };\n",
        "import common::User;\n",
        "fun main() {\n",
        "    let user = User { id = 1, name = \"Ada\" };\n",
        "    let back: User = User::from_json(user.to_json());\n",
        "    back.name\n",
        "}\n",
    );
    let common = Dep {
        import_name: "common",
        files: &[(
            "lib.vl",
            "[derive(Json)]\nstruct User {\n    id: i32,\n    name: str,\n}\n",
        )],
    };
    let errors = analyze_workspace(entry, &[common], Platform::default());
    assert!(
        errors.is_empty(),
        "a derived type from a dependency library should round-trip, got: {errors:#?}"
    );
}

// --- Diagnostic source attribution (backlog E1) --------------------------------

/// As [`analyze_package_raw`], but returns `(message, source-file name)` pairs —
/// the attribution the LSP publishes by.
fn analyze_package_attributed(
    files: &[(&str, &str)],
    entry: &str,
    platform: Platform,
) -> Vec<(String, String)> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("vilan_attr_{}_{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (relative, contents) in files {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }
    let entry_path = dir.join(entry);
    let source = std::fs::read_to_string(&entry_path).unwrap();
    let leaked: &'static str = Box::leak(source.into_boxed_str());
    let (program, _errors) = analyze_source(
        leaked,
        &std_spec(),
        &dir,
        &entry_path,
        Some(platform),
        &Workspace::default(),
    );
    let program = program.expect("analysis should produce a program");
    let attributed = program
        .diagnostics
        .iter()
        .zip(program.diagnostic_sources.iter())
        .map(|(error, source)| {
            let name = program
                .source_path(*source)
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<none>".to_string());
            (error.msg.clone(), name)
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    attributed
}

// A type error INSIDE an imported module is attributed to that module's file,
// not the entry — the root cause of the LSP's vanishing-diagnostics bug (the
// error was mapped through the entry's line index and disappeared).
#[test]
fn a_type_error_in_an_imported_module_is_attributed_to_that_module() {
    let attributed = analyze_package_attributed(
        &[
            (
                "main.vl",
                "import std::print;\nimport pkg::broken::answer;\nfun main() { print(answer()); }\n",
            ),
            ("broken.vl", "fun answer(): i32 {\n\t\"not a number\"\n}\n"),
        ],
        "main.vl",
        Platform::default(),
    );
    let mismatch = attributed
        .iter()
        .find(|(msg, _)| msg.contains("Expected i32"))
        .expect("the return mismatch should be reported");
    assert_eq!(
        mismatch.1, "broken.vl",
        "the error belongs to the module that contains it: {attributed:?}"
    );
}

// An unresolved name inside a module attributes there; an unresolved name in
// the entry attributes to the entry — side by side in one program.
#[test]
fn name_errors_attribute_to_their_own_files() {
    let attributed = analyze_package_attributed(
        &[
            (
                "main.vl",
                "import pkg::helper::greet;\nfun main() {\n\tgreet();\n\tmissing_in_entry();\n}\n",
            ),
            ("helper.vl", "fun greet() {\n\tmissing_in_helper();\n}\n"),
        ],
        "main.vl",
        Platform::default(),
    );
    let helper_error = attributed
        .iter()
        .find(|(msg, _)| msg.contains("missing_in_helper"))
        .expect("the helper's name error should be reported");
    assert_eq!(helper_error.1, "helper.vl", "{attributed:?}");
    let entry_error = attributed
        .iter()
        .find(|(msg, _)| msg.contains("missing_in_entry"))
        .expect("the entry's name error should be reported");
    assert_eq!(entry_error.1, "main.vl", "{attributed:?}");
}

// A module that fails to PARSE attributes its (spanless) parse diagnostics to
// its own file, so the editor can surface them there.
#[test]
fn module_parse_errors_attribute_to_the_broken_module() {
    let attributed = analyze_package_attributed(
        &[
            (
                "main.vl",
                "import pkg::util::util;\nfun main() { let _ = util(); }\n",
            ),
            ("util.vl", "fun util(): i32 { 1 }\nfun broken( {\n"),
        ],
        "main.vl",
        Platform::default(),
    );
    let parse_error = attributed
        .iter()
        .find(|(msg, _)| msg.contains("parse error in"))
        .expect("the module parse error should be reported");
    assert_eq!(parse_error.1, "util.vl", "{attributed:?}");
}
