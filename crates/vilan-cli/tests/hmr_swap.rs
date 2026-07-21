//! End-to-end test for the A13 swap protocol (hmr.md slice S2b): an HMR-active
//! `run --watch` produces an instrumented browser bundle (the shim plus the S2a
//! `__hmr_adopt*`/`__hmr_expose` emission), and `window.__VILAN_HMR__.swap(text)`
//! carries module state across an in-place re-evaluation. The corpus can't reach
//! this — it needs the shim runtime, a DOM, and a second (edited) bundle — so a
//! browser app is built through the real CLI and driven under node against a DOM
//! stub: state is mutated, a rebuilt bundle is swapped in, and the carry / reset
//! matrix is asserted (value + signal payload carried; excluded + fingerprint-
//! changed + function-local reset; `on_teardown` ran; `stash`/`take` round-trip;
//! the old bundle's subscriptions disposed).
//!
//! House process hygiene: the watcher never exits on its own, so it is killed at
//! the end; the legs are quick-exit (the node server prints and returns).

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
        "vilan_hmr_swap_{tag}_{}_{unique}",
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

/// The dev-channel port from the activation line `hmr: dev channel on 127.0.0.1:<port>`.
fn parse_port(line: &str) -> Option<u16> {
    line.strip_prefix("hmr: dev channel on 127.0.0.1:")?
        .trim()
        .parse()
        .ok()
}

/// A plain HTTP GET against the dev channel, returning the response body bytes.
fn http_get(port: u16, path: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect for GET");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("send GET");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let separator = b"\r\n\r\n";
    match response
        .windows(separator.len())
        .position(|window| window == separator)
    {
        Some(index) => response[index + separator.len()..].to_vec(),
        None => response,
    }
}

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

/// The client app (bundle A). Module bindings span every transfer form — a `mut`
/// value (`count`), a `Signal` payload (`tally`), a plain-data value whose type
/// changes in bundle B (`cfg`, a fingerprint miss), and an excluded closure-holder
/// (`strap`). Each initializer and `main` mark a global counter so the harness can
/// tell carry (initializer NOT re-run) from fresh (re-run). `stash`/`take` and
/// `on_teardown` are exercised; a `tally.effect` under the mount root proves
/// subscription disposal.
const CLIENT_A: &str = r#"import std::ui::{ view, View, mount_root };
import std::reactive::Signal;
import std::dev;
import std::option::Option::{ self, Some, None };

[extern("globalThis.__mark")]
external fun mark(tag: str): void;

[extern("globalThis.__record")]
external fun record(tag: str, value: i32): void;

[extern("globalThis.__captureSignal")]
external fun capture_signal(tag: str, signal: Signal<i32>): void;

struct Cfg { a: i32 }

struct Strap { fire: || void }

fun seed_count(): i32 {
	mark("count-init");
	0
}

fun seed_cfg(): Cfg {
	mark("cfg-init");
	Cfg { a = 1 }
}

fun fresh_strap(): Strap {
	mark("strap-init");
	Strap { fire = || {} }
}

mut count = seed_count();
let tally: Signal<i32> = Signal::new(0);
let cfg: Cfg = seed_cfg();
let strap: Strap = fresh_strap();

fun bump() {
	count = count + 1;
	tally.set(tally.get() + 1);
	dev::stash("saved", count);
}

fun main() {
	mark("mount");
	let restored: Option<i32> = dev::take("saved");
	record("restored", restored.unwrap_or(-1));
	record("cfg-a", cfg.a);
	(strap.fire)();
	dev::on_teardown(|| mark("teardown"));
	capture_signal("tally", tally);
	let _root = mount_root("app", || {
		tally.effect(|v| mark("tally-effect"));
		view("div").child(view("button").on("click", || bump()))
	});
}
"#;

/// Bundle B: `Cfg` gains a field, so `cfg`'s structural fingerprint changes and
/// it fresh-initializes (§4). Everything else is identical, so `count`/`tally`
/// carry and `strap` re-inits as the excluded form.
const CLIENT_B: &str = r#"import std::ui::{ view, View, mount_root };
import std::reactive::Signal;
import std::dev;
import std::option::Option::{ self, Some, None };

[extern("globalThis.__mark")]
external fun mark(tag: str): void;

[extern("globalThis.__record")]
external fun record(tag: str, value: i32): void;

[extern("globalThis.__captureSignal")]
external fun capture_signal(tag: str, signal: Signal<i32>): void;

struct Cfg { a: i32, b: i32 }

struct Strap { fire: || void }

fun seed_count(): i32 {
	mark("count-init");
	0
}

fun seed_cfg(): Cfg {
	mark("cfg-init");
	Cfg { a = 1, b = 2 }
}

fun fresh_strap(): Strap {
	mark("strap-init");
	Strap { fire = || {} }
}

mut count = seed_count();
let tally: Signal<i32> = Signal::new(0);
let cfg: Cfg = seed_cfg();
let strap: Strap = fresh_strap();

fun bump() {
	count = count + 1;
	tally.set(tally.get() + 1);
	dev::stash("saved", count);
}

fun main() {
	mark("mount");
	let restored: Option<i32> = dev::take("saved");
	record("restored", restored.unwrap_or(-1));
	record("cfg-a", cfg.a);
	(strap.fire)();
	dev::on_teardown(|| mark("teardown"));
	capture_signal("tally", tally);
	let _root = mount_root("app", || {
		tally.effect(|v| mark("tally-effect"));
		view("div").child(view("button").on("click", || bump()))
	});
}
"#;

const SERVER: &str = "import std::print;\n\nfun main() {\n\tprint(\"server up\");\n}\n";

/// The DOM/host stub plus the swap-matrix assertions, run under node against the
/// two instrumented bundles. `window === globalThis` (as in a browser) so the
/// `__hmr_active` helper sees the shim's singleton; `Blob`/`URL.createObjectURL`
/// are stubbed to a data: URL (node has no `blob:` loader — the sanctioned S2b
/// fallback). One `ok`/`FAIL` line per named assertion; exits 1 on any failure.
const HARNESS: &str = r#"import fs from "node:fs";

class StubElement {
    constructor(tag) {
        this.tagName = tag;
        this.children = [];
        this.parent = null;
        this.listeners = {};
        this._text = "";
        this.attributes = {};
        this.style = { setProperty: () => {} };
        this.hidden = false;
    }
    set textContent(text) { this._text = text; this.children = []; }
    get textContent() { return this._text; }
    setAttribute(name, value) { this.attributes[name] = value; }
    appendChild(child) {
        if (child.parent) child.parent.children = child.parent.children.filter((c) => c !== child);
        child.parent = this;
        this.children.push(child);
    }
    remove() {
        if (this.parent) {
            this.parent.children = this.parent.children.filter((c) => c !== this);
            this.parent = null;
        }
    }
    replaceChildren() { for (const c of this.children) c.parent = null; this.children = []; }
    addEventListener(event, handler) {
        (this.listeners[event] = this.listeners[event] || []).push(handler);
    }
    click() { for (const h of (this.listeners.click || [])) h({ preventDefault() {} }); }
    find(predicate) {
        if (predicate(this)) return this;
        for (const c of this.children) { const hit = c.find(predicate); if (hit) return hit; }
        return null;
    }
}

const appRoot = new StubElement("div");
globalThis.window = globalThis; // window === globalThis, as in a browser
globalThis.document = {
    createElement: (tag) => new StubElement(tag),
    getElementById: (id) => (id === "app" ? appRoot : null),
    querySelector: () => null,
    querySelectorAll: () => [],
};
globalThis.location = { reload: () => { globalThis.__reloaded = true; } };
// No EventSource: the shim's connect() is skipped under the stub.
globalThis.Blob = class {
    constructor(parts) { this.__text = parts.join(""); }
};
// Extend (do NOT replace) the real URL so `new URL(...)` still works; node has
// no blob: loader, so the shim's object URL becomes an importable data: URL.
URL.createObjectURL = (blob) =>
    "data:text/javascript;base64," + Buffer.from(blob.__text).toString("base64");
URL.revokeObjectURL = () => {};

const marks = {};
const records = {};
const signals = {};
globalThis.__mark = (tag) => { marks[tag] = (marks[tag] || 0) + 1; };
globalThis.__record = (tag, value) => { records[tag] = value; };
globalThis.__captureSignal = (tag, signal) => { signals[tag] = signal; };

let failures = 0;
function check(condition, message) {
    if (condition) { console.log("ok   - " + message); }
    else { failures += 1; console.error("FAIL - " + message); }
}

await import("./bundleA.mjs");

check(marks["count-init"] === 1, "A: count initializer ran once");
check(marks["cfg-init"] === 1, "A: cfg initializer ran once");
check(marks["strap-init"] === 1, "A: strap initializer ran once");
check(marks["mount"] === 1, "A: main ran once");
check(records["restored"] === -1, "A: first-boot take() is None");
const tallyA = signals["tally"];
check(!!tallyA && tallyA[1].v.length === 1, "A: tally has one live subscriber");

const button = appRoot.find((element) => element.tagName === "button");
check(!!button, "A: a button mounted");
button.click();
button.click();
button.click();

const hmr = globalThis.window.__VILAN_HMR__;
check(hmr.exposed["pkg::count"].getter() === 3, "A: count mutated to 3");
check(hmr.exposed["pkg::tally"].getter() === 3, "A: tally payload mutated to 3");

const bundleB = fs.readFileSync(new URL("./bundleB.js", import.meta.url), "utf8");

// The heal path (the infinite-refresh regression): a `connected` event at our
// own version is a no-op; one ahead of us fetches the current bundle from the
// dev channel and SWAPS. location.reload() must never be involved — a page
// whose server serves a boot-time stale bundle would reload into the same
// stale bytes and loop forever.
globalThis.fetch = () => Promise.resolve({ text: () => Promise.resolve(bundleB) });
await hmr.handleEvent({ kind: "connected", version: hmr.version });
check(marks["teardown"] === undefined, "heal: an up-to-date connected does nothing");
const target = hmr.version + 1;
await hmr.handleEvent({ kind: "connected", version: target });
check(!globalThis.__reloaded, "heal: a stale connected swapped instead of reloading");
check(hmr.version === target, "heal: the singleton's version advanced to the channel's");

check(marks["teardown"] === 1, "swap: on_teardown ran");
check(marks["count-init"] === 1, "swap: count carried (adopt hit — initializer not re-run)");
check(marks["cfg-init"] === 2, "swap: cfg fingerprint changed → fresh init");
check(marks["strap-init"] === 2, "swap: excluded binding fresh init");
check(marks["mount"] === 2, "swap: main re-ran (function-local state reset)");
check(hmr.exposed["pkg::count"].getter() === 3, "swap: count value carried");
check(hmr.exposed["pkg::tally"].getter() === 3, "swap: signal payload carried");
check(records["restored"] === 3, "swap: stash/take round-tripped");
check(tallyA[1].v.length === 0, "swap: bundle A subscription disposed (old subscribers dead)");
check(!globalThis.__reloaded, "swap: completed without a fallback reload");

process.exit(failures === 0 ? 0 : 1);
"#;

#[test]
fn the_swap_protocol_carries_state_across_a_rebuilt_bundle() {
    let dir = temp_project("carry");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"swapapp\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n",
    );
    write(&dir, "src/client.vl", CLIENT_A);
    write(&dir, "src/server.vl", SERVER);
    write(&dir, "harness.mjs", HARNESS);

    let mut watcher = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", "--watch", "--hmr-port", "0", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn run --watch");

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

        // Round 1: wait for dist, plus a margin so the watcher's baseline snapshot
        // is taken before the edit (so the edit registers as a change).
        assert!(
            wait_for_file(&dir.join("dist/client.js"), deadline),
            "round 1 should have written dist/client.js"
        );
        std::thread::sleep(Duration::from_millis(800));

        // Bundle A: the instrumented client (shim + adopt/expose).
        let bundle_a = http_get(port, "/bundle/client.js");
        assert!(
            String::from_utf8_lossy(&bundle_a).contains("__hmr_adopt"),
            "bundle A should carry the S2a adopt instrumentation"
        );
        std::fs::write(dir.join("bundleA.mjs"), &bundle_a).unwrap();

        // Edit the client → bundle B (cfg's type changes), then poll the served
        // bundle until it differs from A.
        write(&dir, "src/client.vl", CLIENT_B);
        let start = Instant::now();
        let bundle_b = loop {
            let current = http_get(port, "/bundle/client.js");
            if current != bundle_a && current.contains(&b'{') {
                break current;
            }
            assert!(
                start.elapsed() < deadline,
                "the edited client should rebuild into a new bundle"
            );
            std::thread::sleep(Duration::from_millis(200));
        };
        std::fs::write(dir.join("bundleB.js"), &bundle_b).unwrap();

        // Drive the swap under node against the DOM stub.
        let run = Command::new("node")
            .arg("harness.mjs")
            .current_dir(&dir)
            .output()
            .expect("run node harness");
        assert!(
            run.status.success(),
            "swap harness failed:\n{}\n{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
    }));

    let _ = watcher.kill();
    let _ = watcher.wait();
    if outcome.is_ok() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    outcome.unwrap();
}
