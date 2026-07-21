//! End-to-end CLI test for the asset channel (proposal/const-eval.md §3):
//! `vilan build` writes `<output>.<kind>` beside the compiled JS, with the
//! collected lines deduplicated and lexically ordered — and, per hmr.md §11 S0,
//! `vilan run` / `run --watch` write the same sidecar each round so the dev loop
//! serves fresh assets.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

fn temp_project(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "vilan_assets_cli_{tag}_{}_{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn vilan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(args)
        .output()
        .expect("run vilan")
}

#[test]
fn build_writes_assets_beside_the_output() {
    let dir = temp_project("emit");
    write(
        &dir,
        "app.vl",
        r#"import std::print;
import std::asset::emit;

fun base(): i32 {
	emit("css", ".pA3{padding:1rem}");
	emit("css", "@media (min-width: 768px){.mX{padding:2rem}}");
	1
}

fun accent(): i32 {
	emit("css", ".pA3{padding:1rem}");
	emit("css", ".bC7{background:blue}");
	2
}

let _a = const base();
let _b = const accent();

fun main() {
	print("styled");
}
main();
"#,
    );
    let entry = dir.join("app.vl");
    let output = vilan(&["build", entry.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The JS runs (the consts folded; no runtime emit calls survive).
    let js = std::fs::read_to_string(dir.join("app.js")).unwrap();
    assert!(!js.contains("__emit_asset"), "no runtime emit calls:\n{js}");
    // The stylesheet sits beside it: deduplicated, lexically ordered ('.'
    // before '@', so media blocks take the later cascade position).
    let css = std::fs::read_to_string(dir.join("app.css")).unwrap();
    assert_eq!(
        css,
        ".bC7{background:blue}\n.pA3{padding:1rem}\n@media (min-width: 768px){.mX{padding:2rem}}\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A quick-exit single-file program whose `const` initializer emits one CSS line.
/// `main` prints and returns, so Node exits on its own — safe to `run` (and to
/// spawn under `--watch` and kill).
fn quick_exit_program(marker: &str) -> String {
    format!(
        "import std::print;\nimport std::asset::emit;\n\nfun styles(): i32 {{\n\temit(\"css\", \".{marker}{{color:red}}\");\n\t1\n}}\n\nlet _s = const styles();\n\nfun main() {{\n\tprint(\"{marker}\");\n}}\nmain();\n"
    )
}

#[test]
fn run_writes_assets_beside_the_output() {
    // Single-package `vilan run` (the blocking path) must write the sidecar beside
    // the *canonical* build output (`app.css`, where `build` puts it) — not beside
    // the temp script Node executes. The G2 tail: `run`'s missing-CSS gap, closed.
    let dir = temp_project("run_single");
    write(&dir, "app.vl", &quick_exit_program("rS"));
    let entry = dir.join("app.vl");
    let output = vilan(&["run", entry.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("rS"),
        "the program should have run to completion"
    );
    let css = std::fs::read_to_string(dir.join("app.css")).expect("app.css beside the entry");
    assert_eq!(css, ".rS{color:red}\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn workspace_run_writes_fresh_dist_css() {
    // Workspace `vilan run` already routes its build through
    // `build_workspace_artifacts` (which calls `write_assets`), so the client
    // leg's CSS lands in `dist/client.css`. Pinned so the shared helper can't
    // regress. The server leg prints and exits, so `run` (which waits) returns.
    let dir = temp_project("run_ws");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n",
    );
    write(
        &dir,
        "src/client.vl",
        "import std::print;\nimport std::asset::emit;\n\nfun styles(): i32 {\n\temit(\"css\", \".ws{margin:0}\");\n\t1\n}\n\nlet _s = const styles();\n\nfun main() {\n\tprint(\"ui\");\n}\n",
    );
    write(
        &dir,
        "src/server.vl",
        "import std::print;\n\nfun main() {\n\tprint(\"fresh\");\n}\n",
    );
    let output = vilan(&["run", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "workspace run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let css = std::fs::read_to_string(dir.join("dist/client.css")).expect("dist/client.css");
    assert_eq!(css, ".ws{margin:0}\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Polls for `path` to hold `expected`, up to a bounded deadline. Returns the last
/// content seen (for a helpful assert message) if it never matches.
fn wait_for_contents(path: &Path, expected: &str, deadline: Duration) -> Result<(), String> {
    let start = Instant::now();
    let mut last = String::from("<never written>");
    while start.elapsed() < deadline {
        if let Ok(contents) = std::fs::read_to_string(path) {
            if contents == expected {
                return Ok(());
            }
            last = contents;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(last)
}

#[test]
fn watch_round_refreshes_the_sidecar() {
    // `run --watch` never exits, so this is a bounded end-to-end: spawn it, wait
    // for the round-1 sidecar, edit the source, wait for the round-2 sidecar, then
    // kill and reap the watcher. The program is quick-exit (main prints and
    // returns), so each round's Node child terminates on its own — killing the
    // watcher orphans nothing (the house scar tissue: a long-lived Node grandchild
    // would leak).
    let dir = temp_project("run_watch");
    let entry = dir.join("app.vl");
    write(&dir, "app.vl", &quick_exit_program("v1"));

    let mut watcher = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", "--watch", entry.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run --watch");

    let css = dir.join("app.css");
    let deadline = Duration::from_secs(20);
    let round_one = wait_for_contents(&css, ".v1{color:red}\n", deadline);

    // A watch round must rewrite the sidecar from the edited source.
    std::fs::write(&entry, quick_exit_program("v2")).unwrap();
    let round_two = wait_for_contents(&css, ".v2{color:red}\n", deadline);

    let _ = watcher.kill();
    let _ = watcher.wait();

    round_one.expect("round 1 should have written the v1 sidecar");
    round_two
        .map_err(|last| format!("watch round did not refresh the sidecar; last saw: {last:?}"))
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
