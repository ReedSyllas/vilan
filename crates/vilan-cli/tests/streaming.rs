//! End-to-end test for `Server` streaming responses (backlog K1): a
//! `ResponseBuilder::streaming` route writes chunks over time through the
//! held-open response — with an ASYNC `on_open` (spawn semantics; the sleeps
//! interleave with serving) — and the same process reads them back through
//! `fetch`'s body stream until the server closes. The rpc realtime/SSE suites
//! cover the `serve_connected` mount built on this; this pins the PUBLIC
//! surface directly.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_stream_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

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
                panic!("vilan run did not exit within {timeout:?} (stream never closed?)");
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

#[test]
fn a_streaming_response_delivers_chunks_until_close() {
    let dir = temp_project("chunks");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &dir,
        "src/main.vl",
        r#"import std::print;
import std::process::exit;
import std::time::sleep;
import std::http::{ Server, Response };
import std::fetch::fetch;
import std::bytes::new_text_decoder;

fun main() {
	Server::builder()
		.port(45213)
		.on_request(|request| {
			if request.path().starts_with("/stream") {
				Response::builder()
					.set_header("Content-Type", "text/event-stream")
					.streaming(|stream| {
						stream.send("one\n");
						sleep(10);
						stream.send("two\n");
						sleep(10);
						stream.send("three\n");
						stream.close();
					})
					.build()
			} else {
				Response::builder().code(404).body("nope").build()
			}
		})
		.on_start(|server| {
			run_client();
		})
		.build()
		.start();
}

fun run_client() {
	let response = fetch("http://localhost:45213/stream");
	let reader = response.body_stream().reader();
	let decoder = new_text_decoder();
	mut received = "";
	for {
		let chunk = reader.read_chunk();
		if chunk.finished() {
			jump break;
		}
		received += decoder.decode(chunk.payload());
	}
	print(received);
	exit(0);
}
"#,
    );
    let stdout = vilan_run_with_timeout(&dir, Duration::from_secs(45));
    let one = stdout.find("one").expect("first chunk missing");
    let two = stdout.find("two").expect("second chunk missing");
    let three = stdout.find("three").expect("third chunk missing");
    assert!(one < two && two < three, "chunks out of order:\n{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
