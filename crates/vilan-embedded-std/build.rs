//! Embeds the `std` and `macro_std` package trees into the binary.
//!
//! Walks `vilan/std` and `vilan/macro_std` at the workspace root and generates
//! `$OUT_DIR/embedded_std.rs`: a sorted `FILES` table of
//! `("std/src/print.vl", include_str!(..))` entries plus a `CONTENT_HASH` over
//! the whole set. The hash keys the on-disk materialization cache
//! (`~/.vilan/std-cache/<hash>/`), so a rebuilt binary with different std never
//! reads a stale cache.
//!
//! `include_str!` makes every embedded file a compile input of this crate
//! (edits re-embed automatically); the `rerun-if-changed` directives on the
//! directories cover the set itself changing (files added or removed). This
//! lives in its own leaf crate so a std edit relinks the binaries without
//! recompiling the compiler.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let vilan_root = manifest_dir.join("../../vilan");
    let out_path = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("embedded_std.rs");

    let mut files = Vec::new();
    for package in ["std", "macro_std"] {
        let root = vilan_root.join(package);
        assert!(
            root.join("vilan.toml").is_file(),
            "embedded-std build: {} is not a package directory (missing vilan.toml) — \
             is this a complete checkout?",
            root.display()
        );
        collect(&root, Path::new(package), &mut files);
    }
    // Deterministic table and hash, independent of directory iteration order.
    files.sort();

    let mut hasher = Fnv1a64::new();
    for (key, path) in &files {
        hasher.write(key.as_bytes());
        hasher.write(&fs::read(path).unwrap());
    }

    let mut generated = String::new();
    generated.push_str(
        "/// A hash of every embedded path and its contents — the key of the\n\
         /// materialization cache directory.\n",
    );
    generated.push_str(&format!(
        "pub static CONTENT_HASH: &str = \"{:016x}\";\n\n",
        hasher.finish()
    ));
    generated.push_str(
        "/// The embedded `std` and `macro_std` package trees, as\n\
         /// (path relative to the toolchain root, contents) — sorted by path.\n",
    );
    generated.push_str("pub static FILES: &[(&str, &str)] = &[\n");
    for (key, path) in &files {
        generated.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            key,
            path.canonicalize().unwrap()
        ));
    }
    generated.push_str("];\n");

    let mut out = fs::File::create(&out_path).unwrap();
    out.write_all(generated.as_bytes()).unwrap();
}

/// Collects every `.vl` and `vilan.toml` under `directory` into `files` as
/// (forward-slash key relative to the toolchain root, absolute path), and emits
/// `rerun-if-changed` for each directory so added or removed files regenerate
/// the table.
fn collect(directory: &Path, prefix: &Path, files: &mut Vec<(String, PathBuf)>) {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = prefix.join(entry.file_name());
        if path.is_dir() {
            collect(&path, &relative, files);
        } else if path.extension().is_some_and(|extension| extension == "vl")
            || entry.file_name() == "vilan.toml"
        {
            let key = relative
                .components()
                .map(|component| component.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            files.push((key, path));
        }
    }
}

/// FNV-1a, 64-bit — tiny, dependency-free, and stable across builds. The hash
/// only keys a cache directory; it has no security role.
struct Fnv1a64(u64);

impl Fnv1a64 {
    fn new() -> Self {
        Fnv1a64(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
