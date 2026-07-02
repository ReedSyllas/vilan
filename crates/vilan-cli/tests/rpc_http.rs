//! End-to-end test for RPC over a real HTTP transport: one Node process serves a
//! generated `[service(Client)]` dispatcher via `std::http::serve_rpc` and calls
//! itself through `std::rpc::HttpTransport` (host `fetch` → `node:http` on
//! localhost) — contract verification plus two state-mutating round-trips.
//!
//! The test writes a throwaway project and drives the built `vilan` binary; the
//! app `exit(0)`s after its calls, and a watchdog kills it if it ever hangs.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A fresh temp directory for the test's project tree.
fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_rpc_http_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes `contents` to `dir/relative`, creating parent directories.
fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Runs `vilan run <dir>` with a watchdog: a server that never exits is killed
/// (and the test failed) instead of hanging the suite.
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
                panic!("vilan run did not exit within {timeout:?} (server hung?)");
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
fn rpc_round_trips_over_real_http() {
    let dir = temp_project("round_trip");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &dir,
        "src/main.vl",
        r#"import std::print;
import std::shared::Shared;
import std::process::exit;
import std::result::Result::{ self, Ok, Err };
import std::json::Json;
import std::rpc::HttpTransport;
import std::http::serve_rpc;

[service(Client)]
struct Counter {
	count: Shared<i32>,
}

impl Counter {
	[rpc]
	fun add(self, by: i32): i32 {
		self.count.write() = self.count.read() + by;
		self.count.read()
	}
}

fun main() {
	let counter = Counter { count = Shared::new(0) };
	serve_rpc(45177, counter.dispatcher().into_protocol(), |server| {
		run_client();
	});
}

fun run_client() {
	let client = Client { transport = HttpTransport { url = "http://localhost:45177/" } };
	match client.verify() {
		Ok(let same) => print(i"verify = {same}"),
		Err(let error) => print(i"verify err {error.to_json()}"),
	}
	match client.add(2) {
		Ok(let n) => print(i"add -> {n}"),
		Err(let error) => print(i"err {error.to_json()}"),
	}
	match client.add(3) {
		Ok(let n) => print(i"add -> {n}"),
		Err(let error) => print(i"err {error.to_json()}"),
	}
	exit(0);
}
"#,
    );
    let stdout = vilan_run_with_timeout(&dir, Duration::from_secs(60));
    assert!(
        stdout.contains("verify = true"),
        "contract verification failed over HTTP:\n{stdout}"
    );
    assert!(
        stdout.contains("add -> 2") && stdout.contains("add -> 5"),
        "round-trips (with server-side state) failed:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
