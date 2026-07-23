//! End-to-end tests for the A13 dev channel and its full-stack coordination
//! (hmr.md slices S1 and S3).
//!
//! `the_dev_channel_drives_the_watch_round` (S1): `run --watch` on a workspace
//! with a browser leg stands up an SSE dev channel, and each watch round pushes
//! the byte-diff verdict to connected browsers — `swap` on a code change, `css`
//! on a stylesheet-only change, `error` on a compile failure (with the next good
//! round clearing it) — while the artifact routes serve the shim-instrumented
//! bundle and the CSS sidecar.
//!
//! `a_server_edit_restarts_quietly_and_a_shared_edit_swaps` (S3): the two rows of
//! the §6 coordination matrix the S1 test doesn't reach — a **server-only** edit
//! restarts the Node child (witnessed by its per-source boot marker on the
//! watcher's captured stdout) while pushing *nothing* to the browser, and a
//! **shared** edit (a `common` module both legs embed) both restarts the server
//! and pushes a `swap`.
//!
//! House process hygiene (the watcher never exits on its own): the legs are
//! quick-exit (the node server prints and returns), so killing the watcher at
//! the end orphans nothing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

fn temp_project(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "vilan_hmr_cli_{tag}_{}_{unique}",
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

/// A browser client that emits one CSS line from a `const` initializer. The
/// initializer always returns `1`, so changing only `css_marker` leaves the JS
/// bundle byte-identical (a clean CSS-only round); changing `code_marker`
/// changes the bundle (a swap round).
fn client_source(code_marker: &str, css_marker: &str) -> String {
    format!(
        "import std::print;\nimport std::asset::emit;\n\nfun styles(): i32 {{\n\temit(\"css\", \".{css_marker}{{color:red}}\");\n\t1\n}}\n\nlet _s = const styles();\n\nfun main() {{\n\tprint(\"{code_marker}\");\n}}\n"
    )
}

/// Extracts the dev-channel port from the activation line
/// `hmr: dev channel on 127.0.0.1:<port>`.
fn parse_port(line: &str) -> Option<u16> {
    let rest = line.strip_prefix("hmr: dev channel on 127.0.0.1:")?;
    rest.trim().parse().ok()
}

/// A raw SSE client over a `TcpStream`, accumulating bytes and yielding one
/// event `kind` at a time (skipping the whitespace of the HTTP head and the
/// `data:`/blank-line framing).
struct SseClient {
    stream: TcpStream,
    buffer: Vec<u8>,
    cursor: usize,
}

impl SseClient {
    fn connect(port: u16) -> SseClient {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to dev channel");
        stream
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("send SSE request");
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        SseClient {
            stream,
            buffer: Vec::new(),
            cursor: 0,
        }
    }

    /// The raw JSON payload of the next `data: {json}` frame, or `None` at the
    /// deadline.
    fn next_payload(&mut self, deadline: Duration) -> Option<String> {
        let start = Instant::now();
        loop {
            // Consume any complete line already buffered.
            while let Some(newline) = self.buffer[self.cursor..]
                .iter()
                .position(|&byte| byte == b'\n')
            {
                let line_end = self.cursor + newline;
                let line =
                    String::from_utf8_lossy(&self.buffer[self.cursor..line_end]).into_owned();
                self.cursor = line_end + 1;
                if let Some(payload) = line.trim_end().strip_prefix("data: ") {
                    return Some(payload.to_string());
                }
            }
            if start.elapsed() >= deadline {
                return None;
            }
            let mut chunk = [0u8; 1024];
            match self.stream.read(&mut chunk) {
                Ok(0) => return None,
                Ok(read) => self.buffer.extend_from_slice(&chunk[..read]),
                // A read timeout is expected between rounds — keep waiting.
                Err(_) => {}
            }
        }
    }

    /// The `kind` of the next `data: {json}` frame, or `None` at the deadline.
    fn next_kind(&mut self, deadline: Duration) -> Option<String> {
        self.next_payload(deadline)
            .and_then(|payload| kind_of(&payload))
    }

    /// Reads events until one matches `expected` (ignoring others, e.g. the
    /// connect-time `connected`), or fails at the deadline.
    fn expect_kind(&mut self, expected: &str, deadline: Duration) {
        self.expect_event(expected, deadline);
    }

    /// Like [`SseClient::expect_kind`], but returns the matching event's raw JSON
    /// payload — so a test can inspect its `message` (the error overlay's text)
    /// or `asset` (the css sidecar's name).
    fn expect_event(&mut self, expected: &str, deadline: Duration) -> String {
        let start = Instant::now();
        while start.elapsed() < deadline {
            match self.next_payload(deadline - start.elapsed()) {
                Some(payload) if kind_of(&payload).as_deref() == Some(expected) => return payload,
                Some(_other) => continue,
                None => break,
            }
        }
        panic!("did not observe a `{expected}` event within the deadline");
    }

    /// Asserts that none of the `forbidden` event kinds arrive within `window`.
    /// Other kinds (the connect-time `connected`) are ignored — this is the
    /// server-only round's "the browser is told nothing" assertion. The push (if
    /// any) is issued in the same watch round that restarts the Node child, so a
    /// short window after the restart evidence is enough: a spurious event would
    /// already be buffered on the socket.
    fn assert_no(&mut self, forbidden: &[&str], window: Duration) {
        let start = Instant::now();
        while start.elapsed() < window {
            match self.next_kind(window - start.elapsed()) {
                Some(kind) => assert!(
                    !forbidden.contains(&kind.as_str()),
                    "a `{kind}` event was pushed during the quiet window \
                     (a server-only round must be silent)"
                ),
                None => break,
            }
        }
    }
}

/// The `"kind"` field of a tiny event JSON body, by hand (no JSON crate).
fn kind_of(json: &str) -> Option<String> {
    let after = json.split("\"kind\":\"").nth(1)?;
    Some(after.split('"').next()?.to_string())
}

/// A plain (non-SSE) HTTP GET against the dev channel, returning the response
/// body as bytes (the connection closes after the response).
fn http_get(port: u16, path: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect for GET");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("send GET");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    // Split off the body after the header terminator.
    let separator = b"\r\n\r\n";
    match response
        .windows(separator.len())
        .position(|window| window == separator)
    {
        Some(index) => response[index + separator.len()..].to_vec(),
        None => response,
    }
}

/// Waits (bounded) for `path` to exist — round 1 has written `dist/`.
fn wait_for_file(path: &Path, deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Drains the watcher's stdout on a thread, forwarding every line to a channel.
/// The Node server's `print` output flows here too — `spawn_node` gives the child
/// no stdio of its own, so it inherits the watcher's stdout (the piped fd) — which
/// is how the coordination-matrix test witnesses a server restart: a per-source
/// boot marker printed by the freshly spawned child.
fn drain_stdout(stdout: ChildStdout) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
    receiver
}

/// Waits (bounded) for the activation line and returns its announced port.
fn wait_for_port(lines: &mpsc::Receiver<String>, deadline: Duration) -> Option<u16> {
    let start = Instant::now();
    while start.elapsed() < deadline {
        match lines.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                if let Some(port) = parse_port(&line) {
                    return Some(port);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
    None
}

/// Waits (bounded) for a stdout line containing `needle` (a server boot marker).
fn wait_for_line(lines: &mpsc::Receiver<String>, needle: &str, deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        match lines.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                if line.contains(needle) {
                    return true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
    false
}

#[test]
fn the_dev_channel_drives_the_watch_round() {
    let dir = temp_project("channel");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n",
    );
    write(&dir, "src/client.vl", &client_source("a", "x1"));
    write(
        &dir,
        "src/server.vl",
        "import std::print;\n\nfun main() {\n\tprint(\"server\");\n}\n",
    );

    // `--hmr-port 0` asks for an ephemeral port; the CLI announces the bound one.
    let mut watcher = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", "--watch", "--hmr-port", "0", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run --watch");

    // Drain stdout on a thread (so the pipe never fills), forwarding every line.
    let lines = drain_stdout(watcher.stdout.take().unwrap());

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let deadline = Duration::from_secs(20);
        let port = wait_for_port(&lines, deadline)
            .expect("the CLI should announce `hmr: dev channel on 127.0.0.1:<port>`");

        // Round 1 has run once `dist/client.css` lands; a margin ensures the
        // watcher's baseline snapshot is taken before the first edit (so the
        // edit is seen as a change).
        assert!(
            wait_for_file(&dir.join("dist/client.css"), deadline),
            "round 1 should have written dist/client.css"
        );
        std::thread::sleep(Duration::from_millis(500));

        let mut sse = SseClient::connect(port);

        // (a) A code change → `swap`.
        write(&dir, "src/client.vl", &client_source("b", "x1"));
        sse.expect_kind("swap", deadline);

        // (b) A stylesheet-only change (bundle byte-identical) → `css`, and the
        // event names its sidecar so the shim bumps only that stylesheet <link>.
        write(&dir, "src/client.vl", &client_source("b", "x2"));
        let css_event = sse.expect_event("css", deadline);
        assert!(
            css_event.contains("\"asset\":\"client.css\""),
            "the css event should name its changed sidecar: {css_event}"
        );

        // (c) A syntax error → `error` carrying the REAL compiler diagnostics
        // (the S1 residue closed): the message names the failing file and the
        // actual parse error — not the old generic "build failed" string.
        write(&dir, "src/client.vl", "fun main( {\n");
        let error_event = sse.expect_event("error", deadline);
        assert!(
            error_event.contains("client.vl"),
            "the error event should name the failing file: {error_event}"
        );
        assert!(
            error_event.contains("expected"),
            "the error event should carry the real parse diagnostic, \
             not the generic fallback: {error_event}"
        );
        assert!(
            !error_event.contains("build failed — see the terminal"),
            "the generic fallback string must be gone now that real text is threaded: {error_event}"
        );
        // Terminal-unchanged A/B: the SAME diagnostic is still rendered to the
        // watcher's stdout (ariadne), in the same round — the overlay capture is
        // additive (a second sink), never a redirect.
        assert!(
            wait_for_line(&lines, "expected", deadline),
            "the terminal must still print the diagnostic (the overlay capture is additive)"
        );
        // A fix → the next good round (which clears the overlay browser-side).
        write(&dir, "src/client.vl", &client_source("c", "x2"));
        sse.expect_kind("swap", deadline);

        // (d) The artifact routes: the browser bundle carries the shim (the
        // singleton marker), and the sidecar serves the current CSS.
        let bundle = String::from_utf8_lossy(&http_get(port, "/bundle/client.js")).into_owned();
        assert!(
            bundle.contains("window.__VILAN_HMR__"),
            "the served bundle should carry the dev-runtime shim:\n{bundle}"
        );
        let css = String::from_utf8_lossy(&http_get(port, "/asset/client.css")).into_owned();
        assert_eq!(
            css, ".x2{color:red}\n",
            "the sidecar should serve the current CSS"
        );

        // Path traversal is refused.
        let traversal = http_get(port, "/bundle/../secret.js");
        assert!(
            traversal.is_empty(),
            "a traversal path must not serve any bytes"
        );
    }));

    let _ = watcher.kill();
    let _ = watcher.wait();
    let _ = std::fs::remove_dir_all(&dir);
    outcome.unwrap();
}

/// A `common` library both legs import (`pkg::common::banner`). Editing it
/// changes both bundles — the shared-edit row of the §6 matrix.
fn common_source(banner: &str) -> String {
    format!("fun banner(): str {{\n\t\"{banner}\"\n}}\n")
}

/// A browser client that embeds `banner()` (so a shared edit changes this
/// bundle) and emits one CSS line (so the sidecar exists but a server-only edit
/// leaves it untouched — the "no css either" half of the quiet assertion).
fn shared_client_source(css_marker: &str) -> String {
    format!(
        "import std::print;\nimport std::asset::emit;\nimport pkg::common::banner;\n\n\
         fun styles(): i32 {{\n\temit(\"css\", \".{css_marker}{{color:red}}\");\n\t1\n}}\n\n\
         let _s = const styles();\n\nfun main() {{\n\tprint(banner());\n}}\n"
    )
}

/// A server that prints a per-source boot marker AND the shared banner, so the
/// watcher's captured stdout witnesses each restart: a server-only edit bumps
/// the marker; a shared edit bumps the banner.
fn shared_server_source(server_marker: &str) -> String {
    format!(
        "import std::print;\nimport pkg::common::banner;\n\n\
         fun main() {{\n\tprint(\"server-up {server_marker} banner=\" + banner());\n}}\n"
    )
}

/// The two §6 coordination-matrix rows the S1 e2e doesn't reach (hmr.md §§6, 11
/// S3): a server-only edit restarts the Node child while pushing nothing to the
/// browser, and a shared edit (a `common` module both legs embed) restarts the
/// server AND pushes a `swap`.
#[test]
fn a_server_edit_restarts_quietly_and_a_shared_edit_swaps() {
    let dir = temp_project("matrix");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n",
    );
    write(&dir, "src/common.vl", &common_source("BANNER_ONE"));
    write(&dir, "src/client.vl", &shared_client_source("x1"));
    write(&dir, "src/server.vl", &shared_server_source("SRVMARK_ONE"));

    let mut watcher = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", "--watch", "--hmr-port", "0", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run --watch");

    let lines = drain_stdout(watcher.stdout.take().unwrap());

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let deadline = Duration::from_secs(20);
        let port = wait_for_port(&lines, deadline)
            .expect("the CLI should announce `hmr: dev channel on 127.0.0.1:<port>`");

        // Round 1 is done once dist lands and the server has printed its boot
        // marker; a margin then ensures the watcher's baseline snapshot is taken
        // before the first edit (so the edit registers as a change).
        assert!(
            wait_for_file(&dir.join("dist/client.js"), deadline),
            "round 1 should have written dist/client.js"
        );
        assert!(
            wait_for_line(&lines, "SRVMARK_ONE", deadline),
            "the server leg should have booted in round 1"
        );
        std::thread::sleep(Duration::from_millis(800));

        let mut sse = SseClient::connect(port);

        // Row 1 — server-only edit: the server bundle changes, the client bundle
        // does not. The Node child restarts (its new boot marker appears on
        // stdout) and NO `swap`/`css` reaches the connected browser — K6
        // reconnect carries it across the restart (hmr.md §6). Observing the
        // restart first makes the quiet window deterministic: the round's push
        // (here, none) is issued before the child it spawned can print.
        write(&dir, "src/server.vl", &shared_server_source("SRVMARK_TWO"));
        assert!(
            wait_for_line(&lines, "SRVMARK_TWO", deadline),
            "a server-only edit should restart the Node child"
        );
        sse.assert_no(&["swap", "css"], Duration::from_millis(2000));

        // Row 2 — shared edit: a change to `common.vl`, which both legs embed.
        // The server restarts (the banner it prints changes) AND a `swap` reaches
        // the browser (its bundle changed too, so the byte-diff classifies both).
        write(&dir, "src/common.vl", &common_source("BANNER_TWO"));
        sse.expect_kind("swap", deadline);
        assert!(
            wait_for_line(&lines, "banner=BANNER_TWO", deadline),
            "a shared edit should restart the Node child with the new shared code"
        );
    }));

    let _ = watcher.kill();
    let _ = watcher.wait();
    let _ = std::fs::remove_dir_all(&dir);
    outcome.unwrap();
}

/// The per-leg skip (backlog E12, half b): a client-only edit recompiles the
/// client and SKIPS the server — the server's `.vl` sources are unchanged, so
/// its previous artifact is reused and the round prints `hmr: skipped server
/// (sources unchanged)` — while the served client bundle still reflects the
/// edit (the parse cache is content-keyed, never stale). Same single-watcher,
/// quick-exit-legs hygiene as the matrix test.
#[test]
fn a_client_only_edit_skips_the_server_and_still_updates_the_client() {
    let dir = temp_project("skip");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n",
    );
    write(
        &dir,
        "src/client.vl",
        &client_source("clientmark_one", "x1"),
    );
    write(
        &dir,
        "src/server.vl",
        "import std::print;\n\nfun main() {\n\tprint(\"server-booted\");\n}\n",
    );

    let mut watcher = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", "--watch", "--hmr-port", "0", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run --watch");

    let lines = drain_stdout(watcher.stdout.take().unwrap());

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let deadline = Duration::from_secs(20);
        let port = wait_for_port(&lines, deadline)
            .expect("the CLI should announce `hmr: dev channel on 127.0.0.1:<port>`");

        // Round 1 compiles both legs and boots the server; a margin then ensures
        // the watcher's baseline snapshot precedes the edit.
        assert!(
            wait_for_file(&dir.join("dist/client.js"), deadline),
            "round 1 should have written dist/client.js"
        );
        assert!(
            wait_for_line(&lines, "server-booted", deadline),
            "the server leg should have booted in round 1"
        );
        std::thread::sleep(Duration::from_millis(800));

        let mut sse = SseClient::connect(port);
        let bundle_before =
            String::from_utf8_lossy(&http_get(port, "/bundle/client.js")).into_owned();
        assert!(
            bundle_before.contains("clientmark_one"),
            "the round-1 client bundle carries the original marker"
        );
        let server_before = std::fs::read(dir.join("dist/server.js")).expect("dist/server.js");

        // A client-only edit: the client bundle changes, the server's sources do
        // not — so the round SKIPS the server (prints the skip line) and pushes a
        // `swap` for the client.
        write(
            &dir,
            "src/client.vl",
            &client_source("clientmark_two", "x1"),
        );
        assert!(
            wait_for_line(&lines, "hmr: skipped server (sources unchanged)", deadline),
            "a client-only edit must skip recompiling the server"
        );
        sse.expect_kind("swap", deadline);

        // The served client bundle reflects the edit — the content-keyed cache
        // returns the NEW parse, never the stale one.
        let bundle_after =
            String::from_utf8_lossy(&http_get(port, "/bundle/client.js")).into_owned();
        assert!(
            bundle_after.contains("clientmark_two"),
            "the served client bundle must reflect the edit:\n{bundle_after}"
        );
        assert!(
            !bundle_after.contains("clientmark_one"),
            "the stale client content must be gone"
        );

        // Reuse fidelity: the skipped server leg's dist bytes are the round-1
        // artifact, untouched by the skip round.
        let server_after = std::fs::read(dir.join("dist/server.js")).expect("dist/server.js");
        assert_eq!(
            server_after, server_before,
            "a skipped leg's dist bytes must be exactly the reused artifact"
        );
        server_after
    }));

    let _ = watcher.kill();
    let _ = watcher.wait();

    // The cache-hit A/B (review finding, 2026-07-21): after a round that went
    // THROUGH the caches (round 2 skipped the server; the client compiled via
    // parse-cache hits for std), a fresh one-shot build of the same sources
    // must reproduce the reused server bundle byte-for-byte.
    if let Ok(reused) = &outcome {
        let output = Command::new(env!("CARGO_BIN_EXE_vilan"))
            .args(["build", dir.to_str().unwrap()])
            .output()
            .expect("run one-shot build");
        assert!(
            output.status.success(),
            "the one-shot rebuild should succeed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fresh = std::fs::read(dir.join("dist/server.js")).expect("dist/server.js");
        assert_eq!(
            &fresh, reused,
            "a one-shot build must equal the reused (cache-hit round) artifact"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    outcome.unwrap();
}

/// Removes ANSI SGR escape sequences (`\x1b[…m`) so a terminal capture can be
/// asserted as plain text regardless of coloring.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character == '\x1b' {
            // Consume the escape body up to and including its final letter.
            for escape_char in chars.by_ref() {
                if escape_char.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(character);
        }
    }
    out
}

/// Terminal-unchanged (the overlay capture is additive, not a redirect): a broken
/// one-shot `vilan build` still renders the ariadne diagnostic to the terminal —
/// the `build` path passes no overlay sink, so its output is the pre-change shape.
/// This pins the key lines; the HMR path shares the same `compile_to_js`/`report`
/// terminal rendering, so its terminal output is unchanged too.
#[test]
fn a_broken_build_still_renders_the_terminal_diagnostic() {
    let dir = temp_project("terminal");
    write(&dir, "vilan.toml", "[package]\nname = \"app\"\n");
    write(&dir, "src/main.vl", "fun main( {\n");
    let output = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["build", dir.to_str().unwrap()])
        .output()
        .expect("run vilan build");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(!output.status.success(), "a broken build must fail");
    // ariadne renders diagnostics to STDOUT; strip ANSI to assert the plain shape.
    let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    assert!(
        stdout.contains("Error:"),
        "the ariadne error header is present:\n{stdout}"
    );
    assert!(
        stdout.contains("expected"),
        "the real diagnostic message is present:\n{stdout}"
    );
    assert!(
        stdout.contains("main.vl"),
        "the diagnostic names the source file:\n{stdout}"
    );
}

/// A node leg whose `main` prints a distinguishing marker — so the watcher's
/// captured stdout witnesses which Node leg actually ran.
fn node_marker(marker: &str) -> String {
    format!("import std::print;\n\nfun main() {{\n\tprint(\"{marker}\");\n}}\n")
}

/// Accumulates the watcher's stdout into a shared buffer (rather than a consuming
/// channel), so a test can both wait for a marker AND assert one never appears —
/// the negative the A15 test needs (the non-selected leg must never run).
fn collect_stdout(stdout: ChildStdout) -> Arc<Mutex<Vec<String>>> {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let sink = buffer.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            sink.lock().unwrap().push(line);
        }
    });
    buffer
}

/// Polls the accumulated stdout for any line containing `needle`, up to `deadline`.
fn buffer_has(buffer: &Arc<Mutex<Vec<String>>>, needle: &str, deadline: Duration) -> bool {
    let start = Instant::now();
    loop {
        if buffer
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains(needle))
        {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Polls the accumulated stdout for the dev-channel activation line's port.
fn port_from_buffer(buffer: &Arc<Mutex<Vec<String>>>, deadline: Duration) -> Option<u16> {
    let start = Instant::now();
    loop {
        if let Some(port) = buffer
            .lock()
            .unwrap()
            .iter()
            .find_map(|line| parse_port(line))
        {
            return Some(port);
        }
        if start.elapsed() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A15 (`--entry`): a workspace with TWO Node legs (the kolt shape — a `server`
/// and a `probe`) plus a browser leg. `run --watch --entry server` runs the
/// chosen `server` leg (its boot marker appears), while the non-selected `probe`
/// leg still COMPILES into the workspace (`dist/probe.js` exists) but is never
/// launched (its marker never appears). HMR rounds then work under the selection:
/// a client edit swaps, a server edit restarts the chosen leg — and the probe
/// still never runs. Same single-watcher, quick-exit-legs process hygiene as the
/// matrix test.
#[test]
fn run_watch_honors_entry_and_hmr_rounds_work_for_the_chosen_leg() {
    let dir = temp_project("entry");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n\
         [entry.server]\n\n[entry.probe]\n",
    );
    write(&dir, "src/client.vl", &client_source("c1", "x1"));
    write(&dir, "src/server.vl", &node_marker("SERVER_UP one"));
    write(&dir, "src/probe.vl", &node_marker("PROBE_RAN"));

    let mut watcher = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args([
            "run",
            "--watch",
            "--hmr-port",
            "0",
            "--entry",
            "server",
            dir.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run --watch --entry");

    let buffer = collect_stdout(watcher.stdout.take().unwrap());

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let deadline = Duration::from_secs(20);
        let port = port_from_buffer(&buffer, deadline)
            .expect("the CLI should announce `hmr: dev channel on 127.0.0.1:<port>`");

        // Round 1: the browser leg compiled, the SELECTED server ran, and the
        // non-selected probe COMPILED (its bundle exists) but never RAN.
        assert!(
            wait_for_file(&dir.join("dist/client.js"), deadline),
            "round 1 should have written dist/client.js"
        );
        assert!(
            buffer_has(&buffer, "SERVER_UP one", deadline),
            "the `--entry server` leg should run"
        );
        assert!(
            dir.join("dist/probe.js").exists(),
            "the non-selected probe leg still compiles into the workspace"
        );
        assert!(
            !buffer_has(&buffer, "PROBE_RAN", Duration::from_millis(700)),
            "the non-selected probe leg must not be launched"
        );
        std::thread::sleep(Duration::from_millis(800));

        let mut sse = SseClient::connect(port);

        // A client edit → the browser swaps under the selected-entry watcher.
        write(&dir, "src/client.vl", &client_source("c2", "x1"));
        sse.expect_kind("swap", deadline);

        // A server edit → the chosen Node child restarts (its new marker prints);
        // nothing is pushed to the browser and the probe still never runs.
        write(&dir, "src/server.vl", &node_marker("SERVER_UP two"));
        assert!(
            buffer_has(&buffer, "SERVER_UP two", deadline),
            "a server edit should restart the `--entry` leg"
        );
        sse.assert_no(&["swap", "css"], Duration::from_millis(1500));
        assert!(
            !buffer_has(&buffer, "PROBE_RAN", Duration::from_millis(200)),
            "the probe leg still never runs"
        );
    }));

    let _ = watcher.kill();
    let _ = watcher.wait();
    let _ = std::fs::remove_dir_all(&dir);
    outcome.unwrap();
}

/// A/B (backlog E12): the content-addressed parse cache and the watch path must
/// not change a byte. A one-shot `vilan build` and a `run --watch` round compile
/// the SAME sources; the server leg (a node bundle, uninstrumented in both) must
/// come out byte-identical — proving the caching/skip machinery is transparent
/// to emitted output, the same guarantee the corpus gate makes for one-shot.
#[test]
fn a_watch_round_server_bundle_equals_a_one_shot_build() {
    let dir = temp_project("ab");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n",
    );
    write(&dir, "src/client.vl", &client_source("ab_client", "x1"));
    write(
        &dir,
        "src/server.vl",
        "import std::print;\n\nfun main() {\n\tprint(\"server-booted\");\n}\n",
    );

    // One-shot build (a fresh process, cold cache) → capture the server bundle.
    let status = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["build", dir.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run vilan build");
    assert!(status.success(), "the one-shot build should succeed");
    let one_shot_server =
        std::fs::read(dir.join("dist/server.js")).expect("build wrote dist/server.js");

    // A watch round rewrites dist/ from the same sources; its (uninstrumented)
    // server bundle must match byte-for-byte.
    let mut watcher = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", "--watch", "--hmr-port", "0", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run --watch");
    let lines = drain_stdout(watcher.stdout.take().unwrap());

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let deadline = Duration::from_secs(20);
        wait_for_port(&lines, deadline).expect("the dev channel should announce its port");
        // The server boots only after the round has written every dist bundle.
        assert!(
            wait_for_line(&lines, "server-booted", deadline),
            "round 1 should compile and boot the server"
        );
        let watched_server = std::fs::read(dir.join("dist/server.js"))
            .expect("the watch round wrote dist/server.js");
        assert_eq!(
            one_shot_server, watched_server,
            "a watch round's server bundle must be byte-identical to a one-shot build's"
        );
    }));

    let _ = watcher.kill();
    let _ = watcher.wait();
    let _ = std::fs::remove_dir_all(&dir);
    outcome.unwrap();
}
