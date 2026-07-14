//! End-to-end CLI tests for the multi-package workspace model (P2) and the
//! single-package multi-entry form (`[entry.<name>]`,
//! proposal/platform-coloring.md §4.2): building a workspace emits one bundle
//! per host member / per entry, the platform-compatibility rule and dependency
//! cycles are rejected, and the retired `[server]`/`[client]` form fails with
//! its migration hint.
//!
//! Each test writes a throwaway project tree and drives the built `vilan` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

/// A fresh temp directory for one test's project tree.
fn temp_project(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "vilan_ws_cli_{tag}_{}_{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes `contents` to `dir/relative`, creating parent directories.
fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Runs the `vilan` binary with `args`. `std` resolves from the in-repo default.
fn vilan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(args)
        .output()
        .expect("run vilan")
}

/// Writes a small client/server/common workspace into `dir` (the client and server
/// both `import common::greeting`).
fn write_fullstack_workspace(dir: &Path) {
    write(
        dir,
        "vilan.toml",
        "[project]\npackages = [\"common\", \"client\", \"server\"]\n",
    );
    write(dir, "common/vilan.toml", "[library]\nname = \"common\"\n");
    write(dir, "common/src/lib.vl", "fun greeting(): str { \"hi\" }\n");
    write(
        dir,
        "server/vilan.toml",
        "[package]\nname = \"server\"\ntarget = \"node\"\n\n[package.dependencies]\ncommon = { path = \"../common\" }\n",
    );
    write(
        dir,
        "server/src/main.vl",
        "import std::print;\nimport common::greeting;\nfun main() { print(greeting()) }\n",
    );
    write(
        dir,
        "client/vilan.toml",
        "[package]\nname = \"client\"\ntarget = \"browser\"\n\n[package.dependencies]\ncommon = { path = \"../common\" }\n",
    );
    write(
        dir,
        "client/src/main.vl",
        "import common::greeting;\nfun main() { greeting() }\n",
    );
}

#[test]
fn workspace_builds_each_host_member() {
    let dir = temp_project("build");
    write_fullstack_workspace(&dir);
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A bundle per host member; the `none` library is not built on its own.
    assert!(
        dir.join("dist/server.js").is_file(),
        "missing dist/server.js"
    );
    assert!(
        dir.join("dist/client.js").is_file(),
        "missing dist/client.js"
    );
    assert!(
        !dir.join("dist/common.js").exists(),
        "the `none` library should not be built standalone"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A command's combined stdout + stderr (errors render to stdout, resolution
/// failures to stderr — a test asserting on a message wants both).
fn combined(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

#[test]
fn cross_platform_library_module_is_rejected_without_cascade() {
    // A browser app imports a module that lives only in a library's `process` layer:
    // the cross-platform import is a recoverable error (the build fails) — but the
    // module still loads for typing, so `feature` resolves and there's no
    // unresolved-name cascade (L1).
    let dir = temp_project("compat");
    write(
        dir.as_path(),
        "platlib/vilan.toml",
        "[library]\nname = \"platlib\"\n\n[library.layer.process]\nplatform = [\"@process\"]\n",
    );
    write(dir.as_path(), "platlib/src/lib.vl", "");
    write(
        dir.as_path(),
        "platlib/src/process/feature.vl",
        "fun value(): i32 { 1 }\n",
    );
    write(
        &dir,
        "web/vilan.toml",
        "[package]\nname = \"web\"\ntarget = \"browser\"\n\n[package.dependencies]\nplatlib = { path = \"../platlib\" }\n",
    );
    write(
        &dir,
        "web/src/main.vl",
        "import platlib::feature::value;\nfun main() { value() }\n",
    );
    let output = vilan(&["build", dir.join("web").to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "expected a cross-platform failure"
    );
    let text = combined(&output);
    assert!(
        text.contains("requires the `process` layer of `platlib`") && text.contains("main → value"),
        "expected a chain-rendered coloring violation: {text}"
    );
    assert!(
        !text.contains("cannot find"),
        "the module should still type-check (no cascade): {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_package_dependency_is_allowed_and_colors_inferentially() {
    // Platform coloring's blessed shape (platform-coloring.md §7.3): an app
    // may depend on a `[package]`. Its neutral items are reachable from any
    // build; reaching a function that touches platform std is the chain
    // diagnostic — the dependency's `target` declares its entry, not a gate.
    let dir = temp_project("pkgdep");
    write(
        dir.as_path(),
        "applib/vilan.toml",
        "[package]\nname = \"applib\"\ntarget = \"node\"\n",
    );
    write(dir.as_path(), "applib/src/main.vl", "fun main() {}\n");
    write(
        dir.as_path(),
        "applib/src/util.vl",
        "import std::fs::write_file;\nfun neutral(): i32 { 2 }\nfun save() { write_file(\"x\", \"y\") }\n",
    );
    write(
        &dir,
        "web/vilan.toml",
        "[package]\nname = \"web\"\ntarget = \"browser\"\n\n[package.dependencies]\napplib = { path = \"../applib\" }\n",
    );
    // Reaching the neutral item from the browser: fine.
    write(
        &dir,
        "web/src/main.vl",
        "import applib::util::neutral;\nfun main() { neutral(); }\n",
    );
    let output = vilan(&["build", dir.join("web").to_str().unwrap()]);
    assert!(
        output.status.success(),
        "a neutral package-dependency item should build for the browser: {}",
        combined(&output)
    );
    // Reaching the fs-colored item: the chain diagnostic.
    write(
        &dir,
        "web/src/main.vl",
        "import applib::util::save;\nfun main() { save(); }\n",
    );
    let output = vilan(&["build", dir.join("web").to_str().unwrap()]);
    assert!(!output.status.success(), "expected a coloring violation");
    let text = combined(&output);
    assert!(
        text.contains("requires the `process` layer of `std`") && text.contains("main → save"),
        "expected the chain diagnostic: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dependency_cycle_is_rejected() {
    // App `web` → library `liba` → library `libb` → `liba` (a cycle).
    let dir = temp_project("cycle");
    write(
        &dir,
        "web/vilan.toml",
        "[package]\nname = \"web\"\ntarget = \"node\"\n\n[package.dependencies]\nliba = { path = \"../liba\" }\n",
    );
    write(
        &dir,
        "web/src/main.vl",
        "import liba::va;\nfun main() { va() }\n",
    );
    write(
        &dir,
        "liba/vilan.toml",
        "[library]\nname = \"liba\"\n\n[library.dependencies]\nlibb = { path = \"../libb\" }\n",
    );
    write(&dir, "liba/src/lib.vl", "fun va(): i32 { 1 }\n");
    write(
        &dir,
        "libb/vilan.toml",
        "[library]\nname = \"libb\"\n\n[library.dependencies]\nliba = { path = \"../liba\" }\n",
    );
    write(&dir, "libb/src/lib.vl", "fun vb(): i32 { 1 }\n");
    let output = vilan(&["check", dir.join("web").to_str().unwrap()]);
    assert!(!output.status.success(), "expected a cycle failure");
    let text = combined(&output);
    assert!(text.contains("cycle"), "unexpected output: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes a two-entry single package (§4.2): a browser `client` and a node
/// `server`, sharing a `pkg::store` module whose `load` reaches `std::fs`.
/// Only the server calls `load` — the shape a full-stack app actually has.
fn write_multi_entry_package(dir: &Path) {
    write(
        dir,
        "vilan.toml",
        "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n",
    );
    write(
        dir,
        "src/store.vl",
        "import std::fs;\n\nfun load(): bool {\n\tfs::exists(\"state\")\n}\n",
    );
    write(
        dir,
        "src/server.vl",
        "import std::print;\nimport pkg::store::load;\n\nfun main() {\n\tif load() { print(\"loaded\") } else { print(\"fresh\") }\n}\n",
    );
    write(
        dir,
        "src/client.vl",
        "import std::print;\n\nfun main() {\n\tprint(\"ui\");\n}\n",
    );
}

#[test]
fn a_multi_entry_package_builds_every_entry_into_dist() {
    // `[entry.<name>]` lowers onto the workspace orchestration: one
    // `dist/<name>.js` per entry, each compiled for its own target — the
    // node-only `store.load` is fine because the client never reaches it.
    let dir = temp_project("entries");
    write_multi_entry_package(&dir);
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "multi-entry build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        dir.join("dist/server.js").is_file(),
        "missing dist/server.js"
    );
    assert!(
        dir.join("dist/client.js").is_file(),
        "missing dist/client.js"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_picks_the_single_node_entry() {
    // `vilan run` on a multi-entry package builds everything, then runs the
    // one node entry (the workspace rule, unchanged).
    let dir = temp_project("entries_run");
    write_multi_entry_package(&dir);
    let output = vilan(&["run", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fresh"), "server should run: {stdout}");
    assert!(
        dir.join("dist/client.js").is_file(),
        "run should have built the client bundle first"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_colors_each_entry_against_its_own_target() {
    // `vilan check` checks every entry, always (§7 decision 4). The same
    // package is clean until the CLIENT entry reaches the store — then the
    // browser build fails with the coloring chain while the server stays fine.
    let dir = temp_project("entries_check");
    write_multi_entry_package(&dir);
    let clean = vilan(&["check", dir.to_str().unwrap()]);
    assert!(
        clean.status.success(),
        "clean check failed: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
    write(
        &dir,
        "src/client.vl",
        "import std::print;\nimport pkg::store::load;\n\nfun main() {\n\tif load() { print(\"?\") }\n}\n",
    );
    let violating = vilan(&["check", dir.to_str().unwrap()]);
    assert!(
        !violating.status.success(),
        "the client's reach into `std::fs` must fail the browser entry"
    );
    let text = combined(&violating);
    assert!(
        text.contains("requires the `process` layer of `std`")
            && text.contains("cannot run on `browser`"),
        "unexpected output: {text}"
    );
    assert!(
        text.contains("main → load → exists (std::fs)"),
        "the chain should name the path: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn colliding_output_names_are_rejected() {
    // A workspace member named `app` and a sibling's `[entry.app]` would both
    // write `dist/app.js` — rejected at lowering instead of silently racing.
    let dir = temp_project("collide");
    write(
        &dir,
        "vilan.toml",
        "[project]\npackages = [\"app\", \"site\"]\n",
    );
    write(&dir, "app/vilan.toml", "[package]\nname = \"app\"\n");
    write(&dir, "app/src/main.vl", "fun main() { }\n");
    write(
        &dir,
        "site/vilan.toml",
        "[package]\nname = \"site\"\n\n[entry.app]\ntarget = \"browser\"\n",
    );
    write(&dir, "site/src/app.vl", "fun main() { }\n");
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(!output.status.success(), "expected a collision failure");
    let text = combined(&output);
    assert!(text.contains("dist/app.js"), "unexpected output: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_retired_server_client_form_fails_with_the_migration_hint() {
    // The old top-level pair doesn't lower any more — it names its
    // replacement instead of building.
    let dir = temp_project("retired");
    write(
        &dir,
        "vilan.toml",
        "[server]\nentry = \"server.vl\"\n[client]\nentry = \"client.vl\"\n",
    );
    write(
        &dir,
        "server.vl",
        "import std::print;\nfun main() { print(1) }\n",
    );
    write(&dir, "client.vl", "fun main() { }\n");
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(!output.status.success(), "the retired form must not build");
    let text = combined(&output);
    assert!(
        text.contains("[entry.server]"),
        "the error should name the replacement: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn standalone_library_check_verifies_the_platform_contract() {
    // A `[library]` has no fixed platform: `vilan check` verifies its contract (every
    // module's `pkg::` imports resolve across the platforms its layer serves) rather
    // than a single-platform build — and `vilan build` rejects it (a library is built
    // only as a dependency).
    let dir = temp_project("contract");
    write(
        dir.as_path(),
        "vilan.toml",
        "[library]\nname = \"lib\"\n\n[library.layer.process]\nplatform = [\"@process\"]\n",
    );
    write(dir.as_path(), "src/lib.vl", "");
    write(dir.as_path(), "src/util.vl", "fun util(): i32 { 1 }\n");
    write(
        dir.as_path(),
        "src/process/service.vl",
        "import pkg::util::util;\nfun service(): i32 { util() }\n",
    );
    let check = vilan(&["check", dir.to_str().unwrap()]);
    assert!(
        check.status.success(),
        "a well-formed library's contract should pass: {}",
        combined(&check)
    );
    let build = vilan(&["build", dir.to_str().unwrap()]);
    assert!(!build.status.success(), "a `[library]` is not buildable");
    assert!(
        combined(&build).contains("[library]"),
        "unexpected build output: {}",
        combined(&build)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn standalone_library_check_flags_a_contract_violation() {
    // A base module (serves every platform) importing a process-only module is a
    // completeness violation (the browser can't provide it); `vilan check` reports it
    // and fails.
    let dir = temp_project("contract_bad");
    write(
        dir.as_path(),
        "vilan.toml",
        "[library]\nname = \"lib\"\n\n[library.layer.process]\nplatform = [\"@process\"]\n",
    );
    write(dir.as_path(), "src/lib.vl", "");
    write(
        dir.as_path(),
        "src/core.vl",
        "import pkg::feature::feature;\nfun core(): i32 { feature() }\n",
    );
    write(
        dir.as_path(),
        "src/process/feature.vl",
        "fun feature(): i32 { 1 }\n",
    );
    let output = vilan(&["check", dir.to_str().unwrap()]);
    assert!(!output.status.success(), "expected a contract violation");
    let text = combined(&output);
    assert!(
        text.contains("not available for") && text.contains("browser"),
        "unexpected output: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_parse_error_inside_a_package_module_fails_the_build_loudly() {
    // Package modules (std, libraries, `pkg::` siblings) load through the
    // error-recovering parser: a syntax error used to be silently swallowed —
    // the recovered `Node::Error` compiled to *nothing*, so the module built
    // with the broken statements simply gone. It must fail, naming the file
    // and position.
    let dir = temp_project("module-parse-error");
    write(dir.as_path(), "vilan.toml", "[package]\nname = \"app\"\n");
    write(
        dir.as_path(),
        "src/main.vl",
        "import pkg::util::util;\nfun main() { let _ = util(); }\n",
    );
    write(
        dir.as_path(),
        "src/util.vl",
        "fun util(): i32 { 1 }\nfun broken( {\n",
    );
    let build = vilan(&["build", dir.to_str().unwrap()]);
    assert!(
        !build.status.success(),
        "a module with a parse error must not build"
    );
    let output = combined(&build);
    assert!(
        output.contains("parse error in") && output.contains("util.vl"),
        "the diagnostic should name the broken module: {output}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// Block-scoped imports (backlog H2), the multi-package path: a dependency and a
// `pkg::` sibling referenced ONLY inside function bodies must still seed the
// loader's reachable set — `collect_module_refs` finds references at any depth.
#[test]
fn body_scoped_imports_load_dependencies_and_siblings() {
    let dir = temp_project("body_imports");
    write(
        &dir,
        "vilan.toml",
        "[project]\npackages = [\"common\", \"server\"]\n",
    );
    write(&dir, "common/vilan.toml", "[library]\nname = \"common\"\n");
    write(
        &dir,
        "common/src/lib.vl",
        "fun greeting(): str { \"hi\" }\n",
    );
    write(
        &dir,
        "server/vilan.toml",
        "[package]\nname = \"server\"\ntarget = \"node\"\n\n[package.dependencies]\ncommon = { path = \"../common\" }\n",
    );
    write(
        &dir,
        "server/src/main.vl",
        "import std::print;\n\nfun main() {\n    import common::greeting;\n    import pkg::helper;\n    print(greeting());\n    print(helper::tagline());\n}\n",
    );
    write(
        &dir,
        "server/src/helper.vl",
        "fun tagline(): str { \"from a sibling\" }\n",
    );
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new("node")
        .arg(dir.join("dist/server.js"))
        .output()
        .expect("run node");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "hi\nfrom a sibling\n",
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

// The §4.2 completeness check sees imports at any depth (backlog H2): a base
// module smuggling a process-only module through a FUNCTION-BODY import is the
// same violation as a top-level one.
#[test]
fn standalone_library_check_flags_a_body_scoped_violation() {
    let dir = temp_project("contract_body");
    write(
        dir.as_path(),
        "vilan.toml",
        "[library]\nname = \"lib\"\n\n[library.layer.process]\nplatform = [\"@process\"]\n",
    );
    write(dir.as_path(), "src/lib.vl", "");
    write(
        dir.as_path(),
        "src/core.vl",
        "fun core(): i32 {\n    import pkg::feature::feature;\n    feature()\n}\n",
    );
    write(
        dir.as_path(),
        "src/process/feature.vl",
        "fun feature(): i32 { 1 }\n",
    );
    let output = vilan(&["check", dir.to_str().unwrap()]);
    assert!(!output.status.success(), "expected a contract violation");
    let text = combined(&output);
    assert!(
        text.contains("not available for") && text.contains("browser"),
        "unexpected output: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The docs walkthrough app (docs/guide/walkthrough.md quotes its files) must
/// keep building — it is the book's capstone example.
#[test]
fn the_walkthrough_example_builds() {
    let example =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/examples/walkthrough");
    let output = vilan(&["build", example.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "walkthrough build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(example.join("dist/client.js").is_file());
    assert!(example.join("dist/client.css").is_file());
    assert!(example.join("dist/server.js").is_file());
}
