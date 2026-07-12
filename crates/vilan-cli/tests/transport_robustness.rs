//! End-to-end test for K6 transport robustness
//! (vilan/proposal/transport-robustness.md): a generated client rides a real
//! WebSocket to a real server, which is then STOPPED (SIGSTOP — the in-flight
//! call hangs), KILLED (the socket closes), and RESTARTED with different
//! state. Asserts the whole contract: the pending call rejects with a typed
//! transport error (never dangles), the state signal walks
//! Connected → Reconnecting → Connected, a call made while down fails fast,
//! the mirror RE-SYNCS to the restarted server's value through the re-attach
//! hook, and calls work again afterwards.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_robust_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A node child whose stdout lines stream to a channel; killed on drop.
struct LineChild {
    child: Child,
    lines: Receiver<String>,
}

impl LineChild {
    fn spawn(bundle: &Path, argument: Option<&str>) -> LineChild {
        let mut command = Command::new("node");
        command.arg(bundle);
        if let Some(argument) = argument {
            command.arg(argument);
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn node");
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        LineChild { child, lines }
    }

    /// Blocks until a stdout line containing `needle` arrives; returns it.
    fn await_line(&self, needle: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for `{needle}` on stdout"
            );
            match self.lines.recv_timeout(remaining) {
                Ok(line) if line.contains(needle) => return line,
                Ok(_other) => {}
                Err(_) => panic!("stdout ended or timed out before `{needle}`"),
            }
        }
    }

    fn signal(&self, name: &str) {
        let status = Command::new("kill")
            .args([name, &self.child.id().to_string()])
            .status()
            .expect("send signal");
        assert!(status.success(), "kill {name} failed");
    }
}

impl Drop for LineChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const COMMON: &str = r#"import std::reactive::Signal;

[service(StatusClient)]
struct StatusBoard {
	[expose] status: Signal<i32>,
}

impl StatusBoard {
	[rpc]
	fun set_status(self, value: i32): i32 {
		self.status.set(value);
		value
	}

	[rpc]
	fun echo(self, value: i32): i32 {
		value
	}
}
"#;

const SERVER: &str = r#"import std::print;
import std::reactive::Signal;
import std::json::json_codec;
import std::option::Option::{ self, Some, None };
import std::process::args;
import std::http::Response;
import std::rpc_server::serve_service;
import common::StatusBoard;

async fun main() {
	let initial = match args().get(0) {
		Some(let raw) => match raw.parse_i32() {
			Some(let value) => value,
			None => 0,
		},
		None => 0,
	};
	let board = StatusBoard { status = Signal::new(initial) };
	serve_service(9297, board.dispatcher().into_protocol(json_codec()), |request| {
		Response::builder().code(404).body("nope").build()
	}, || print(i"listening {initial}"));
}
"#;

const CLIENT: &str = r#"import std::print;
import std::shared::Shared;
import std::json::json_codec;
import std::result::Result::{ self, Ok, Err };
import std::time::sleep;
import std::process::exit;
import std::rpc::ConnectionState;
import common::{ StatusBoard, StatusClient };

async fun main() {
	match StatusClient::connect("ws://localhost:9297/", json_codec()) {
		Ok(let client) => {
			let state = client.transport.connection_state();
			let fast_fired: Shared<bool> = Shared::new(false);
			let resynced: Shared<bool> = Shared::new(false);

			let watching_state = state.sub(|current| {
				print(i"state:{current.debug()}");
				// The moment the drop is noticed, prove fail-fast: a call
				// while down errors immediately instead of hanging.
				if current == ConnectionState::Reconnecting && !fast_fired.read() {
					fast_fired.write() = true;
					match client.set_status(9) {
						Ok(let value) => print(i"fast:ok:{value}"),
						Err(let error) => print(i"fast:err:{error.debug()}"),
					}
				}
			});

			let watching_mirror = client.status.sub(|value| {
				print(i"mirror:{value}");
				// The restarted server announces itself with status 2; a call
				// on the reconnected transport must succeed again.
				if value == 2 && !resynced.read() {
					resynced.write() = true;
					match client.set_status(5) {
						Ok(let confirmed) => {
							print(i"call:ok:{confirmed}");
							exit(0);
						},
						Err(let error) => print(i"call:err:{error.debug()}"),
					}
				}
			});

			// Give the harness a beat, then send the call that will be caught
			// in flight by the stop/kill.
			sleep(500);
			print("doomed:sent");
			match client.echo(7) {
				Ok(let value) => print(i"doomed:ok:{value}"),
				Err(let error) => print(i"doomed:err:{error.debug()}"),
			}
			// Keep main open through the outage — on node a COMPLETED main
			// exits the process; the success path exits from the mirror sub.
			sleep(600000);
		},
		Err(let error) => print(i"connect failed: {error.debug()}"),
	}
}
"#;

#[test]
fn a_dropped_connection_reconnects_and_resyncs() {
    let dir = temp_project("reconnect");
    write(
        &dir,
        "vilan.toml",
        "[project]\npackages = [\"common\", \"server\", \"client\"]\n",
    );
    write(&dir, "common/vilan.toml", "[library]\nname = \"common\"\n");
    write(
        &dir,
        "server/vilan.toml",
        "[package]\nname = \"server\"\ntarget = \"node\"\n\n[package.dependencies]\ncommon = { path = \"../common\" }\n",
    );
    write(
        &dir,
        "client/vilan.toml",
        "[package]\nname = \"client\"\ntarget = \"node\"\n\n[package.dependencies]\ncommon = { path = \"../common\" }\n",
    );
    write(&dir, "common/src/lib.vl", COMMON);
    write(&dir, "server/src/main.vl", SERVER);
    write(&dir, "client/src/main.vl", CLIENT);

    let build = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["build", dir.to_str().unwrap()])
        .output()
        .expect("run vilan build");
    assert!(
        build.status.success(),
        "build failed:\n{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let wait = Duration::from_secs(20);

    // Phase 1: server up (status 1), client syncs.
    let server = LineChild::spawn(&dir.join("dist/server.js"), Some("1"));
    server.await_line("listening 1", wait);
    let client = LineChild::spawn(&dir.join("dist/client.js"), None);
    client.await_line("state:Connected", wait);
    client.await_line("mirror:1", wait);

    // Phase 2: freeze the server so the next call hangs in flight, then kill.
    server.signal("-STOP");
    client.await_line("doomed:sent", wait);
    std::thread::sleep(Duration::from_millis(300));
    server.signal("-KILL");
    drop(server);

    // The drop is noticed: state flips, a call while down fails fast (fired
    // synchronously inside the state notification, so it precedes the
    // pending rejection on stdout), and the in-flight call REJECTS (typed,
    // never dangling). `await_line` consumes skipped lines, so the order
    // here mirrors the client's emission order.
    client.await_line("state:Reconnecting", wait);
    let fast = client.await_line("fast:err", wait);
    assert!(
        fast.contains("not connected"),
        "call while down should fail fast, got: {fast}"
    );
    let doomed = client.await_line("doomed:err", wait);
    assert!(
        doomed.contains("connection lost"),
        "in-flight call should reject with the drop reason, got: {doomed}"
    );

    // Phase 3: restart with DIFFERENT state — the backoff loop reconnects,
    // the hook re-attaches, the mirror resyncs, calls work again.
    let revived = LineChild::spawn(&dir.join("dist/server.js"), Some("2"));
    revived.await_line("listening 2", wait);
    client.await_line("state:Connected", wait);
    client.await_line("mirror:2", wait);
    client.await_line("call:ok:5", wait);

    let _ = std::fs::remove_dir_all(&dir);
}

/// B21 (backlog): a unit that consumes a DEPENDENCY package's `[service]`
/// without importing `std::rpc` itself mistypes the generated `connect` body
/// (`socket` types as `connect_socket`'s raw `Result`, cascading through
/// `transport()`/`__attach`). Any direct `import std::rpc::..` in the
/// consumer masks it — so the bug is scope/order-sensitive resolution of the
/// expansion, not the expansion's text (four template shapes failed
/// identically). Un-ignore when fixed; the kolt probe carries the
/// import workaround until then.
#[test]
#[ignore = "B21: dependency-package [service] consumer without a direct std::rpc import mistypes the generated connect"]
fn a_library_service_client_compiles_without_an_rpc_import() {
    let dir = temp_project("b21");
    write(
        &dir,
        "vilan.toml",
        "[project]\npackages = [\"common\", \"app\"]\n",
    );
    write(&dir, "common/vilan.toml", "[library]\nname = \"common\"\n");
    write(
        &dir,
        "app/vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n\n[package.dependencies]\ncommon = { path = \"../common\" }\n",
    );
    write(&dir, "common/src/lib.vl", COMMON);
    write(
        &dir,
        "app/src/main.vl",
        r#"import std::print;
import std::json::json_codec;
import std::result::Result::{ self, Ok, Err };
import common::StatusClient;

async fun main() {
	match StatusClient::connect("ws://localhost:1/", json_codec()) {
		Ok(let client) => print("connected"),
		Err(let error) => print("no server"),
	}
}
main();
"#,
    );
    let build = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["build", dir.to_str().unwrap()])
        .output()
        .expect("run vilan build");
    assert!(
        build.status.success(),
        "the generated connect mistyped without a consumer-side std::rpc import:\n{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
