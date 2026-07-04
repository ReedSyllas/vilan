//! The leftover-JSON boundary check (proposal/p6-followups.md, "Final fixes"):
//! after the codec re-plumb, direct `to_json`/`from_json` use in std and the
//! example/benchmark packages is limited to the SANCTIONED sites — std::json
//! itself, the reactive protocol's JSON-over-text (retired by the
//! reactive-on-codec follow-up), connection-id parsing (a recorded small fix),
//! error rendering, and the todo app's persistence. Each file's count is
//! pinned; a new or moved use must update this table deliberately, with the
//! doc's audit section.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Non-comment lines mentioning `to_json`/`from_json` in one file.
fn json_lines(path: &Path) -> usize {
    let source = std::fs::read_to_string(path).unwrap_or_default();
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//")
                && (trimmed.contains("to_json") || trimmed.contains("from_json"))
        })
        .count()
}

/// Every `.vl` file under `root`, recursively.
fn vl_files(root: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            vl_files(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "vl") {
            into.push(path);
        }
    }
}

#[test]
fn json_usage_stays_within_the_sanctioned_sites() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan");
    // (relative path, allowed non-comment lines). `std/src/json.vl` is the
    // codec's own module and unrestricted; everything else is enumerated.
    let allowed: BTreeMap<&str, usize> = BTreeMap::from([
        // std, post reactive-on-codec: ONLY the multiplex id parses remain —
        // `route_socket_frame`'s (rpc.vl) and the `/send` + text-RPC-turn
        // parses (rpc_server.vl). All of them retire when `str -> i32` parsing
        // lands in std::number (the recorded small fix).
        ("std/src/rpc.vl", 1),
        ("std/src/process/rpc_server.vl", 3),
        // examples/benchmarks: connection-id parses (same small fix), error
        // rendering (`error.to_json()` — retired by a Debug derive on
        // RpcError), and the todo server's JSON-at-rest persistence. The
        // mirror decodes are GONE — mirrors are typed now.
        ("examples/todo/server/src/main.vl", 2),
        ("examples/todo/client/src/main.vl", 1),
        ("examples/todo/client/src/todos.vl", 1),
        ("examples/rpc/src/main.vl", 5),
        ("benchmarks/src/throughput.vl", 1),
        ("benchmarks/src/realtime.vl", 1),
    ]);
    let mut files = Vec::new();
    for root in ["std/src", "examples", "benchmarks"] {
        vl_files(&base.join(root), &mut files);
    }
    let mut violations = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "std/src/json.vl" {
            continue;
        }
        let found = json_lines(&path);
        let permitted = allowed.get(relative.as_str()).copied().unwrap_or(0);
        if found != permitted {
            violations.push(format!(
                "{relative}: {found} json-usage line(s), {permitted} sanctioned"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "to_json/from_json usage moved outside the sanctioned table — if \
         deliberate, re-sanction it here AND in proposal/p6-followups.md \
         (\"Final fixes\"):\n{}",
        violations.join("\n")
    );
}
