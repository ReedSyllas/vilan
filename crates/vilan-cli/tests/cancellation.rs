//! End-to-end cancellation over real IO (async-polymorphism.md Part B's open
//! pin): a node process serves an endpoint that never answers in time, a
//! nursery-spawned task `fetch`es it, and `n.cancel()` must abort the request
//! IN FLIGHT — the ambient `AbortSignal` riding `std::fetch` — so the join
//! returns promptly instead of waiting the server out. A broken bridge shows
//! up as the watchdog killing a hung run.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A fresh temp directory for the test's project tree.
fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_cancel_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes `contents` to `dir/relative`, creating parent directories.
fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Runs `vilan run <dir>` with a watchdog: a hang (the abort not reaching the
/// in-flight request) kills the child and fails the test.
fn vilan_run_with_timeout(dir: &Path, timeout: Duration) -> String {
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
                panic!("vilan run did not exit within {timeout:?} (the abort never landed?)");
            }
            None => std::thread::sleep(Duration::from_millis(100)),
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
    assert!(
        stderr.trim().is_empty(),
        "vilan run wrote to stderr:\n{stderr}\nstdout:\n{stdout}"
    );
    stdout
}

#[test]
fn cancel_aborts_an_in_flight_fetch() {
    let dir = temp_project("fetch_abort");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &dir,
        "src/main.vl",
        r#"import std::print;
import std::process::exit;
import std::time::sleep;
import std::task::nursery;
import std::fetch::fetch;
import std::http::{ Server, Response };

fun main() {
	Server::builder()
		.port(45211)
		.on_request(|request| {
			// Never answers within the test window: only an in-flight abort
			// lets the client finish.
			sleep(60000);
			Response::builder().body("too late").build()
		})
		.on_start(|server| {
			run_client();
		})
		.build()
		.start();
}

fun fetch_hanging(): i32 {
	let response = fetch("http://localhost:45211/hang");
	print("unreachable-response");
	response.status()
}

fun run_client() {
	nursery(|n| {
		let _ = async fetch_hanging();
		sleep(150);   // let the request get in flight
		n.cancel();   // aborts it via the ambient signal
		0
	});
	print("aborted-fast");
	exit(0);
}
"#,
    );
    let started = Instant::now();
    let stdout = vilan_run_with_timeout(&dir, Duration::from_secs(45));
    assert!(
        stdout.contains("aborted-fast"),
        "the nursery never returned:\n{stdout}"
    );
    assert!(
        !stdout.contains("unreachable-response"),
        "the fetch completed instead of aborting:\n{stdout}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the join should return promptly after the abort, took {:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
