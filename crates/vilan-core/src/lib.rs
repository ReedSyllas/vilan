//! The Vilan compiler as a library: lexing, parsing, semantic analysis, and JS
//! code generation. Both the `vilan` CLI and the `vilan-lsp` language server are
//! thin front-ends over this crate.

pub mod analyzer;
pub mod async_infer;
pub mod call_graph;
pub mod context;
pub mod error;
pub mod formatter;
pub mod id;
pub mod lexer;
pub mod manifest;
pub mod node;
pub mod options;
pub mod parser;
pub mod span;
pub mod target;
pub mod token;
pub mod transformer;
pub mod type_;
pub mod util;

// The common pipeline + core types, re-exported for convenience.
pub use analyzer::{PackageSpec, Program, Workspace, analyze};
pub use error::Error;
pub use lexer::lexer;
pub use manifest::Manifest;
pub use options::{BuildOptions, Preset};
pub use parser::parser;
pub use span::{Span, Spanned};
pub use target::Target;
pub use transformer::transform;

use std::path::Path;

use node::{ImportBranch, Node, NodeList};
use target::Platform;

/// Infers a compilation target for editor analysis (which has no `--target`) from
/// a file's top-level imports: a file importing the browser DOM layer
/// (`std::dom`) is a browser file, otherwise Node. This lets the language server
/// analyze a `std::dom` client without the platform gate false-flagging it.
fn infer_target(root: &NodeList) -> Target {
    fn std_child_is_browser(branch: &ImportBranch) -> bool {
        match branch {
            ImportBranch::Path(module, _, _) => {
                Platform::of_std_module(module) == Platform::Browser
            }
            ImportBranch::Set(branches) => branches.iter().any(std_child_is_browser),
        }
    }
    let imports_browser_layer = |branch: &ImportBranch| matches!(branch, ImportBranch::Path("std", _, Some(child)) if std_child_is_browser(child));
    let references_browser = root.iter().any(|(node, _)| match node {
        Node::Import(branch) | Node::Use(branch) => imports_browser_layer(branch),
        _ => false,
    });
    if references_browser {
        Target::Browser
    } else {
        Target::Node
    }
}

/// Lex, parse, and fully analyze a source string. The source must already be
/// leaked to `'static` so the returned `Program` (which borrows it) can outlive
/// this call — the front-end that owns the document lifecycle does the leak.
///
/// Returns the analyzed program — present whenever parsing produced a tree, even
/// a partial one recovered from syntax errors — together with every diagnostic
/// (lexer, parser, and analyzer) for the entry file. Analysis is wrapped so a
/// panic on malformed input degrades to "no program" rather than taking the
/// process down, which matters when an editor analyzes on every keystroke.
/// `target` is the build target to analyze against — pass `Some` when the
/// front-end knows it (e.g. the language server resolved it from the project's
/// `vilan.toml`), or `None` to infer it from the file's imports.
pub fn analyze_source(
    source: &'static str,
    std_root: &Path,
    pkg_root: &Path,
    entry_path: &Path,
    target: Option<Target>,
    workspace: &Workspace,
) -> (Option<Program<'static>>, Vec<Error>) {
    use chumsky::prelude::*;

    let (tokens, lex_errors) = lexer().parse(source).into_output_errors();
    let mut diagnostics: Vec<Error> = lex_errors
        .iter()
        .map(|error| Error {
            span: *error.span(),
            msg: error.to_string(),
        })
        .collect();
    let Some(tokens) = tokens else {
        return (None, diagnostics);
    };

    let (ast, parse_errors) = parser()
        .map_with(|ast, extra| (ast, extra.span()))
        .parse(
            tokens
                .as_slice()
                .map((source.len()..source.len()).into(), |(token, span)| {
                    (token, span)
                }),
        )
        .into_output_errors();
    diagnostics.extend(parse_errors.iter().map(|error| Error {
        span: *error.span(),
        msg: error.to_string(),
    }));
    let Some((root, _file_span)) = ast else {
        return (None, diagnostics);
    };

    let root = Box::leak(Box::new(root));
    // Use the front-end's resolved target (e.g. from `vilan.toml`), else infer one
    // from the file's own imports: a file importing the browser DOM layer is a
    // browser file, otherwise Node. This keeps the platform gate from
    // false-flagging valid `std::dom` usage while still catching a genuine
    // cross-target import (e.g. `std::http` in a file that also reaches for
    // `std::dom`).
    let target = target.unwrap_or_else(|| infer_target(&root.0));
    let analyzed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut program = analyze(root, std_root, pkg_root, entry_path, target, workspace);
        context::thread_contexts(&mut program);
        async_infer::infer(&mut program);
        program
    }));
    match analyzed {
        Ok(program) => {
            diagnostics.extend(program.diagnostics.iter().cloned());
            (Some(program), diagnostics)
        }
        Err(_) => (None, diagnostics),
    }
}
