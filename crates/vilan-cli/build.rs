//! Stamps the binary with its git commit (proposal/releases.md §4):
//! `vilan --version` prints `vilan <version> (<short-sha>)`, so bug reports
//! against moving alpha builds are precise. Builds without a git checkout
//! (a source tarball) stamp `unknown`.

use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = Path::new(&manifest_dir).join("../..");

    // Re-stamp on commit or branch switch. (A dirty working tree can go stale
    // between builds — acceptable for a dev-only marker; release builds are
    // always clean checkouts.)
    let git_dir = repo_root.join(".git");
    if git_dir.is_dir() {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join("packed-refs").display()
        );
        if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) {
            if let Some(reference) = head.trim().strip_prefix("ref: ") {
                println!(
                    "cargo:rerun-if-changed={}",
                    git_dir.join(reference).display()
                );
            }
        }
    }

    let sha = git(&repo_root, &["rev-parse", "--short=9", "HEAD"]);
    let stamp = match sha {
        Some(sha) => {
            // `-uno`: untracked files don't change the binary.
            let dirty = git(&repo_root, &["status", "--porcelain", "-uno"])
                .is_some_and(|status| !status.is_empty());
            if dirty { format!("{sha}-dirty") } else { sha }
        }
        None => "unknown".to_string(),
    };
    println!("cargo:rustc-env=VILAN_BUILD_SHA={stamp}");
}

fn git(repo_root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}
