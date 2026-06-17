//! The Vilan compiler as a library: lexing, parsing, semantic analysis, and JS
//! code generation. Both the `vilan` CLI and the `vilan-lsp` language server are
//! thin front-ends over this crate.

pub mod analyzer;
pub mod async_infer;
pub mod call_graph;
pub mod context;
pub mod error;
pub mod id;
pub mod lexer;
pub mod node;
pub mod parser;
pub mod span;
pub mod token;
pub mod transformer;
pub mod type_;
pub mod util;

// The common pipeline + core types, re-exported for convenience.
pub use analyzer::{Program, analyze};
pub use error::Error;
pub use lexer::lexer;
pub use parser::parser;
pub use span::{Span, Spanned};
pub use transformer::transform;

use std::path::Path;

/// Lex, parse, and fully analyze a source string. The source must already be
/// leaked to `'static` so the returned `Program` (which borrows it) can outlive
/// this call — the front-end that owns the document lifecycle does the leak.
///
/// Returns the analyzed program — present whenever parsing produced a tree, even
/// a partial one recovered from syntax errors — together with every diagnostic
/// (lexer, parser, and analyzer) for the entry file. Analysis is wrapped so a
/// panic on malformed input degrades to "no program" rather than taking the
/// process down, which matters when an editor analyzes on every keystroke.
pub fn analyze_source(
    source: &'static str,
    std_root: &Path,
    entry_path: &Path,
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
    let analyzed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut program = analyze(root, std_root, entry_path);
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
