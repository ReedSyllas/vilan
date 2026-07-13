//! The installed-binary smoke test (proposal/releases.md §8, slice 1): a
//! `vilan` copied out of the repo — no checkout in any ancestor, no
//! `$VILAN_STD`, a fresh `$HOME` — must still compile and run a program, by
//! materializing its embedded std into `~/.vilan/std-cache/<hash>/`. This is
//! the exact shape of every binary the install script delivers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch "user machine": an empty home and a working directory, both under
/// the system temp dir (outside any checkout), holding a copy of the binary.
struct Machine {
    root: PathBuf,
    home: PathBuf,
    binary: PathBuf,
}

impl Machine {
    fn new(name: &str) -> Machine {
        let root =
            std::env::temp_dir().join(format!("vilan-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create scratch home");
        let binary = root.join("vilan");
        fs::copy(env!("CARGO_BIN_EXE_vilan"), &binary).expect("copy the binary out of the repo");
        Machine { root, home, binary }
    }

    fn vilan(&self, arguments: &[&str]) -> Output {
        // Retry ETXTBSY: a concurrent test's fork can briefly hold this
        // binary's just-written fd until its own exec closes it (CLOEXEC), and
        // an exec landing in that window fails spuriously. Cargo carries the
        // same retry for the same race.
        const ETXTBSY: i32 = 26;
        let mut attempts = 0;
        loop {
            let result = Command::new(&self.binary)
                .args(arguments)
                .current_dir(&self.root)
                .env_remove("VILAN_STD")
                .env("HOME", &self.home)
                .env_remove("USERPROFILE")
                .output();
            match result {
                Err(error) if error.raw_os_error() == Some(ETXTBSY) && attempts < 100 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other.expect("run the copied binary"),
            }
        }
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn an_installed_binary_compiles_and_runs_without_a_checkout() {
    let machine = Machine::new("run");
    fs::write(
        machine.root.join("hello.vl"),
        "import std::print;\n\nfun main() {\n    print(\"hello from an installed vilan\");\n}\n",
    )
    .expect("write hello.vl");

    let first = machine.vilan(&["run", "hello.vl"]);
    assert!(
        first.status.success(),
        "first run failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("hello from an installed vilan"),
        "program output missing: {}",
        String::from_utf8_lossy(&first.stdout)
    );

    // The embedded toolchain landed in the scratch home, keyed by content.
    let cache = machine.home.join(".vilan").join("std-cache");
    let entries: Vec<_> = fs::read_dir(&cache)
        .expect("the std cache directory exists")
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert_eq!(entries.len(), 1, "one content-keyed cache entry");
    let toolchain = entries[0].path();
    assert!(toolchain.join("std/vilan.toml").is_file());
    assert!(toolchain.join("macro_std/vilan.toml").is_file());

    // Second run: the cache is reused, not rebuilt (the tree is immutable).
    let manifest_modified = modified(&toolchain.join("std/vilan.toml"));
    let second = machine.vilan(&["run", "hello.vl"]);
    assert!(second.status.success(), "second run failed");
    assert_eq!(
        modified(&toolchain.join("std/vilan.toml")),
        manifest_modified,
        "a warm cache must not be rewritten"
    );
}

#[test]
fn version_reports_the_toolchain_and_its_build() {
    let machine = Machine::new("version");
    let output = machine.vilan(&["--version"]);
    assert!(output.status.success());
    let version = String::from_utf8_lossy(&output.stdout);
    // `vilan <semver> (<sha or unknown>)` — the sha keeps alpha bug reports precise.
    assert!(
        version.starts_with("vilan ")
            && version.contains(" (")
            && version.trim_end().ends_with(')'),
        "unexpected --version shape: {version}"
    );
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .expect("metadata")
        .modified()
        .expect("mtime")
}
