//! End-to-end: `Database` is an affine `resource` that closes its `node:sqlite`
//! handle on drop (destruction.md §9). A file-backed database is written in an
//! inner scope that ends — the scope-end drop closes the handle — and again via
//! an explicit `drop(db)`, then the same file is reopened and read back. The
//! round-trip returning the written rows proves each writer's `drop` ran to
//! completion (it did not throw) and the file is usable afterward. The emitted
//! `db.close()` in the finally is pinned separately by the `db.vl` corpus golden;
//! this drives it against the real host database (node ships `node:sqlite`).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh temp directory for the test's project tree (and its `.db` files).
fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_db_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes `contents` to `dir/relative`, creating parent directories.
fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn a_dropped_database_closes_and_the_file_reopens() {
    let dir = temp_project("close");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    // `write_scoped` closes on return (scope-end drop); `write_early` closes at
    // `drop(db)` before it returns. `read_back` reopens the same file — reading
    // the written row proves the writer's teardown was clean.
    write(
        &dir,
        "src/main.vl",
        r#"import std::print;
import std::db::Database;
import std::drop::drop;
import std::option::Option::{ self, Some, None };

fun write_scoped(path: str) {
	let db = Database::open(path);
	db.exec("CREATE TABLE IF NOT EXISTS t (v TEXT)");
	db.prepare("INSERT INTO t VALUES (?)").run(["scope-end"]);
}

fun write_early(path: str) {
	let db = Database::open(path);
	db.exec("CREATE TABLE IF NOT EXISTS t (v TEXT)");
	db.prepare("INSERT INTO t VALUES (?)").run(["early"]);
	drop(db);
}

fun read_back(path: str): str {
	let db = Database::open(path);
	match db.prepare("SELECT v FROM t").first([]) {
		Some(let row) => row.text("v"),
		None => "MISSING",
	}
}

fun main() {
	write_scoped("scoped.db");
	print(read_back("scoped.db"));
	write_early("early.db");
	print(read_back("early.db"));
}
main();
"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["run", dir.to_str().unwrap()])
        // Relative `.db` paths resolve against the child's cwd — pin it to the
        // temp project so the files never touch the repo.
        .current_dir(&dir)
        .output()
        .expect("run vilan");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `node:sqlite` prints an ExperimentalWarning to stderr — expected, not a
    // failure — so the assertion is on stdout and the exit status only.
    assert!(
        output.status.success(),
        "vilan run failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout, "scope-end\nearly\n",
        "the reopen-and-read round-trip did not return the written rows — a writer's drop did not close cleanly"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
