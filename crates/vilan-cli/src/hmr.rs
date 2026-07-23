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

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
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

    /// Pushes one round event to every connected client. `message` is carried
    /// only by `error` events; `swap`/`reload` carry neither a message nor an
    /// asset. A `css` event names its sidecar — use [`DevChannel::push_css`].
    pub fn push(&self, kind: &str, message: Option<&str>) {
        self.broadcast(&event_json(kind, self.version(), message, None));
    }

    /// Pushes a `css` hot-swap event naming the changed sidecar file
    /// (`client.css`), so the shim bumps only the matching stylesheet `<link>`s
    /// (hmr.md §2). One event per changed sidecar keeps a multi-browser-leg
    /// workspace refreshing exactly the stylesheet that changed.
    pub fn push_css(&self, asset: &str) {
        self.broadcast(&event_json("css", self.version(), None, Some(asset)));
    }

    /// Frames one payload as SSE and writes it to every connected client,
    /// pruning any whose socket has closed (detected on write failure).
    fn broadcast(&self, payload: &str) {
        let frame = sse_frame(payload);
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
/// an `error` (the rendered diagnostics), or `"asset":".."` for a `css` event
/// (the changed sidecar's filename). Both extra fields are escaped.
pub fn event_json(kind: &str, version: u64, message: Option<&str>, asset: Option<&str>) -> String {
    let mut body = format!("{{\"kind\":\"{kind}\",\"version\":{version}");
    if let Some(message) = message {
        body.push_str(&format!(",\"message\":\"{}\"", escape_json(message)));
    }
    if let Some(asset) = asset {
        body.push_str(&format!(",\"asset\":\"{}\"", escape_json(asset)));
    }
    body.push('}');
    body
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
    /// The sources this artifact was compiled from — each loaded file's path
    /// mapped to the `content_hash` of the text the compiler actually consumed
    /// (`Program.source_hashes`). Covers the entry plus every std/package
    /// module the compile loaded; `macro_std` files pulled in only by macro
    /// *bodies* during world compilation are not listed (toolchain-development
    /// territory — user macro code lives in ordinary package modules, which
    /// are). Not part of the classifier — it compares `bundle`/`css` only.
    pub sources: BTreeMap<PathBuf, u64>,
}

/// Whether a leg's previous artifact is still current — the per-leg skip
/// decision (backlog E12, half b). True exactly when the leg's recorded source
/// map is non-empty and **every** source re-hashes, now, to the hash it was
/// compiled with. Reuse is decided by CONTENT, never by mtime: the watcher's
/// mtime scan only *triggers* rounds, and an mtime-preserving write, a
/// coarse-mtime filesystem, or a same-tick re-edit all still fail the re-hash
/// and recompile (the 2026-07-21 review's staleness finding). A deleted or
/// unreadable source yields `None` from `current_hash` and disqualifies the
/// skip; so does an empty map (no compile records zero sources — it means the
/// artifact predates source tracking).
pub fn leg_is_current(
    previous: &BTreeMap<PathBuf, u64>,
    current_hash: impl Fn(&Path) -> Option<u64>,
) -> bool {
    !previous.is_empty()
        && previous
            .iter()
            .all(|(path, hash)| current_hash(path) == Some(*hash))
}

/// The round-level guards that force a FULL recompile regardless of per-leg
/// verification (backlog E12): the first round (nothing recorded yet), a prior
/// failed round (its artifacts are not trustworthy), and a manifest change
/// (a `vilan.toml` can alter output without touching any `.vl` source).
/// Kept as a pure function so the safety cases stay pinned as logic.
pub fn round_forces_full(first_round: bool, prior_failed: bool, manifest_changed: bool) -> bool {
    first_round || prior_failed || manifest_changed
}

/// What the browser is told changed this round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Push {
    /// A browser bundle changed — reload (S1) / state-preserving swap (S2).
    Swap,
    /// Only CSS sidecar(s) changed — hot-swap those stylesheets, no reload. The
    /// payload names the changed sidecar files (`client.css`), so the shim bumps
    /// only the matching `<link>`s (hmr.md §2); a multi-browser-leg workspace
    /// thus refreshes exactly the stylesheet that changed.
    Css(Vec<String>),
}

/// The round's decision: whether to restart the Node child, what (if anything)
/// to push to browsers, and whether a new browser bundle means a version bump.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// - **the run server's bundle changed** → restart the child (and still classify
///   the client legs); a server-only change pushes nothing — K6 reconnect carries
///   the client across the restart;
/// - **browser bundle changed** → `swap`, and a version bump;
/// - **no bundle changed but a browser CSS sidecar changed** → `css`, naming the
///   changed sidecars;
/// - **nothing changed** → nothing, no restart.
///
/// `server_leg` is the ONE Node leg this `run --watch` executes (A15 entry
/// selection): a non-selected Node leg's bundle change drives no restart, because
/// that leg isn't run and isn't served to the browser — only its dist bytes are
/// refreshed. `None` when the workspace runs no Node leg (a browser-only dev
/// session, or an ambiguous multi-node choice deferred to the restart step).
///
/// A compile failure is handled by the caller (push `error`, retain the old
/// artifacts) and never reaches this classifier.
pub fn classify(
    previous: &[LegArtifact],
    next: &[LegArtifact],
    server_leg: Option<&str>,
) -> RoundDecision {
    if previous.is_empty() {
        return RoundDecision {
            restart_server: true,
            push: None,
            bump_version: true,
        };
    }
    let prior = |name: &str| previous.iter().find(|leg| leg.name == name);
    let is_server = |name: &str| server_leg == Some(name);
    let mut server_changed = false;
    let mut browser_changed = false;
    // The changed browser CSS sidecars, named for the `css` event so the shim
    // bumps exactly the matching `<link>`s (a node leg's CSS, if any, is not
    // linked by the page, so only browser sidecars participate).
    let mut changed_css: Vec<String> = Vec::new();
    let mut note_bundle_change = |leg: &LegArtifact| {
        if leg.is_browser {
            browser_changed = true;
        } else if is_server(&leg.name) {
            server_changed = true;
        }
    };
    for leg in next {
        match prior(&leg.name) {
            Some(old) => {
                if old.bundle != leg.bundle {
                    note_bundle_change(leg);
                }
                if old.css != leg.css && leg.is_browser {
                    changed_css.push(format!("{}.css", leg.name));
                }
            }
            // A newly-appearing leg is a change of its class.
            None => {
                note_bundle_change(leg);
                if leg.css.is_some() && leg.is_browser {
                    changed_css.push(format!("{}.css", leg.name));
                }
            }
        }
    }
    // A leg that vanished between rounds is likewise a change of its class.
    for old in previous {
        if !next.iter().any(|leg| leg.name == old.name) {
            if old.is_browser {
                browser_changed = true;
            } else if is_server(&old.name) {
                server_changed = true;
            }
        }
    }
    RoundDecision {
        restart_server: server_changed,
        push: if browser_changed {
            Some(Push::Swap)
        } else if !changed_css.is_empty() {
            Some(Push::Css(changed_css))
        } else {
            None
        },
        bump_version: browser_changed,
    }
}

// --- The error overlay's plain-text rendering (hmr.md §§2, 6) ----------------

/// The overlay caps at this many diagnostics before collapsing the tail to
/// "… and N more" — enough to see the shape of a broken build without an
/// unbounded event payload.
pub const OVERLAY_DIAGNOSTIC_CAP: usize = 20;

/// One diagnostic bound for the browser error overlay: a byte span into the
/// entry source, the already-built message (the SAME text the terminal renders —
/// an analyzer `error.msg`, or the parser's `render` — this module never rebuilds
/// it, only frames it), and an optional note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayDiagnostic {
    pub span: std::ops::Range<usize>,
    pub message: String,
    pub note: Option<String>,
}

/// Renders a failed leg's diagnostics as ANSI-free plain text for the `error`
/// event (the in-page overlay, hmr.md §§2/§6). A per-file header line, then each
/// diagnostic as `<file>:<line>:<col>  <message>` with any note indented beneath,
/// diagnostics separated by a blank line. Line/column are 1-based, computed from
/// the byte span against `src` — the same source the terminal renders — mirroring
/// the LSP's byte→line/col mapping without depending on that crate. Capped at
/// `cap`; the remainder collapses to a trailing "… and N more". The terminal path
/// is untouched: this is a second, additive rendering of the same messages.
pub fn render_overlay(
    filename: &str,
    src: &str,
    diagnostics: &[OverlayDiagnostic],
    cap: usize,
) -> String {
    use std::fmt::Write;
    let shown = diagnostics.len().min(cap);
    let mut out = format!("{filename} — {} error", diagnostics.len());
    if diagnostics.len() != 1 {
        out.push('s');
    }
    for diagnostic in diagnostics.iter().take(cap) {
        let (line, column) = line_col(src, diagnostic.span.start);
        // `file:line:col` on its own line (the shim styles it as a distinct
        // location line), then the message — the SAME text the terminal renders.
        let _ = write!(
            out,
            "\n\n{filename}:{line}:{column}\n{}",
            diagnostic.message
        );
        if let Some(note) = &diagnostic.note {
            let _ = write!(out, "\n    note: {note}");
        }
    }
    let remaining = diagnostics.len() - shown;
    if remaining > 0 {
        let _ = write!(out, "\n\n… and {remaining} more");
    }
    out
}

/// The 1-based (line, column) of a byte offset in `src`, columns counted in
/// Unicode scalar values (as the terminal renderer does). Clamped to the source
/// length so an end-of-input span resolves to the last position.
fn line_col(src: &str, byte: usize) -> (usize, usize) {
    let byte = byte.min(src.len());
    let mut line = 1usize;
    let mut column = 1usize;
    for (index, character) in src.char_indices() {
        if index >= byte {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
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
            sources: BTreeMap::new(),
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
        let decision = classify(&[], &round("s0", "c0", Some(".a{}")), Some("server"));
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
            classify(&previous, &next, Some("server")),
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
            classify(&previous, &next, Some("server")),
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
            classify(&previous, &next, Some("server")),
            RoundDecision {
                restart_server: false,
                push: Some(Push::Css(vec!["client.css".to_string()])),
                bump_version: false,
            }
        );
    }

    #[test]
    fn server_and_client_change_restarts_and_swaps() {
        let previous = round("s0", "c0", Some(".a{}"));
        let next = round("s1", "c1", Some(".a{}"));
        assert_eq!(
            classify(&previous, &next, Some("server")),
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
            classify(&previous, &next, Some("server")),
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
        assert_eq!(
            classify(&previous, &next, Some("server")).push,
            Some(Push::Swap)
        );
    }

    #[test]
    fn sse_framing_is_data_json_blank_line() {
        assert_eq!(
            sse_frame(&event_json("swap", 3, None, None)),
            "data: {\"kind\":\"swap\",\"version\":3}\n\n"
        );
        assert_eq!(
            sse_frame(&event_json("error", 4, Some("oops \"x\"\nline"), None)),
            "data: {\"kind\":\"error\",\"version\":4,\"message\":\"oops \\\"x\\\"\\nline\"}\n\n"
        );
    }

    #[test]
    fn a_css_event_names_its_sidecar() {
        // The recorded S1 residue closed: the `css` event carries the changed
        // sidecar's filename so the shim bumps only the matching `<link>`.
        assert_eq!(
            event_json("css", 5, None, Some("client.css")),
            "{\"kind\":\"css\",\"version\":5,\"asset\":\"client.css\"}"
        );
    }

    #[test]
    fn a_non_selected_node_leg_change_drives_no_restart() {
        // A15 + the byte-diff classifier: a workspace runs ONE node leg
        // (`server`); a sibling node leg (`probe`) that isn't run and isn't
        // served must not trigger a restart when only its bundle changes — the
        // page is unaffected, the run server unchanged.
        let previous = vec![
            leg("server", false, "s0", None),
            leg("probe", false, "p0", None),
            leg("client", true, "c0", None),
        ];
        let next = vec![
            leg("server", false, "s0", None),
            leg("probe", false, "p1", None), // only the non-run leg changed
            leg("client", true, "c0", None),
        ];
        assert_eq!(
            classify(&previous, &next, Some("server")),
            RoundDecision {
                restart_server: false,
                push: None,
                bump_version: false,
            }
        );
        // …while a change to the SELECTED server leg does restart.
        let next_server = vec![
            leg("server", false, "s1", None),
            leg("probe", false, "p0", None),
            leg("client", true, "c0", None),
        ];
        assert_eq!(
            classify(&previous, &next_server, Some("server")).restart_server,
            true
        );
    }

    #[test]
    fn a_per_sidecar_css_change_names_only_the_changed_browser_leg() {
        // Two browser legs each with a sidecar; only `admin`'s CSS changed → the
        // event names `admin.css` alone (per-sidecar behavior, hmr.md §2).
        let previous = vec![
            leg("client", true, "c0", Some(".a{}")),
            leg("admin", true, "a0", Some(".x{}")),
        ];
        let next = vec![
            leg("client", true, "c0", Some(".a{}")),
            leg("admin", true, "a0", Some(".y{}")),
        ];
        assert_eq!(
            classify(&previous, &next, None).push,
            Some(Push::Css(vec!["admin.css".to_string()]))
        );
    }

    #[test]
    fn the_overlay_renders_file_line_col_message_and_note() {
        // The overlay shows the REAL diagnostics: file:line:col, the message
        // (the same text the terminal renders), and a note where present.
        let src = "fun main() {\n\tlet x = broken\n}\n";
        // `broken` starts at byte 22 (line 2, column 10 — after "\tlet x = ",
        // the leading tab counting as one column).
        let diagnostics = vec![
            OverlayDiagnostic {
                span: 22..28,
                message: "cannot find `broken` in this scope".to_string(),
                note: Some("declared nowhere".to_string()),
            },
            OverlayDiagnostic {
                span: 0..3,
                message: "second problem".to_string(),
                note: None,
            },
        ];
        let rendered = render_overlay("src/app.vl", src, &diagnostics, OVERLAY_DIAGNOSTIC_CAP);
        assert_eq!(
            rendered,
            "src/app.vl — 2 errors\n\n\
             src/app.vl:2:10\n\
             cannot find `broken` in this scope\n\
             \x20   note: declared nowhere\n\n\
             src/app.vl:1:1\n\
             second problem"
        );
        // ANSI-free: the overlay is HTML text, never a terminal control stream.
        assert!(!rendered.contains('\x1b'));
    }

    #[test]
    fn the_overlay_caps_and_counts_the_remainder() {
        let src = "x\n";
        let diagnostics: Vec<OverlayDiagnostic> = (0..25)
            .map(|index| OverlayDiagnostic {
                span: 0..1,
                message: format!("problem {index}"),
                note: None,
            })
            .collect();
        let rendered = render_overlay("a.vl", src, &diagnostics, 20);
        assert!(rendered.starts_with("a.vl — 25 errors\n"));
        assert!(rendered.contains("a.vl:1:1\nproblem 0"));
        assert!(rendered.contains("a.vl:1:1\nproblem 19"));
        assert!(!rendered.contains("problem 20"));
        assert!(rendered.ends_with("… and 5 more"));
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

    // --- The per-leg skip decision (backlog E12, half b) ---------------------

    fn recorded(entries: &[(&str, u64)]) -> BTreeMap<PathBuf, u64> {
        entries
            .iter()
            .map(|(name, hash)| (PathBuf::from(name), *hash))
            .collect()
    }

    /// A verifier over a fixed "current filesystem": present files hash to the
    /// listed value, everything else reads as unreadable.
    fn on_disk(entries: &[(&str, u64)]) -> impl Fn(&Path) -> Option<u64> {
        let current: BTreeMap<PathBuf, u64> = recorded(entries);
        move |path: &Path| current.get(path).copied()
    }

    #[test]
    fn an_unchanged_leg_verifies_and_is_current() {
        let previous = recorded(&[("server.vl", 1), ("std/list.vl", 2), ("common.vl", 3)]);
        let disk = on_disk(&[("server.vl", 1), ("std/list.vl", 2), ("common.vl", 3)]);
        assert!(leg_is_current(&previous, disk));
    }

    #[test]
    fn a_content_drift_disqualifies_the_leg_regardless_of_any_other_signal() {
        // The review's staleness finding, pinned: reuse is decided by CONTENT.
        // No mtime, changed-set, or watcher signal participates — if a source's
        // bytes differ from what was compiled (an mtime-preserving write, a
        // coarse-mtime filesystem, a same-tick re-edit), the re-hash fails and
        // the leg recompiles.
        let previous = recorded(&[("client.vl", 1), ("common.vl", 3)]);
        let disk = on_disk(&[("client.vl", 9), ("common.vl", 3)]);
        assert!(!leg_is_current(&previous, disk));
    }

    #[test]
    fn an_unreadable_or_deleted_source_disqualifies_the_leg() {
        let previous = recorded(&[("client.vl", 1), ("gone.vl", 2)]);
        let disk = on_disk(&[("client.vl", 1)]);
        assert!(!leg_is_current(&previous, disk));
    }

    #[test]
    fn an_empty_source_map_never_qualifies() {
        // No compile records zero sources — an empty map means the artifact
        // predates source tracking, and trusting it would skip on nothing.
        let previous = recorded(&[]);
        let disk = on_disk(&[]);
        assert!(!leg_is_current(&previous, disk));
    }

    #[test]
    fn a_leg_only_verifies_against_its_own_sources() {
        // A drift in a file the leg never loaded is invisible to it — the
        // client's edit leaves the server current (the client-only-edit skip).
        let server = recorded(&[("server.vl", 1), ("common.vl", 3)]);
        let disk = on_disk(&[("server.vl", 1), ("common.vl", 3), ("client.vl", 42)]);
        assert!(leg_is_current(&server, disk));
    }

    #[test]
    fn the_round_guards_force_a_full_recompile() {
        // The three safety cases that bypass per-leg verification entirely.
        assert!(round_forces_full(true, false, false), "first round");
        assert!(round_forces_full(false, true, false), "prior failure");
        assert!(round_forces_full(false, false, true), "manifest change");
        assert!(!round_forces_full(false, false, false));
    }
}
