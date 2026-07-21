//! The A13 dev channel (hmr.md §§2, 6, and slice S1). A tiny HTTP endpoint
//! hand-rolled on `std::net::TcpListener`, bound to `127.0.0.1` only, in keeping
//! with the dependency-free watcher — Server-Sent Events need no websocket
//! handshake and no crate. It serves three routes:
//!
//! - `GET /events` — an SSE stream held open; on connect the current build
//!   version, then one event per watch round (`swap` / `css` / `reload` /
//!   `error`).
//! - `GET /bundle/<leg>.js` and `GET /asset/<leg>.css` — the current artifacts
//!   from `dist/`, with `Access-Control-Allow-Origin: *`. Only bare
//!   `<name>.<ext>` names resolve; anything with a path separator or `..` is a
//!   404 (the traversal guard).
//!
//! The browser side is [`SHIM`], a small dev-runtime prepended to browser-leg
//! bundles by an HMR-active `run --watch` (never by `build`, so goldens are
//! untouched). [`classify`] is the pure byte-diff decision the watch round runs
//! each rebuild; it is unit-tested without processes.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The default dev-channel port (hmr.md §2). `--hmr-port 0` asks the OS for an
/// ephemeral port instead.
pub const DEFAULT_HMR_PORT: u16 = 35917;

/// The dev-runtime shim, prepended to each browser leg's bundle when HMR is
/// active. Its `__VILAN_HMR_PORT__` / `__VILAN_HMR_VERSION__` /
/// `__VILAN_HMR_BUNDLE__` placeholders are substituted at write time by
/// [`instrument`].
const SHIM: &str = include_str!("hmr_shim.js");

/// One browser leg's bundle, with the shim's port, version, and own bundle name
/// embedded (the last so a `swap` event can fetch `/bundle/<leg>.js`, hmr.md §3).
pub fn instrument(bundle: &str, port: u16, version: u64, leg: &str) -> String {
    let shim = SHIM
        .replace("__VILAN_HMR_PORT__", &port.to_string())
        .replace("__VILAN_HMR_VERSION__", &version.to_string())
        .replace("__VILAN_HMR_BUNDLE__", leg);
    format!("{shim}\n{bundle}")
}

/// A live dev channel: an SSE server running on a background thread, plus the
/// shared build version the main watch thread bumps and embeds. Dropping it
/// leaves the accept thread parked on a socket that closes when the process
/// exits — a dev tool, not a service to shut down cleanly.
pub struct DevChannel {
    port: u16,
    version: Arc<AtomicU64>,
    clients: Arc<Mutex<Vec<TcpStream>>>,
}

impl DevChannel {
    /// Binds the channel on `127.0.0.1:port` (port `0` ⇒ an OS-assigned
    /// ephemeral port) and spawns the accept loop. `dist` is the directory the
    /// artifact routes serve from. `Err` if the port is already in use — the
    /// caller warns and continues watching without HMR.
    pub fn bind(port: u16, dist: PathBuf) -> std::io::Result<DevChannel> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let version = Arc::new(AtomicU64::new(0));
        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let version = version.clone();
            let clients = clients.clone();
            std::thread::spawn(move || serve(listener, clients, version, dist));
        }
        Ok(DevChannel {
            port,
            version,
            clients,
        })
    }

    /// The bound port (the actual one when `0` was requested).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// This build's version — the counter embedded in the browser shim.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// Advances to the next build version (a new browser bundle was written).
    pub fn bump_version(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    /// Pushes one round event to every connected client, pruning any whose
    /// socket has closed (detected on write failure). `message` is carried only
    /// by `error` events.
    pub fn push(&self, kind: &str, message: Option<&str>) {
        let payload = event_json(kind, self.version(), message);
        let frame = sse_frame(&payload);
        let mut clients = self.clients.lock().unwrap();
        clients.retain_mut(|stream| {
            stream
                .write_all(frame.as_bytes())
                .and_then(|()| stream.flush())
                .is_ok()
        });
    }
}

/// The SSE wire framing for one payload: `data: <json>\n\n`. The payload is
/// always single-line JSON (a message's newlines are `\n`-escaped by
/// [`escape_json`]), so the blank-line terminator is never ambiguous.
pub fn sse_frame(payload: &str) -> String {
    format!("data: {payload}\n\n")
}

/// One event's JSON body: `{"kind":..,"version":N}`, plus `"message":".."` for
/// an `error`.
pub fn event_json(kind: &str, version: u64, message: Option<&str>) -> String {
    match message {
        Some(message) => format!(
            "{{\"kind\":\"{kind}\",\"version\":{version},\"message\":\"{}\"}}",
            escape_json(message)
        ),
        None => format!("{{\"kind\":\"{kind}\",\"version\":{version}}}"),
    }
}

/// Escapes a string for embedding in a JSON string literal (the diagnostic text
/// of an `error` event).
fn escape_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32))
            }
            other => out.push(other),
        }
    }
    out
}

/// The accept loop: one thread per connection (fine for a localhost dev tool).
fn serve(
    listener: TcpListener,
    clients: Arc<Mutex<Vec<TcpStream>>>,
    version: Arc<AtomicU64>,
    dist: PathBuf,
) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let clients = clients.clone();
        let version = version.clone();
        let dist = dist.clone();
        std::thread::spawn(move || handle(stream, clients, version, dist));
    }
}

/// Handles one connection: parse the request line, ignore headers, route.
fn handle(
    mut stream: TcpStream,
    clients: Arc<Mutex<Vec<TcpStream>>>,
    version: Arc<AtomicU64>,
    dist: PathBuf,
) {
    let Some(request_line) = read_request_head(&mut stream) else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if method != "GET" {
        respond_404(&mut stream);
        return;
    }
    if path == "/events" {
        // Server-Sent Events: hold the connection open, send the current
        // version immediately, then hand the socket to the client registry so
        // each watch round can push to it. `Access-Control-Allow-Origin: *`
        // because the page origin is the user's server, not the CLI.
        let headers = "HTTP/1.1 200 OK\r\n\
             Content-Type: text/event-stream\r\n\
             Cache-Control: no-cache\r\n\
             Connection: keep-alive\r\n\
             Access-Control-Allow-Origin: *\r\n\r\n";
        if stream.write_all(headers.as_bytes()).is_err() {
            return;
        }
        let hello = sse_frame(&event_json(
            "connected",
            version.load(Ordering::SeqCst),
            None,
        ));
        if stream.write_all(hello.as_bytes()).is_err() || stream.flush().is_err() {
            return;
        }
        clients.lock().unwrap().push(stream);
        return;
    }
    if let Some(name) = path.strip_prefix("/bundle/") {
        serve_asset(&mut stream, &dist, name, "js");
        return;
    }
    if let Some(name) = path.strip_prefix("/asset/") {
        serve_asset(&mut stream, &dist, name, "css");
        return;
    }
    respond_404(&mut stream);
}

/// Reads the whole HTTP request head (through the blank `\r\n\r\n` line) and
/// returns its first line (`GET /path HTTP/1.1`); the rest is ignored. Draining
/// the head matters: leaving unread request bytes in the socket buffer makes the
/// close after a response an RST instead of a FIN, which discards the not-yet-
/// delivered response body. A bounded read guards a client that never terminates
/// its head. `None` if the connection closed before any head arrived.
fn read_request_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") || head.len() > 16384 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if head.is_empty() {
        return None;
    }
    String::from_utf8_lossy(&head)
        .lines()
        .next()
        .map(str::to_string)
}

/// Serves `dist/<name>` for an artifact route, with the traversal guard: only a
/// bare `<base>.<ext>` name (matching the route's extension) resolves; anything
/// with a path separator or `..` is a 404.
fn serve_asset(stream: &mut TcpStream, dist: &std::path::Path, name: &str, ext: &str) {
    if !is_safe_asset_name(name, ext) {
        respond_404(stream);
        return;
    }
    match std::fs::read(dist.join(name)) {
        Ok(bytes) => {
            let content_type = if ext == "js" {
                "text/javascript"
            } else {
                "text/css"
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: {content_type}; charset=utf-8\r\n\
                 Content-Length: {}\r\n\
                 Access-Control-Allow-Origin: *\r\n\
                 Cache-Control: no-cache\r\n\r\n",
                bytes.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&bytes);
        }
        Err(_) => respond_404(stream),
    }
}

/// Whether `name` is a bare `<base>.<ext>` artifact name — no path separators,
/// no `..`, ending in the route's extension. The traversal guard.
pub fn is_safe_asset_name(name: &str, ext: &str) -> bool {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    let suffix = format!(".{ext}");
    // A bare `.js`/`.css` with no stem isn't a real artifact name either.
    name.ends_with(&suffix) && name.len() > suffix.len()
}

fn respond_404(stream: &mut TcpStream) {
    let response = "HTTP/1.1 404 Not Found\r\n\
         Content-Length: 0\r\n\
         Access-Control-Allow-Origin: *\r\n\r\n";
    let _ = stream.write_all(response.as_bytes());
}

// --- The byte-diff classifier (hmr.md §§0.2, 6) ------------------------------

/// One build leg's artifacts for a round, as the classifier sees them: the
/// **raw** bundle bytes (before the shim is prepended — the shim embeds the
/// version, so shim-inclusive bytes differ every round) and the assembled CSS
/// sidecar content, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegArtifact {
    pub name: String,
    pub is_browser: bool,
    pub bundle: String,
    pub css: Option<String>,
}

/// What the browser is told changed this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// A browser bundle changed — reload (S1) / state-preserving swap (S2).
    Swap,
    /// Only a CSS sidecar changed — hot-swap the stylesheet, no reload.
    Css,
}

/// The round's decision: whether to restart the Node child, what (if anything)
/// to push to browsers, and whether a new browser bundle means a version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundDecision {
    pub restart_server: bool,
    pub push: Option<Push>,
    pub bump_version: bool,
}

/// Classifies a rebuild by comparing this round's raw artifacts to the previous
/// round's (hmr.md §6):
///
/// - **first round** (`previous` empty) → spawn the child, bump to version 1,
///   push nothing (no clients yet);
/// - **server bundle changed** → restart the child (and still classify the
///   client legs); a server-only change pushes nothing — K6 reconnect carries
///   the client across the restart;
/// - **browser bundle changed** → `swap`, and a version bump;
/// - **no bundle changed but a CSS sidecar changed** → `css`;
/// - **nothing changed** → nothing, no restart.
///
/// A compile failure is handled by the caller (push `error`, retain the old
/// artifacts) and never reaches this classifier.
pub fn classify(previous: &[LegArtifact], next: &[LegArtifact]) -> RoundDecision {
    if previous.is_empty() {
        return RoundDecision {
            restart_server: true,
            push: None,
            bump_version: true,
        };
    }
    let prior = |name: &str| previous.iter().find(|leg| leg.name == name);
    let mut server_changed = false;
    let mut browser_changed = false;
    let mut css_changed = false;
    for leg in next {
        match prior(&leg.name) {
            Some(old) => {
                if old.bundle != leg.bundle {
                    if leg.is_browser {
                        browser_changed = true;
                    } else {
                        server_changed = true;
                    }
                }
                if old.css != leg.css {
                    css_changed = true;
                }
            }
            // A newly-appearing leg is a change of its class.
            None => {
                if leg.is_browser {
                    browser_changed = true;
                } else {
                    server_changed = true;
                }
                if leg.css.is_some() {
                    css_changed = true;
                }
            }
        }
    }
    // A leg that vanished between rounds is likewise a change of its class.
    for old in previous {
        if !next.iter().any(|leg| leg.name == old.name) {
            if old.is_browser {
                browser_changed = true;
            } else {
                server_changed = true;
            }
        }
    }
    RoundDecision {
        restart_server: server_changed,
        push: if browser_changed {
            Some(Push::Swap)
        } else if css_changed {
            Some(Push::Css)
        } else {
            None
        },
        bump_version: browser_changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(name: &str, is_browser: bool, bundle: &str, css: Option<&str>) -> LegArtifact {
        LegArtifact {
            name: name.to_string(),
            is_browser,
            bundle: bundle.to_string(),
            css: css.map(str::to_string),
        }
    }

    fn round(server: &str, client: &str, css: Option<&str>) -> Vec<LegArtifact> {
        vec![
            leg("server", false, server, None),
            leg("client", true, client, css),
        ]
    }

    #[test]
    fn first_round_spawns_and_bumps_but_pushes_nothing() {
        // No previous artifacts: the child spawns, version becomes 1, and there
        // are no clients to push to yet.
        let decision = classify(&[], &round("s0", "c0", Some(".a{}")));
        assert_eq!(
            decision,
            RoundDecision {
                restart_server: true,
                push: None,
                bump_version: true,
            }
        );
    }

    #[test]
    fn server_only_change_restarts_and_pushes_nothing() {
        // The server bundle changed, the client didn't: restart the Node child,
        // but push nothing — the client is unaffected and K6 reconnect carries
        // it across the restart.
        let previous = round("s0", "c0", Some(".a{}"));
        let next = round("s1", "c0", Some(".a{}"));
        assert_eq!(
            classify(&previous, &next),
            RoundDecision {
                restart_server: true,
                push: None,
                bump_version: false,
            }
        );
    }

    #[test]
    fn client_only_change_pushes_swap_without_restart() {
        let previous = round("s0", "c0", Some(".a{}"));
        let next = round("s0", "c1", Some(".a{}"));
        assert_eq!(
            classify(&previous, &next),
            RoundDecision {
                restart_server: false,
                push: Some(Push::Swap),
                bump_version: true,
            }
        );
    }

    #[test]
    fn css_only_change_pushes_css_without_restart() {
        let previous = round("s0", "c0", Some(".a{}"));
        let next = round("s0", "c0", Some(".b{}"));
        assert_eq!(
            classify(&previous, &next),
            RoundDecision {
                restart_server: false,
                push: Some(Push::Css),
                bump_version: false,
            }
        );
    }

    #[test]
    fn server_and_client_change_restarts_and_swaps() {
        let previous = round("s0", "c0", Some(".a{}"));
        let next = round("s1", "c1", Some(".a{}"));
        assert_eq!(
            classify(&previous, &next),
            RoundDecision {
                restart_server: true,
                push: Some(Push::Swap),
                bump_version: true,
            }
        );
    }

    #[test]
    fn no_change_does_nothing() {
        let previous = round("s0", "c0", Some(".a{}"));
        let next = round("s0", "c0", Some(".a{}"));
        assert_eq!(
            classify(&previous, &next),
            RoundDecision {
                restart_server: false,
                push: None,
                bump_version: false,
            }
        );
    }

    #[test]
    fn a_bundle_change_outranks_a_simultaneous_css_change() {
        // When the browser bundle AND its css both change, the event is `swap`
        // (a reload subsumes the stylesheet refresh), not `css`.
        let previous = round("s0", "c0", Some(".a{}"));
        let next = round("s0", "c1", Some(".b{}"));
        assert_eq!(classify(&previous, &next).push, Some(Push::Swap));
    }

    #[test]
    fn sse_framing_is_data_json_blank_line() {
        assert_eq!(
            sse_frame(&event_json("swap", 3, None)),
            "data: {\"kind\":\"swap\",\"version\":3}\n\n"
        );
        assert_eq!(
            sse_frame(&event_json("error", 4, Some("oops \"x\"\nline"))),
            "data: {\"kind\":\"error\",\"version\":4,\"message\":\"oops \\\"x\\\"\\nline\"}\n\n"
        );
    }

    #[test]
    fn asset_names_reject_traversal() {
        assert!(is_safe_asset_name("client.js", "js"));
        assert!(is_safe_asset_name("server.css", "css"));
        assert!(!is_safe_asset_name("../secret.js", "js"));
        assert!(!is_safe_asset_name("sub/client.js", "js"));
        assert!(!is_safe_asset_name("client.css", "js")); // wrong extension
        assert!(!is_safe_asset_name(".js", "js")); // no stem
        assert!(!is_safe_asset_name("..", "js"));
    }
}
