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

#[test]
fn realtime_sync_reaches_every_session_over_sse() {
    // The realtime milestone's mechanics: two sessions connect over SplitDuplex
    // (SSE + POST), both subscribe to the server's signal through their own
    // reactive channels, and one session's RPC mutation is observed by BOTH —
    // multi-session sync over a real wire.
    let dir = temp_project("realtime");
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
import std::time::sleep;
import std::option::Option::{ self, Some, None };
import std::result::Result::{ self, Ok, Err };
import std::json::{ Json, FromJson };
import std::reactive::Signal;
import std::rpc::{
	HttpTransport, connect_split, bridge,
	ReactiveServer, ReactiveClient, DuplexEnd,
};
import std::http::{ serve_connected, Response };

// Per-connection reactive servers, so `attach` can expose the board's signal on
// the caller's own wire.
let sessions: Shared<List<(i32, ReactiveServer)>> = Shared::new([]);

[service(Client)]
struct Board {
	count: Signal<i32>,
}

impl Board {
	// Expose the shared counter on the calling connection's wire; the returned
	// channel id is what the client's RemoteSource subscribes to.
	[rpc]
	fun attach(self, connection: i32): i32 {
		mut channel = 0 - 1;
		for entry in sessions.read() {
			let (id, reactive) = entry;
			if id == connection {
				channel = reactive.expose(self.count);
			}
		}
		channel
	}

	[rpc]
	fun add(self, by: i32): i32 {
		self.count.set(self.count.get() + by);
		self.count.get()
	}
}

fun main() {
	let board = Board { count = Signal::new(0) };
	serve_connected(
		9273,
		board.dispatcher().into_protocol(),
		|id, end| {
			sessions.write().push((id, ReactiveServer::new(end)));
		},
		|request| Response::builder().code(404).body("nope").build(),
		|| {
			run_clients();
		},
	);
}

fun watch(name: str, base: str): Client<HttpTransport> {
	let split = connect_split(base);
	let reactive = ReactiveClient::new(bridge(split));
	let client = Client { transport = HttpTransport { url = i"{base}/rpc" } };
	match client.attach(i32::from_json(split.connection)) {
		Ok(let channel) => {
			let _ = reactive.source(channel).sub(|json| {
				let n: i32 = i32::from_json(json);
				print(i"{name} sees {n}");
			});
		},
		Err(let error) => print(i"{name} attach err {error.to_json()}"),
	}
	client
}

fun run_clients() {
	let base = "http://localhost:9273";
	let alice = watch("alice", base);
	let bob = watch("bob", base);
	sleep(300);   // let both Subscribe frames land
	match alice.add(7) {
		Ok(let n) => print(i"alice add -> {n}"),
		Err(let error) => print(i"add err {error.to_json()}"),
	}
	sleep(300);   // let the SSE deliveries land
	exit(0);
}
"#,
    );
    let stdout = vilan_run_with_timeout(&dir, Duration::from_secs(60));
    for expected in [
        "alice sees 0",
        "bob sees 0",
        "alice sees 7",
        "bob sees 7",
        "alice add -> 7",
    ] {
        assert!(
            stdout.contains(expected),
            "missing `{expected}` in realtime output:\n{stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
