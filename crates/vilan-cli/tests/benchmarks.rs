//! Pins the benchmark HARNESS (vilan/benchmarks): it must build, run to
//! completion, and report the deterministic facts — the coalescing and
//! fan-out frame counts are exact invariants of the reactive protocol.
//! Timings are machine-dependent and deliberately not asserted.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn benchmarks_run_and_report_the_deterministic_counts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/benchmarks");
    let mut child = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vilan run");
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match child.try_wait().expect("poll vilan run") {
            Some(_status) => break,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("benchmarks did not finish within 90s");
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
        "benchmarks wrote to stderr:\n{stderr}\nstdout:\n{stdout}"
    );
    for expected in [
        "== payload sizes ==",
        "== coalescing (update frames counted at the wire) ==",
        "subscribe -> 1 update frame(s)",
        "100 lone sets -> 100 update frames",
        "100 sets in one batch -> 1 update frame(s)",
        "3 writes in one rpc handler -> 1 update frame(s)",
        "deliveries observed by the subscriber = 103",
        "== rpc round-trip throughput (sequential) ==",
        "== realtime fan-out (sse + post, 3 sessions, 50 mutations) ==",
        "per-session update frames: 51 51 51",
        "done",
    ] {
        assert!(
            stdout.contains(expected),
            "missing `{expected}` in benchmark output:\n{stdout}"
        );
    }
}
