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
pub mod interpreter;
pub mod lexer;
pub(crate) mod macros;
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
pub use analyzer::{Layer, PackageSpec, Program, Workspace, analyze};
pub use error::Error;
pub use lexer::lexer;
pub use macros::MacroLimits;
pub use manifest::Manifest;
pub use options::{BuildOptions, Preset};
pub use parser::parser;
pub use span::{Span, Spanned};
pub use target::{Backend, Platform, PlatformPattern};
pub use transformer::{JsProgram, transform, transform_to_ast};

use std::path::Path;

use node::{Func, ImportBranch, Node, NodeList};
use target::PlatformPattern as Pattern;

/// Infers a build platform for editor analysis (which has no `--platform`) from a
/// file's top-level imports: a file importing a module from one of `std`'s
/// **browser**-serving layers (e.g. `std::dom`) is a browser file, otherwise Node.
/// This lets the language server analyze a browser client without the cross-platform
/// gate false-flagging it. The module's layer is read from `std`'s layer directory,
/// not a hardcoded list.
fn infer_platform(root: &NodeList, std: &PackageSpec) -> Platform {
    let Some(browser_root) = std
        .layers
        .iter()
        .find(|layer| layer.patterns.iter().any(|p| matches!(p, Pattern::Browser)))
        .map(|layer| layer.root.as_path())
    else {
        return Platform::default();
    };
    // Whether `name` is a module file in the browser layer (`name.vl` or `name/lib.vl`).
    let in_browser_layer = |name: &str| {
        browser_root.join(format!("{name}.vl")).exists()
            || browser_root.join(name).join("lib.vl").exists()
    };
    fn std_child_in_browser(
        branch: &ImportBranch,
        in_browser_layer: &impl Fn(&str) -> bool,
    ) -> bool {
        match branch {
            ImportBranch::Path(module, _, _) => in_browser_layer(module),
            ImportBranch::Set(branches) => branches
                .iter()
                .any(|branch| std_child_in_browser(branch, in_browser_layer)),
        }
    }
    let imports_browser_layer = |branch: &ImportBranch| matches!(branch, ImportBranch::Path("std", _, Some(child)) if std_child_in_browser(child, &in_browser_layer));
    // Imports are block-scoped statements (backlog H2), so scan at every depth —
    // a browser import inside a function body flags the file too.
    fn any_node(nodes: &NodeList, matches: &mut dyn FnMut(&Node) -> bool) -> bool {
        fn walk(node: &Spanned<Node>, matches: &mut dyn FnMut(&Node) -> bool) -> bool {
            if matches(&node.0) {
                return true;
            }
            let mut found = false;
            node.0
                .for_each_child(&mut |child| found = found || walk(child, matches));
            found
        }
        nodes.iter().any(|node| walk(node, matches))
    }
    let references_browser = any_node(root, &mut |node| match node {
        Node::Import(branch) | Node::Use(branch) => imports_browser_layer(branch),
        _ => false,
    });
    if references_browser {
        Platform::Browser
    } else {
        Platform::default()
    }
}

/// One fast lex + parse with zero-size errors: `Some(root)` exactly when the
/// source is completely clean — no lex errors, no parse errors, no recovery.
/// On any failure the caller re-runs the `Rich` pipeline (`lexer()`/`parser()`)
/// to get the real diagnostics; this double-parse trade is the point.
/// Profiling (2026-07-08, the todo example) showed `Rich` bookkeeping — merging
/// failed alternatives, deduplicating expected-token sets, allocating and
/// dropping the reasons — dominating SUCCESSFUL parses, roughly 40% of a whole
/// build's instructions. Clean files (the overwhelming case: every std module,
/// every macro world, every warm re-analysis) now skip all of it, and broken
/// files pay one extra cheap pass before their diagnostics, which reproduce
/// the all-rich pipeline's byte for byte.
pub fn parse_clean(source: &str) -> Option<Spanned<node::NodeList<'_>>> {
    use chumsky::prelude::*;

    let (tokens, lex_errors) = lexer::lexer_with::<extra::Default>()
        .parse(source)
        .into_output_errors();
    if !lex_errors.is_empty() {
        return None;
    }
    let tokens = tokens?;
    let end = source.len();
    let (root, parse_errors) = parser::parser_with::<_, extra::Default>()
        .parse(
            tokens
                .as_slice()
                .map((end..end).into(), |(token, span)| (token, span)),
        )
        .into_output_errors();
    if !parse_errors.is_empty() {
        return None;
    }
    root
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
/// `platform` is the build platform to analyze against — pass `Some` when the
/// front-end knows it (e.g. the language server resolved it from the project's
/// `vilan.toml`), or `None` to infer it from the file's imports.
pub fn analyze_source(
    source: &'static str,
    std: &PackageSpec,
    pkg_root: &Path,
    entry_path: &Path,
    platform: Option<Platform>,
    workspace: &Workspace,
) -> (Option<Program<'static>>, Vec<Error>) {
    use chumsky::prelude::*;

    // Fast path: a clean file parses once with zero-size errors. The rich
    // pipeline below runs only when something is actually wrong (or was
    // recovered), so diagnostics are unchanged — just no longer paid for by
    // every clean keystroke and module.
    let (mut root, mut diagnostics) = if let Some(root) = parse_clean(source) {
        (root, Vec::new())
    } else {
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
        (root, diagnostics)
    };

    // A macro WORLD's entry gets the ambient meta prelude (macro-engine.md
    // §3/§10): the reflection vocabulary binds at file scope. Names the file
    // defines itself are excluded, so an explicit definition shadows the
    // prelude.
    if macros::in_macro_world() {
        // `macro { .. }` blocks survive world blanking verbatim and parse at
        // the world's top level; wrap each into the synthetic zero-argument
        // `fun __macro_block_<n>(): Source` the expansion engine dispatches
        // (macro-engine.md Phase 4). Numbering is source order — the same
        // order registration assigned.
        let mut block_ordinal = 0usize;
        for node in root.0.iter_mut() {
            if matches!(node.0, Node::MacroBlock(_)) {
                let placeholder = std::mem::replace(&mut node.0, Node::Error);
                let Node::MacroBlock(body) = placeholder else {
                    unreachable!("just matched MacroBlock");
                };
                let name: &'static str =
                    Box::leak(macros::block_entry_name(block_ordinal).into_boxed_str());
                block_ordinal += 1;
                let start = node.1.into_range().start;
                let head: Span = (start..start).into();
                node.0 = Node::Func(Func {
                    name: (name, head),
                    is_async: false,
                    external: false,
                    extern_binding: None,
                    must_use: false,
                    rpc: false,
                    trait_only: false,
                    doc_hidden: false,
                    generic_parameters: None,
                    parameters: (Vec::new(), head),
                    return_type: Some(Box::new((Node::Accessor("Source"), head))),
                    borrows: None,
                    body: Some(body),
                });
            }
        }
        let mut defined = std::collections::HashSet::new();
        for (node, _span) in root.0.iter() {
            let function = match node {
                Node::Func(function) => Some(function),
                Node::Export(inner) => match &inner.0 {
                    Node::Func(function) => Some(function),
                    _ => None,
                },
                _ => None,
            };
            if let Some(function) = function {
                defined.insert(function.name.0);
            }
        }
        if let Some(prelude) = macros::world_prelude_nodes(&defined) {
            root.0.splice(0..0, prelude);
        }
    }

    let root = Box::leak(Box::new(root));
    // Use the front-end's resolved platform (e.g. from `vilan.toml`), else infer
    // one from the file's own imports: a file importing the browser DOM layer is a
    // browser file, otherwise Node. This keeps the platform gate from
    // false-flagging valid `std::dom` usage while still catching a genuine
    // cross-platform import (e.g. `std::http` in a file that also reaches for
    // `std::dom`).
    let platform = platform.unwrap_or_else(|| infer_platform(&root.0, std));
    let analyzed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut program = analyze(root, source, std, pkg_root, entry_path, platform, workspace);
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
