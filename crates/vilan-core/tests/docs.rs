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

/// A fenced block in progress: how it was tagged, where it opened, and — the
/// crux of the indent handling — the column its opening fence sat at.
struct OpenFence {
    platform: Platform,
    compile: bool,
    opened_at: usize,
    indent: usize,
    body: String,
}

/// The count of leading ASCII spaces on `line` — the fence-indent measure. Tabs
/// are deliberately NOT counted: our pages indent fenced blocks with spaces
/// (mdBook renders through pulldown-cmark, a CommonMark implementation, and the
/// book uses space indentation throughout), so a tab-indented fence is out of
/// scope and simply reads as column 0.
fn leading_spaces(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

/// Strip up to `indent` leading spaces from `line` — CommonMark's fenced-code
/// dedent (§4.5): a line indented at least `indent` loses exactly `indent`
/// spaces; a line indented less loses only what it has (never more). Slices the
/// input, no allocation.
fn dedent(line: &str, indent: usize) -> &str {
    &line[leading_spaces(line).min(indent)..]
}

/// Pull the compilable examples out of one markdown file, tracking the nearest
/// heading for failure reports.
///
/// NOTE — keep in sync with `parse_differential.rs`'s `collect_doc_examples`,
/// which carries a copy of this fence logic (test targets cannot import one
/// another). The two MUST agree on which fences are examples and on their
/// dedented bodies; a change to the fence rules here belongs there too.
fn extract_examples(path: &Path) -> Vec<Example> {
    let text = std::fs::read_to_string(path).expect("read doc file");
    extract_examples_from(&text, path)
}

/// The pure core of [`extract_examples`], over in-memory text so the fence rules
/// — indent tracking, same-indent close, and dedent — can be unit-tested without
/// touching the filesystem.
fn extract_examples_from(text: &str, path: &Path) -> Vec<Example> {
    let mut examples = Vec::new();
    let mut heading = String::from("(top)");
    let mut fence: Option<OpenFence> = None;
    for (index, line) in text.lines().enumerate() {
        match &mut fence {
            Some(open) => {
                // CommonMark closes a fenced block on a fence line at the SAME
                // indentation as its opener; a ``` at any other indent is body
                // (e.g. a fence-like line inside the code). This is what stops an
                // indented fence from running past its close into the prose. The
                // indent check is first so the slice below is always in bounds.
                if leading_spaces(line) == open.indent && line[open.indent..].trim_end() == "```" {
                    if open.compile {
                        examples.push(Example {
                            file: path.to_path_buf(),
                            heading: heading.clone(),
                            line: open.opened_at + 1,
                            source: std::mem::take(&mut open.body),
                            platform: open.platform,
                        });
                    }
                    fence = None;
                } else {
                    open.body.push_str(dedent(line, open.indent));
                    open.body.push('\n');
                }
            }
            None => {
                let indent = leading_spaces(line);
                if let Some(info) = line[indent..].strip_prefix("```") {
                    let info = info.trim();
                    let (platform, compile) = match info {
                        "vilan" | "vilan,norun" => (Platform::default(), true),
                        "vilan,browser" => (Platform::Browser, true),
                        "vilan,fragment" => (Platform::default(), false),
                        // Non-vilan fences (sh, toml, text, js …) are prose.
                        _ => (Platform::default(), false),
                    };
                    fence = Some(OpenFence {
                        platform,
                        compile,
                        opened_at: index,
                        indent,
                        body: String::new(),
                    });
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
    // The repo-root README carries examples too; hold it to the same gate.
    let readme = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    if readme.is_file() {
        files.push(readme);
    }
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

/// Unit pins for the fence extractor itself (the indented-fence hardening, D3).
/// These exercise `extract_examples_from` on hand-built markdown so the fence
/// rules are proven directly, independent of what the real docs happen to hold.
/// The mirror in `parse_differential.rs` carries its own copy of these.
#[cfg(test)]
mod extract_pins {
    use super::*;

    /// Build a markdown document from explicit lines (each verbatim, so leading
    /// spaces are exactly as written) with a trailing newline, then extract.
    fn examples(lines: &[&str]) -> Vec<Example> {
        let mut text = lines.join("\n");
        text.push('\n');
        extract_examples_from(&text, Path::new("test.md"))
    }

    #[test]
    fn flush_left_fence_is_unchanged() {
        // The pre-existing behavior: a column-0 fence, body verbatim, heading
        // tracked. Every current doc example is this shape, so it must not move.
        let got = examples(&[
            "# Signals",
            "",
            "```vilan",
            "let x = 1;",
            "```",
            "",
            "after",
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, "let x = 1;\n");
        assert_eq!(got[0].heading, "Signals");
        assert_eq!(got[0].line, 3);
    }

    #[test]
    fn bullet_indented_fence_extracts_and_dedents() {
        // A fence indented two columns under a bullet: it is found, and its body
        // is dedented by the fence's own indent so it compiles as flush source.
        let got = examples(&[
            "- A bullet:",
            "",
            "  ```vilan",
            "  let x = 1;",
            "  ```",
            "",
            "- Next bullet",
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, "let x = 1;\n");
    }

    #[test]
    fn nested_deeper_body_lines_keep_relative_indent() {
        // Only the fence's columns come off; a line indented deeper than the
        // fence keeps the extra (the body's own structure survives). This is the
        // real shape: a 2-space bullet indent outside, deeper indent inside.
        let got = examples(&[
            "  ```vilan",
            "  fun main() {",
            "      let x = 1;",
            "  }",
            "  ```",
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, "fun main() {\n    let x = 1;\n}\n");
    }

    #[test]
    fn an_indented_fence_does_not_swallow_following_prose() {
        // The D3 bug itself: the old close test required a column-0 ```, so an
        // indented fence ran past its real close into the prose and the next
        // fence. Both examples must come back, the prose must stay prose.
        let got = examples(&[
            "- Bullet:",
            "",
            "  ```vilan",
            "  fun first() {}",
            "  ```",
            "",
            "This prose must stay prose.",
            "",
            "```vilan",
            "fun second() {}",
            "```",
        ]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].source, "fun first() {}\n");
        assert_eq!(got[1].source, "fun second() {}\n");
        assert!(got.iter().all(|example| !example.source.contains("prose")));
    }

    #[test]
    fn a_fence_like_line_inside_the_body_at_a_different_indent_does_not_close() {
        // A ``` deeper than the opener is body, not the closer — even though its
        // trimmed text is exactly "```". Only the matching-indent ``` closes, so
        // the block stays open across the impostor.
        let got = examples(&["  ```vilan", "  outer", "    ```", "  more outer", "  ```"]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, "outer\n  ```\nmore outer\n");
    }

    #[test]
    fn an_indented_fragment_closes_and_does_not_swallow_the_next_example() {
        // A fragment is not emitted, but its indented fence must still close on
        // the matching indent so a real example after it survives — the exact
        // dev-docs shape (an indented signature fragment, then a real example)
        // that first hit this bug.
        let got = examples(&[
            "- A note:",
            "",
            "  ```vilan,fragment",
            "  signal.map(f)   // just a signature",
            "  ```",
            "",
            "```vilan",
            "fun after() {}",
            "```",
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source, "fun after() {}\n");
    }
}
