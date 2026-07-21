//! The Vilan compiler as a library: lexing, parsing, semantic analysis, and JS
//! code generation. Both the `vilan` CLI and the `vilan-lsp` language server are
//! thin front-ends over this crate.

pub mod analyzer;
pub mod async_infer;
pub mod call_graph;
pub mod const_eval;
pub mod context;
pub mod error;
pub mod formatter;
pub mod id;
pub mod interpreter;
pub mod leak_tally;
pub mod lexer;
pub mod lift;
pub(crate) mod macros;
pub mod manifest;
pub mod node;
pub mod options;
pub mod parser;
pub mod platform_color;
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

/// A targeted hint for a known-confusing PARSE failure shape
/// (diagnostics-standard.md §4 — the worst chumsky messages get labels, the
/// rest wait for the handwritten parser). Today: the `!=` soup — `a!==b`
/// lexes as `!=` then `=`, and the resulting "expected expression" says
/// nothing about the real fix.
pub fn parse_error_hint(source: &str, error_start: usize) -> Option<&'static str> {
    let head = &source[..error_start.min(source.len())];
    let trimmed = head.trim_end();
    if trimmed.ends_with("!=") {
        return Some(
            "if this was postfix `!` before a comparison, the space is required: `a! == b` (`!=` always lexes as not-equals)",
        );
    }
    None
}

/// Renders one parse error as user-facing text — chumsky's format, minus two
/// kinds of noise (diagnostics-standard.md B4):
///
/// - The optional-continuation labels `context clause` and `generic arguments`
///   are grammatically admissible after every type, so chumsky offers them at
///   nearly every type-position failure — where they are never the fix. They
///   are dropped whenever any other expectation remains.
/// - Context entries keep their label (`in expression`) but lose the raw byte
///   offsets (`at 17..46`), which mean nothing to a user; the diagnostic's own
///   span already carries the location.
///
/// Known-confusing failure shapes also gain their [`parse_error_hint`].
pub fn render_parse_error<T: std::fmt::Display>(
    error: &chumsky::error::Rich<'_, T, Span>,
    source: &str,
) -> String {
    use std::fmt::Write;

    let mut message = render_parse_error_reason(error);
    for (label, _span) in error.contexts() {
        write!(message, " in {}", render_parse_pattern(label)).unwrap();
    }
    if let Some(hint) = parse_error_hint(source, error.span().into_range().start) {
        write!(message, " — {hint}").unwrap();
    }
    message
}

/// [`render_parse_error`] without the trailing `in <context>` entries — for
/// renderers that show parse contexts as their own labels (the CLI's ariadne
/// report) rather than inline.
pub fn render_parse_error_reason<T: std::fmt::Display>(
    error: &chumsky::error::Rich<'_, T, Span>,
) -> String {
    use chumsky::error::{RichPattern, RichReason};
    use std::fmt::Write;

    match error.reason() {
        RichReason::Custom(custom) => custom.clone(),
        RichReason::ExpectedFound { .. } => {
            let expected: Vec<String> = {
                let all: Vec<&RichPattern<'_, T>> = error.expected().collect();
                let noise = |candidate: &&RichPattern<'_, T>| {
                    matches!(candidate, RichPattern::Label(label)
                        if *label == "context clause" || *label == "generic arguments")
                };
                let kept: Vec<&RichPattern<'_, T>> = all
                    .iter()
                    .copied()
                    .filter(|candidate| !noise(candidate))
                    .collect();
                if kept.is_empty() { all } else { kept }
                    .into_iter()
                    .map(render_parse_pattern)
                    .collect()
            };
            let found = match error.found() {
                Some(token) => format!("'{token}'"),
                None => "end of input".to_string(),
            };
            let mut message = format!("found {found} expected ");
            match expected.as_slice() {
                [] => message.push_str("something else"),
                [only] => message.push_str(only),
                [first, second] => {
                    write!(message, "{first} or {second}").unwrap();
                }
                many => {
                    for one in &many[..many.len() - 1] {
                        write!(message, "{one}, ").unwrap();
                    }
                    write!(message, "or {}", many[many.len() - 1]).unwrap();
                }
            }
            message
        }
    }
}

/// One expected/context pattern, rendered as chumsky's `Display` renders it
/// (`'token'`, bare label text, `end of input`).
fn render_parse_pattern<T: std::fmt::Display>(
    pattern: &chumsky::error::RichPattern<'_, T>,
) -> String {
    pattern.to_string()
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

/// A process-global, content-addressed cache of clean parses, shared by every
/// compile in the process — the CLI's long-lived `--watch` loop, the language
/// server, the test harness. The key is a hash of the source; the value is the
/// leaked `'static` AST (already lift-rewritten, so callers must not lift again)
/// and its leaked source text. Returns `None` when the source is not perfectly
/// clean, so the caller falls back to its rich-diagnostic pipeline — an erroring
/// file is not the hot path.
///
/// This is the same mechanism [`analyzer::load_package_module`] uses to reuse
/// `std` and package modules, lifted so the **entry** file — the one file the
/// CLI parses directly — shares it too. Across watch rounds an unchanged leg's
/// entry (and every unchanged module) is served from the cache instead of being
/// re-lexed and re-parsed (backlog E12). Keying on content (never mtime) keeps
/// it correct: an edited file hashes differently and is parsed afresh; only
/// byte-identical content is reused. A cache hit returns the identical `'static`
/// pointer it stored, which is how a test proves reuse without timing.
pub fn parse_clean_cached(
    source: &str,
) -> Option<(&'static Spanned<node::NodeList<'static>>, &'static str)> {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<
        Mutex<HashMap<u64, (&'static Spanned<node::NodeList<'static>>, &'static str)>>,
    > = OnceLock::new();
    // Content hashes known NOT to parse clean — so a broken file (an entry
    // mid-edit under `--watch`, say) is leaked and re-parsed once per distinct
    // content, not once per round.
    static BROKEN: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let broken = BROKEN.get_or_init(|| Mutex::new(HashSet::new()));

    let key = content_hash(source);
    if let Some(cached) = cache.lock().unwrap().get(&key) {
        return Some(*cached);
    }
    if broken.lock().unwrap().contains(&key) {
        return None;
    }

    // Cache miss: leak the source so the parsed tree (which borrows it) can live
    // for the whole process, then parse. A non-clean source yields `None` — the
    // caller re-parses it for real diagnostics (leaking the source first mirrors
    // `load_package_module`, whose rich path also reuses the leaked text).
    let leaked: &'static str = Box::leak(source.to_string().into_boxed_str());
    leak_tally::record(leak_tally::LeakSite::ParseCleanCacheText, leaked.len());
    let Some(mut root) = parse_clean(leaked) else {
        broken.lock().unwrap().insert(key);
        return None;
    };
    lift::rewrite_items(&mut root.0);
    let leaked_root: &'static Spanned<node::NodeList<'static>> = Box::leak(Box::new(root));
    leak_tally::record(
        leak_tally::LeakSite::ParseCleanCacheAst,
        std::mem::size_of_val(leaked_root),
    );
    cache.lock().unwrap().insert(key, (leaked_root, leaked));
    Some((leaked_root, leaked))
}

/// The content hash the compiler keys its caches and source fingerprints on —
/// one definition, so the parse cache and the watch loop's per-leg source
/// verification can never disagree about what "same content" means.
pub fn content_hash(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
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
                note: None,
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
            note: None,
            span: *error.span(),
            msg: render_parse_error(error, source),
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
                leak_tally::record(leak_tally::LeakSite::MacroBlockEntryName, name.len());
                block_ordinal += 1;
                let start = node.1.into_range().start;
                let head: Span = (start..start).into();
                node.0 = Node::Func(Func {
                    name: (name, head),
                    is_async: false,
                    external: false,
                    extern_binding: None,
                    must_use: false,
                    platform_fence: Vec::new(),
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

    // Bare-`?` marks become lift regions before the tree freezes
    // (expression-lifting.md) — the formatter parses separately and keeps
    // raw trees, so source text prints back verbatim.
    lift::rewrite_items(&mut root.0);
    let root = Box::leak(Box::new(root));
    leak_tally::record(leak_tally::LeakSite::EntryAst, std::mem::size_of_val(root));
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
        // `drop` must be synchronous (destruction.md §5): reject an async drop
        // body now that `async_functions` is settled — an awaiting body is async
        // only by inference, so this cannot run inside `analyze`.
        analyzer::check_async_drops(&mut program);
        // And teardown must be context-free (destruction.md §8): a `drop` body
        // whose call sites (scope exits) can thread no context is rejected. Runs
        // after `thread_contexts` fills `context_dependent_functions`.
        analyzer::check_context_drops(&mut program);
        platform_color::check(&mut program, platform);
        // The const pass (proposal/const-eval.md): evaluate `const`-marked
        // expressions in dependency order; results serialize in place at
        // transform time, failures are ordinary diagnostics. Runs here so
        // `check`, the LSP, and every build path agree.
        let (const_results, const_assets, const_errors) =
            const_eval::evaluate(&program, &options::BuildOptions::default());
        program.const_results = const_results;
        program.const_assets = const_assets;
        program.diagnostics.extend(const_errors);
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
