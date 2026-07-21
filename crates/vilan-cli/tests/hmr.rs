//! End-to-end test for the A13 dev channel (hmr.md slice S1): `run --watch` on
//! a workspace with a browser leg stands up an SSE dev channel, and each watch
//! round pushes the byte-diff verdict to connected browsers — `swap` on a code
//! change, `css` on a stylesheet-only change, `error` on a compile failure (with
//! the next good round clearing it) — while the artifact routes serve the
//! shim-instrumented bundle and the CSS sidecar.
//!
//! House process hygiene (the watcher never exits on its own): the legs are
//! quick-exit (the node server prints and returns), so killing the watcher at
//! the end orphans nothing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
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

    /// The `kind` of the next `data: {json}` frame, or `None` at the deadline.
    fn next_kind(&mut self, deadline: Duration) -> Option<String> {
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
                    if let Some(kind) = kind_of(payload) {
                        return Some(kind);
                    }
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

    /// Reads events until one matches `expected` (ignoring others, e.g. the
    /// connect-time `connected`), or fails at the deadline.
    fn expect_kind(&mut self, expected: &str, deadline: Duration) {
        let start = Instant::now();
        while start.elapsed() < deadline {
            match self.next_kind(deadline - start.elapsed()) {
                Some(kind) if kind == expected => return,
                Some(_other) => continue,
                None => break,
            }
        }
        panic!("did not observe a `{expected}` event within the deadline");
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

    // Drain stdout on a thread (so the pipe never fills), forwarding the port.
    let stdout = watcher.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(port) = parse_port(&line) {
                let _ = sender.send(port);
            }
        }
    });

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let port = receiver
            .recv_timeout(Duration::from_secs(20))
            .expect("the CLI should announce `hmr: dev channel on 127.0.0.1:<port>`");
        let deadline = Duration::from_secs(20);

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

        // (b) A stylesheet-only change (bundle byte-identical) → `css`.
        write(&dir, "src/client.vl", &client_source("b", "x2"));
        sse.expect_kind("css", deadline);

        // (c) A syntax error → `error`; then a fix → the next good round.
        write(&dir, "src/client.vl", "fun main( {\n");
        sse.expect_kind("error", deadline);
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
