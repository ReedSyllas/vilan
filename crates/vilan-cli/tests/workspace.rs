//! End-to-end CLI tests for the multi-package workspace model (P2): building a
//! workspace emits one bundle per host member, the target-compatibility rule and
//! dependency cycles are rejected, and the legacy `[server]`/`[client]` form still
//! builds (the examples have migrated to workspaces, so this is its only coverage).
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
    write(
        dir,
        "common/vilan.toml",
        "[package]\nname = \"common\"\ntarget = \"none\"\n",
    );
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
fn incompatible_target_dependency_is_rejected_without_cascade() {
    // A browser package depending on a `node`-target library: the cross-target
    // import is a recoverable error (the build fails) — but the dependency still
    // loads for typing, so `helper` resolves and there's no unresolved-name
    // cascade (P3).
    let dir = temp_project("compat");
    write(
        dir.as_path(),
        "nodelib/vilan.toml",
        "[package]\nname = \"nodelib\"\ntarget = \"node\"\n",
    );
    write(
        dir.as_path(),
        "nodelib/src/lib.vl",
        "fun helper(): i32 { 1 }\n",
    );
    write(
        &dir,
        "web/vilan.toml",
        "[package]\nname = \"web\"\ntarget = \"browser\"\n\n[package.dependencies]\nnodelib = { path = \"../nodelib\" }\n",
    );
    write(
        &dir,
        "web/src/main.vl",
        "import nodelib::helper;\nfun main() { helper() }\n",
    );
    let output = vilan(&["build", dir.join("web").to_str().unwrap()]);
    assert!(!output.status.success(), "expected a compat failure");
    let text = combined(&output);
    assert!(text.contains("target"), "unexpected output: {text}");
    assert!(
        !text.contains("cannot find"),
        "the dependency should still type-check (no cascade): {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dependency_cycle_is_rejected() {
    let dir = temp_project("cycle");
    write(
        &dir,
        "a/vilan.toml",
        "[package]\nname = \"a\"\ntarget = \"none\"\n\n[package.dependencies]\nb = { path = \"../b\" }\n",
    );
    write(
        &dir,
        "a/src/lib.vl",
        "import b::vb;\nfun va(): i32 { vb() }\n",
    );
    write(
        &dir,
        "b/vilan.toml",
        "[package]\nname = \"b\"\ntarget = \"none\"\n\n[package.dependencies]\na = { path = \"../a\" }\n",
    );
    write(&dir, "b/src/lib.vl", "import a::va;\nfun vb(): i32 { 1 }\n");
    let output = vilan(&["check", dir.join("a").to_str().unwrap()]);
    assert!(!output.status.success(), "expected a cycle failure");
    let text = combined(&output);
    assert!(text.contains("cycle"), "unexpected output: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_server_client_still_builds() {
    // The deprecated `[server]`/`[client]` form lowers onto a two-member workspace
    // and must keep building `dist/{server,client}.js` (the examples no longer use
    // it, so this is its regression).
    let dir = temp_project("legacy");
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
    assert!(
        output.status.success(),
        "legacy build failed: {}",
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
