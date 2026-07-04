//! End-to-end tests for RPC over a real HTTP transport: a Node process serves a
//! generated `[service(Client)]` dispatcher via the `std::http` mounts and is
//! driven through `std::rpc`'s transports (host `fetch` → `node:http` on
//! localhost) — request/response round-trips, multi-session realtime sync over
//! SSE, and connection-close teardown.
//!
//! Each test writes a throwaway project and drives the built `vilan` binary;
//! self-contained apps `exit(0)` after their calls (a watchdog kills a hang),
//! and the disconnect test keeps a server child running while separate client
//! processes come and go.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
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

/// A long-running server child whose stdout lines stream to a channel (a reader
/// thread forwards them), killed on drop so a panic can't leak a listener. The
/// project is built with `vilan build` first and the bundle run under `node`
/// directly — killing a `vilan run` child would orphan its node grandchild,
/// which keeps the port bound across test runs.
struct StreamingServer {
    child: Child,
    lines: Receiver<String>,
}

impl StreamingServer {
    fn spawn(dir: &Path) -> StreamingServer {
        let build = Command::new(env!("CARGO_BIN_EXE_vilan"))
            .args(["build", dir.to_str().unwrap()])
            .output()
            .expect("run vilan build");
        assert!(
            build.status.success(),
            "server build failed:\n{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let mut child = Command::new("node")
            .arg(dir.join("src/main.js"))
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
        StreamingServer { child, lines }
    }

    /// Blocks until a stdout line containing `needle` arrives.
    fn await_line(&self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for `{needle}` from the server"
            );
            match self.lines.recv_timeout(remaining) {
                Ok(line) if line.contains(needle) => return,
                Ok(_other) => {}
                Err(_) => panic!("server stdout ended or timed out before `{needle}`"),
            }
        }
    }
}

impl Drop for StreamingServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
import std::json::json_codec;
import std::rpc::HttpTransport;
import std::rpc_server::serve_rpc;

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
	serve_rpc(45177, counter.dispatcher().into_protocol(json_codec()), |server| {
		run_client();
	});
}

fun run_client() {
	let client = Client { transport = HttpTransport { url = "http://localhost:45177/" }, codec = json_codec() };
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
import std::json::{ Json, FromJson, json_codec };
import std::reactive::Signal;
import std::rpc::{
	HttpTransport, connect_split, bridge,
	ReactiveServer, ReactiveClient, RemoteSource, DuplexEnd,
};
import std::http::Response;
import std::rpc_server::serve_connected;

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
		board.dispatcher().into_protocol(json_codec()),
		|id, end| {
			sessions.write().push((id, ReactiveServer::new(end, json_codec())));
		},
		|id| {},
		|request| Response::builder().code(404).body("nope").build(),
		|| {
			run_clients();
		},
	);
}

fun watch(name: str, base: str): Client<HttpTransport> {
	let split = connect_split(base);
	let reactive = ReactiveClient::new(bridge(split), json_codec());
	let client = Client { transport = HttpTransport { url = i"{base}/rpc" }, codec = json_codec() };
	match client.attach(split.connection) {
		Ok(let channel) => {
			let mirror: RemoteSource<i32> = reactive.source(channel);
			let _ = mirror.sub(|n| {
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

#[test]
fn a_closed_connection_tears_its_session_down_and_spares_the_rest() {
    // Connection lifecycle over SplitDuplex: a subscribed client PROCESS dies
    // (its SSE socket closes), which must fire `serve_connected`'s
    // `on_disconnect` so the app can dispose that session's `ReactiveServer` —
    // and disposing it must not disturb another session subscribed to the SAME
    // signal, which still sees a later mutation.
    let server_dir = temp_project("close_server");
    write(
        &server_dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &server_dir,
        "src/main.vl",
        r#"import std::print;
import std::shared::Shared;
import std::option::Option::{ self, Some, None };
import std::result::Result::{ self, Ok, Err };
import std::json::{ Json, FromJson, json_codec };
import std::reactive::Signal;
import std::rpc::{ ReactiveServer, DuplexEnd };
import std::http::Response;
import std::rpc_server::serve_connected;

let sessions: Shared<List<(i32, ReactiveServer)>> = Shared::new([]);

[service(Client)]
struct Board {
	count: Signal<i32>,
}

impl Board {
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

fun drop_session(target: i32) {
	mut kept: List<(i32, ReactiveServer)> = [];
	for entry in sessions.read() {
		let (id, session) = entry;
		if id == target {
			session.dispose();
		} else {
			kept.push((id, session));
		}
	}
	sessions.write() = kept;
}

fun main() {
	let board = Board { count = Signal::new(0) };
	serve_connected(
		47161,
		board.dispatcher().into_protocol(json_codec()),
		|id, end| {
			sessions.write().push((id, ReactiveServer::new(end, json_codec())));
		},
		|id| {
			drop_session(id);
			print(i"closed {id}");
		},
		|request| Response::builder().code(404).body("nope").build(),
		|| print("ready"),
	);
}
"#,
    );

    // The client processes speak the §4.1 foundation directly (`call`), since
    // the generated `Client` lives in the server's package.
    let watcher = |name: &str, also_add: &str| {
        format!(
            r#"import std::print;
import std::time::sleep;
import std::process::exit;
import std::json::{{ Json, FromJson, json_codec }};
import std::wire::Serializer;
import std::result::Result::{{ self, Ok, Err }};
import std::rpc::{{ HttpTransport, RpcError, call, connect_split, bridge, ReactiveClient, RemoteSource }};

fun main() {{
	let base = "http://localhost:47161";
	let split = connect_split(base);
	let reactive = ReactiveClient::new(bridge(split), json_codec());
	let transport = HttpTransport {{ url = i"{{base}}/rpc" }};
	let connection = split.connection;
	let attached: Result<i32, RpcError> = call(transport, json_codec(), "attach", [|s: Serializer| connection.describe(s)]);
	match attached {{
		Ok(let channel) => {{
			let mirror: RemoteSource<i32> = reactive.source(channel);
			let watching = mirror.sub(|n| {{
				print(i"{name} sees {{n}}");
			}});
		}},
		Err(let error) => print(i"attach err {{error.to_json()}}"),
	}}
	sleep(200);
{also_add}	exit(0);
}}
"#
        )
    };

    let doomed_dir = temp_project("close_doomed");
    write(
        &doomed_dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(&doomed_dir, "src/main.vl", &watcher("doomed", ""));

    let survivor_dir = temp_project("close_survivor");
    write(
        &survivor_dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &survivor_dir,
        "src/main.vl",
        &watcher(
            "survivor",
            "\tlet by = 5;\n\tlet added: Result<i32, RpcError> = call(transport, json_codec(), \"add\", [|s: Serializer| by.describe(s)]);\n\tmatch added {\n\t\tOk(let n) => print(i\"add -> {n}\"),\n\t\tErr(let error) => print(i\"add err {error.to_json()}\"),\n\t}\n\tsleep(300);\n",
        ),
    );

    let server = StreamingServer::spawn(&server_dir);
    server.await_line("ready", Duration::from_secs(60));

    // Session 0 subscribes, then its process exits — the socket close must
    // reach the app as a disconnect.
    let doomed_out = vilan_run_with_timeout(&doomed_dir, Duration::from_secs(60));
    assert!(
        doomed_out.contains("doomed sees 0"),
        "the doomed session never saw the initial value:\n{doomed_out}"
    );
    server.await_line("closed 0", Duration::from_secs(10));

    // A fresh session subscribes and mutates: the disposed session must not
    // have taken the signal's other observers with it.
    let survivor_out = vilan_run_with_timeout(&survivor_dir, Duration::from_secs(60));
    for expected in ["survivor sees 0", "add -> 5", "survivor sees 5"] {
        assert!(
            survivor_out.contains(expected),
            "missing `{expected}` in survivor output:\n{survivor_out}"
        );
    }

    drop(server);
    let _ = std::fs::remove_dir_all(&server_dir);
    let _ = std::fs::remove_dir_all(&doomed_dir);
    let _ = std::fs::remove_dir_all(&survivor_dir);
}

#[test]
fn realtime_sync_over_a_true_websocket() {
    // The WebSocket transport end to end (transport-rpc.md §5): the RFC 6455
    // handshake vector, the in-language server half (upgrade + frame layer on
    // serve_connected's port), the host-WebSocket client, and the drop-in
    // promise — this is the SSE realtime test with `connect_split` swapped for
    // `connect_socket`, nothing else.
    let dir = temp_project("websocket");
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
import std::json::{ Json, FromJson, json_codec };
import std::wire::Serializer;
import std::reactive::Signal;
import std::rpc::{
	HttpTransport, connect_socket, bridge,
	ReactiveServer, ReactiveClient, RemoteSource, DuplexEnd,
};
import std::rpc_server::{ serve_connected, ws_accept_key };
import std::http::Response;

let sessions: Shared<List<(i32, ReactiveServer)>> = Shared::new([]);

[service(Client)]
struct Board {
	count: Signal<i32>,
}

impl Board {
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
	// RFC 6455 §1.3's worked handshake example.
	let accept = ws_accept_key("dGhlIHNhbXBsZSBub25jZQ==");
	print(i"accept ok = {accept == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="}");

	let board = Board { count = Signal::new(0) };
	serve_connected(
		9291,
		board.dispatcher().into_protocol(json_codec()),
		|id, end| {
			sessions.write().push((id, ReactiveServer::new(end, json_codec())));
		},
		|id| print(i"closed {id}"),
		|request| Response::builder().code(404).body("nope").build(),
		|| {
			run_clients();
		},
	);
}

fun watch(name: str, base: str): Client<HttpTransport> {
	let socket = connect_socket("ws://localhost:9291");
	let reactive = ReactiveClient::new(bridge(socket), json_codec());
	let client = Client { transport = HttpTransport { url = i"{base}/rpc" }, codec = json_codec() };
	match client.attach(socket.connection) {
		Ok(let channel) => {
			let mirror: RemoteSource<i32> = reactive.source(channel);
			let watching = mirror.sub(|n| {
				print(i"{name} sees {n}");
			});
		},
		Err(let error) => print(i"{name} attach err {error.to_json()}"),
	}
	client
}

fun run_clients() {
	let base = "http://localhost:9291";
	let alice = watch("alice", base);
	let bob = watch("bob", base);
	sleep(300);
	match alice.add(7) {
		Ok(let n) => print(i"alice add -> {n}"),
		Err(let error) => print(i"add err {error.to_json()}"),
	}
	sleep(300);
	exit(0);
}
"#,
    );
    let stdout = vilan_run_with_timeout(&dir, Duration::from_secs(60));
    for expected in [
        "accept ok = true",
        "alice sees 0",
        "bob sees 0",
        "alice sees 7",
        "bob sees 7",
        "alice add -> 7",
    ] {
        assert!(
            stdout.contains(expected),
            "missing `{expected}` in websocket output:\n{stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rpc_and_realtime_multiplex_over_one_socket() {
    // §5's multiplex: RPC (attach, verify, interleaved adds from two clients —
    // exercising the correlation ids) and the reactive updates ALL ride each
    // client's one WebSocket; no HTTP requests after the upgrade.
    let dir = temp_project("multiplex");
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
import std::json::{ Json, FromJson, json_codec };
import std::reactive::Signal;
import std::rpc::{
	connect_socket, bridge, SocketTransport,
	ReactiveServer, ReactiveClient, RemoteSource, DuplexEnd,
};
import std::rpc_server::serve_connected;
import std::http::Response;

let sessions: Shared<List<(i32, ReactiveServer)>> = Shared::new([]);

[service(Client)]
struct Board {
	count: Signal<i32>,
}

impl Board {
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
		9293,
		board.dispatcher().into_protocol(json_codec()),
		|id, end| {
			sessions.write().push((id, ReactiveServer::new(end, json_codec())));
		},
		|id| {},
		|request| Response::builder().code(404).body("nope").build(),
		|| {
			run_clients();
		},
	);
}

// EVERYTHING rides the one socket: RPC via the transport() view, updates via
// the duplex — no HTTP requests at all after the upgrade.
fun watch(name: str): Client<SocketTransport> {
	let socket = connect_socket("ws://localhost:9293");
	let client = Client { transport = socket.transport(), codec = json_codec() };
	let reactive = ReactiveClient::new(bridge(socket), json_codec());
	match client.attach(socket.connection) {
		Ok(let channel) => {
			let mirror: RemoteSource<i32> = reactive.source(channel);
			let watching = mirror.sub(|n| {
				print(i"{name} sees {n}");
			});
		},
		Err(let error) => print(i"{name} attach err {error.to_json()}"),
	}
	client
}

fun run_clients() {
	let alice = watch("alice");
	let bob = watch("bob");
	sleep(300);
	match alice.verify() {
		Ok(let same) => print(i"verify over socket = {same}"),
		Err(let error) => print(i"verify err {error.to_json()}"),
	}
	match alice.add(7) {
		Ok(let n) => print(i"alice add -> {n}"),
		Err(let error) => print(i"add err {error.to_json()}"),
	}
	// Interleaved calls exercise the correlation ids.
	match bob.add(1) {
		Ok(let n) => print(i"bob add -> {n}"),
		Err(let error) => print(i"add err {error.to_json()}"),
	}
	sleep(300);
	exit(0);
}
"#,
    );
    let stdout = vilan_run_with_timeout(&dir, Duration::from_secs(60));
    for expected in [
        "alice sees 0",
        "bob sees 0",
        "verify over socket = true",
        "alice add -> 7",
        "alice sees 7",
        "bob sees 7",
        "bob add -> 8",
        "alice sees 8",
        "bob sees 8",
    ] {
        assert!(
            stdout.contains(expected),
            "missing `{expected}` in multiplex output:\n{stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_binary_codec_rides_the_socket_end_to_end() {
    // The reactive-on-codec slice's new capability: with `binary_codec()` on
    // both sides, RPC requests/replies AND reactive updates cross the one
    // WebSocket as BINARY messages (the tag-byte lanes: 0x64 duplex,
    // 0x72 + LE id RPC) — exercising `transmit_bytes`/`binaryType` and the
    // text/binary discrimination on the client, and the `WsEvent::Binary`
    // lanes answering in kind on the server. Same scenario as the text
    // multiplex test: interleaved adds from two clients, fan-out to both.
    let dir = temp_project("binary-socket");
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
import std::binary::binary_codec;
import std::reactive::Signal;
import std::rpc::{
	connect_socket, bridge, SocketTransport,
	ReactiveServer, ReactiveClient, RemoteSource, DuplexEnd,
};
import std::rpc_server::serve_connected;
import std::http::Response;

let sessions: Shared<List<(i32, ReactiveServer)>> = Shared::new([]);

[service(Client)]
struct Board {
	count: Signal<i32>,
}

impl Board {
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
		9294,
		board.dispatcher().into_protocol(binary_codec()),
		|id, end| {
			sessions.write().push((id, ReactiveServer::new(end, binary_codec())));
		},
		|id| {},
		|request| Response::builder().code(404).body("nope").build(),
		|| {
			run_clients();
		},
	);
}

fun watch(name: str): Client<SocketTransport> {
	let socket = connect_socket("ws://localhost:9294");
	let client = Client { transport = socket.transport(), codec = binary_codec() };
	let reactive = ReactiveClient::new(bridge(socket), binary_codec());
	match client.attach(socket.connection) {
		Ok(let channel) => {
			let mirror: RemoteSource<i32> = reactive.source(channel);
			let watching = mirror.sub(|n| {
				print(i"{name} sees {n}");
			});
		},
		Err(let error) => print(i"{name} attach err {error.to_json()}"),
	}
	client
}

fun run_clients() {
	let alice = watch("alice");
	let bob = watch("bob");
	sleep(300);
	match alice.verify() {
		Ok(let same) => print(i"verify over binary socket = {same}"),
		Err(let error) => print(i"verify err {error.to_json()}"),
	}
	match alice.add(7) {
		Ok(let n) => print(i"alice add -> {n}"),
		Err(let error) => print(i"add err {error.to_json()}"),
	}
	match bob.add(1) {
		Ok(let n) => print(i"bob add -> {n}"),
		Err(let error) => print(i"add err {error.to_json()}"),
	}
	sleep(300);
	exit(0);
}
"#,
    );
    let stdout = vilan_run_with_timeout(&dir, Duration::from_secs(60));
    for expected in [
        "alice sees 0",
        "bob sees 0",
        "verify over binary socket = true",
        "alice add -> 7",
        "alice sees 7",
        "bob sees 7",
        "bob add -> 8",
        "alice sees 8",
        "bob sees 8",
    ] {
        assert!(
            stdout.contains(expected),
            "missing `{expected}` in binary-socket output:\n{stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
