//! The corpus byte gate (backlog E5): every `vilan/test/*.vl` with a `.js`
//! golden compiles — via the CURRENT `vilan` binary, exactly the command that
//! generated the goldens — to byte-identical output (`.css` assets included).
//!
//! This replaces the by-hand loop (rebuild the debug binary, regenerate,
//! `git diff`) that the golden-regen discipline existed to police: the binary
//! under test is always the one Cargo just built from this tree, so a stale
//! binary can no longer write or check goldens. A deliberate output change
//! still regenerates goldens by hand; this gate then verifies the commit.

use std::path::{Path, PathBuf};
use std::process::Command;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/test")
}

fn std_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std")
}

/// First differing line between golden and rebuilt output, for the report.
fn first_difference(golden: &str, rebuilt: &str) -> String {
    for (line, (a, b)) in golden.lines().zip(rebuilt.lines()).enumerate() {
        if a != b {
            return format!("line {}: golden {a:?} vs rebuilt {b:?}", line + 1);
        }
    }
    format!(
        "lengths differ (golden {} lines, rebuilt {})",
        golden.lines().count(),
        rebuilt.lines().count()
    )
}

#[test]
fn every_corpus_golden_is_byte_identical() {
    let corpus = corpus_dir();
    // A full copy: corpus programs may import sibling modules, and building
    // in place would overwrite the goldens under comparison.
    let work = std::env::temp_dir().join(format!("vilan_corpus_gate_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("create corpus work dir");
    let mut programs: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&corpus).expect("corpus directory") {
        let path = entry.expect("corpus entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(extension) = path.extension() else {
            continue;
        };
        if extension == "vl" {
            std::fs::copy(&path, work.join(name)).expect("copy corpus source");
            if path.with_extension("js").is_file() {
                programs.push(name.to_string());
            }
        }
    }
    programs.sort();
    assert!(
        programs.len() > 60,
        "suspiciously few corpus programs: {}",
        programs.len()
    );

    // Builds run against the repo's std (the goldens were generated with it),
    // in parallel chunks — each program is an independent compile.
    let failures: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = programs
            .chunks(programs.len().div_ceil(8).max(1))
            .map(|chunk| {
                let work = &work;
                let corpus = &corpus;
                scope.spawn(move || {
                    let mut failures = Vec::new();
                    for name in chunk {
                        let source = work.join(name);
                        let output = Command::new(env!("CARGO_BIN_EXE_vilan"))
                            .arg("build")
                            .arg(&source)
                            .env("VILAN_STD", std_dir())
                            .output()
                            .expect("run vilan build");
                        if !output.status.success() {
                            failures.push(format!(
                                "{name}: build failed:\n{}",
                                String::from_utf8_lossy(&output.stderr)
                            ));
                            continue;
                        }
                        for asset in ["js", "css"] {
                            let golden_path = corpus.join(name).with_extension(asset);
                            if !golden_path.is_file() {
                                continue;
                            }
                            let golden =
                                std::fs::read_to_string(&golden_path).expect("read golden");
                            let rebuilt = std::fs::read_to_string(source.with_extension(asset))
                                .unwrap_or_default();
                            if golden != rebuilt {
                                failures.push(format!(
                                    "{name} (.{asset}): {}",
                                    first_difference(&golden, &rebuilt)
                                ));
                            }
                        }
                    }
                    failures
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("corpus worker"))
            .collect()
    });
    let _ = std::fs::remove_dir_all(&work);
    assert!(
        failures.is_empty(),
        "{} corpus golden(s) diverged:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The equivalence-gate rationale for HMR (A13, `hmr.md` §5): the `build` path
/// never sets `BuildOptions.hmr`, so no corpus golden may carry `__hmr_`
/// instrumentation. A cheap read-only sweep of the committed goldens — instrumentation
/// is a `run --watch`-only concern and must never reach a built bundle.
#[test]
fn no_corpus_golden_carries_hmr_instrumentation() {
    let corpus = corpus_dir();
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&corpus).expect("corpus directory") {
        let path = entry.expect("corpus entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("js") {
            continue;
        }
        let golden = std::fs::read_to_string(&path).expect("read golden");
        assert!(
            !golden.contains("__hmr_"),
            "{path:?} carries HMR instrumentation but was built off the `build` path"
        );
        checked += 1;
    }
    assert!(checked > 60, "suspiciously few goldens swept: {checked}");
}
