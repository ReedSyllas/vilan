//! End-to-end for `OwnedNursery` and detached-mode reporting (destruction.md
//! §9, C4 S4b). An `OwnedNursery` owns the tasks spawned inside `enter`;
//! dropping it cancels them (bridged IO aborts, frames unwind, drops run), and
//! its nursery runs DETACHED — never joined — so a real child failure reports
//! to the console with its spawn origin instead of being silently absorbed, and
//! children do not cancel their siblings. Cancellation echoes stay silent.
//!
//! These run real `node` processes (timers, the AbortSignal bridge, the deferred
//! console report), so they live here rather than in the inference harness.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A fresh temp directory for the test's project tree.
fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_owned_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes `contents` to `dir/relative`, creating parent directories.
fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Runs `vilan run <dir>` with a watchdog and returns `(stdout, stderr)`. A hang
/// (a drop that never cancels, so the 60s sleep runs the clock out) kills the
/// child and fails the test. Unlike `cancellation.rs`, this keeps stderr — the
/// detached failure report is written there.
fn run_project(dir: &Path, timeout: Duration) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vilan run");
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("poll vilan run") {
            Some(_status) => break,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("vilan run did not exit within {timeout:?} (a drop never cancelled?)");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (stdout, stderr)
}

/// (a) Dropping the owner cancels an in-flight sleeping task — the bridged sleep
/// rejects, the task frame unwinds, and its resource drops run — and (d) the
/// cancellation is an echo, so nothing is reported (stderr stays empty).
#[test]
fn owner_drop_cancels_an_in_flight_task_runs_its_drops_and_stays_silent() {
    let dir = temp_project("drop_cancel");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &dir,
        "src/main.vl",
        r#"import std::print;
import std::drop::Drop;
import std::task::OwnedNursery;
import std::time::sleep;

resource struct Guard {
	tag: str,
}

impl Guard with Drop {
	fun drop(&mut self) {
		print("cleanup-" + self.tag);
	}
}

fun slow() {
	let guard = Guard { tag = "A" };
	print("task-started");
	sleep(60000);
	print("task-finished");
}

fun scope() {
	let owner = OwnedNursery::new();
	let _ = owner.enter(|| {
		let _ = async slow();
		0
	});
	print("entered");
	// `owner` drops here: cancel() aborts the ambient signal, the in-flight
	// sleep rejects, `slow` unwinds, and `guard` drops.
}

fun main() {
	scope();
	print("after-scope");
}
"#,
    );
    let (stdout, stderr) = run_project(&dir, Duration::from_secs(20));
    assert!(
        stdout.contains("task-started"),
        "the owned task never started:\n{stdout}"
    );
    assert!(
        stdout.contains("cleanup-A"),
        "the drop did not run on cancellation-unwind:\n{stdout}"
    );
    assert!(
        !stdout.contains("task-finished"),
        "the sleep completed instead of being cancelled:\n{stdout}"
    );
    assert!(
        stdout.contains("after-scope"),
        "the process did not continue past the owner's scope:\n{stdout}"
    );
    // (d) A cancellation echo is silent — no free-task report.
    assert!(
        stderr.trim().is_empty(),
        "a cancellation echo was reported instead of staying silent:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// (b) A detached child's REAL failure (a panic, not a cancellation) reaches the
/// console with its spawn origin, and the process continues; (c) that failure
/// does NOT cancel its sibling — the sibling runs to completion.
#[test]
fn a_detached_failure_reports_with_origin_and_does_not_cancel_the_sibling() {
    let dir = temp_project("detached_report");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &dir,
        "src/main.vl",
        r#"import std::print;
import std::io::panic;
import std::task::OwnedNursery;
import std::time::sleep;

fun boom() {
	panic("boom-B");
}

fun survivor() {
	sleep(50);
	print("survivor-done");
}

fun main() {
	let owner = OwnedNursery::new();
	let _ = owner.enter(|| {
		let _ = async boom();
		let _ = async survivor();
		// Hold the extent open long enough for both children to settle
		// (boom fails at once; survivor finishes at ~50ms).
		sleep(400);
		0
	});
	print("main-continues");
}
"#,
    );
    let (stdout, stderr) = run_project(&dir, Duration::from_secs(20));
    // (b) the real failure is reported to the console, with the spawn origin.
    assert!(
        stderr.contains("unhandled task error") && stderr.contains("spawned in"),
        "the detached failure was not reported with its origin:\n{stderr}"
    );
    assert!(
        stderr.contains("boom-B"),
        "the report did not carry the failure:\n{stderr}"
    );
    // (c) the sibling was not cancelled by the failure — it ran to completion.
    assert!(
        stdout.contains("survivor-done"),
        "the sibling was cancelled by its sibling's failure:\n{stdout}"
    );
    // (b) the process continued past the failure.
    assert!(
        stdout.contains("main-continues"),
        "the process did not continue past the detached failure:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
