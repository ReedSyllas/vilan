//! End-to-end CLI test for the asset channel (proposal/const-eval.md §3):
//! `vilan build` writes `<output>.<kind>` beside the compiled JS, with the
//! collected lines deduplicated and lexically ordered.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn temp_project(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "vilan_assets_cli_{tag}_{}_{unique}",
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

fn vilan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(args)
        .output()
        .expect("run vilan")
}

#[test]
fn build_writes_assets_beside_the_output() {
    let dir = temp_project("emit");
    write(
        &dir,
        "app.vl",
        r#"import std::print;
import std::asset::emit;

fun base(): i32 {
	emit("css", ".pA3{padding:1rem}");
	emit("css", "@media (min-width: 768px){.mX{padding:2rem}}");
	1
}

fun accent(): i32 {
	emit("css", ".pA3{padding:1rem}");
	emit("css", ".bC7{background:blue}");
	2
}

let _a = const base();
let _b = const accent();

fun main() {
	print("styled");
}
main();
"#,
    );
    let entry = dir.join("app.vl");
    let output = vilan(&["build", entry.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The JS runs (the consts folded; no runtime emit calls survive).
    let js = std::fs::read_to_string(dir.join("app.js")).unwrap();
    assert!(!js.contains("__emit_asset"), "no runtime emit calls:\n{js}");
    // The stylesheet sits beside it: deduplicated, lexically ordered ('.'
    // before '@', so media blocks take the later cascade position).
    let css = std::fs::read_to_string(dir.join("app.css")).unwrap();
    assert_eq!(
        css,
        ".bC7{background:blue}\n.pA3{padding:1rem}\n@media (min-width: 768px){.mX{padding:2rem}}\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
