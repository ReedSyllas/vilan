//! Pins for the embedded toolchain (proposal/releases.md §3): the embedded
//! table must mirror the working tree exactly (both directions — a collector
//! that misses a directory is a silently incomplete toolchain), and
//! materialization must produce a complete, idempotent, real-file copy.

use std::fs;
use std::path::{Path, PathBuf};

use vilan_embedded_std::{CONTENT_HASH, FILES, materialize_into};

fn vilan_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan")
}

#[test]
fn the_table_carries_the_expected_packages() {
    let keys: Vec<&str> = FILES.iter().map(|(key, _)| *key).collect();
    // Both package manifests (resolve_std needs them for the platform layers),
    // a base module, a layer module, and macro_std's entry.
    for expected in [
        "std/vilan.toml",
        "std/src/lib.vl",
        "std/src/reactive.vl",
        "std/src/browser/dom.vl",
        "std/src/process/fs.vl",
        "macro_std/vilan.toml",
        "macro_std/src/lib.vl",
    ] {
        assert!(
            keys.contains(&expected),
            "missing embedded file: {expected}"
        );
    }
    assert!(keys.is_sorted(), "the table must be sorted (stable hash)");
    for key in &keys {
        assert!(
            !key.contains('\\') && !key.starts_with('/'),
            "keys are relative, forward-slash paths: {key}"
        );
    }
    assert_eq!(CONTENT_HASH.len(), 16, "the hash is 16 hex characters");
}

#[test]
fn the_table_matches_the_working_tree_in_both_directions() {
    let root = vilan_root();
    // Embedded → disk: every entry is byte-identical to the checkout.
    for (key, contents) in FILES {
        let on_disk = fs::read_to_string(root.join(key))
            .unwrap_or_else(|error| panic!("embedded {key} missing on disk: {error}"));
        assert!(
            on_disk == *contents,
            "embedded {key} differs from the working tree (stale build script output?)"
        );
    }
    // Disk → embedded: every toolchain file in the checkout is embedded.
    let mut on_disk = Vec::new();
    for package in ["std", "macro_std"] {
        walk(&root.join(package), Path::new(package), &mut on_disk);
    }
    let keys: Vec<&str> = FILES.iter().map(|(key, _)| *key).collect();
    for file in on_disk {
        assert!(
            keys.contains(&file.as_str()),
            "{file} exists in the checkout but is not embedded (collector gap)"
        );
    }
}

#[test]
fn materialization_is_complete_and_idempotent() {
    let cache_root = std::env::temp_dir().join(format!(
        "vilan-embedded-std-test-{}-{CONTENT_HASH}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&cache_root);

    let std_dir = materialize_into(&cache_root).expect("first materialization");
    assert!(std_dir.ends_with(Path::new(CONTENT_HASH).join("std")));
    assert!(std_dir.join("vilan.toml").is_file(), "std manifest");
    // macro_std must land beside std — resolve_macro_std finds it as a sibling.
    let toolchain_root = std_dir.parent().unwrap();
    assert!(toolchain_root.join("macro_std/vilan.toml").is_file());
    for (key, contents) in FILES {
        let written = fs::read_to_string(toolchain_root.join(key)).expect(key);
        assert!(written == *contents, "materialized {key} differs");
    }

    // A second call finds the cache and does not rewrite it.
    let before = fs::metadata(std_dir.join("vilan.toml"))
        .unwrap()
        .modified()
        .unwrap();
    let again = materialize_into(&cache_root).expect("second materialization");
    assert_eq!(again, std_dir);
    let after = fs::metadata(std_dir.join("vilan.toml"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        before, after,
        "an existing cache entry must not be rewritten"
    );

    let _ = fs::remove_dir_all(&cache_root);
}

#[test]
fn pruning_removes_only_entries_older_than_the_guard() {
    use vilan_embedded_std::prune_stale;

    let cache_root =
        std::env::temp_dir().join(format!("vilan-embedded-std-prune-{}", std::process::id()));
    let _ = fs::remove_dir_all(&cache_root);
    for entry in [
        "fresh-entry",
        "stale-entry",
        ".staging-stale",
        ".staging-fresh",
    ] {
        fs::create_dir_all(cache_root.join(entry).join("std")).expect("create entry");
    }
    for stale in ["stale-entry", ".staging-stale"] {
        let backdate = std::process::Command::new("touch")
            .args(["-d", "2020-01-01"])
            .arg(cache_root.join(stale))
            .status()
            .expect("touch");
        assert!(backdate.success());
    }

    let one_day = std::time::Duration::from_secs(24 * 60 * 60);
    assert_eq!(
        prune_stale(&cache_root, one_day),
        2,
        "both backdated dirs go"
    );
    assert!(
        cache_root.join("fresh-entry").is_dir(),
        "young entries stay"
    );
    assert!(
        cache_root.join(".staging-fresh").is_dir(),
        "a staging dir inside the guard may be a materialization in flight"
    );
    assert!(!cache_root.join("stale-entry").exists());
    assert!(!cache_root.join(".staging-stale").exists());

    // A missing root is a quiet no-op, not an error.
    let _ = fs::remove_dir_all(&cache_root);
    assert_eq!(prune_stale(&cache_root, one_day), 0);
}

/// The build script's collection rule, restated independently: every `.vl` and
/// `vilan.toml` under the package directory.
fn walk(directory: &Path, prefix: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(directory).expect("readable package directory") {
        let entry = entry.expect("readable entry");
        let path = entry.path();
        let relative = prefix.join(entry.file_name());
        if path.is_dir() {
            walk(&path, &relative, files);
        } else if path.extension().is_some_and(|extension| extension == "vl")
            || entry.file_name() == "vilan.toml"
        {
            files.push(
                relative
                    .components()
                    .map(|component| component.as_os_str().to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}
