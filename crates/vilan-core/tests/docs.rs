//! The docs compile gate (proposal/documentation.md §4): every fenced code
//! block in `vilan/docs/**/*.md` is extracted and compiled through the real
//! pipeline, so a std or language change that breaks a documented example
//! fails the suite until the doc is updated.
//!
//! Fence tags:
//! - ```vilan           — complete program, node target
//! - ```vilan,browser   — complete program, browser target
//! - ```vilan,norun     — compiles (node) but isn't runnable standalone
//! - ```vilan,fragment  — NOT compiled (a signature, a diff, a deliberate error)
//!
//! Failures report `file — nearest heading` so a broken example is a one-jump
//! fix.

use std::path::{Path, PathBuf};

use vilan_core::{BuildOptions, PackageSpec, Platform, Workspace, analyze_source, transform};

fn std_spec() -> PackageSpec {
    vilan_core::manifest::resolve_std(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"),
    )
}

fn docs_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/docs")
}

/// One extracted example: where it came from, what it contains, and how the
/// fence asked for it to be compiled.
struct Example {
    file: PathBuf,
    heading: String,
    line: usize,
    source: String,
    platform: Platform,
}

fn collect_markdown_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            // The rendered-site build output is not content.
            if path.file_name().is_some_and(|name| name == "book") {
                continue;
            }
            collect_markdown_files(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            into.push(path);
        }
    }
}

/// Pull the compilable examples out of one markdown file, tracking the nearest
/// heading for failure reports.
fn extract_examples(path: &Path) -> Vec<Example> {
    let text = std::fs::read_to_string(path).expect("read doc file");
    let mut examples = Vec::new();
    let mut heading = String::from("(top)");
    let mut fence: Option<(Platform, bool, usize, String)> = None; // (platform, compile?, line, body)
    for (index, line) in text.lines().enumerate() {
        match &mut fence {
            Some((platform, compile, opened_at, body)) => {
                if line.trim_end() == "```" {
                    if *compile {
                        examples.push(Example {
                            file: path.to_path_buf(),
                            heading: heading.clone(),
                            line: *opened_at + 1,
                            source: body.clone(),
                            platform: *platform,
                        });
                    }
                    fence = None;
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
            None => {
                if let Some(info) = line.trim_start().strip_prefix("```") {
                    let info = info.trim();
                    let (platform, compile) = match info {
                        "vilan" | "vilan,norun" => (Platform::default(), true),
                        "vilan,browser" => (Platform::Browser, true),
                        "vilan,fragment" => (Platform::default(), false),
                        // Non-vilan fences (sh, toml, text, js …) are prose.
                        _ => (Platform::default(), false),
                    };
                    fence = Some((platform, compile, index, String::new()));
                } else if let Some(title) = line.strip_prefix('#') {
                    heading = title.trim_start_matches('#').trim().to_string();
                }
            }
        }
    }
    assert!(fence.is_none(), "unclosed code fence in {}", path.display());
    examples
}

/// Compile one example through the full pipeline on a large-stack worker
/// (mirroring the CLI and `inference.rs`); a panic becomes an error rather
/// than aborting the suite.
fn compile(source: &str, platform: Platform) -> Result<(), Vec<String>> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let leaked: &'static str = Box::leak(source.into_boxed_str());
                let (program, errors) = analyze_source(
                    leaked,
                    &std_spec(),
                    Path::new("."),
                    Path::new("doc-example.vl"),
                    Some(platform),
                    &Workspace::default(),
                );
                match program {
                    Some(program) if errors.is_empty() => {
                        transform(&program, &BuildOptions::default())
                            .map(|_| ())
                            .map_err(|error| vec![error.msg])
                    }
                    _ => Err(errors.into_iter().map(|error| error.msg).collect()),
                }
            }))
            .unwrap_or_else(|_| Err(vec!["compiler panicked".to_string()]))
        })
        .expect("spawn worker")
        .join()
        .unwrap_or_else(|_| {
            Err(vec![
                "compiler thread aborted (likely a stack overflow)".to_string(),
            ])
        })
}

#[test]
fn every_doc_example_compiles() {
    let mut files = Vec::new();
    collect_markdown_files(&docs_root(), &mut files);
    assert!(
        !files.is_empty(),
        "no markdown files under {} — docs missing?",
        docs_root().display()
    );
    let mut failures = Vec::new();
    let mut compiled = 0;
    for file in &files {
        for example in extract_examples(file) {
            compiled += 1;
            if let Err(errors) = compile(&example.source, example.platform) {
                failures.push(format!(
                    "{}:{} — {} — {:?}",
                    example.file.display(),
                    example.line,
                    example.heading,
                    errors
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} doc examples failed to compile:\n{}",
        failures.len(),
        compiled,
        failures.join("\n")
    );
    // The gate is only meaningful while examples exist.
    assert!(compiled > 0, "no compilable examples found in docs/");
}

/// The rendered site's sidebar (proposal/docs-site.md §5): every docs page
/// appears in SUMMARY.md, and every SUMMARY.md entry points at a real file —
/// a page added without nav wiring, or moved without fixing it, fails here.
#[test]
fn the_sidebar_covers_every_page() {
    let root = docs_root();
    let summary_path = root.join("SUMMARY.md");
    let summary = std::fs::read_to_string(&summary_path).expect("read SUMMARY.md");

    let mut listed = std::collections::BTreeSet::new();
    for capture in summary.split("](").skip(1) {
        let Some(target) = capture.split(')').next() else {
            continue;
        };
        if target.ends_with(".md") {
            listed.insert(target.to_string());
        }
    }

    let mut files = Vec::new();
    collect_markdown_files(&root, &mut files);
    let mut missing_from_summary = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(&root)
            .expect("docs file under docs root")
            .to_string_lossy()
            .replace('\\', "/");
        // The build output and the sidebar itself are not pages.
        if relative.starts_with("book/") || relative == "SUMMARY.md" {
            continue;
        }
        if !listed.remove(&relative) {
            missing_from_summary.push(relative);
        }
    }
    assert!(
        missing_from_summary.is_empty(),
        "pages missing from SUMMARY.md (add them to the sidebar): {missing_from_summary:?}"
    );
    assert!(
        listed.is_empty(),
        "SUMMARY.md entries with no file behind them: {listed:?}"
    );
}
