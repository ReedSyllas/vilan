//! The corpus-through-the-new-frontend byte gate (H6 S3, `proposal/frontend.md`
//! §3 S3).
//!
//! Gate 1 (`parse_differential.rs`) proves the handwritten frontend's TREES are
//! byte-identical (span-inclusive) to chumsky's over every corpus file. This gate
//! proves the stronger, end-to-end claim: those trees, driven through the REAL
//! `analyze` + post-analysis passes + `transform` — the exact sequence the CLI's
//! `compile_to_js` runs on the `build` path — produce the committed `.js`/`.css`
//! goldens byte-for-byte. The pipeline itself is untouched (S5 does the wiring);
//! this target reaches into the library and swaps ONLY the entry file's parse.
//!
//! It is self-diagnosing. For each corpus program it drives the SAME harness twice
//! — once on the chumsky tree (`parse_clean`), once on the handwritten tree
//! (`parsing::parse`) — and asserts:
//!   * the two outputs are byte-identical (FRONTEND faithfulness end-to-end), and
//!   * the handwritten output equals the committed golden (byte-identical to the
//!     `build` command's output).
//! If the chumsky-tree output already differs from the golden, the divergence is
//! in this HARNESS (a build-parameter it failed to mirror), not the frontend — the
//! two assertions separate those causes. The build parameters mirror a bare-file
//! `vilan build`: repo std, the entry's parent as `pkg_root`, the default `node`
//! platform, the default (`debug`) build options, an empty workspace.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use vilan_core::analyzer::{PackageSpec, Program, Workspace};
use vilan_core::node::NodeList;
use vilan_core::options::BuildOptions;
use vilan_core::span::Spanned;
use vilan_core::target::Platform;
use vilan_core::{
    analyzer, async_infer, const_eval, context, lift, parse_clean, parsing, platform_color,
    transform,
};

fn repo_vilan() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan")
}

/// Leak `source` to `'static` (the parse cache's model — the tree borrows it), so
/// the analyzed program can outlive the parse, exactly as `analyze_source` /
/// `compile_to_js` leak the entry text.
fn leak(source: &str) -> &'static str {
    Box::leak(source.to_string().into_boxed_str())
}

/// Parse `leaked` with the chumsky fast path, lift, and leak the tree — the oracle
/// for this harness's own fidelity. `None` if the source is not perfectly clean.
fn chumsky_tree(leaked: &'static str) -> Option<&'static Spanned<NodeList<'static>>> {
    let mut root = parse_clean(leaked)?;
    lift::rewrite_items(&mut root.0);
    Some(Box::leak(Box::new(root)))
}

/// Parse `leaked` with the HANDWRITTEN frontend, lift, and leak the tree — the
/// subject under test. `None` if the source is not perfectly clean.
fn handwritten_tree(leaked: &'static str) -> Option<&'static Spanned<NodeList<'static>>> {
    let (tree, errors) = parsing::parse(leaked);
    if !errors.is_empty() {
        return None;
    }
    let mut root = tree?;
    lift::rewrite_items(&mut root.0);
    Some(Box::leak(Box::new(root)))
}

/// Drive a pre-parsed entry tree through the full build path — `analyze`, the
/// post-analysis passes (context threading, async inference, the drop checks, the
/// platform check, const evaluation), and `transform` — returning the emitted JS
/// and the assembled assets (`kind -> content`, e.g. `"css"`). Mirrors
/// `crates/vilan-cli/src/main.rs::compile_to_js` on the `build` path.
fn build(
    root: &'static Spanned<NodeList<'static>>,
    source: &'static str,
    std: &PackageSpec,
    pkg_root: &Path,
    entry_path: &Path,
    platform: Platform,
    workspace: &Workspace,
    options: &BuildOptions,
) -> Result<(String, BTreeMap<String, String>), String> {
    let mut program: Program<'static> =
        analyzer::analyze(root, source, std, pkg_root, entry_path, platform, workspace);
    context::thread_contexts(&mut program);
    async_infer::infer(&mut program);
    analyzer::check_async_drops(&mut program);
    analyzer::check_context_drops(&mut program);
    platform_color::check(&mut program, platform);
    let (const_results, const_assets, const_errors) =
        const_eval::evaluate(&program, &BuildOptions::default());
    program.const_results = const_results;
    program.const_assets = const_assets;
    program.diagnostics.extend(const_errors);

    if !program.diagnostics.is_empty() {
        return Err(format!(
            "{} analyzer diagnostic(s), first: {}",
            program.diagnostics.len(),
            program.diagnostics[0].msg
        ));
    }
    let javascript = transform(&program, options).map_err(|error| error.msg)?;
    let assets = const_eval::assemble_assets(&program.const_assets);
    Ok((javascript, assets))
}

/// The first differing line between two outputs, for a compact report.
fn first_difference(expected: &str, actual: &str) -> String {
    for (line, (a, b)) in expected.lines().zip(actual.lines()).enumerate() {
        if a != b {
            return format!("line {}: expected {a:?} vs got {b:?}", line + 1);
        }
    }
    format!(
        "lengths differ (expected {} lines, got {})",
        expected.lines().count(),
        actual.lines().count()
    )
}

#[test]
fn every_corpus_golden_is_byte_identical_through_the_new_frontend() {
    let test_dir = repo_vilan().join("test");
    let std = vilan_core::manifest::resolve_std(&repo_vilan().join("std"));
    let workspace = Workspace::default();
    let options = BuildOptions::default();
    let platform = Platform::default();

    // Every `.vl` in the corpus that has a `.js` golden — the exact set the
    // corpus byte gate (`corpus.rs`) checks by shelling out to `vilan build`.
    let mut programs: Vec<PathBuf> = std::fs::read_dir(&test_dir)
        .expect("read corpus dir")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "vl")
                && path.with_extension("js").is_file()
        })
        .collect();
    programs.sort();
    assert!(
        programs.len() > 60,
        "suspiciously few corpus programs: {}",
        programs.len()
    );

    let mut failures: Vec<String> = Vec::new();
    for entry_path in &programs {
        let name = entry_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let source = std::fs::read_to_string(entry_path).expect("read corpus source");
        // The entry text is leaked once and shared by both trees (both borrow it).
        let leaked = leak(&source);

        let Some(handwritten_root) = handwritten_tree(leaked) else {
            failures.push(format!(
                "{name}: the handwritten frontend declined a corpus source the build path accepts"
            ));
            continue;
        };
        let Some(chumsky_root) = chumsky_tree(leaked) else {
            failures.push(format!(
                "{name}: the chumsky fast path declined this source"
            ));
            continue;
        };

        // `pkg_root` is the entry's directory (the corpus dir) so sibling-module
        // imports resolve — those modules parse via the normal cached path inside
        // `analyze`; only the ENTRY is swapped here.
        let pkg_root = entry_path.parent().unwrap();
        let handwritten = build(
            handwritten_root,
            leaked,
            &std,
            pkg_root,
            entry_path,
            platform,
            &workspace,
            &options,
        );
        let chumsky = build(
            chumsky_root,
            leaked,
            &std,
            pkg_root,
            entry_path,
            platform,
            &workspace,
            &options,
        );

        let (handwritten_js, handwritten_assets) = match handwritten {
            Ok(output) => output,
            Err(error) => {
                failures.push(format!("{name}: handwritten build failed: {error}"));
                continue;
            }
        };
        let (chumsky_js, chumsky_assets) = match chumsky {
            Ok(output) => output,
            Err(error) => {
                failures.push(format!("{name}: chumsky build failed (HARNESS): {error}"));
                continue;
            }
        };

        // (a) Frontend faithfulness end-to-end: the two trees produce identical
        // output. A failure here is unambiguously the FRONTEND.
        if handwritten_js != chumsky_js {
            failures.push(format!(
                "{name} (.js): handwritten vs chumsky FRONTEND divergence: {}",
                first_difference(&chumsky_js, &handwritten_js)
            ));
        }
        if handwritten_assets != chumsky_assets {
            failures.push(format!(
                "{name}: handwritten vs chumsky asset divergence (FRONTEND)"
            ));
        }

        // (b) Byte-identical to the committed golden. If (a) held but this fails,
        // the chumsky-vs-golden line below localizes it to the HARNESS.
        let golden_js =
            std::fs::read_to_string(entry_path.with_extension("js")).unwrap_or_default();
        if handwritten_js != golden_js {
            let culprit = if chumsky_js == golden_js {
                "FRONTEND"
            } else {
                "HARNESS (chumsky tree also diverges from the golden)"
            };
            failures.push(format!(
                "{name} (.js) != golden [{culprit}]: {}",
                first_difference(&golden_js, &handwritten_js)
            ));
        }

        // Assets: for each committed `<name>.<kind>` sidecar, the assembled asset
        // of that kind must match. `.js` is the bundle above, not an asset kind.
        for (kind, content) in &handwritten_assets {
            if kind == "js" {
                continue;
            }
            let golden_path = entry_path.with_extension(kind);
            if !golden_path.is_file() {
                continue;
            }
            let golden = std::fs::read_to_string(&golden_path).unwrap_or_default();
            if *content != golden {
                failures.push(format!(
                    "{name} (.{kind}) != golden: {}",
                    first_difference(&golden, content)
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} corpus program(s) diverged through the new frontend:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
