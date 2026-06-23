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
fn cross_target_library_module_is_rejected_without_cascade() {
    // A browser app imports a module that lives only in a library's `node` overlay:
    // the cross-target import is a recoverable error (the build fails) — but the
    // module still loads for typing, so `feature` resolves and there's no
    // unresolved-name cascade (L1).
    let dir = temp_project("compat");
    write(
        dir.as_path(),
        "platlib/vilan.toml",
        "[library]\nname = \"platlib\"\n\n[library.target.node]\nroot = \"src/node\"\n",
    );
    write(dir.as_path(), "platlib/src/lib.vl", "");
    write(
        dir.as_path(),
        "platlib/src/node/feature.vl",
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
    assert!(!output.status.success(), "expected a cross-target failure");
    let text = combined(&output);
    assert!(
        text.contains("another target's layer"),
        "unexpected output: {text}"
    );
    assert!(
        !text.contains("cannot find"),
        "the module should still type-check (no cascade): {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dependency_must_be_a_library() {
    // You depend on libraries, not apps (L1, Q2): a `[package]` dependency is an
    // error with a migration hint.
    let dir = temp_project("notlib");
    write(
        dir.as_path(),
        "applib/vilan.toml",
        "[package]\nname = \"applib\"\ntarget = \"node\"\n",
    );
    write(dir.as_path(), "applib/src/main.vl", "fun main() {}\n");
    write(
        &dir,
        "web/vilan.toml",
        "[package]\nname = \"web\"\ntarget = \"node\"\n\n[package.dependencies]\napplib = { path = \"../applib\" }\n",
    );
    write(&dir, "web/src/main.vl", "fun main() {}\n");
    let output = vilan(&["check", dir.join("web").to_str().unwrap()]);
    assert!(!output.status.success(), "expected a not-a-library failure");
    let text = combined(&output);
    assert!(text.contains("[library]"), "unexpected output: {text}");
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
