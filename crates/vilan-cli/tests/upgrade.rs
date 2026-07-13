//! `vilan upgrade` pins (proposal/releases.md §6), fully offline: a fake
//! release tree served over `file://` (curl speaks it) drives the real
//! discovery/download/verify/swap path against a copied binary. The fake
//! "release" binaries are shell scripts that identify themselves, so a swap
//! is observable by running the result.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

/// A scratch install: a copied `vilan` in its own bin dir, plus a fake
/// release tree for `$VILAN_UPGRADE_BASE`.
struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    base_url: String,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("vilan-upgrade-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("create bin dir");
        fs::copy(env!("CARGO_BIN_EXE_vilan"), bin.join("vilan")).expect("copy the binary");

        // The fake v9.9.9 release: self-identifying shell scripts, tarred and
        // checksummed exactly as the release workflow does.
        let stage = root.join("stage");
        fs::create_dir_all(&stage).expect("create stage");
        for (binary, banner) in [
            ("vilan", "vilan 9.9.9 (fake)"),
            ("vilan-lsp", "vilan-lsp 9.9.9 (fake)"),
        ] {
            let path = stage.join(binary);
            fs::write(&path, format!("#!/bin/sh\necho \"{banner}\"\n")).expect("write dummy");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod dummy");
        }
        let assets = root.join("releases/download/v9.9.9");
        fs::create_dir_all(&assets).expect("create release dir");
        let asset_name = format!("vilan-{}.tar.gz", env!("VILAN_TARGET"));
        run_ok(
            Command::new("tar")
                .args(["-czf"])
                .arg(assets.join(&asset_name))
                .args(["-C"])
                .arg(&stage)
                .args(["vilan", "vilan-lsp"]),
        );
        let sums = run_ok(
            Command::new("sha256sum")
                .arg(&asset_name)
                .current_dir(&assets),
        );
        fs::write(assets.join("sha256sums.txt"), sums.stdout).expect("write sums");

        // A scratch HOME with a pre-seeded std cache: one entry backdated past
        // the prune guard, one fresh — a successful upgrade prunes exactly the
        // stale one.
        let home = root.join("home");
        let cache = home.join(".vilan/std-cache");
        fs::create_dir_all(cache.join("fresh-entry/std")).expect("seed fresh cache entry");
        fs::create_dir_all(cache.join("stale-entry/std")).expect("seed stale cache entry");
        run_ok(
            Command::new("touch")
                .args(["-d", "2020-01-01"])
                .arg(cache.join("stale-entry")),
        );

        let base_url = format!("file://{}", root.display());
        Fixture {
            root,
            bin,
            home,
            base_url,
        }
    }

    fn cache_entry(&self, name: &str) -> PathBuf {
        self.home.join(".vilan/std-cache").join(name)
    }

    fn upgrade(&self, arguments: &[&str], latest: &str) -> Output {
        run_retrying(
            Command::new(self.bin.join("vilan"))
                .arg("upgrade")
                .args(arguments)
                .env("HOME", &self.home)
                .env("VILAN_UPGRADE_BASE", &self.base_url)
                .env("VILAN_UPGRADE_LATEST", latest),
        )
    }

    fn installed_banner(&self) -> String {
        // `--version` on the real binary; the fake one echoes its banner
        // regardless of arguments.
        let output = run_retrying(Command::new(self.bin.join("vilan")).arg("--version"));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn upgrade_swaps_both_binaries_and_reports_the_new_version() {
    let fixture = Fixture::new("swap");
    let output = fixture.upgrade(&[], "9.9.9");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "upgrade failed:\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("→ v9.9.9"),
        "announces the target version: {stdout}"
    );
    assert!(
        stdout.contains("installed vilan 9.9.9 (fake)"),
        "reports the swapped version: {stdout}"
    );

    // The running binary's path now holds the new release, lsp beside it.
    assert_eq!(fixture.installed_banner(), "vilan 9.9.9 (fake)");
    // And ~/.vilan housekeeping ran: the stale cache entry is gone, the
    // fresh one (a running binary could be reading it) stays.
    assert!(
        stdout.contains("pruned 1 stale std cache entry"),
        "{stdout}"
    );
    assert!(!fixture.cache_entry("stale-entry").exists());
    assert!(fixture.cache_entry("fresh-entry").is_dir());
    let lsp = run_retrying(&mut Command::new(fixture.bin.join("vilan-lsp")));
    assert_eq!(
        String::from_utf8_lossy(&lsp.stdout).trim(),
        "vilan-lsp 9.9.9 (fake)"
    );
}

#[test]
fn upgrade_check_reports_and_changes_nothing() {
    let fixture = Fixture::new("check");
    let output = fixture.upgrade(&["--check"], "9.9.9");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("9.9.9 available") && stdout.contains("vilan upgrade"),
        "points at the command: {stdout}"
    );
    // Still the real binary, no lsp appeared, and the cache is untouched —
    // `--check` changes nothing.
    assert!(
        fixture
            .installed_banner()
            .starts_with(&format!("vilan {}", env!("CARGO_PKG_VERSION")))
    );
    assert!(!fixture.bin.join("vilan-lsp").exists());
    assert!(fixture.cache_entry("stale-entry").is_dir());
}

#[test]
fn upgrade_declines_when_already_the_newest() {
    let fixture = Fixture::new("newest");
    let output = fixture.upgrade(&[], "0.1.0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("is the newest release"), "{stdout}");
    assert!(
        fixture
            .installed_banner()
            .starts_with(&format!("vilan {}", env!("CARGO_PKG_VERSION")))
    );
}

#[test]
fn upgrade_aborts_on_a_checksum_mismatch_without_touching_the_install() {
    let fixture = Fixture::new("badsum");
    // Corrupt the recorded hash: flip its first hex digit.
    let sums_path = fixture.root.join("releases/download/v9.9.9/sha256sums.txt");
    let sums = fs::read_to_string(&sums_path).expect("read sums");
    let flipped = if sums.starts_with('0') { "f" } else { "0" };
    fs::write(&sums_path, format!("{flipped}{}", &sums[1..])).expect("corrupt sums");

    let output = fixture.upgrade(&[], "9.9.9");
    assert!(
        !output.status.success(),
        "a bad checksum must fail the upgrade"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"),
        "names the problem: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fixture
            .installed_banner()
            .starts_with(&format!("vilan {}", env!("CARGO_PKG_VERSION")))
    );
    assert!(!fixture.bin.join("vilan-lsp").exists());
}

fn run_ok(command: &mut Command) -> Output {
    let output = command.output().expect("spawn");
    assert!(
        output.status.success(),
        "fixture command failed: {command:?}"
    );
    output
}

/// Spawn with an ETXTBSY retry — the fork/CLOEXEC exec race between parallel
/// tests copying binaries (same guard as tests/install.rs).
fn run_retrying(command: &mut Command) -> Output {
    const ETXTBSY: i32 = 26;
    let mut attempts = 0;
    loop {
        match command.output() {
            Err(error) if error.raw_os_error() == Some(ETXTBSY) && attempts < 100 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => return other.expect("run the binary"),
        }
    }
}
