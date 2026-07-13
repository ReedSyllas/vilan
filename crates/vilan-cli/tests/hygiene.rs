//! Publication hygiene: no tracked file may contain an absolute
//! home-directory path. Build artifacts and pasted terminal output are how
//! development machine paths leak into public repos; anything path-shaped
//! that must live in the tree should be relative (and everything that needs
//! an absolute path derives it at runtime or via CARGO_MANIFEST_DIR).
//!
//! The check is deliberately generic — any absolute path under the Linux,
//! macOS, or Windows user-profile roots — so it is safe to publish and
//! independent of any particular machine or username. (The needles are
//! assembled at runtime so this file doesn't trip itself.)

use std::path::PathBuf;
use std::process::Command;

#[test]
fn no_tracked_file_contains_an_absolute_home_path() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let listing = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&repo_root)
        .output()
        .expect("git ls-files");
    assert!(listing.status.success(), "git ls-files failed");
    let names = String::from_utf8_lossy(&listing.stdout);

    let needles = [
        format!("/{}/", "home"),
        format!("/{}/", "Users"),
        format!("C:\\{}\\", "Users"),
    ];
    let mut offenders = Vec::new();
    for name in names.split('\0').filter(|name| !name.is_empty()) {
        let path = repo_root.join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            continue; // deleted-but-staged etc.
        };
        // Skip binaries; every leakable path in this repo is in text.
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if needles.iter().any(|needle| line.contains(needle.as_str())) {
                offenders.push(format!("{name}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "absolute home-directory paths in tracked files (use relative paths):\n{}",
        offenders.join("\n")
    );
}
