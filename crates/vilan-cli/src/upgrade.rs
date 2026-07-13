//! `vilan upgrade` — update this binary (and `vilan-lsp` beside it) to the
//! newest release (proposal/releases.md §6).
//!
//! The CLI never touches the network except here, on explicit request. The
//! work is delegated to the same tools the install script already requires
//! (`curl`, `tar`, `sha256sum`/`shasum`), so upgrading works exactly where
//! installing worked and the binary carries no HTTP/TLS machinery.
//!
//! Release assets are versionless (`vilan-<target>.tar.gz`), so the newest
//! version is discovered without an API round-trip: `releases/latest`
//! redirects to `releases/tag/v<version>`, and the assets are then fetched
//! from that tag's own download path (pinned — a release published mid-run
//! can't mix versions). The swap is atomic per binary: the new file is staged
//! *inside* the install directory and renamed over the old one (same
//! filesystem, and a running executable keeps its inode on unix).
//!
//! Test seams (undocumented, for the integration tests): `$VILAN_UPGRADE_BASE`
//! replaces the repository base URL (a `file://` tree works — `curl` speaks
//! it), and `$VILAN_UPGRADE_LATEST` skips the redirect discovery.

use std::path::Path;
use std::process::{Command, ExitCode};

const DEFAULT_BASE: &str = "https://github.com/ReedSyllas/vilan";

pub fn upgrade(check_only: bool) -> ExitCode {
    match run(check_only) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(check_only: bool) -> Result<(), String> {
    let current = parse_version(env!("CARGO_PKG_VERSION"))
        .ok_or_else(|| "this binary's own version is unparseable".to_string())?;
    let base = std::env::var("VILAN_UPGRADE_BASE").unwrap_or_else(|_| DEFAULT_BASE.to_string());

    let latest_label = discover_latest(&base)?;
    let latest = parse_version(&latest_label)
        .ok_or_else(|| format!("cannot parse the latest release version from `{latest_label}`"))?;

    if latest <= current {
        println!("vilan {} is the newest release.", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if check_only {
        println!(
            "vilan {} → {latest_label} available — run `vilan upgrade`.",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate the running binary: {error}"))?;
    let install_dir = executable
        .parent()
        .ok_or_else(|| "the running binary has no parent directory".to_string())?
        .to_path_buf();

    let asset = format!("vilan-{}.tar.gz", env!("VILAN_TARGET"));
    let download_base = format!("{base}/releases/download/v{latest_label}");
    let workdir = std::env::temp_dir().join(format!("vilan-upgrade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir)
        .map_err(|error| format!("cannot create {}: {error}", workdir.display()))?;
    let result = download_verify_swap(
        &download_base,
        &asset,
        &workdir,
        &install_dir,
        &latest_label,
    );
    let _ = std::fs::remove_dir_all(&workdir);
    result
}

fn download_verify_swap(
    download_base: &str,
    asset: &str,
    workdir: &Path,
    install_dir: &Path,
    latest_label: &str,
) -> Result<(), String> {
    println!(
        "vilan {} → v{latest_label} — downloading {asset} ...",
        env!("CARGO_PKG_VERSION")
    );
    fetch(&format!("{download_base}/{asset}"), &workdir.join(asset))?;
    fetch(
        &format!("{download_base}/sha256sums.txt"),
        &workdir.join("sha256sums.txt"),
    )?;
    verify_checksum(workdir, asset)?;

    let status = Command::new("tar")
        .args(["-xzf", asset])
        .current_dir(workdir)
        .status()
        .map_err(|error| format!("cannot run tar: {error}"))?;
    if !status.success() {
        return Err(format!("unpacking {asset} failed"));
    }

    // Sanity before touching anything: the downloaded binary must execute.
    let unpacked = workdir.join("vilan");
    let version_probe = Command::new(&unpacked)
        .arg("--version")
        .output()
        .map_err(|error| format!("the downloaded vilan does not execute: {error}"))?;
    if !version_probe.status.success() {
        return Err("the downloaded vilan does not report a version".to_string());
    }

    // Stage inside the install directory, then rename — atomic on the same
    // filesystem, and safe over a running executable on unix. `vilan-lsp`
    // first so the pair is never newer-cli/older-lsp.
    for binary in ["vilan-lsp", "vilan"] {
        let staged = install_dir.join(format!(".{binary}.upgrade-{}", std::process::id()));
        std::fs::copy(workdir.join(binary), &staged)
            .map_err(|error| format!("cannot stage into {}: {error}", install_dir.display()))?;
        std::fs::rename(&staged, install_dir.join(binary)).map_err(|error| {
            let _ = std::fs::remove_file(&staged);
            format!(
                "cannot replace {}: {error}",
                install_dir.join(binary).display()
            )
        })?;
    }

    let installed = String::from_utf8_lossy(&version_probe.stdout)
        .trim()
        .to_string();
    println!("installed {installed} to {}", install_dir.display());
    Ok(())
}

/// The newest release's version label (no `v`), from `$VILAN_UPGRADE_LATEST`
/// or the `releases/latest` redirect.
fn discover_latest(base: &str) -> Result<String, String> {
    if let Ok(forced) = std::env::var("VILAN_UPGRADE_LATEST") {
        return Ok(forced);
    }
    let output = Command::new("curl")
        .args([
            "-fsSLI",
            "-o",
            "/dev/null",
            "-w",
            "%{url_effective}",
            &format!("{base}/releases/latest"),
        ])
        .output()
        .map_err(|error| format!("cannot run curl: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot reach {base}/releases/latest — check your connection (or see {base}/releases)"
        ));
    }
    let final_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    version_from_tag_url(&final_url)
        .ok_or_else(|| format!("`{final_url}` does not name a release tag"))
}

fn fetch(url: &str, to: &Path) -> Result<(), String> {
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(to)
        .arg(url)
        .status()
        .map_err(|error| format!("cannot run curl: {error}"))?;
    if !status.success() {
        return Err(format!("download failed: {url}"));
    }
    Ok(())
}

/// Verify `asset` against the release's `sha256sums.txt` (both already in
/// `workdir`), with whichever checksum tool the platform has.
fn verify_checksum(workdir: &Path, asset: &str) -> Result<(), String> {
    let sums = std::fs::read_to_string(workdir.join("sha256sums.txt"))
        .map_err(|error| format!("cannot read sha256sums.txt: {error}"))?;
    let line = sums
        .lines()
        .find(|line| line.ends_with(&format!(" {asset}")))
        .ok_or_else(|| format!("sha256sums.txt has no entry for {asset}"))?;
    let expectation = workdir.join(".expected-sum");
    std::fs::write(&expectation, format!("{line}\n"))
        .map_err(|error| format!("cannot write the checksum expectation: {error}"))?;

    for tool in [&["sha256sum", "-c"][..], &["shasum", "-a", "256", "-c"][..]] {
        let run = Command::new(tool[0])
            .args(&tool[1..])
            .arg(&expectation)
            .current_dir(workdir)
            .output();
        match run {
            Err(_) => continue, // tool not present; try the next
            Ok(output) if output.status.success() => return Ok(()),
            Ok(_) => return Err(format!("checksum mismatch for {asset} — aborting")),
        }
    }
    Err("no sha256 tool found (sha256sum or shasum) — cannot verify the download".to_string())
}

/// `".../releases/tag/v0.3.0"` → `"0.3.0"`.
fn version_from_tag_url(url: &str) -> Option<String> {
    let (_, tag) = url.rsplit_once("/tag/")?;
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    if parse_version(tag).is_some() {
        Some(tag.to_string())
    } else {
        None
    }
}

/// `"0.2.0"` → `(0, 2, 0)`; a missing patch or minor reads as zero.
fn parse_version(label: &str) -> Option<(u64, u64, u64)> {
    let mut parts = label.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |part| part.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |part| part.parse().ok())?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::{parse_version, version_from_tag_url};

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(parse_version("0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
        assert_eq!(
            parse_version("v0.2.0"),
            None,
            "the v prefix is the tag's, not the version's"
        );
        assert_eq!(parse_version("0.2.0.1"), None);
        assert_eq!(parse_version("not-a-version"), None);
        assert!(parse_version("0.3.0") > parse_version("0.2.9"));
        assert!(
            parse_version("0.10.0") > parse_version("0.9.9"),
            "numeric, not lexicographic"
        );
        assert!(parse_version("1.0.0") > parse_version("0.99.99"));
    }

    #[test]
    fn the_release_tag_comes_from_the_redirect_url() {
        assert_eq!(
            version_from_tag_url("https://github.com/ReedSyllas/vilan/releases/tag/v0.3.0"),
            Some("0.3.0".to_string())
        );
        // No tag in the URL (e.g. no releases yet → /releases) or garbage: None.
        assert_eq!(
            version_from_tag_url("https://github.com/ReedSyllas/vilan/releases"),
            None
        );
        assert_eq!(
            version_from_tag_url("https://github.com/x/y/releases/tag/nightly"),
            None
        );
    }
}
