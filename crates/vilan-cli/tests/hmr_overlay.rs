//! The dev-runtime shim's error overlay, driven in isolation under a minimal DOM
//! stub (hmr.md §2). No compiler round is needed: the shim is `include_str!`'d
//! straight from the source, its placeholders substituted, and its `handleEvent`
//! is driven directly — pinning the show-on-error / clear-on-good lifecycle and
//! that the overlay renders the REAL diagnostic text plus the header chrome.
//!
//! Requires `node` on PATH (as the other end-to-end suites do); the stub replaces
//! the browser's `window`/`document` with just enough for the overlay code.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// The dev-runtime shim, read from source so this test exercises exactly what
/// ships. Its three placeholders are substituted the way `hmr::instrument` does.
const SHIM: &str = include_str!("../src/hmr_shim.js");

fn temp_file(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "vilan_hmr_overlay_{tag}_{}_{unique}.mjs",
        std::process::id()
    ))
}

/// A tiny DOM stub — only what the overlay code touches — plus the shim with its
/// placeholders filled, plus the driver assertions. `check(cond, msg)` prints one
/// `ok`/`FAIL` line each; the process exits non-zero on any failure.
fn harness() -> String {
    let shim = SHIM
        .replace("__VILAN_HMR_PORT__", "0")
        .replace("__VILAN_HMR_VERSION__", "1")
        .replace("__VILAN_HMR_BUNDLE__", "client");
    format!(
        r#"class El {{
    constructor(tag) {{ this.tagName = tag; this.id = ""; this.children = []; this.parent = null; this.style = {{}}; this._text = ""; }}
    set textContent(text) {{ this._text = text; this.children = []; }}
    get textContent() {{ return this._text; }}
    appendChild(child) {{ child.parent = this; this.children.push(child); return child; }}
    remove() {{ if (this.parent) {{ this.parent.children = this.parent.children.filter((c) => c !== this); this.parent = null; }} }}
    allText() {{ let text = this._text; for (const child of this.children) text += "\n" + child.allText(); return text; }}
    findById(id) {{ if (this.id === id) return this; for (const child of this.children) {{ const hit = child.findById(id); if (hit) return hit; }} return null; }}
}}
const body = new El("body");
globalThis.window = globalThis;
globalThis.document = {{
    createElement: (tag) => new El(tag),
    getElementById: (id) => body.findById(id),
    body,
    documentElement: body,
    querySelectorAll: () => [],
}};

{shim}

let failures = 0;
function check(condition, message) {{
    if (condition) console.log("ok   - " + message);
    else {{ failures += 1; console.error("FAIL - " + message); }}
}}

const hmr = window.__VILAN_HMR__;
check(!!hmr, "the shim installed its singleton");

// show-on-error: the overlay appears and carries the REAL diagnostics + chrome.
hmr.handleEvent({{ kind: "error", version: 1, message: "src/app.vl:2:10\ncannot find `x` in this scope\n    note: declared nowhere" }});
const overlay = document.getElementById("__vilan_hmr_overlay__");
check(!!overlay, "an error event shows the overlay");
const text = overlay ? overlay.allText() : "";
check(text.includes("vilan — build failed"), "the overlay has the header bar");
check(text.includes("cannot find `x` in this scope"), "the overlay carries the real diagnostic message");
check(text.includes("src/app.vl:2:10"), "the overlay shows the file:line:col");
check(text.includes("note: declared nowhere"), "the overlay shows the note");
check(text.includes("1 error"), "the header counts the diagnostic");
check(text.toLowerCase().includes("next save"), "the overlay has the fixed-on-next-save hint");

// clear-on-good: any non-error round removes the overlay (a css round here).
hmr.handleEvent({{ kind: "css", version: 1, asset: "app.css" }});
check(document.getElementById("__vilan_hmr_overlay__") === null, "a good round clears the overlay");

// singleton lifecycle: a second error shows exactly one overlay (no duplicate).
hmr.handleEvent({{ kind: "error", version: 2, message: "src/app.vl:1:1\nagain" }});
let count = 0;
(function walk(node) {{ if (node.id === "__vilan_hmr_overlay__") count += 1; for (const child of node.children) walk(child); }})(body);
check(count === 1, "re-erroring leaves a single overlay (cleared then re-shown)");

process.exit(failures === 0 ? 0 : 1);
"#
    )
}

#[test]
fn the_error_overlay_shows_real_diagnostics_and_clears_on_a_good_round() {
    let path = temp_file("lifecycle");
    std::fs::write(&path, harness()).expect("write harness");
    let output = Command::new("node")
        .arg(&path)
        .output()
        .expect("run node overlay harness (is node on PATH?)");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "the overlay harness assertions failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
