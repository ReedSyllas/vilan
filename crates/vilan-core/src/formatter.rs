//! The source formatter behind `vilan fmt`: it reparses a file and reprints the
//! AST in canonical style (tab indentation, normalized spacing and blank lines),
//! reattaching the comments the lexer drops as trivia.
//!
//! Safety: reprinting from the AST could, given a bug, silently change a program.
//! So `format` re-lexes its own output and checks the token stream matches the
//! input's (ignoring spans, whitespace, and comments); on any mismatch it returns
//! the source unchanged rather than risk corrupting the file.

use crate::node::{
    BinaryOp, Convention, ExternBinding, Func, GenericParameters, ImportBranch, Node, NodeIfBranch,
    NodeList, Pattern,
};
use crate::span::{Span, Spanned};
use crate::token::Token;

/// Extracts `//` line comments from `source` as `(span, text)` in source order.
/// `text` keeps the leading `//` and is trimmed of trailing whitespace. String
/// literals are skipped so a `//` inside a string isn't taken for a comment.
pub fn extract_comments(source: &str) -> Vec<(Span, &str)> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                let text = source[start..index].trim_end();
                comments.push((Span::new((), start..start + text.len()), text));
            }
            _ => index += 1,
        }
    }
    comments
}

/// The lexer's token stream with spans stripped — the formatter's notion of "the
/// same code", used to check a reprint didn't change anything but trivia.
fn code_tokens(source: &str) -> Option<Vec<Token<'_>>> {
    let (tokens, lex_errors) = crate::lexing::tokenize(source);
    lex_errors
        .is_empty()
        .then(|| tokens.into_iter().map(|(token, _)| token).collect())
}

/// The formatter's token-level canonicalization, used to check a reprint changed
/// nothing but trivia and the canonical import order. Two order-insensitivities
/// are folded in so the safety check accepts them: insignificant trailing commas
/// (dropped), and the canonical ordering of a top-level import run (see the
/// canonical-import-order section below). Everything else must match token for
/// token, so the net still catches every *other* reordering.
fn normalize(tokens: Vec<Token<'_>>) -> Vec<Token<'_>> {
    sort_import_runs(&drop_trailing_commas(tokens))
}

/// Drops every comma that sits immediately before a closing `}`, `)`, or `]`.
/// Vilan treats such a trailing comma as insignificant (tuples need two or more
/// elements, so there is no `(a,)` one-tuple to confuse it with), which lets the
/// safety check accept the formatter normalizing trailing commas in or out.
fn drop_trailing_commas(tokens: Vec<Token<'_>>) -> Vec<Token<'_>> {
    let mut result: Vec<Token<'_>> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if matches!(
            token,
            Token::Ctrl('}') | Token::Ctrl(')') | Token::Ctrl(']')
        ) {
            while let Some(Token::Ctrl(',')) = result.last() {
                result.pop();
            }
        }
        result.push(token);
    }
    result
}

// --- Canonical import order --------------------------------------------------
//
// `vilan fmt` canonicalizes the order of a file's top-level `import`/`use`
// statements (the pruning of unused imports is the editor's job, not the
// formatter's). The rule, defined once here and applied by both the printer
// (which reorders AST items) and `normalize` (which reorders token statements)
// through the shared [`import_sort_key`], is:
//
//   * A *run* is a maximal span of consecutive top-level import/use statements.
//     Blank lines between them do not break a run — they coalesce, and the run
//     reprints as one block. A standalone (own-line) comment *does* break a run
//     (it pins a deliberate grouping), so imports never reorder across it; a
//     trailing same-line comment travels with its own import.
//   * Within a run, statements sort by: kind (`import` before `use`; an
//     `export import`/`export use` re-export sorts as a plain import/use — the
//     `export` prefix does not change grouping), then root namespace (`std`
//     first, dependency packages alphabetically, `pkg` last), then the full
//     `::`-separated path compared case-sensitively segment by segment.
//   * A brace-set import (`import std::x::{ b, a }`) sorts its inner branch list
//     the same way (`{ a, b }`), recursively.
//   * Only *top-level* runs are touched. Block-scoped imports (inside
//     `fn`/`impl`/`mod` bodies — backlog H2) are deliberate placements and are
//     left exactly as written, order and brace sets both.
//
// `normalize` applies the same canonicalization to *both* sides of the safety
// check, so the check passes whatever order the printer emits (its job is only
// to confirm no import was dropped or corrupted and no *other* code moved),
// while the printer's own tested logic is what fixes the visible order.

/// `import` vs `use` for the canonical order — imports sort before uses. The
/// `export` re-export prefix does not participate.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ImportKind {
    Import,
    Use,
}

/// The root-namespace rank: `std` first, then dependency packages ordered by
/// name, then `pkg` (the current package), then a bare brace-set import with no
/// leading namespace (`import { a, b }`, rare) last.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum RootRank {
    Std,
    Dependency(String),
    Pkg,
    Unrooted,
}

/// A `::`-separated import path reduced to an order-only comparable form: names
/// compare case-sensitively segment by segment, a shorter path sorts before a
/// longer one extending it (`a` before `a::b`, via `End` < `Path`), and a brace
/// set's branches are pre-sorted so the whole set compares canonically.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum BranchKey {
    End,
    Path(String, Box<BranchKey>),
    Set(Vec<BranchKey>),
}

/// The full sort key for one top-level import/use statement — the single
/// definition of the canonical order, shared by the printer and `normalize` so
/// the two cannot disagree. Ordered by kind, then root namespace, then path.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ImportSortKey {
    kind: ImportKind,
    root: RootRank,
    rest: BranchKey,
}

/// A parsed import path — `ImportBranch` without the source spans. Both the AST
/// (via [`branch_from_ast`]) and the token stream (via [`parse_token_branch`])
/// reduce to this shape, from which the shared key and the canonical token
/// re-emission are derived.
enum TokenBranch<'src> {
    Path(&'src str, Option<Box<TokenBranch<'src>>>),
    Set(Vec<TokenBranch<'src>>),
}

/// Drops the spans from an `ImportBranch`, giving the span-free [`TokenBranch`]
/// the shared key operates on.
fn branch_from_ast<'src>(branch: &ImportBranch<'src>) -> TokenBranch<'src> {
    match branch {
        ImportBranch::Path(name, _, child) => TokenBranch::Path(
            name,
            child.as_ref().map(|child| Box::new(branch_from_ast(child))),
        ),
        ImportBranch::Set(branches) => {
            TokenBranch::Set(branches.iter().map(branch_from_ast).collect())
        }
    }
}

/// The order key for one import path — brace sets are sorted internally so equal
/// paths, whatever their source branch order, produce equal keys.
fn branch_key(branch: &TokenBranch<'_>) -> BranchKey {
    match branch {
        TokenBranch::Path(name, None) => {
            BranchKey::Path((*name).to_string(), Box::new(BranchKey::End))
        }
        TokenBranch::Path(name, Some(child)) => {
            BranchKey::Path((*name).to_string(), Box::new(branch_key(child)))
        }
        TokenBranch::Set(branches) => {
            let mut keys: Vec<BranchKey> = branches.iter().map(branch_key).collect();
            keys.sort();
            BranchKey::Set(keys)
        }
    }
}

/// The canonical sort key for an import/use of `kind` importing `branch`:
/// root-namespace rank first, then the path after the root.
fn import_sort_key(kind: ImportKind, branch: &TokenBranch<'_>) -> ImportSortKey {
    let (root, rest) = match branch {
        TokenBranch::Path(name, child) => {
            let root = match *name {
                "std" => RootRank::Std,
                "pkg" => RootRank::Pkg,
                other => RootRank::Dependency(other.to_string()),
            };
            let rest = match child {
                Some(child) => branch_key(child),
                None => BranchKey::End,
            };
            (root, rest)
        }
        TokenBranch::Set(_) => (RootRank::Unrooted, branch_key(branch)),
    };
    ImportSortKey { kind, root, rest }
}

/// If `node` is an import-like item — `import`/`use`, or an `export import` /
/// `export use` re-export — returns its kind and imported path (the `export`
/// prefix does not change the kind). `None` for any other item, which breaks a
/// run.
fn import_kind_and_branch<'node, 'src>(
    node: &'node Node<'src>,
) -> Option<(ImportKind, &'node ImportBranch<'src>)> {
    match node {
        Node::Import(branch) => Some((ImportKind::Import, branch)),
        Node::Use(branch) => Some((ImportKind::Use, branch)),
        Node::Export(inner) => import_kind_and_branch(&inner.0),
        _ => None,
    }
}

/// The canonical key of an import-like `node` (panics if it is not one — callers
/// gate on [`import_kind_and_branch`] first).
fn node_import_key(node: &Node<'_>) -> ImportSortKey {
    let (kind, branch) =
        import_kind_and_branch(node).expect("node_import_key on a non-import item");
    import_sort_key(kind, &branch_from_ast(branch))
}

/// Whether the tokens at `index` begin a top-level import/use statement —
/// `import …`, `use …`, or `export import …` / `export use …`.
fn starts_import(tokens: &[Token<'_>], index: usize) -> bool {
    match tokens.get(index) {
        Some(Token::Import | Token::Use) => true,
        Some(Token::Export) => {
            matches!(tokens.get(index + 1), Some(Token::Import | Token::Use))
        }
        _ => false,
    }
}

/// The path-segment name at `index`, mirroring the parser's `eat_name` (an
/// identifier, or the `true`/`false` literals treated as names).
fn token_name<'src>(tokens: &[Token<'src>], index: usize) -> Option<&'src str> {
    match tokens.get(index) {
        Some(&Token::Ident(name)) => Some(name),
        Some(&Token::Bool(true)) => Some("true"),
        Some(&Token::Bool(false)) => Some("false"),
        _ => None,
    }
}

/// Parses the `::`-separated import path beginning at `index` (mirroring the
/// parser's `parse_namespace_path`: a name-headed path is tried before a brace
/// set), returning the branch and the index just past it, or `None` if the
/// tokens do not match the import-path grammar.
fn parse_token_branch<'src>(
    tokens: &[Token<'src>],
    index: usize,
) -> Option<(TokenBranch<'src>, usize)> {
    if let Some(name) = token_name(tokens, index) {
        let mut next = index + 1;
        let continuation = if tokens.get(next) == Some(&Token::Op("::")) {
            let (child, after) = parse_token_branch(tokens, next + 1)?;
            next = after;
            Some(Box::new(child))
        } else {
            None
        };
        Some((TokenBranch::Path(name, continuation), next))
    } else if tokens.get(index) == Some(&Token::Ctrl('{')) {
        let mut branches = Vec::new();
        let mut next = index + 1;
        // An empty set `{}` closes immediately; otherwise each element is a
        // name-headed single path, comma-separated, allow-trailing.
        while tokens.get(next) != Some(&Token::Ctrl('}')) {
            let name = token_name(tokens, next)?;
            let mut after = next + 1;
            let continuation = if tokens.get(after) == Some(&Token::Op("::")) {
                let (child, past) = parse_token_branch(tokens, after + 1)?;
                after = past;
                Some(Box::new(child))
            } else {
                None
            };
            branches.push(TokenBranch::Path(name, continuation));
            next = after;
            match tokens.get(next) {
                Some(Token::Ctrl(',')) => next += 1,
                Some(Token::Ctrl('}')) => break,
                _ => return None,
            }
        }
        Some((TokenBranch::Set(branches), next + 1))
    } else {
        None
    }
}

/// Parses one import/use statement beginning at `index` into its kind, whether
/// it is an `export` re-export, its path, and the index past its `;` — or `None`
/// if the tokens do not match the import grammar (leaving the run unsorted, a
/// safe no-op). Callers gate on [`starts_import`] first.
fn parse_import_statement<'src>(
    tokens: &[Token<'src>],
    index: usize,
) -> Option<(ImportKind, bool, TokenBranch<'src>, usize)> {
    let mut next = index;
    let export = tokens.get(next) == Some(&Token::Export);
    if export {
        next += 1;
    }
    let kind = match tokens.get(next) {
        Some(Token::Import) => ImportKind::Import,
        Some(Token::Use) => ImportKind::Use,
        _ => return None,
    };
    next += 1;
    let (branch, after) = parse_token_branch(tokens, next)?;
    next = after;
    if tokens.get(next) != Some(&Token::Ctrl(';')) {
        return None;
    }
    Some((kind, export, branch, next + 1))
}

/// Appends the canonical token form of an import path, brace sets sorted.
fn emit_branch_tokens<'src>(branch: &TokenBranch<'src>, out: &mut Vec<Token<'src>>) {
    match branch {
        TokenBranch::Path(name, child) => {
            out.push(Token::Ident(name));
            if let Some(child) = child {
                out.push(Token::Op("::"));
                emit_branch_tokens(child, out);
            }
        }
        TokenBranch::Set(branches) => {
            out.push(Token::Ctrl('{'));
            let mut order: Vec<&TokenBranch<'src>> = branches.iter().collect();
            order.sort_by_cached_key(|branch| branch_key(branch));
            for (position, child) in order.iter().enumerate() {
                if position > 0 {
                    out.push(Token::Ctrl(','));
                }
                emit_branch_tokens(child, out);
            }
            out.push(Token::Ctrl('}'));
        }
    }
}

/// Reorders each contiguous run of top-level (brace-depth-zero) import/use
/// statements into the canonical order, re-emitting each in a canonical token
/// form (brace sets sorted) so that a source run and the printer's reordered
/// reprint reduce to the same token sequence. Statements inside a block
/// (`fn`/`impl`/`mod` bodies — brace depth ≥ 1) and every non-import token keep
/// their positions, so the safety net still catches every other reordering.
// `pub` (doc-hidden) only so the external corpus tripwire in
// `tests/parse_differential.rs` mirrors the net's import canonicalization through
// this ONE implementation rather than a divergent copy — the "cannot disagree"
// guarantee. Not part of the supported API.
#[doc(hidden)]
pub fn sort_import_runs<'src>(tokens: &[Token<'src>]) -> Vec<Token<'src>> {
    let mut result = Vec::with_capacity(tokens.len());
    let mut depth: i32 = 0;
    let mut index = 0;
    while index < tokens.len() {
        if depth == 0 && starts_import(tokens, index) {
            // Parse the maximal run of consecutive import statements. Each
            // statement consumes its own brace set, so depth stays 0 across it.
            let mut statements: Vec<(ImportSortKey, ImportKind, bool, TokenBranch<'src>)> =
                Vec::new();
            let mut cursor = index;
            let mut parsed_cleanly = true;
            while cursor < tokens.len() && starts_import(tokens, cursor) {
                match parse_import_statement(tokens, cursor) {
                    Some((kind, export, branch, next)) => {
                        let key = import_sort_key(kind, &branch);
                        statements.push((key, kind, export, branch));
                        cursor = next;
                    }
                    None => {
                        parsed_cleanly = false;
                        break;
                    }
                }
            }
            if parsed_cleanly && !statements.is_empty() {
                statements.sort_by(|left, right| left.0.cmp(&right.0));
                for (_, kind, export, branch) in &statements {
                    if *export {
                        result.push(Token::Export);
                    }
                    result.push(match kind {
                        ImportKind::Import => Token::Import,
                        ImportKind::Use => Token::Use,
                    });
                    emit_branch_tokens(branch, &mut result);
                    result.push(Token::Ctrl(';'));
                }
                index = cursor;
                continue;
            }
            // A parse failure (never expected for a cleanly-parsed source) falls
            // through to the raw passthrough below — a safe no-op.
        }
        match &tokens[index] {
            Token::Ctrl('{') | Token::Ctrl('(') | Token::Ctrl('[') => depth += 1,
            Token::Ctrl('}') | Token::Ctrl(')') | Token::Ctrl(']') => depth -= 1,
            _ => {}
        }
        result.push(tokens[index].clone());
        index += 1;
    }
    result
}

// --- Organize imports (the editor action) ------------------------------------
//
// The LSP "Organize Imports" source action both *sorts* a file's top-level
// import runs — in exactly the order `vilan fmt` produces, through the same
// [`import_sort_key`], so the two can never disagree — and *prunes* the leaves
// an editor's analyzer reports as unused. Sorting is the formatter's job either
// way; the usage decision is the editor's, threaded in as the `keep` predicate.
// Pruning is deliberately NOT part of `vilan fmt` (fmt has no analyzer), so it
// lives only behind this entry point.

/// One organized top-level import run: the source span it currently occupies and
/// the canonical replacement text (empty when the whole run pruned away). The LSP
/// turns each into a `TextEdit`.
pub struct ImportRunEdit {
    pub span: Span,
    pub replacement: String,
}

/// A pruned import statement awaiting canonical rendering. A re-export is surface,
/// not usage, so it is never pruned and renders from its original node; an
/// `import`/`use` that survived (whole or in part) renders from a node rebuilt to
/// carry only the leaves `keep` retained.
enum PrunedStatement<'ast, 'src> {
    ReExport(&'ast Node<'src>),
    Rebuilt(Node<'src>),
}

impl<'src> PrunedStatement<'_, 'src> {
    fn node(&self) -> &Node<'src> {
        match self {
            PrunedStatement::ReExport(node) => node,
            PrunedStatement::Rebuilt(node) => node,
        }
    }
}

/// Prunes an import path to the leaves `keep` retains, returning the surviving
/// branch — or `None` if every leaf was dropped. `keep(name_span)` is asked of
/// each *terminal* segment (the actual imported name): a `Path` with a `::`
/// continuation survives iff its continuation does, and a brace `Set` keeps its
/// surviving branches (`{ a, b }` with `b` unused becomes `{ a }`), dying only
/// when all of them go.
fn prune_import_branch<'src>(
    branch: &ImportBranch<'src>,
    keep: &dyn Fn(Span) -> bool,
) -> Option<ImportBranch<'src>> {
    match branch {
        ImportBranch::Path(name, span, None) => {
            keep(*span).then(|| ImportBranch::Path(name, *span, None))
        }
        ImportBranch::Path(name, span, Some(child)) => prune_import_branch(child, keep)
            .map(|pruned| ImportBranch::Path(name, *span, Some(Box::new(pruned)))),
        ImportBranch::Set(branches) => {
            let kept: Vec<ImportBranch<'src>> = branches
                .iter()
                .filter_map(|branch| prune_import_branch(branch, keep))
                .collect();
            (!kept.is_empty()).then_some(ImportBranch::Set(kept))
        }
    }
}

/// Organizes a file's *top-level* import runs: sorts each into canonical order
/// (the shared [`import_sort_key`], identical to `vilan fmt`) and, per `keep`,
/// prunes unused leaves. Returns one [`ImportRunEdit`] per run whose canonical
/// form differs from the source — an already-organized run yields nothing.
/// `keep(name_span)` decides whether the import leaf whose terminal name occupies
/// `name_span` survives; pass `|_| true` for sort-only. `None` when the source
/// doesn't parse cleanly (no edit would be safe). Block-scoped imports live
/// inside item bodies, not the top-level list, so they are never considered.
pub fn organize_import_runs(
    source: &str,
    keep: &dyn Fn(Span) -> bool,
) -> Option<Vec<ImportRunEdit>> {
    let items = parse(source)?;
    let mut printer = Printer {
        out: String::new(),
        indent: 0,
        comments: extract_comments(source),
        cursor: 0,
        source,
        bailed: false,
    };
    Some(printer.organize_runs(&items, keep))
}

/// Parses `source` into its top-level item list, or `None` if it doesn't parse
/// perfectly cleanly — the formatter reprints only sources it fully understands.
fn parse(source: &str) -> Option<NodeList<'_>> {
    let (tree, errors) = crate::parsing::parse(source);
    tree.filter(|_| errors.is_empty()).map(|(items, _)| items)
}

/// Formats `source`, returning the reprinted text. Returns the input unchanged if
/// it doesn't lex/parse, if the printer hits a construct it doesn't yet handle, or
/// if the reprint would change the code (see the safety note).
pub fn format(source: &str) -> String {
    let Some(original) = code_tokens(source) else {
        return source.to_string();
    };
    let Some(items) = parse(source) else {
        return source.to_string();
    };
    let mut printer = Printer {
        out: String::new(),
        indent: 0,
        comments: extract_comments(source),
        cursor: 0,
        source,
        bailed: false,
    };
    let prev_end = printer.print_items(&items, 0, true);
    // Comments after the last item (trailing end-of-file comments).
    printer.flush_comments_before(source.len(), prev_end);
    printer.out.push('\n');
    if printer.bailed {
        return source.to_string();
    }
    let matches = code_tokens(&printer.out)
        .is_some_and(|reprinted| normalize(reprinted) == normalize(original));
    if matches {
        printer.out
    } else {
        source.to_string()
    }
}

struct Printer<'src> {
    out: String,
    indent: usize,
    comments: Vec<(Span, &'src str)>,
    cursor: usize,
    source: &'src str,
    bailed: bool,
}

impl<'src> Printer<'src> {
    /// Whether the source between `from` and `to` contains a blank line (a run of
    /// only-whitespace with two or more newlines), used to preserve paragraph gaps.
    fn has_blank_between(&self, from: usize, to: usize) -> bool {
        from < to
            && self
                .source
                .get(from..to)
                .is_some_and(|gap| gap.bytes().filter(|byte| *byte == b'\n').count() >= 2)
    }

    /// Whether a standalone (own-line) comment sits between source offsets
    /// `after` and `before` — a comment preceded within the gap by a newline,
    /// i.e. not a trailing same-line comment of whatever ends at `after`. Such a
    /// comment pins an import run: the run breaks there, so imports never
    /// reorder across it.
    fn standalone_comment_between(&self, after: usize, before: usize) -> bool {
        self.comments.iter().any(|(span, _)| {
            let start = span.into_range().start;
            after <= start
                && start < before
                && self
                    .source
                    .get(after..start)
                    .is_some_and(|gap| gap.contains('\n'))
        })
    }

    /// Starts a fresh line at the current indentation (no leading newline at the
    /// very start of the output).
    fn line(&mut self) {
        if !self.out.is_empty() {
            self.out.push('\n');
            for _ in 0..self.indent {
                self.out.push('\t');
            }
        }
    }

    /// Emits a blank line (used to preserve a paragraph gap before the next item).
    fn blank_line(&mut self) {
        if !self.out.is_empty() {
            self.out.push('\n');
        }
    }

    /// Emits the standalone comments that appear before `pos`, each on its own
    /// line, preserving a blank line before a comment that the source had one
    /// before. Returns the source offset just past the last comment emitted (or
    /// `start_from` if none), so the caller can judge the gap before the item.
    fn flush_comments_before(&mut self, pos: usize, start_from: usize) -> usize {
        let mut at = start_from;
        while self.cursor < self.comments.len() {
            let (span, text) = self.comments[self.cursor];
            let range = span.into_range();
            if range.start >= pos {
                break;
            }
            if self.has_blank_between(at, range.start) {
                self.blank_line();
            }
            self.line();
            self.out.push_str(text);
            at = range.end;
            self.cursor += 1;
        }
        at
    }

    /// Emits a trailing (same-line) comment if the next pending comment starts on
    /// the same source line as `after` — i.e. it sat at the end of the item just
    /// printed (`foo(); // note`) rather than on its own line. Spacing collapses to
    /// a single space.
    fn flush_trailing_comment(&mut self, after: usize) {
        if let Some(text) = self.take_trailing_comment(after) {
            self.out.push(' ');
            self.out.push_str(text);
        }
    }

    /// Consumes and returns the next pending comment when it is a trailing
    /// same-line comment of whatever ends at `after` (`import x; // note`), so a
    /// caller reordering the items it belongs to can re-emit it in the new
    /// place. Returns `None` (consuming nothing) otherwise.
    fn take_trailing_comment(&mut self, after: usize) -> Option<&'src str> {
        if let Some((span, text)) = self.comments.get(self.cursor).copied() {
            let start = span.into_range().start;
            if start >= after
                && self
                    .source
                    .get(after..start)
                    .is_some_and(|gap| !gap.contains('\n'))
            {
                self.cursor += 1;
                return Some(text);
            }
        }
        None
    }

    /// Prints a list of items (top level or a block body), interleaving standalone
    /// comments and preserved blank lines. Returns the source offset past the last
    /// item, for any trailing comments. When `top_level`, a run of import/use
    /// statements prints in the canonical order (see the canonical-import-order
    /// section); block-scoped imports are left as written.
    fn print_items(
        &mut self,
        items: &[Spanned<Node<'src>>],
        start_from: usize,
        top_level: bool,
    ) -> usize {
        let mut prev_end = start_from;
        let mut index = 0;
        while index < items.len() {
            if top_level && import_kind_and_branch(&items[index].0).is_some() {
                let run_end = self.import_run_end(items, index);
                prev_end = self.print_import_run(&items[index..run_end], prev_end);
                index = run_end;
                continue;
            }
            let item = &items[index];
            let range = item.1.into_range();
            let after_comments = self.flush_comments_before(range.start, prev_end);
            if self.has_blank_between(after_comments, range.start) {
                self.blank_line();
            }
            self.line();
            self.print_item(item);
            if Self::needs_semicolon(&item.0) {
                self.out.push(';');
            }
            self.flush_trailing_comment(range.end);
            prev_end = range.end;
            index += 1;
        }
        prev_end
    }

    /// The exclusive end of the import run starting at `start`: the longest span
    /// of consecutive import-like items that no standalone comment breaks (a
    /// blank line does not break it — the block coalesces).
    fn import_run_end(&self, items: &[Spanned<Node<'src>>], start: usize) -> usize {
        let mut end = start + 1;
        while end < items.len() {
            if import_kind_and_branch(&items[end].0).is_none() {
                break;
            }
            let previous_end = items[end - 1].1.into_range().end;
            let this_start = items[end].1.into_range().start;
            if self.standalone_comment_between(previous_end, this_start) {
                break;
            }
            end += 1;
        }
        end
    }

    /// Prints a run of top-level import-like items in canonical order (see
    /// [`import_sort_key`]): reordered by kind/root/path, brace sets sorted,
    /// blank lines coalesced into one block. Each item's trailing same-line
    /// comment travels with it. Returns the source offset past the run.
    fn print_import_run(&mut self, run: &[Spanned<Node<'src>>], prev_end: usize) -> usize {
        let first_start = run[0].1.into_range().start;
        let after_comments = self.flush_comments_before(first_start, prev_end);
        if self.has_blank_between(after_comments, first_start) {
            self.blank_line();
        }
        // Attach each item's trailing comment (in source order, as the cursor
        // reaches it) before reordering. A run has no standalone comments within
        // it (they break it), so every comment here is one item's trailing one.
        let mut entries: Vec<(ImportSortKey, usize, Option<&'src str>)> =
            Vec::with_capacity(run.len());
        for (position, item) in run.iter().enumerate() {
            let end = item.1.into_range().end;
            let key = node_import_key(&item.0);
            let trailing = self.take_trailing_comment(end);
            entries.push((key, position, trailing));
        }
        // A stable canonical sort — duplicate imports keep their source order.
        entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        for (_, position, trailing) in &entries {
            self.line();
            self.print_import_like(&run[*position].0);
            if let Some(text) = trailing {
                self.out.push(' ');
                self.out.push_str(text);
            }
        }
        run.last().unwrap().1.into_range().end
    }

    /// Organizes the top-level import runs among `items` (sort + `keep`-prune),
    /// returning one edit per run that changes. Non-import items — and everything
    /// inside a block body — are untouched: only the top-level list is walked, and
    /// block-scoped imports never appear in it.
    fn organize_runs(
        &mut self,
        items: &[Spanned<Node<'src>>],
        keep: &dyn Fn(Span) -> bool,
    ) -> Vec<ImportRunEdit> {
        let mut edits = Vec::new();
        let mut index = 0;
        while index < items.len() {
            if import_kind_and_branch(&items[index].0).is_some() {
                let run_end = self.import_run_end(items, index);
                if let Some(edit) = self.organize_run(&items[index..run_end], keep) {
                    edits.push(edit);
                }
                index = run_end;
            } else {
                index += 1;
            }
        }
        edits
    }

    /// Organizes one import run (sort + prune) into a single replacement edit, or
    /// `None` when the run is already canonical. See [`organize_import_runs`].
    fn organize_run(
        &mut self,
        run: &[Spanned<Node<'src>>],
        keep: &dyn Fn(Span) -> bool,
    ) -> Option<ImportRunEdit> {
        let run_start = run[0].1.into_range().start;
        // Reach this run's own trailing comments; a standalone comment before the
        // run stays put (it is outside the replaced span, and it broke the run).
        self.skip_comments_before(run_start);

        // Source-order pass: prune each statement and claim its trailing comment,
        // which travels with it exactly as the printer's reorder does.
        let mut entries: Vec<(
            ImportSortKey,
            usize,
            PrunedStatement<'_, 'src>,
            Option<&'src str>,
        )> = Vec::with_capacity(run.len());
        for (position, item) in run.iter().enumerate() {
            let end = item.1.into_range().end;
            let statement = match &item.0 {
                // A re-export is surface, not usage — never pruned.
                Node::Export(_) => Some(PrunedStatement::ReExport(&item.0)),
                Node::Import(branch) => prune_import_branch(branch, keep)
                    .map(|pruned| PrunedStatement::Rebuilt(Node::Import(pruned))),
                Node::Use(branch) => prune_import_branch(branch, keep)
                    .map(|pruned| PrunedStatement::Rebuilt(Node::Use(pruned))),
                _ => None,
            };
            let trailing = self.take_trailing_comment(end);
            if let Some(statement) = statement {
                let key = node_import_key(statement.node());
                entries.push((key, position, statement, trailing));
            }
            // A statement pruned to nothing drops its trailing comment with it.
        }

        // The replaced span covers the whole run. An import node's span ends at
        // its path, so reach past the terminating `;` and then past the last
        // statement's trailing comment (the canonical text re-emits both).
        let last_terminator = self.statement_terminator_end(run.last().unwrap().1.into_range().end);
        let source_end = self
            .trailing_comment_end(last_terminator)
            .unwrap_or(last_terminator);

        // Every statement pruned away: delete the run, taking one line break so no
        // blank line is left behind.
        if entries.is_empty() {
            let mut deletion_end = source_end;
            if self.source.as_bytes().get(deletion_end) == Some(&b'\n') {
                deletion_end += 1;
            }
            return Some(ImportRunEdit {
                span: Span::from(run_start..deletion_end),
                replacement: String::new(),
            });
        }

        // Canonical order — a stable sort, so equal keys keep their source order.
        entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        // Render through the printer's own import printing, so an organized run is
        // byte-for-byte what `vilan fmt` would produce for it (minus pruned
        // leaves). `self.out` doubles as the scratch buffer for one statement.
        let mut replacement = String::new();
        for (position, (_, _, statement, trailing)) in entries.iter().enumerate() {
            if position > 0 {
                replacement.push('\n');
            }
            self.out.clear();
            self.print_import_like(statement.node());
            replacement.push_str(&self.out);
            if let Some(text) = trailing {
                replacement.push(' ');
                replacement.push_str(text);
            }
        }

        // An already-organized run offers no edit.
        if self.source.get(run_start..source_end) == Some(replacement.as_str()) {
            return None;
        }
        Some(ImportRunEdit {
            span: Span::from(run_start..source_end),
            replacement,
        })
    }

    /// Advances the comment cursor past every comment starting before `pos`,
    /// emitting nothing (unlike `flush_comments_before`) — the organizer uses it
    /// to reach a run's trailing comments while leaving earlier standalone
    /// comments in place.
    fn skip_comments_before(&mut self, pos: usize) {
        while let Some((span, _)) = self.comments.get(self.cursor) {
            if span.into_range().start >= pos {
                break;
            }
            self.cursor += 1;
        }
    }

    /// The offset just past the `;` that terminates an import statement whose
    /// path ends at `path_end` (the import node's span stops at its path; the
    /// `;`, possibly after whitespace, is a separate token). Falls back to
    /// `path_end` if no `;` is found — a cleanly parsed import always has one.
    fn statement_terminator_end(&self, path_end: usize) -> usize {
        let bytes = self.source.as_bytes();
        let mut index = path_end;
        while index < bytes.len() && bytes[index] != b';' {
            if !bytes[index].is_ascii_whitespace() {
                return path_end;
            }
            index += 1;
        }
        if bytes.get(index) == Some(&b';') {
            index + 1
        } else {
            path_end
        }
    }

    /// The end offset of the trailing same-line comment of an item ending at
    /// `after` (a comment starting at/after `after` with no intervening newline),
    /// or `None` — so the organizer's replaced span covers a comment it re-emits.
    fn trailing_comment_end(&self, after: usize) -> Option<usize> {
        self.comments.iter().find_map(|(span, _)| {
            let range = span.into_range();
            (range.start >= after
                && self
                    .source
                    .get(after..range.start)
                    .is_some_and(|gap| !gap.contains('\n')))
            .then_some(range.end)
        })
    }

    /// Prints one top-level import-like item with its brace sets sorted —
    /// `import`/`use`, or an `export import`/`export use` re-export.
    /// (Block-scoped imports print through `print_item` without sorting.)
    fn print_import_like(&mut self, node: &Node<'src>) {
        match node {
            Node::Use(branch) => {
                self.out.push_str("use ");
                self.print_import_branch(branch, true);
                self.out.push(';');
            }
            Node::Import(branch) => {
                self.out.push_str("import ");
                self.print_import_branch(branch, true);
                self.out.push(';');
            }
            Node::Export(inner) => {
                self.out.push_str("export ");
                self.print_import_like(&inner.0);
            }
            _ => {}
        }
    }

    /// Whether `node`, printed as a statement, takes a terminating `;`. Expression
    /// statements (`let`, assignments, calls, a `macro name(..)` invocation, …) do;
    /// control-flow forms (`if`/`for`/`match`/block), declarations (including a
    /// `macro fun`, a `macro { .. }` block, and a `[name] item` macro attribute),
    /// and `use`/`import` (which already emit their own `;`) do not.
    fn needs_semicolon(node: &Node<'src>) -> bool {
        !matches!(
            node,
            Node::If(_)
                | Node::For(_, _)
                | Node::ForIn(_, _, _)
                | Node::Match(_, _)
                | Node::Block(_)
                | Node::Func(_)
                | Node::Struct(_, _, _, _, _)
                | Node::Enum(_, _, _, _)
                | Node::Impl(_, _, _)
                | Node::Trait(_, _, _, _)
                | Node::Module(_, _)
                | Node::Derive(_, _)
                | Node::Service(_, _)
                | Node::Export(_)
                | Node::Use(_)
                | Node::Import(_)
                | Node::MacroFun(_)
                | Node::MacroBlock(_)
                | Node::MacroAttribute(_, _, _, _)
        )
    }

    /// Prints one top-level / block item. Sets `bailed` for anything not yet
    /// handled, so `format` falls back to the original source.
    fn print_item(&mut self, item: &Spanned<Node<'src>>) {
        match &item.0 {
            // `[resource ][external ]struct Name[<…>][;|{ fields }]` — canonical
            // modifier order is `resource external struct` (destruction.md §3).
            Node::Struct(name, generics, external, resource, body) => {
                if *resource {
                    self.out.push_str("resource ");
                }
                if *external {
                    self.out.push_str("external ");
                }
                self.out.push_str("struct ");
                self.out.push_str(name.0);
                self.print_generic_parameters(generics);
                match body {
                    None => self.out.push(';'),
                    Some(fields) if fields.0.is_empty() => self.out.push_str(" {}"),
                    Some(fields) => {
                        self.out.push_str(" {");
                        self.indent += 1;
                        let mut prev_end = fields.1.into_range().start + 1;
                        for ((field_name, field_type, exposed), span) in &fields.0 {
                            let range = span.into_range();
                            let after_comments = self.flush_comments_before(range.start, prev_end);
                            if self.has_blank_between(after_comments, range.start) {
                                self.blank_line();
                            }
                            self.line();
                            if *exposed {
                                self.out.push_str("[expose] ");
                            }
                            self.out.push_str(field_name.0);
                            if let Some(field_type) = field_type {
                                self.out.push_str(": ");
                                self.print_type(&field_type.0);
                            }
                            self.out.push(',');
                            self.flush_trailing_comment(range.end);
                            prev_end = range.end;
                        }
                        self.flush_comments_before(fields.1.into_range().end, prev_end);
                        self.indent -= 1;
                        self.line();
                        self.out.push('}');
                    }
                }
            }
            // `[resource ]enum Name[<…>] { Variant[(payload)][ = discriminant], … }`.
            Node::Enum(name, generics, resource, variants) => {
                if *resource {
                    self.out.push_str("resource ");
                }
                self.out.push_str("enum ");
                self.out.push_str(name.0);
                self.print_generic_parameters(generics);
                if variants.0.is_empty() {
                    self.out.push_str(" {}");
                } else {
                    self.out.push_str(" {");
                    self.indent += 1;
                    let mut prev_end = variants.1.into_range().start + 1;
                    for ((variant_name, payload, discriminant), span) in &variants.0 {
                        let range = span.into_range();
                        let after_comments = self.flush_comments_before(range.start, prev_end);
                        if self.has_blank_between(after_comments, range.start) {
                            self.blank_line();
                        }
                        self.line();
                        self.out.push_str(variant_name);
                        if !payload.is_empty() {
                            self.out.push('(');
                            for (index, (payload_type, _)) in payload.iter().enumerate() {
                                if index > 0 {
                                    self.out.push_str(", ");
                                }
                                self.print_type(payload_type);
                            }
                            self.out.push(')');
                        }
                        if let Some(discriminant) = discriminant {
                            self.out.push_str(" = ");
                            self.out.push_str(&discriminant.to_string());
                        }
                        self.out.push(',');
                        self.flush_trailing_comment(range.end);
                        prev_end = range.end;
                    }
                    self.flush_comments_before(variants.1.into_range().end, prev_end);
                    self.indent -= 1;
                    self.line();
                    self.out.push('}');
                }
            }
            // A block-scoped `use`/`import` (inside a `fn`/`impl`/`mod` body):
            // printed as written, brace set unsorted — a deliberate placement.
            // Top-level import runs print through `print_import_run` instead.
            Node::Use(branch) => {
                self.out.push_str("use ");
                self.print_import_branch(branch, false);
                self.out.push(';');
            }
            Node::Import(branch) => {
                self.out.push_str("import ");
                self.print_import_branch(branch, false);
                self.out.push(';');
            }
            Node::Func(func) => self.print_func(func),
            // `impl Subject[ with A + B] { items }`.
            Node::Impl(subject, traits, body) => {
                self.out.push_str("impl ");
                self.print_type(&subject.0);
                self.print_with_clause(traits);
                self.out.push(' ');
                self.print_braced_items(body);
            }
            // `trait Name[ with A + B] { items }`.
            Node::Trait(name, generics, supertraits, body) => {
                self.out.push_str("trait ");
                self.out.push_str(name.0);
                self.print_generic_parameters(generics);
                self.print_with_clause(supertraits);
                self.out.push(' ');
                self.print_braced_items(body);
            }
            // `[derive(A, B)]` sits on its own line above the item it annotates.
            Node::Derive(names, derived) => {
                self.out.push_str("[derive(");
                let names: Vec<&str> = names.iter().map(|(name, _)| *name).collect();
                self.out.push_str(&names.join(", "));
                self.out.push_str(")]");
                self.line();
                self.print_item(derived);
            }
            // `[service]` / `[service(Client)]` likewise sits above its struct.
            Node::Service(client_name, item) => {
                self.out.push_str("[service");
                if let Some(client_name) = client_name {
                    self.out.push('(');
                    self.out.push_str(client_name);
                    self.out.push(')');
                }
                self.out.push(']');
                self.line();
                self.print_item(item);
            }
            Node::Export(exported) => {
                self.out.push_str("export ");
                self.print_item(exported);
            }
            // `mod name { items }`.
            Node::Module(name, body) => {
                self.out.push_str("mod ");
                self.out.push_str(name);
                self.out.push(' ');
                self.print_braced_items(body);
            }
            // `macro fun name(..) { .. }` — a macro definition. The `macro`
            // keyword then the ordinary function form.
            Node::MacroFun(func) => {
                self.out.push_str("macro ");
                self.print_func(func);
            }
            // `[name(args)?] <item>` — a user macro attribute, on its own line
            // above the struct/enum/function it annotates (like `[derive(..)]`).
            // The optional arguments are SYNTAX — reprinted verbatim from source.
            Node::MacroAttribute(name, _name_span, argument_spans, annotated) => {
                self.out.push('[');
                self.out.push_str(name);
                if !argument_spans.is_empty() {
                    self.out.push('(');
                    self.print_argument_spans(argument_spans);
                    self.out.push(')');
                }
                self.out.push(']');
                self.line();
                self.print_item(annotated);
            }
            // Anything else is an expression appearing as a statement.
            _ => self.print_expr(item),
        }
    }

    /// Prints an import/use path: `a::b::{ c, d }`. When `sort`, a brace set's
    /// branches print in canonical order (`{ c, d }`) — used for top-level
    /// imports; block-scoped imports pass `false` to print them as written.
    fn print_import_branch(&mut self, branch: &ImportBranch<'src>, sort: bool) {
        match branch {
            ImportBranch::Path(name, _, child) => {
                self.out.push_str(name);
                if let Some(child) = child {
                    self.out.push_str("::");
                    self.print_import_branch(child, sort);
                }
            }
            ImportBranch::Set(branches) => {
                self.out.push_str("{ ");
                let mut order: Vec<&ImportBranch<'src>> = branches.iter().collect();
                if sort {
                    order.sort_by_cached_key(|branch| branch_key(&branch_from_ast(branch)));
                }
                for (index, child) in order.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_import_branch(child, sort);
                }
                self.out.push_str(" }");
            }
        }
    }

    /// Prints a type expression: `i32`, `List<T>`, `Map<str, i32>`, `&mut T`.
    /// Bails (falling `format` back to the source) on any type form not yet handled.
    fn print_type(&mut self, node: &Node<'src>) {
        match node {
            Node::Accessor(name) => self.out.push_str(name),
            Node::AccessorWithGenerics(name, arguments) => {
                self.out.push_str(name);
                self.out.push('<');
                for (index, (argument, _)) in arguments.0.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_type(argument);
                }
                self.out.push('>');
            }
            Node::Reference(mutable, inner) => {
                self.out.push('&');
                if *mutable {
                    self.out.push_str("mut ");
                }
                self.print_type(&inner.0);
            }
            // `async |A| B` / `sync |A| B` — closure-type contract markers.
            Node::AsyncType(inner) => {
                self.out.push_str("async ");
                self.print_type(&inner.0);
            }
            Node::SyncType(inner) => {
                self.out.push_str("sync ");
                self.print_type(&inner.0);
            }
            // `|A, B| Ret` (or `||` for no parameters) — a closure type.
            Node::ClosureType(parameters, return_type) => {
                if parameters.0.is_empty() {
                    self.out.push_str("||");
                } else {
                    self.out.push('|');
                    for (index, (name, parameter_type)) in parameters.0.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        if let Some(name) = name {
                            self.out.push_str(name);
                            self.out.push_str(": ");
                        }
                        self.print_type(&parameter_type.0);
                    }
                    self.out.push('|');
                }
                if let Some(return_type) = return_type {
                    self.out.push(' ');
                    self.print_type(&return_type.0);
                }
            }
            // `type T[: A + B]` — a generic binder inside an impl subject pattern.
            Node::TypeBinder(name, bounds) => {
                self.out.push_str("type ");
                self.out.push_str(name);
                self.print_bounds(bounds);
            }
            // `(A, B)` — a tuple type.
            Node::Tuple(elements) => {
                self.out.push('(');
                for (index, (element, _)) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_type(element);
                }
                self.out.push(')');
            }
            // `[T; n]` — a fixed-length array type (proposal/fixed-arrays.md): the
            // element type and a length (an integer literal). Nests as `[[T; m]; n]`.
            Node::ArrayType(element, length) => {
                self.out.push('[');
                self.print_type(&element.0);
                self.out.push_str("; ");
                self.print_expr(length);
                self.out.push(']');
            }
            _ => self.bailed = true,
        }
    }

    /// Prints a `: A + B` trait-bound list, or nothing when `bounds` is empty.
    fn print_bounds(&mut self, bounds: &[Spanned<Node<'src>>]) {
        if bounds.is_empty() {
            return;
        }
        self.out.push_str(": ");
        for (index, (bound, _)) in bounds.iter().enumerate() {
            if index > 0 {
                self.out.push_str(" + ");
            }
            self.print_type(bound);
        }
    }

    /// Prints a `with A + B` clause (the traits of an `impl`/`trait`), or nothing
    /// when there are none.
    fn print_with_clause(&mut self, traits: &[Spanned<Node<'src>>]) {
        if traits.is_empty() {
            return;
        }
        self.out.push_str(" with ");
        for (index, (trait_, _)) in traits.iter().enumerate() {
            if index > 0 {
                self.out.push_str(" + ");
            }
            self.print_type(trait_);
        }
    }

    /// Prints the `<T, U: Bound = Default>` parameter list of a generic item, or
    /// nothing when there are none.
    fn print_generic_parameters(&mut self, parameters: &Option<GenericParameters<'src>>) {
        let Some((parameters, _)) = parameters else {
            return;
        };
        self.out.push('<');
        for (index, parameter) in parameters.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            if parameter.is_type {
                self.out.push_str("type ");
            }
            self.out.push_str(parameter.name);
            self.print_bounds(&parameter.bounds);
            if let Some(default) = &parameter.default {
                self.out.push_str(" = ");
                self.print_type(&default.0);
            }
        }
        self.out.push('>');
    }

    /// Prints a `{ items }` block of declarations (an `impl`/`trait`/`mod` body),
    /// each item on its own line, preserving interior comments and blank lines.
    fn print_braced_items(&mut self, body: &Spanned<NodeList<'src>>) {
        let range = body.1.into_range();
        if body.0.is_empty() && !self.has_comment_in(range.start, range.end) {
            self.out.push_str("{}");
            return;
        }
        self.out.push('{');
        self.indent += 1;
        let prev_end = self.print_items(&body.0, range.start + 1, false);
        self.flush_comments_before(range.end, prev_end);
        self.indent -= 1;
        self.line();
        self.out.push('}');
    }

    /// Prints a function declaration: its `[extern]`/`[must_use]`/`[rpc]`
    /// attributes (if any) each on their own line, then
    /// `[async ][external ]fun name[<…>](…)[: T][ borrows p]` followed by the
    /// body block, or a `;` for a signature with no body.
    fn print_func(&mut self, func: &Func<'src>) {
        if let Some(binding) = &func.extern_binding {
            self.print_extern_attribute(binding);
            self.line();
        }
        if func.must_use {
            self.out.push_str("[must_use]");
            self.line();
        }
        if func.rpc {
            self.out.push_str("[rpc]");
            self.line();
        }
        if func.trait_only {
            self.out.push_str("[trait_only]");
            self.line();
        }
        if func.doc_hidden {
            self.out.push_str("[doc(hidden)]");
            self.line();
        }
        if !func.platform_fence.is_empty() {
            let patterns = func
                .platform_fence
                .iter()
                .map(|(pattern, _)| format!("\"{pattern}\""))
                .collect::<Vec<_>>()
                .join(", ");
            self.out.push_str(&format!("[platform({patterns})]"));
            self.line();
        }
        if func.is_async {
            self.out.push_str("async ");
        }
        if func.external {
            self.out.push_str("external ");
        }
        self.out.push_str("fun ");
        self.out.push_str(func.name.0);
        self.print_generic_parameters(&func.generic_parameters);
        self.print_parameters(&func.parameters.0);
        if let Some(return_type) = &func.return_type {
            self.out.push_str(": ");
            self.print_type(&return_type.0);
        }
        if let Some(borrows) = func.borrows {
            self.out.push_str(" borrows ");
            self.out.push_str(borrows);
        }
        match &func.body {
            Some(body) => {
                self.out.push(' ');
                self.print_block(body);
            }
            None => self.out.push(';'),
        }
    }

    /// Prints a `[extern(..)]` host-binding attribute in its canonical form.
    fn print_extern_attribute(&mut self, binding: &ExternBinding<'src>) {
        self.out.push_str("[extern(");
        match binding {
            ExternBinding::Function {
                module: None,
                symbol,
            } => {
                self.out.push('"');
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Function {
                module: Some(module),
                symbol,
            } => {
                self.out.push('"');
                self.out.push_str(module);
                self.out.push_str("\", \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Method { symbol: None } => self.out.push_str("method"),
            ExternBinding::Method {
                symbol: Some(symbol),
            } => {
                self.out.push_str("method, \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::New { module, symbol } => {
                self.out.push_str("new, ");
                if let Some(module) = module {
                    self.out.push('"');
                    self.out.push_str(module);
                    self.out.push_str("\", ");
                }
                self.out.push('"');
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Get { symbol } => {
                self.out.push_str("get, \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Set { symbol } => {
                self.out.push_str("set, \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
        }
        self.out.push_str(")]");
    }

    /// Prints a `(name: T, &mut self, …)` parameter list. The `&`/`&mut`/`own`
    /// convention prefix is reprinted only when it came from a prefix rather than
    /// the parameter's reference type (which already carries it).
    fn print_parameters(&mut self, parameters: &[crate::node::Parameter<'src>]) {
        self.out.push('(');
        self.print_parameters_inner(parameters);
        self.out.push(')');
    }

    /// Prints the comma-separated parameters themselves, without the surrounding
    /// delimiters (shared by function `(…)` and closure `|…|` lists).
    fn print_parameters_inner(&mut self, parameters: &[crate::node::Parameter<'src>]) {
        for (index, (binder, parameter_type, convention, _)) in parameters.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            let type_is_reference = matches!(
                parameter_type.as_deref().map(|spanned| &spanned.0),
                Some(Node::Reference(..))
            );
            match convention {
                Convention::Own => self.out.push_str("own "),
                Convention::Ref if !type_is_reference => self.out.push('&'),
                Convention::RefMut if !type_is_reference => self.out.push_str("&mut "),
                _ => {}
            }
            self.print_binder(binder);
            if let Some(parameter_type) = parameter_type {
                self.out.push_str(": ");
                self.print_type(&parameter_type.0);
            }
        }
    }

    /// Prints a braced statement block `{ … }` — the body of a function, loop,
    /// `if`, or a block expression. An empty block (no statements, no tail, no
    /// interior comment) stays inline as `{}`.
    fn print_block(&mut self, block: &Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>) {
        let range = block.1.into_range();
        let (statements, tail) = &block.0;
        let empty = statements.is_empty() && matches!(tail.0, Node::Void);
        if empty && !self.has_comment_in(range.start, range.end) {
            self.out.push_str("{}");
            return;
        }
        self.out.push('{');
        self.indent += 1;
        let mut prev_end = self.print_items(statements, range.start + 1, false);
        if !matches!(tail.0, Node::Void) {
            let tail_range = tail.1.into_range();
            let after_comments = self.flush_comments_before(tail_range.start, prev_end);
            if self.has_blank_between(after_comments, tail_range.start) {
                self.blank_line();
            }
            self.line();
            self.print_expr(tail);
            self.flush_trailing_comment(tail_range.end);
            prev_end = tail_range.end;
        }
        self.flush_comments_before(range.end, prev_end);
        self.indent -= 1;
        self.line();
        self.out.push('}');
    }

    /// Whether the next non-trivia character at or after `from` in the source is a
    /// comma — used to preserve a match arm's optional separator comma (which the
    /// AST drops) so either corpus style round-trips.
    fn source_has_comma_at(&self, from: usize) -> bool {
        let bytes = self.source.as_bytes();
        let mut index = from;
        while index < bytes.len() {
            match bytes[index] {
                b' ' | b'\t' | b'\n' | b'\r' => index += 1,
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                b',' => return true,
                _ => return false,
            }
        }
        false
    }

    /// Whether any extracted comment falls within `[from, to)` — used to decide
    /// whether an otherwise-empty block must expand to carry its comments.
    fn has_comment_in(&self, from: usize, to: usize) -> bool {
        self.comments.iter().any(|(span, _)| {
            let range = span.into_range();
            range.start >= from && range.start < to
        })
    }

    /// The binding precedence of a binary operator (higher binds tighter), used to
    /// decide where operands need parentheses.
    fn binary_precedence(operator: BinaryOp) -> u8 {
        match operator {
            BinaryOp::Or => 0,
            BinaryOp::And => 1,
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Lt
            | BinaryOp::Gt
            | BinaryOp::LtEq
            | BinaryOp::GtEq => 3,
            // Vilan's source order (Rust-style): bitwise binds tighter than
            // comparison, looser than arithmetic. (JS's differs; the
            // transformer's own table handles emission.)
            BinaryOp::BitOr => 4,
            BinaryOp::BitXor => 5,
            BinaryOp::BitAnd => 6,
            BinaryOp::Shl | BinaryOp::Shr | BinaryOp::UShr => 7,
            BinaryOp::Add | BinaryOp::Sub => 8,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 9,
        }
    }

    /// The precedence of an expression as an operand — `100` for atoms/postfix
    /// (never need wrapping), `0` for statement-like forms (always wrapped as an
    /// operand). Mirrors the parser's expression layering.
    fn expression_precedence(node: &Node<'src>) -> u8 {
        match node {
            Node::Binary(operator, _, _) => Self::binary_precedence(*operator),
            Node::Is(_, _) => 2,
            Node::Unary(_, _)
            | Node::Reference(_, _)
            | Node::Dereference(_)
            | Node::Await(_)
            | Node::Async(_) => 10,
            Node::Assign(_, _, _)
            | Node::Let(_, _, _, _)
            | Node::Closure(_)
            | Node::If(_)
            | Node::For(_, _)
            | Node::ForIn(_, _, _)
            | Node::Match(_, _)
            | Node::Jump(_)
            | Node::FuncReturn(_)
            | Node::Const(_) => 0,
            _ => 100,
        }
    }

    /// Prints `expr` as an operand, wrapping it in parentheses when its precedence
    /// is below `minimum` (so the reprint reparses to the same tree). An
    /// interpolated string is reprinted verbatim and never wrapped — it already
    /// carries its own parentheses in the expanded token stream.
    fn print_operand(&mut self, expr: &Spanned<Node<'src>>, minimum: u8) {
        if self.interpolated_source(expr).is_some() {
            self.print_expr(expr);
        } else if Self::expression_precedence(&expr.0) < minimum {
            self.out.push('(');
            self.print_expr(expr);
            self.out.push(')');
        } else {
            self.print_expr(expr);
        }
    }

    /// Prints the subject of a `.member` or `[index]` postfix. A `Lift` (an
    /// `a?.b` chain) greedily absorbs any following `.member` / `[index]` / call
    /// into its continuation, so as a postfix subject it must be parenthesized —
    /// `(a?.b).c`, not `a?.b.c` (which reparses with `.c` pulled inside the
    /// lift). Every other subject follows the ordinary operand rule (min 100): an
    /// atom / call / member / index needs no parens; a binary / `is` / prefix
    /// form gets its source parens back through precedence.
    fn print_postfix_subject(&mut self, subject: &Spanned<Node<'src>>) {
        if matches!(subject.0, Node::Lift(_, _)) {
            self.out.push('(');
            self.print_expr(subject);
            self.out.push(')');
        } else {
            self.print_operand(subject, 100);
        }
    }

    /// Prints a comma-separated list of macro arguments, each reprinted VERBATIM
    /// from its source span. A macro's arguments are syntax (the parser keeps only
    /// their spans, not a tree), so — like an interpolated string — they are
    /// recovered from the source text rather than rebuilt. Whitespace inside an
    /// argument is preserved; the separator is normalized to `, `.
    fn print_argument_spans(&mut self, argument_spans: &[Span]) {
        for (index, span) in argument_spans.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            let range = span.into_range();
            self.out.push_str(&self.source[range]);
        }
    }

    /// Prints a comma-separated expression list (call arguments, list/tuple
    /// elements) inline.
    fn print_expression_list(&mut self, elements: &[Spanned<Node<'src>>]) {
        for (index, element) in elements.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            self.print_expr(element);
        }
    }

    /// If `expr`'s source span is an `i"..."` interpolated string — which the
    /// lexer rewrites into a parenthesized `("" + parts..)` concatenation before
    /// parsing, with every produced token sharing the literal's span — return the
    /// literal's original source text. Reprinting that verbatim is exact;
    /// rebuilding it from the expanded AST would have to re-derive the lexer's
    /// brace/quote escaping.
    fn interpolated_source(&self, expr: &Spanned<Node<'src>>) -> Option<&'src str> {
        let range = expr.1.into_range();
        if self.source.get(range.start..range.start + 2) != Some("i\"") {
            return None;
        }
        // The lexer reports an i-string's span ending *at* its closing quote
        // rather than after it, so the recovered slice would drop the quote (and
        // swallow the rest of the file into one string). Include it when present.
        let end = if self.source.as_bytes().get(range.end) == Some(&b'"') {
            range.end + 1
        } else {
            range.end
        };
        self.source.get(range.start..end)
    }

    /// Prints any expression. Sets `bailed` for forms not yet handled.
    fn print_expr(&mut self, expr: &Spanned<Node<'src>>) {
        if let Some(interpolated) = self.interpolated_source(expr) {
            self.out.push_str(interpolated);
            return;
        }
        match &expr.0 {
            Node::Number(whole, fraction, suffix) => {
                self.out.push_str(whole);
                if let Some(fraction) = fraction {
                    self.out.push('.');
                    self.out.push_str(fraction);
                }
                if let Some(suffix) = suffix {
                    self.out.push_str(suffix);
                }
            }
            Node::String(text) => {
                self.out.push('"');
                self.out.push_str(text);
                self.out.push('"');
            }
            // A triple-quoted string reprints VERBATIM: its inner whitespace is
            // semantic (the closing delimiter's indentation is the trim prefix),
            // so the formatter must never re-indent it.
            Node::MultilineString(text) => {
                self.out.push_str("\"\"\"");
                self.out.push_str(text);
                self.out.push_str("\"\"\"");
            }
            Node::Bool(value) => self.out.push_str(if *value { "true" } else { "false" }),
            Node::Null => self.out.push_str("null"),
            Node::Void => {}
            Node::Accessor(name) => self.out.push_str(name),
            Node::AccessorWithGenerics(name, arguments) => {
                self.out.push_str(name);
                self.out.push('<');
                for (index, (argument, _)) in arguments.0.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_type(argument);
                }
                self.out.push('>');
            }
            Node::MemberAccessor(subject, member) => {
                self.print_postfix_subject(subject);
                self.out.push('.');
                self.print_expr(member);
            }
            Node::StaticAccessor(subject, member) => {
                self.print_operand(subject, 100);
                self.out.push_str("::");
                self.out.push_str(member);
            }
            Node::Index(subject, index) => {
                self.print_postfix_subject(subject);
                self.out.push('[');
                self.print_expr(index);
                self.out.push(']');
            }
            Node::Call(callee, generic_arguments, arguments) => {
                // A call binds tighter than `.`/`[]`, so `a.b(c)` parses as
                // `a.(b(c))`. To call the *result* of a member/index access the
                // callee must be parenthesized — `(a.b)(c)` — or it reparses wrong.
                // A `?.` lift chain likewise absorbs a following call into its
                // continuation, so a `Lift` callee needs its own parens: `(a?.b)()`.
                if matches!(
                    callee.0,
                    Node::MemberAccessor(_, _) | Node::Index(_, _) | Node::Lift(_, _)
                ) {
                    self.out.push('(');
                    self.print_expr(callee);
                    self.out.push(')');
                } else {
                    self.print_operand(callee, 100);
                }
                if let Some((generic_arguments, _)) = generic_arguments {
                    self.out.push('<');
                    for (index, (argument, _)) in generic_arguments.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.print_type(argument);
                    }
                    self.out.push('>');
                }
                self.out.push('(');
                self.print_expression_list(&arguments.0);
                self.out.push(')');
            }
            Node::Binary(operator, left, right) => {
                let precedence = Self::binary_precedence(*operator);
                self.print_operand(left, precedence);
                self.out.push(' ');
                self.out.push_str(binary_operator_symbol(*operator));
                self.out.push(' ');
                self.print_operand(right, precedence + 1);
            }
            // A prefix operator (`-x`, `!x`, `&x`, `*x`, `await x`) binds tighter
            // than every binary operator (the parser recurses on the unary chain
            // for the operand), so a binary operand must be parenthesized to
            // reparse the same way — `-(2 + 3)`, not `-2 + 3`. Operand minimum 10
            // (the prefix tier in `expression_precedence`) wraps every binary
            // (precedence 0–9) while leaving a nested prefix (`- -x`) and atoms
            // unwrapped.
            Node::Unary(operator, operand) => {
                self.out.push(*operator);
                self.print_operand(operand, 10);
            }
            Node::TryAssert(subject) => {
                self.print_operand(subject, 100);
                self.out.push('!');
            }
            Node::Lift(subject, continuation) => {
                // `a?.b.c`: the subject, `?`, then the continuation — whose
                // innermost `LiftBinder` prints nothing, so its leading
                // `.member` renders right after the `?`.
                self.print_operand(subject, 100);
                self.out.push('?');
                self.print_expr(continuation);
            }
            Node::LiftBinder => {}
            // A bare-`?` expression-lifting mark — the formatter parses raw
            // trees (the region rewrite runs only at the analyzer's entry),
            // so the mark prints back exactly as written.
            Node::Lifted(subject) => {
                self.print_operand(subject, 100);
                self.out.push('?');
            }
            // A recorded region-delimiting paren group: the parens are
            // semantic (they bound the lift), so they always reprint.
            Node::LiftGroup(inner) => {
                self.out.push('(');
                self.print_expr(inner);
                self.out.push(')');
            }
            // Rewrite output — never present in the formatter's raw parse.
            Node::LiftRegion(..) | Node::LiftHole(_) => {
                unreachable!("lift regions exist only after the analyzer-entry rewrite")
            }
            Node::Reference(mutable, operand) => {
                self.out.push('&');
                if *mutable {
                    self.out.push_str("mut ");
                }
                self.print_operand(operand, 10);
            }
            Node::Dereference(operand) => {
                self.out.push('*');
                self.print_operand(operand, 10);
            }
            Node::Await(operand) => {
                self.out.push_str("await ");
                self.print_operand(operand, 10);
            }
            Node::Async(operand) => {
                self.out.push_str("async ");
                self.print_expr(operand);
            }
            // Weak precedence: `const` captures everything to its right, so
            // the inner expression never needs wrapping; as an OPERAND the
            // whole `const ..` is parenthesized (precedence 0 above).
            Node::Const(inner) => {
                self.out.push_str("const ");
                self.print_expr(inner);
            }
            Node::Let(name, declared_type, value, mutable) => {
                self.out.push_str(if *mutable { "mut " } else { "let " });
                self.out.push_str(name.0);
                if let Some(declared_type) = declared_type {
                    self.out.push_str(": ");
                    self.print_type(&declared_type.0);
                }
                if let Some(value) = value {
                    self.out.push_str(" = ");
                    self.print_expr(value);
                }
            }
            // `let (a, b) = …` / `mut [x, y] = …` — a destructuring binding. As
            // `Let`, but the bound name is an irrefutable tuple/array binder
            // (a name, or a nesting of them) printed by `print_binder`.
            Node::LetDestructure(pattern, declared_type, value, mutable) => {
                self.out.push_str(if *mutable { "mut " } else { "let " });
                self.print_binder(&pattern.0);
                if let Some(declared_type) = declared_type {
                    self.out.push_str(": ");
                    self.print_type(&declared_type.0);
                }
                if let Some(value) = value {
                    self.out.push_str(" = ");
                    self.print_expr(value);
                }
            }
            Node::Assign(target, operator, value) => {
                self.print_expr(target);
                self.out.push(' ');
                if let Some(operator) = operator {
                    self.out.push_str(binary_operator_symbol(*operator));
                }
                self.out.push_str("= ");
                self.print_expr(value);
            }
            Node::If(branch) => self.print_if_branch(branch),
            Node::Match(subject, legs) => {
                self.out.push_str("match ");
                self.print_expr(subject);
                self.out.push_str(" {");
                self.indent += 1;
                let mut prev_end = legs.1.into_range().start + 1;
                for leg in &legs.0 {
                    let (patterns, _, body) = leg;
                    let start = patterns
                        .first()
                        .map(|(_, span)| span.into_range().start)
                        .unwrap_or(prev_end);
                    let after_comments = self.flush_comments_before(start, prev_end);
                    if self.has_blank_between(after_comments, start) {
                        self.blank_line();
                    }
                    self.line();
                    self.print_match_leg(leg);
                    // The arm separator comma is optional and not kept in the AST
                    // (the corpus mixes `=> { .. },` and `=> { .. }`), so preserve
                    // whatever the source had to round-trip either style faithfully.
                    let body_end = body.1.into_range().end;
                    if self.source_has_comma_at(body_end) {
                        self.out.push(',');
                    }
                    self.flush_trailing_comment(body_end);
                    prev_end = body_end;
                }
                self.flush_comments_before(legs.1.into_range().end, prev_end);
                self.indent -= 1;
                self.line();
                self.out.push('}');
            }
            Node::For(condition, body) => {
                self.out.push_str("for");
                if let Some(condition) = condition {
                    self.out.push(' ');
                    self.print_expr(condition);
                }
                self.out.push(' ');
                self.print_block(body);
            }
            Node::ForIn(variable, iterable, body) => {
                self.out.push_str("for ");
                self.out.push_str(variable);
                self.out.push_str(" in ");
                self.print_expr(iterable);
                self.out.push(' ');
                self.print_block(body);
            }
            Node::FuncReturn(value) => {
                self.out.push_str("ret");
                if let Some(value) = value {
                    self.out.push(' ');
                    self.print_expr(value);
                }
            }
            Node::Jump(target) => {
                self.out.push_str("jump ");
                self.out.push_str(target);
            }
            Node::Block(block) => self.print_block(block),
            // `macro { .. }` — an anonymous immediately-expanded macro. Legal in
            // both item and expression position; the body is a statement block.
            Node::MacroBlock(body) => {
                self.out.push_str("macro ");
                self.print_block(body);
            }
            // `macro name(args)` — a macro invocation (item or expression
            // position). The arguments are SYNTAX — reprinted verbatim from their
            // source spans, never rebuilt from a parsed tree (only spans are kept).
            Node::MacroInvocation(name, _name_span, argument_spans) => {
                self.out.push_str("macro ");
                self.out.push_str(name);
                self.out.push('(');
                self.print_argument_spans(argument_spans);
                self.out.push(')');
            }
            Node::StructInitializer(name, generic_arguments, fields) => {
                self.out.push_str(name);
                if let Some((generic_arguments, _)) = generic_arguments {
                    self.out.push('<');
                    for (index, (argument, _)) in generic_arguments.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.print_type(argument);
                    }
                    self.out.push('>');
                }
                if fields.0.is_empty() {
                    self.out.push_str(" {}");
                } else {
                    self.out.push_str(" { ");
                    for (index, ((field_name, value), _)) in fields.0.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.out.push_str(field_name);
                        if let Some(value) = value {
                            self.out.push_str(" = ");
                            self.print_expr(value);
                        }
                    }
                    self.out.push_str(" }");
                }
            }
            Node::List(elements) => {
                self.out.push('[');
                self.print_expression_list(elements);
                self.out.push(']');
            }
            // `[value; n]` — a fixed-length array repeat literal (proposal/
            // fixed-arrays.md): the value copied into each of `n` slots. `; ` is
            // the canonical spelling of the length separator.
            Node::Repeat(value, length) => {
                self.out.push('[');
                self.print_expr(value);
                self.out.push_str("; ");
                self.print_expr(length);
                self.out.push(']');
            }
            Node::Tuple(elements) => {
                self.out.push('(');
                self.print_expression_list(elements);
                self.out.push(')');
            }
            Node::Closure(closure) => {
                self.print_closure_parameters(&closure.parameters.0);
                if let Some(return_type) = &closure.return_type {
                    self.out.push_str(": ");
                    self.print_type(&return_type.0);
                }
                self.out.push(' ');
                self.print_expr(&closure.return_value);
            }
            Node::Is(subject, pattern) => {
                self.print_operand(subject, 3);
                self.out.push_str(" is ");
                self.print_match_pattern(pattern);
            }
            _ => self.bailed = true,
        }
    }

    /// Prints the closure parameter list `|a, b|` (or `||`) for a closure value.
    /// Closures share the function parameter syntax, but with `|` delimiters and a
    /// single-token `||` for the empty list.
    fn print_closure_parameters(&mut self, parameters: &[crate::node::Parameter<'src>]) {
        if parameters.is_empty() {
            self.out.push_str("||");
            return;
        }
        self.out.push('|');
        self.print_parameters_inner(parameters);
        self.out.push('|');
    }

    /// Prints an `if`/`else if`/`else` chain.
    fn print_if_branch(&mut self, branch: &NodeIfBranch<'src>) {
        match branch {
            NodeIfBranch::If(if_) => {
                self.out.push_str("if ");
                self.print_expr(&if_.condition);
                self.out.push(' ');
                self.print_block(&if_.then);
                if let Some((else_branch, _)) = &if_.else_ {
                    self.out.push_str(" else ");
                    match else_branch {
                        NodeIfBranch::If(_) => self.print_if_branch(else_branch),
                        NodeIfBranch::Else(block) => self.print_block(block),
                    }
                }
            }
            NodeIfBranch::Else(block) => self.print_block(block),
        }
    }

    /// Prints one `match` leg: `pattern[, pattern][ if guard] => body`.
    fn print_match_leg(&mut self, leg: &crate::node::MatchLeg<'src>) {
        let (patterns, guard, body) = leg;
        for (index, pattern) in patterns.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            self.print_match_pattern(pattern);
        }
        if let Some(guard) = guard {
            self.out.push_str(" if ");
            self.print_expr(guard);
        }
        self.out.push_str(" => ");
        self.print_expr(body);
    }

    /// Prints a binder in `let`/parameter position: a bare name (no `let `
    /// keyword), a tuple `(a, b)`, or a fixed-array `[a, b]` binder — each of
    /// which may nest binders. Distinct from a match pattern (`print_pattern`),
    /// where a binding reads `let x` / `mut x`.
    fn print_binder(&mut self, binder: &Pattern<'src>) {
        match binder {
            Pattern::Binding(name, _) => self.out.push_str(name),
            Pattern::Tuple(elements) => {
                self.out.push('(');
                for (index, (element, _)) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_binder(element);
                }
                self.out.push(')');
            }
            Pattern::Array(elements) => {
                self.out.push('[');
                for (index, (element, _)) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_binder(element);
                }
                self.out.push(']');
            }
            // A binder is only ever a name, a tuple, or an array of binders;
            // other pattern shapes can't reach here from the parser.
            other => self.print_pattern(other),
        }
    }

    /// Prints a match pattern, consulting the source to keep a binding tuple's
    /// spelling. `let (a, b)` (the keyword factored out, before the tuple) and
    /// `(let a, let b)` (a tuple of per-element binders) parse to the *same*
    /// `Tuple` of `Binding`s, and both appear in the corpus (`let (a, b)` in
    /// `destructuring.vl`; `Some((let x, let y))` in `option.vl`). Neither is
    /// canonically preferable, so the printer reproduces whichever the source
    /// used — decided by the tuple span's first byte: `l`/`m` (the `let`/`mut`
    /// keyword) is the factored form, `(` the per-element form. Every other
    /// pattern prints identically regardless of span.
    fn print_match_pattern(&mut self, pattern: &Spanned<Pattern<'src>>) {
        if let Pattern::Tuple(_) = &pattern.0 {
            let start = pattern.1.into_range().start;
            match self.source.as_bytes().get(start) {
                Some(b'l') => {
                    self.out.push_str("let ");
                    self.print_binder(&pattern.0);
                    return;
                }
                Some(b'm') => {
                    self.out.push_str("mut ");
                    self.print_binder(&pattern.0);
                    return;
                }
                _ => {}
            }
        }
        self.print_pattern(&pattern.0);
    }

    fn print_pattern(&mut self, pattern: &Pattern<'src>) {
        match pattern {
            Pattern::Wildcard => self.out.push('_'),
            Pattern::Binding(name, mutable) => {
                self.out.push_str(if *mutable { "mut " } else { "let " });
                self.out.push_str(name);
            }
            Pattern::Variant(path, payload) => {
                for (index, segment) in path.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str("::");
                    }
                    self.out.push_str(segment);
                }
                if let Some(payload) = payload {
                    self.out.push('(');
                    for (index, sub_pattern) in payload.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.print_match_pattern(sub_pattern);
                    }
                    self.out.push(')');
                }
            }
            Pattern::Tuple(elements) => {
                self.out.push('(');
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_match_pattern(element);
                }
                self.out.push(')');
            }
            Pattern::Array(elements) => {
                self.out.push('[');
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_match_pattern(element);
                }
                self.out.push(']');
            }
            Pattern::Literal(literal) => self.print_expr(literal),
        }
    }
}

/// The source spelling of a binary operator.
fn binary_operator_symbol(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Rem => "%",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        // JS-only (the transformer's unsigned right shift); never in a parsed
        // source tree, but total for safety.
        BinaryOp::UShr => ">>",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitXor => "^",
        BinaryOp::BitOr => "|",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Gt => ">",
        BinaryOp::LtEq => "<=",
        BinaryOp::GtEq => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_comments_skipping_strings() {
        let source = "let url = \"http://x\"; // a note\n// lead\nfun f() {}\n";
        let comments: Vec<&str> = extract_comments(source)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(comments, vec!["// a note", "// lead"]);
    }

    #[test]
    fn comment_spans_are_forward_and_trimmed() {
        let source = "fun f() {} // trailing  \n";
        let (span, text) = extract_comments(source)[0];
        assert_eq!(text, "// trailing");
        let range = span.into_range();
        assert!(range.start <= range.end);
        assert_eq!(&source[range.start..range.end], "// trailing");
    }
}

#[cfg(test)]
mod reformats {
    use super::format;

    fn assert_formats(source: &str, expected: &str) {
        assert_eq!(format(source), expected);
        // The output must be a fixed point — formatting it again is a no-op.
        assert_eq!(format(expected), expected, "output is not idempotent");
    }

    // `async`/`sync` closure-type markers round-trip (they used to BAIL,
    // leaving marker-bearing files unformattable).
    #[test]
    fn closure_type_markers_round_trip() {
        let source =
            "fun take(f: async || i32, g: sync |i32| bool) {\n\tlet h: async || void = f;\n}\n";
        assert_formats(source, source);
    }

    #[test]
    fn struct_fields_onto_their_own_lines() {
        assert_formats(
            "struct Point{x:i32,y:i32}\n",
            "struct Point {\n\tx: i32,\n\ty: i32,\n}\n",
        );
    }

    #[test]
    fn a_struct_literal_operand_reformats() {
        // §H.1: a struct literal as an operator operand round-trips through
        // the formatter (parse → print → parse must hold).
        assert_formats(
            "fun f(p: Point): bool {\n\tPoint{x=1}==p\n}\n",
            "fun f(p: Point): bool {\n\tPoint { x = 1 } == p\n}\n",
        );
    }

    #[test]
    fn generic_and_reference_field_types() {
        assert_formats(
            "struct Boxed { item :  List<i32> , next : &mut Node }\n",
            "struct Boxed {\n\titem: List<i32>,\n\tnext: &mut Node,\n}\n",
        );
    }

    #[test]
    fn empty_struct_body_stays_inline() {
        assert_formats("struct Unit{\n}\n", "struct Unit {}\n");
    }

    #[test]
    fn enum_variants_onto_their_own_lines() {
        assert_formats("enum E{A,B}\n", "enum E {\n\tA,\n\tB,\n}\n");
    }

    #[test]
    fn generic_enum_with_payloads() {
        assert_formats(
            "enum Option<T>{Some(T),None}\n",
            "enum Option<T> {\n\tSome(T),\n\tNone,\n}\n",
        );
    }

    // C4 S1 (destruction.md §3): the `resource` declaration modifier prints back
    // in canonical position — `resource` before `struct`/`enum`, `resource
    // external struct` for the leaf host case — and re-formats to a fixed point.
    #[test]
    fn resource_struct_modifier_round_trips() {
        assert_formats(
            "resource struct S{x:i32}\n",
            "resource struct S {\n\tx: i32,\n}\n",
        );
    }

    #[test]
    fn resource_external_struct_keeps_canonical_order() {
        assert_formats(
            "resource external struct Database;\n",
            "resource external struct Database;\n",
        );
    }

    #[test]
    fn resource_enum_modifier_round_trips() {
        assert_formats(
            "resource enum E{A,B}\n",
            "resource enum E {\n\tA,\n\tB,\n}\n",
        );
    }

    #[test]
    fn function_signature_and_body() {
        assert_formats(
            "fun add(a:i32,b:i32):i32{a+b}\n",
            "fun add(a: i32, b: i32): i32 {\n\ta + b\n}\n",
        );
    }

    #[test]
    fn statements_take_semicolons_tail_does_not() {
        assert_formats(
            "fun f(){let x=1;print(x);x}\n",
            "fun f() {\n\tlet x = 1;\n\tprint(x);\n\tx\n}\n",
        );
    }

    #[test]
    fn precedence_parentheses_are_minimal() {
        assert_formats("fun f(){(a+b)*c}\n", "fun f() {\n\t(a + b) * c\n}\n");
        assert_formats("fun f(){a+b*c}\n", "fun f() {\n\ta + b * c\n}\n");
    }

    #[test]
    fn call_through_a_member_keeps_its_parentheses() {
        // `(self.fn)()` calls a field-closure; `self.fn()` is a method call.
        assert_formats(
            "fun f(self){(self.fn)()}\n",
            "fun f(self) {\n\t(self.fn)()\n}\n",
        );
    }

    #[test]
    fn trailing_comment_stays_on_its_line() {
        assert_formats(
            "fun f(){print(1);    // note\n}\n",
            "fun f() {\n\tprint(1); // note\n}\n",
        );
    }

    #[test]
    fn interpolated_string_is_reprinted_verbatim() {
        // The lexer expands `i"..."` to `("" + ..)` before parsing; the printer
        // recovers the original literal from the source rather than the AST.
        assert_formats(
            "fun f(self){print(i\"hi {self.name}!\")}\n",
            "fun f(self) {\n\tprint(i\"hi {self.name}!\")\n}\n",
        );
    }

    #[test]
    fn interpolated_string_with_escaped_braces() {
        assert_formats(
            "fun f(){let x=i\"a \\{b\\} c\";x}\n",
            "fun f() {\n\tlet x = i\"a \\{b\\} c\";\n\tx\n}\n",
        );
    }

    #[test]
    fn impl_with_match_and_closure() {
        // `fn: |T| U` keeps the space after `:` — `:|` would lex as one operator.
        // The last arm has no source comma, so the faithful output keeps none.
        assert_formats(
            "impl Option<type T> { fun map<U>(self, fn: |T| U): Option<U> { match self { Some(let x)=>Some(fn(x)), None=>None } } }\n",
            "impl Option<type T> {\n\tfun map<U>(self, fn: |T| U): Option<U> {\n\t\tmatch self {\n\t\t\tSome(let x) => Some(fn(x)),\n\t\t\tNone => None\n\t\t}\n\t}\n}\n",
        );
    }
}

#[cfg(test)]
mod idempotency {
    use super::format;

    /// The real invariant: formatting is a fixed point. `format(x)` may tidy `x`,
    /// but formatting the result again must change nothing.
    fn assert_fixed_point(name: &str, source: &str) {
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice, "formatting {name} is not a fixed point");
    }

    macro_rules! fixed_point_tests {
        ($($name:ident => $path:literal),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    assert_fixed_point($path, include_str!(concat!("../../../vilan/std/src/", $path)));
                }
            )*
        };
    }

    // A spread of std modules exercising functions, impls, traits, generics,
    // enums with payloads, matches, closures, and `[extern]` bindings —
    // `reactive.vl` also exercises `[must_use]` (which the formatter once
    // dropped, tripping its safety check into a silent no-op).
    fixed_point_tests! {
        null_vl => "null.vl",
        boolean_vl => "boolean.vl",
        option_vl => "option.vl",
        result_vl => "result.vl",
        list_vl => "list.vl",
        string_vl => "string.vl",
        set_vl => "set.vl",
        iterator_vl => "iterator.vl",
        arena_vl => "arena.vl",
        shared_vl => "shared.vl",
        display_vl => "display.vl",
        reactive_vl => "reactive.vl",
    }

    /// The bitwise/shift operators and hex literals print back exactly —
    /// well-formatted source containing them is a fixed point (`<<`/`>>` must
    /// re-lex as adjacent control tokens, `0xFF` keeps its spelling).
    #[test]
    fn bitwise_operators_and_hex_are_a_fixed_point() {
        let source = "fun main() {\n\tlet mask = 0xFFu32;\n\tlet mixed = 1 << 2 & 3 ^ 4 | 5;\n\tlet shifted = mask >> 4;\n\tlet big = 0xDEADn;\n}\n";
        assert_fixed_point("bitwise", source);
    }

    /// Attributes must be *retained*, not just idempotent — a formatter that
    /// deterministically deleted an attribute would still be a fixed point, so
    /// the retention is asserted directly. (Dropping one used to trip the
    /// safety check, silently leaving the whole file unformatted.)
    #[test]
    fn attributes_round_trip() {
        let source = "trait Source {\n\t[must_use]\n\t[platform(\"@process\", \"browser\")]\n\tfun sub(self): i32;\n\
                      \t[trait_only]\n\tfun tag(self): str;\n\
                      \t[doc(hidden)]\n\tfun internal(self): i32;\n}\n\
                      [service(Client)]\n\
                      struct Sess {\n\t[expose] status: Signal<str>,\n\thidden: i32,\n}\n\
                      impl Sess {\n\t[rpc]\n\tfun login(self, name: str): bool {\n\t\ttrue\n\t}\n}\n";
        let formatted = format(source);
        assert!(
            formatted.contains("[service(Client)]"),
            "service attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[must_use]")
                && formatted.contains("[platform(\"@process\", \"browser\")]"),
            "attributes lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[expose] status"),
            "expose attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[rpc]"),
            "rpc attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[trait_only]"),
            "trait_only attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[doc(hidden)]"),
            "doc(hidden) attribute lost:\n{formatted}"
        );
        assert_fixed_point("attributes", source);
    }
}

#[cfg(test)]
mod bailing_constructs {
    //! Backlog E13 — the constructs that used to make `vilan fmt` silently
    //! no-op: each hit a `_ => bailed` printer fallback or tripped the
    //! re-lex-and-compare safety net, so the formatter returned the file
    //! unchanged (indistinguishable from an already-canonical file). Each now
    //! round-trips. Per construct, `assert_construct` proves the whole contract
    //! loudly: the output re-lexes to the SAME token stream as the input (the
    //! net's own criterion), the formatter did NOT silently bail (a
    //! token-preserving perturbation canonicalizes identically), the output is
    //! the canonical spelling, formatting is idempotent, and the canonical form
    //! round-trips unchanged.
    use super::{format, normalize};
    use crate::lexing::tokenize;
    use crate::token::Token;

    /// The formatter's notion of "the same code": the lexer's tokens with spans
    /// stripped and insignificant trailing commas normalized away.
    fn code_tokens(text: &str) -> Vec<Token<'_>> {
        let (tokens, errors) = tokenize(text);
        assert!(
            errors.is_empty(),
            "did not lex cleanly: {text:?} ({errors:?})"
        );
        normalize(tokens.into_iter().map(|(token, _)| token).collect())
    }

    fn assert_construct(source: &str, expected: &str) {
        let formatted = format(source);
        // (a) The output carries the SAME tokens as the source — the safety
        // net's criterion, asserted here rather than trusted silently.
        assert_eq!(
            code_tokens(&formatted),
            code_tokens(source),
            "output token-drifted from the input on {source:?}"
        );
        // Not a silent bail: a bail returns the input verbatim, so appending
        // blank lines (pure trivia) would survive instead of canonicalizing.
        assert_eq!(
            format(&format!("{source}\n\n")),
            formatted,
            "formatter silently bailed on {source:?}"
        );
        // (b) The canonical spelling.
        assert_eq!(
            formatted, expected,
            "unexpected canonical form for {source:?}"
        );
        // (c) Idempotent, and (d) the canonical form round-trips unchanged.
        assert_eq!(format(&formatted), formatted, "not idempotent: {source:?}");
        assert_eq!(
            format(expected),
            expected,
            "canonical form did not round-trip: {expected:?}"
        );
    }

    // --- Prefix-operator precedence (unary-minus.vl) -------------------------

    // A prefix operator binds tighter than every binary operator, so a
    // parenthesized binary operand keeps its parens — dropping them reparses
    // `-2 + 3`. (This tripped the net; the operand minimum was too low.)
    #[test]
    fn unary_minus_over_a_parenthesized_binary_keeps_parens() {
        assert_construct("fun f() {\n\t-(2 + 3)\n}\n", "fun f() {\n\t-(2 + 3)\n}\n");
    }

    // `- -x` and `--x` lex identically (vilan has no `--` operator); a nested
    // prefix (precedence 10) never wraps, so double negation collapses.
    #[test]
    fn double_negation_collapses() {
        assert_construct("fun f() {\n\t- -x\n}\n", "fun f() {\n\t--x\n}\n");
    }

    // A binary subtraction of a negated operand needs no parens — the right
    // operand is a prefix form, which binds tighter than `-`.
    #[test]
    fn binary_subtract_of_a_negative() {
        assert_construct("fun f() {\n\t3 - -2\n}\n", "fun f() {\n\t3 - -2\n}\n");
    }

    #[test]
    fn plain_prefix_operands_round_trip() {
        assert_construct("fun f() {\n\t-x\n}\n", "fun f() {\n\t-x\n}\n");
        assert_construct("fun f() {\n\t!ok\n}\n", "fun f() {\n\t!ok\n}\n");
    }

    // The same precedence rule applies to every prefix operator, not just `-`.
    #[test]
    fn reference_of_a_parenthesized_binary_keeps_parens() {
        assert_construct("fun f() {\n\t&(a + b)\n}\n", "fun f() {\n\t&(a + b)\n}\n");
    }

    // --- Lift-chain postfix subjects (lift-chain.vl) -------------------------

    // A `?.` chain absorbs a following `.member` into its continuation, so a
    // member access on the *result* of a lift must parenthesize it —
    // `(a?.b).c`, not `a?.b.c` (which pulls `.c` inside the lift).
    #[test]
    fn member_access_on_a_lift_result_wraps_the_lift() {
        assert_construct("fun f() {\n\t(x?.y).z\n}\n", "fun f() {\n\t(x?.y).z\n}\n");
    }

    // Likewise a call on a lift result: `(a?.b)()`.
    #[test]
    fn call_on_a_lift_result_wraps_the_lift() {
        assert_construct("fun f() {\n\t(x?.y)()\n}\n", "fun f() {\n\t(x?.y)()\n}\n");
    }

    // Without parens the postfixes belong inside the lift, so none are added:
    // `.z` is absorbed, `!` (assert-or-return) is not, and both chain flat.
    #[test]
    fn absorbed_and_unabsorbed_lift_postfixes_need_no_parens() {
        assert_construct("fun f() {\n\tx?.y.z\n}\n", "fun f() {\n\tx?.y.z\n}\n");
        assert_construct("fun f() {\n\tx?.y!\n}\n", "fun f() {\n\tx?.y!\n}\n");
        assert_construct("fun f() {\n\tx?.y!.z\n}\n", "fun f() {\n\tx?.y!.z\n}\n");
    }

    // --- Tuple / array destructuring bindings (destructuring.vl, math.vl,
    //     reactive-owner.vl, fixed-arrays.vl) --------------------------------

    #[test]
    fn let_tuple_destructure() {
        assert_construct(
            "fun f() {\n\tlet (a,b)=pair;\n}\n",
            "fun f() {\n\tlet (a, b) = pair;\n}\n",
        );
    }

    #[test]
    fn nested_let_tuple_destructure() {
        assert_construct(
            "fun f() {\n\tlet (n, (m, label)) = x;\n}\n",
            "fun f() {\n\tlet (n, (m, label)) = x;\n}\n",
        );
    }

    #[test]
    fn let_and_mut_array_destructure() {
        assert_construct(
            "fun f() {\n\tlet [a, b] = arr;\n}\n",
            "fun f() {\n\tlet [a, b] = arr;\n}\n",
        );
        assert_construct(
            "fun f() {\n\tmut [r0, r1] = right;\n}\n",
            "fun f() {\n\tmut [r0, r1] = right;\n}\n",
        );
    }

    #[test]
    fn typed_tuple_destructure() {
        assert_construct(
            "fun f() {\n\tlet (a, b): (i32, str) = x;\n}\n",
            "fun f() {\n\tlet (a, b): (i32, str) = x;\n}\n",
        );
    }

    // A match tuple binding has two source spellings that parse identically:
    // `let (a, b)` (keyword factored out) and `(let a, let b)` (per-element).
    // Both are in the corpus, so the printer reproduces whichever was written —
    // this round-trip fails if the printer canonicalizes to one form.
    #[test]
    fn match_tuple_binding_keeps_its_source_spelling() {
        let canonical = "fun f() {\n\tmatch z {\n\t\tlet (a, b) => 0,\n\t\tSome(let (c, d)) => 1,\n\t\tSome((let e, let g)) => 2,\n\t}\n}\n";
        assert_construct(canonical, canonical);
    }

    // --- Fixed-array literals and types (fixed-arrays.vl) --------------------

    #[test]
    fn array_repeat_literal() {
        assert_construct(
            "fun f() {\n\tlet z = [0;4];\n}\n",
            "fun f() {\n\tlet z = [0; 4];\n}\n",
        );
    }

    // An aggregate repeat — a struct literal value copied into each slot.
    #[test]
    fn aggregate_array_repeat_literal() {
        assert_construct(
            "fun f() {\n\tmut cells = [Cell { n = 7 }; 3];\n}\n",
            "fun f() {\n\tmut cells = [Cell { n = 7 }; 3];\n}\n",
        );
    }

    #[test]
    fn fixed_array_type_in_a_signature() {
        assert_construct(
            "fun total(values:[i32;3]):i32 { 0 }\n",
            "fun total(values: [i32; 3]): i32 {\n\t0\n}\n",
        );
    }

    // Nested fixed-array type: `[[i32; 2]; 3]`.
    #[test]
    fn nested_fixed_array_type() {
        assert_construct(
            "fun grid(): [[i32; 2]; 3] {\n\tg\n}\n",
            "fun grid(): [[i32; 2]; 3] {\n\tg\n}\n",
        );
    }

    // --- Sized / hex / suffixed numerics (numeric-types.vl, math.vl) ---------

    // The number printer round-trips a width suffix, a float suffix, a hex
    // literal, and a `BigInt` suffix (all already handled — pinned as an edge).
    #[test]
    fn suffixed_hex_and_float_numerics() {
        let canonical = "fun f() {\n\tlet a = 0xFFu8;\n\tlet b = 2.25f32;\n\tlet c = 7n;\n\tlet d = 9007199254740992i53;\n}\n";
        assert_construct(canonical, canonical);
    }

    // --- Macro forms (macro-block.vl, macro-derive.vl, macro-invoke.vl) ------

    #[test]
    fn macro_fun_definition() {
        assert_construct(
            "macro fun make(): Source { source(\"\") }\n",
            "macro fun make(): Source {\n\tsource(\"\")\n}\n",
        );
    }

    // A `macro { .. }` block in item position: its body is a statement block and
    // it takes no `;` (like an item declaration). A body with several statements
    // (a "family" of items stamped at expansion) reprints on its own lines.
    #[test]
    fn macro_block_in_item_position() {
        assert_construct(
            "macro {\n\tmut generated = \"\";\n\tsource(generated)\n}\n",
            "macro {\n\tmut generated = \"\";\n\tsource(generated)\n}\n",
        );
    }

    // A `macro { .. }` block in expression position is the `let`'s value, so the
    // terminating `;` belongs to the `let`.
    #[test]
    fn macro_block_in_expression_position() {
        assert_construct(
            "fun f() {\n\tlet folded = macro {\n\t\tsource(i\"1\")\n\t};\n}\n",
            "fun f() {\n\tlet folded = macro {\n\t\tsource(i\"1\")\n\t};\n}\n",
        );
    }

    // A `macro name(args)` invocation in item position takes a `;`; its
    // arguments are syntax, reprinted verbatim from source.
    #[test]
    fn macro_invocation_in_item_position() {
        assert_construct(
            "macro constants(zero, one, two);\n",
            "macro constants(zero, one, two);\n",
        );
    }

    // In expression position the invocation splices in place; a closure argument
    // is reprinted verbatim (spans only are kept, so it is never rebuilt).
    #[test]
    fn macro_invocation_in_expression_position() {
        assert_construct(
            "fun f() {\n\tprint(macro unroll(4, |i: i32| accumulate(i)))\n}\n",
            "fun f() {\n\tprint(macro unroll(4, |i: i32| accumulate(i)))\n}\n",
        );
    }

    // A user macro attribute sits on its own line above the item it annotates —
    // with no arguments (`[derive_display]`) and with verbatim ones (`[grow(a, b)]`).
    #[test]
    fn macro_attribute_without_and_with_arguments() {
        assert_construct(
            "[derive_display]\nstruct Point {\n\tx: i32,\n}\n",
            "[derive_display]\nstruct Point {\n\tx: i32,\n}\n",
        );
        assert_construct(
            "[grow(a, b)]\nstruct Grid {\n\tn: i32,\n}\n",
            "[grow(a, b)]\nstruct Grid {\n\tn: i32,\n}\n",
        );
    }

    // A `[derive(A, B)]` built-in derive on a struct (already handled — pinned
    // as an E13 edge alongside the user-macro attribute above).
    #[test]
    fn derive_attributed_struct() {
        assert_construct(
            "[derive(Json, Debug)]\nstruct Packet {\n\tkind: u8,\n}\n",
            "[derive(Json, Debug)]\nstruct Packet {\n\tkind: u8,\n}\n",
        );
    }
}

#[cfg(test)]
mod import_sorting {
    //! `vilan fmt` reorders a file's top-level `import`/`use` statements into the
    //! canonical order (see the canonical-import-order section): kind (`import`
    //! before `use`), then root namespace (`std`, dependencies, `pkg`), then the
    //! full path, with brace sets sorted internally. Runs coalesce across blank
    //! lines and break at standalone comments; a trailing comment travels with
    //! its import; block-scoped imports are left as written.
    use super::{format, normalize};
    use crate::lexing::tokenize;
    use crate::token::Token;

    /// The reprint carries the canonical order, is idempotent, and did not
    /// silently bail (a bail returns the input verbatim, so appended blank
    /// lines — pure trivia — would survive instead of canonicalizing).
    fn assert_sorts(source: &str, expected: &str) {
        assert_eq!(format(source), expected, "unexpected canonical order");
        assert_eq!(format(expected), expected, "not idempotent");
        assert_eq!(
            format(&format!("{source}\n\n")),
            expected,
            "silently bailed on {source:?}"
        );
    }

    /// The lexer's tokens with spans stripped — a raw stream to feed `normalize`
    /// directly (unlike the reprint path, this does not sort anything itself).
    fn raw_tokens(text: &str) -> Vec<Token<'_>> {
        let (tokens, errors) = tokenize(text);
        assert!(errors.is_empty(), "did not lex cleanly: {text:?}");
        tokens.into_iter().map(|(token, _)| token).collect()
    }

    // A run mixing every root and both kinds sorts to canonical order: imports
    // before uses, then `std` < dependency (`acme`) < `pkg`, then path.
    #[test]
    fn shuffled_std_dependency_pkg_run_sorts_canonically() {
        assert_sorts(
            "import pkg::z::thing;\nimport std::io::print;\nimport acme::widget;\n\
             use std::option::Option;\nimport std::alpha;\nuse acme::helper;\n",
            "import std::alpha;\nimport std::io::print;\nimport acme::widget;\n\
             import pkg::z::thing;\nuse std::option::Option;\nuse acme::helper;\n",
        );
    }

    // A brace set's inner branch list sorts alphabetically (case-sensitive), the
    // path is otherwise unchanged.
    #[test]
    fn branch_set_inner_list_sorts() {
        assert_sorts(
            "import std::x::{ delta, beta, alpha };\n",
            "import std::x::{ alpha, beta, delta };\n",
        );
        // Case-sensitive: capitalized names sort before lowercase (ASCII).
        assert_sorts(
            "import std::option::Option::{ self, Some, None };\n",
            "import std::option::Option::{ None, Some, self };\n",
        );
    }

    // A `use` always sorts after every `import`, whatever the paths — the kind
    // is the primary key.
    #[test]
    fn use_sorts_after_import() {
        assert_sorts(
            "use std::a;\nimport std::z;\n",
            "import std::z;\nuse std::a;\n",
        );
    }

    // The `export` re-export prefix does not change grouping: an `export import`
    // sorts as a plain import (by root and path), keeping its prefix. A
    // `std`-rooted plain import still precedes `pkg` re-exports.
    #[test]
    fn export_reexports_sort_by_the_same_key() {
        assert_sorts(
            "export import pkg::string::str;\nexport import pkg::io::print;\n\
             import std::option::Option;\nexport import pkg::number::{ u32, BigInt, i8 };\n",
            "import std::option::Option;\nexport import pkg::io::print;\n\
             export import pkg::number::{ BigInt, i8, u32 };\nexport import pkg::string::str;\n",
        );
    }

    // A block-scoped import (inside a `fn` body — backlog H2) is a deliberate
    // placement: neither its run order nor its brace set is touched, even when
    // both are non-canonical. Byte-identical output, and the net did not trip.
    #[test]
    fn block_scoped_imports_are_untouched() {
        let source = "fun f() {\n\tuse zeta::last;\n\timport std::b::{ y, x };\n\
                      \timport std::a;\n\tb::go();\n}\n";
        assert_eq!(
            format(source),
            source,
            "block-scoped imports were reordered"
        );
    }

    // A standalone (own-line) comment pins the run: imports do not reorder
    // across it, so the two sides sort independently and the comment stays put.
    // A blank line, by contrast, coalesces (the run reprints as one block).
    #[test]
    fn standalone_comment_pins_the_run_blank_line_coalesces() {
        assert_sorts(
            "import std::b;\n\nimport std::c;\n// pin\nimport std::z;\nimport std::m;\n",
            "import std::b;\nimport std::c;\n// pin\nimport std::m;\nimport std::z;\n",
        );
    }

    // A trailing same-line comment travels with its import to the new position.
    #[test]
    fn trailing_comment_travels_with_its_import() {
        assert_sorts(
            "import std::c; // the c note\nimport std::a;\n",
            "import std::a;\nimport std::c; // the c note\n",
        );
    }

    // Formatting an already-canonical run changes nothing (no spurious churn) —
    // the property the whole corpus fallout depends on.
    #[test]
    fn already_canonical_run_is_a_fixed_point() {
        let canonical = "import std::a;\nimport std::b;\nuse dep::x;\n\nfun main() {}\n";
        assert_eq!(format(canonical), canonical);
    }

    // The safety net forgives import-run order (and brace-set order) — on BOTH
    // sides — so the printer's reorder passes. This is what makes the reprint
    // land instead of bailing.
    #[test]
    fn normalize_forgives_import_run_and_branch_order() {
        assert_eq!(
            normalize(raw_tokens("import std::b;\nimport std::a;\n")),
            normalize(raw_tokens("import std::a;\nimport std::b;\n")),
            "normalize must canonicalize top-level import-run order"
        );
        assert_eq!(
            normalize(raw_tokens("import std::x::{ b, a };\n")),
            normalize(raw_tokens("import std::x::{ a, b };\n")),
            "normalize must canonicalize brace-set order"
        );
    }

    // ...but nothing else. Swapping two top-level functions stays a detectable
    // difference — the net did not go order-insensitive beyond import runs, so
    // it still catches a genuine reprint reordering bug.
    #[test]
    fn net_still_catches_a_non_import_reordering() {
        assert_ne!(
            normalize(raw_tokens("fun a() {}\nfun b() {}\n")),
            normalize(raw_tokens("fun b() {}\nfun a() {}\n")),
            "the net went order-insensitive beyond import runs"
        );
    }

    // A block-scoped import run (brace depth ≥ 1) is likewise NOT forgiven by
    // the net: the two orders stay distinct, so the net still polices the
    // placements the printer deliberately never reorders.
    #[test]
    fn net_does_not_forgive_block_scoped_import_order() {
        assert_ne!(
            normalize(raw_tokens("fun f() {\nuse b::y;\nuse a::x;\n}\n")),
            normalize(raw_tokens("fun f() {\nuse a::x;\nuse b::y;\n}\n")),
            "normalize must not reorder block-scoped imports"
        );
    }
}

#[cfg(test)]
mod organize {
    //! `organize_import_runs` backs the LSP "Organize Imports" action: it sorts
    //! top-level import runs into the same canonical order `vilan fmt` produces
    //! and prunes the leaves an analyzer reports unused. Here the analyzer's role
    //! is faked by a name list — `keep` rejects any leaf whose terminal name is
    //! in `dead` — so these pin the sort/prune/edit mechanics in isolation; the
    //! LSP pins cover the real (usage-driven, macro-aware) predicate.
    use super::organize_import_runs;
    use crate::span::Span;

    /// Applies the organizer's edits to `source`, treating every leaf named in
    /// `dead` as unused. Edits apply back-to-front so earlier offsets stay valid.
    fn organize(source: &str, dead: &[&str]) -> String {
        let keep = |span: Span| !dead.contains(&&source[span.into_range()]);
        let mut edits = organize_import_runs(source, &keep).expect("source parses cleanly");
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.into_range().start));
        let mut result = source.to_string();
        for edit in edits {
            result.replace_range(edit.span.into_range(), &edit.replacement);
        }
        result
    }

    /// The organizer offers no edit at all (already organized / nothing to prune).
    fn assert_no_edit(source: &str, dead: &[&str]) {
        let keep = |span: Span| !dead.contains(&&source[span.into_range()]);
        let edits = organize_import_runs(source, &keep).expect("source parses cleanly");
        assert!(
            edits.is_empty(),
            "expected no edit, got {} edit(s)",
            edits.len()
        );
    }

    // Sort-only (nothing dead): a shuffled run reorders exactly as `vilan fmt`.
    #[test]
    fn sort_only_reorders_a_run() {
        assert_eq!(
            organize("import std::z;\nimport std::a;\n", &[]),
            "import std::a;\nimport std::z;\n",
        );
    }

    // A whole import with no live leaf is dropped; the survivor stays.
    #[test]
    fn a_dead_import_is_pruned() {
        assert_eq!(
            organize("import std::used;\nimport std::dead;\n", &["dead"]),
            "import std::used;\n",
        );
    }

    // A dead brace-set branch shrinks the set (`{ a, b }` → `{ a }`); the whole
    // import survives because a live branch remains.
    #[test]
    fn a_dead_branch_shrinks_its_set() {
        assert_eq!(
            organize("import std::x::{ alpha, beta };\n", &["beta"]),
            "import std::x::{ alpha };\n",
        );
        // The middle of three goes, leaving the two live ones in canonical order.
        assert_eq!(
            organize("import std::x::{ a, b, c };\n", &["b"]),
            "import std::x::{ a, c };\n",
        );
    }

    // A whole import prunes only when EVERY branch is dead.
    #[test]
    fn an_import_dies_only_when_all_branches_do() {
        assert_eq!(
            organize("import std::x::{ a, b };\nfun main() {}\n", &["a", "b"]),
            "fun main() {}\n",
        );
    }

    // A re-export is surface, not usage — never pruned, even when its name is
    // reported unused. (It still participates in sorting.)
    #[test]
    fn a_reexport_is_never_pruned() {
        assert_no_edit("export import std::api;\n", &["api"]);
        // And a dead plain import next to a same-named re-export: the re-export
        // stays, the plain import goes.
        assert_eq!(
            organize(
                "export import std::api;\nimport std::dead;\n",
                &["api", "dead"],
            ),
            "export import std::api;\n",
        );
    }

    // A trailing same-line comment travels with its surviving import when the run
    // is reordered.
    #[test]
    fn a_trailing_comment_travels_when_sorting() {
        assert_eq!(
            organize("import std::z; // note z\nimport std::a;\n", &[]),
            "import std::a;\nimport std::z; // note z\n",
        );
    }

    // An already-canonical run with nothing dead offers no edit (the no-op the
    // action relies on to stay quiet).
    #[test]
    fn already_organized_offers_no_edit() {
        assert_no_edit(
            "import std::a;\nimport std::b;\nuse dep::x;\n\nfun main() {}\n",
            &[],
        );
    }

    // Block-scoped imports live in a block body, never the top-level list, so the
    // organizer leaves them entirely alone — order AND unused leaves both.
    #[test]
    fn block_scoped_imports_are_left_alone() {
        assert_no_edit(
            "fun f() {\n\tuse zeta::last;\n\tuse alpha::first;\n\tfirst();\n}\n",
            &["last"],
        );
    }

    // A whole run pruned away is deleted, taking its line break so no blank line
    // is left behind.
    #[test]
    fn a_fully_dead_run_is_deleted() {
        assert_eq!(
            organize("import std::dead;\nfun main() {}\n", &["dead"]),
            "fun main() {}\n",
        );
    }

    // Sort and prune compose: the run reorders and the dead leaf disappears in one
    // edit.
    #[test]
    fn sort_and_prune_compose() {
        assert_eq!(
            organize(
                "import std::z;\nimport std::a::{ keep, drop };\n",
                &["drop"],
            ),
            "import std::a::{ keep };\nimport std::z;\n",
        );
    }
}
