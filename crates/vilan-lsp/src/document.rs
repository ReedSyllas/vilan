//! Per-document analysis state and the navigation queries the language-server
//! handlers run against it: position→entity lookup, hover, go-to-definition,
//! find-references, and rename.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use vilan_core::analyzer::{DERIVED_SOURCE, Expr, Implementation, Parameter, SourceId};
use vilan_core::id::Id;
use vilan_core::lexing::tokenize;
use vilan_core::node::Convention;
use vilan_core::token::Token;
use vilan_core::type_::Type;
use vilan_core::{
    Error, Manifest, Platform as BuildPlatform, Program, Span, Workspace as BuildWorkspace,
    analyze_source,
};

use crate::line_index::LineIndex;

/// A file's project context, resolved from the nearest `vilan.toml`: the build
/// platform to analyze it against, and the package source root (where `import
/// pkg::..` siblings resolve). Either is `None` when there's no project (or the
/// file's role can't be determined) — analysis then infers the platform from the
/// file's imports and roots `pkg::` at the file's own directory.
struct ProjectContext {
    platform: Option<BuildPlatform>,
    pkg_root: Option<PathBuf>,
    /// The file's resolved dependency workspace (P2), so cross-package imports
    /// (`import <dep>::..`) type-check in the editor.
    workspace: BuildWorkspace,
}

impl ProjectContext {
    fn none() -> ProjectContext {
        ProjectContext {
            platform: None,
            pkg_root: None,
            workspace: BuildWorkspace::default(),
        }
    }
}

/// Resolves a file's [`ProjectContext`] from the nearest ancestor `vilan.toml`.
/// A `[package]` roots `pkg::` at its source `root`, analyzes its files against
/// its platform (the package `target`, or per-entry targets under the
/// `[entry.<name>]` form), and resolves its dependency workspace (so
/// cross-package imports type-check). Anything unreadable / unrecognized
/// yields [`ProjectContext::none`].
fn resolve_project_context(entry_path: &Path) -> ProjectContext {
    let mut directory = entry_path.parent();
    let (manifest_path, root) = loop {
        let Some(current) = directory else {
            return ProjectContext::none();
        };
        let candidate = current.join("vilan.toml");
        if candidate.is_file() {
            break (candidate, current);
        }
        directory = current.parent();
    };
    let Ok(contents) = std::fs::read_to_string(&manifest_path) else {
        return ProjectContext::none();
    };
    let Ok((manifest, _warnings)) = Manifest::parse(&contents) else {
        return ProjectContext::none();
    };

    // A package: root `pkg::` at its declared source root and resolve its
    // dependency workspace (best-effort — a resolution error degrades to no
    // deps). The platform: the classic single-entry form analyzes every file
    // under the root against the package target; a multi-entry package
    // (proposal/platform-coloring.md §4.2) analyzes an ENTRY file under its
    // declared target, and any other file with a platform inferred from its
    // own imports — a module may be reached from several entries, and having
    // no `main` it faces no admission walk, so the choice only affects
    // scratch-style inference (hover colors are platform-independent).
    if let Some(package) = &manifest.package {
        let pkg_root = root.join(package.root());
        let platform = if manifest.entries.is_empty() {
            let build_platform = package.resolved_target().unwrap_or_default();
            is_within(&pkg_root, entry_path).then_some(build_platform)
        } else {
            manifest.entries.iter().find_map(|(name, entry)| {
                same_file(&pkg_root.join(entry.path(name)), entry_path)
                    .then(|| entry.resolved_target().unwrap_or_default())
            })
        };
        let workspace = vilan_core::manifest::resolve_workspace(root).unwrap_or_default();
        return ProjectContext {
            platform,
            pkg_root: Some(pkg_root),
            workspace,
        };
    }

    // A `[project]` workspace root has no buildable package of its own.
    ProjectContext::none()
}

/// Whether two paths name the same file (canonicalizing when possible).
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Whether `file` lives within `directory` (canonicalizing when possible).
fn is_within(directory: &Path, file: &Path) -> bool {
    match (
        std::fs::canonicalize(directory),
        std::fs::canonicalize(file),
    ) {
        (Ok(directory), Ok(file)) => file.starts_with(directory),
        _ => file.starts_with(directory),
    }
}

/// A package source root for a file with no manifest: its own directory.
fn pkg_root_fallback(entry_path: &Path) -> PathBuf {
    entry_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

/// A content hash of a document's text, used to skip re-analysis when an edit
/// leaves the buffer byte-for-byte unchanged (undo/redo, a cursor-only change).
pub fn hash_text(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// What a use site ultimately refers to — the key for find-references / rename.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    /// A `let`/`mut` local or a parameter, by its binding id.
    Binding(Id),
    /// A struct field, by owning struct id and field index.
    Field(Id, usize),
    /// A method, by its function id (call sites carry a precise member span).
    Method(Id),
    /// A struct/enum/trait definition, by its id (uses live in `type_references`).
    Type(Id),
}

/// A kind of declaration, for the document outline.
pub enum SymbolKind {
    Function,
    Struct,
    Field,
    Enum,
    Trait,
}

/// One node in the document outline.
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// The whole declaration span.
    pub full: Span,
    /// The name span (must lie within `full`).
    pub selection: Span,
    pub children: Vec<Symbol>,
}

/// A scope-position construct snippet's insertion text (E14). The server
/// renders `body` for a snippet-capable client and falls back to `fallback` (the
/// bare keyword) otherwise — a `${1:…}` body would surface as literal text on a
/// client that cannot expand tab-stops.
pub struct SnippetInsertion {
    /// The `${n:…}`-tabstopped snippet body (LSP `InsertTextFormat::SNIPPET`).
    pub body: String,
    /// The plain keyword inserted when the client lacks snippet support.
    pub fallback: String,
}

/// A completion candidate offered at the cursor (mapped to an LSP `CompletionItem`
/// by the server).
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    /// The signature (functions/methods) or type (variables) shown in the
    /// completion popup's detail line — the same house rendering hover uses.
    /// `None` for keywords, macros, modules, types, and fields (WO-3: a field's
    /// type is not cheaply renderable from the analyzed `Program`).
    pub detail: Option<String>,
    /// The first paragraph of the declaration's `///` doc, where present.
    pub documentation: Option<String>,
    /// The parameter names (`self` excluded) when this candidate is a function
    /// or method that should insert call-shaped — `Some(names)`, possibly empty
    /// for a zero-parameter callable. `None` requires a bare-name insertion: a
    /// non-callable, a callee already followed by `(`, or a use/import path.
    /// The server (`to_completion_item`) turns this into the actual insert text
    /// per the `vilan.completion.functionCall` setting.
    pub call_parameters: Option<Vec<String>>,
    /// The template insertion when this candidate is a construct snippet
    /// (`CompletionKind::Snippet`, from [`CONSTRUCT_SNIPPETS`]); `None` for every
    /// other candidate (E14).
    pub snippet: Option<SnippetInsertion>,
}

impl Completion {
    /// A plain candidate — a bare-name insertion, no signature and no
    /// call-shaping (keywords, macros, fields, enum variants, type names).
    fn bare(label: String, kind: CompletionKind) -> Self {
        Completion {
            label,
            kind,
            detail: None,
            documentation: None,
            call_parameters: None,
            snippet: None,
        }
    }

    /// A construct-snippet candidate (E14): a distinguishing `label`, a short
    /// `detail`, the `${n:…}` `body`, and the bare `keyword` fallback for a
    /// client without snippet support. Offered alongside the bare keyword at
    /// scope positions only.
    fn snippet(label: &str, detail: &str, body: &str, keyword: &str) -> Self {
        Completion {
            label: label.to_string(),
            kind: CompletionKind::Snippet,
            detail: Some(detail.to_string()),
            documentation: None,
            call_parameters: None,
            snippet: Some(SnippetInsertion {
                body: body.to_string(),
                fallback: keyword.to_string(),
            }),
        }
    }
}

/// The category of a completion, for its editor icon.
pub enum CompletionKind {
    Macro,
    Function,
    Method,
    Field,
    Struct,
    Enum,
    EnumVariant,
    Trait,
    Variable,
    Module,
    Keyword,
    /// A fill-in-the-blanks construct template (E14) — a distinct icon from the
    /// bare keyword it accompanies.
    Snippet,
}

/// The vilan book's published base URL — keyword hovers deep-link into it.
const BOOK_BASE: &str = "https://reedsyllas.github.io/vilan/";

/// Every keyword the lexer classifies (`token.rs`), each with a one-line
/// meaning and a deep link into the book: `(keyword, sentence, page#anchor)`.
/// Semantics-bearing keywords point at the specification; the rest point where
/// the book teaches them best. The set is kept in lockstep with the lexer by
/// [`keyword_lexeme`], whose every keyword arm has an entry here.
const KEYWORD_DOCS: &[(&str, &str, &str)] = &[
    (
        "fun",
        "Declares a function.",
        "tour/functions-and-closures.html#functions",
    ),
    (
        "struct",
        "Declares a struct — a product type with named fields.",
        "tour/data-and-traits.html#structs",
    ),
    (
        "enum",
        "Declares an enum — a sum type whose value is one of several variants.",
        "tour/data-and-traits.html#enums",
    ),
    (
        "trait",
        "Declares a trait — a set of methods a type can implement.",
        "tour/data-and-traits.html#traits",
    ),
    (
        "impl",
        "Implements methods for a type — and, with a trait, that trait.",
        "tour/data-and-traits.html#impl--methods-and-statics",
    ),
    (
        "with",
        "Names the trait(s) an `impl` provides (or a trait's supertraits).",
        "spec/types.html#54-impls",
    ),
    (
        "type",
        "Declares a type alias.",
        "spec/types.html#53-declarations",
    ),
    (
        "external",
        "Declares a host (FFI) type or function — its surface comes from the host, not vilan.",
        "spec/types.html#53-declarations",
    ),
    (
        "macro",
        "Declares a macro — code that runs at compile time to produce code.",
        "spec/macros.html#101-declaring-and-invoking",
    ),
    (
        "const",
        "Evaluates an expression at compile time (`const expr`).",
        "spec/const.html#91-the-const-expression",
    ),
    (
        "import",
        "Loads a module and binds the named items into this module's scope.",
        "spec/names.html#43-imports",
    ),
    (
        "use",
        "Binds names from an already-visible type's namespace (variants, statics) without loading a module.",
        "spec/names.html#43-imports",
    ),
    (
        "export",
        "Re-exports a statement's names so importers see them as if declared here.",
        "spec/names.html#43-imports",
    ),
    ("mod", "Declares a submodule.", "spec/names.html#41-modules"),
    (
        "let",
        "Binds an immutable local or module-level value.",
        "tour/values-and-types.html#bindings",
    ),
    (
        "mut",
        "Binds a mutable value — one that can be reassigned.",
        "tour/values-and-types.html#bindings",
    ),
    (
        "own",
        "Passes a parameter by value as an owned copy; for a `resource` this moves ownership into the callee.",
        "spec/memory.html#63-rule-3--references-are-second-class-views",
    ),
    (
        "borrows",
        "Names which parameter a function returns a view into — the one sanctioned way a view escapes a function (often inferred).",
        "spec/memory.html#65-projections-borrows",
    ),
    (
        "resource",
        "An owned value with exactly one owner, moved rather than copied, and torn down at scope end.",
        "spec/memory.html#68-resources-and-destruction",
    ),
    (
        "if",
        "Chooses between branches; `if` is an expression that produces a value.",
        "tour/control-flow.html#if--else",
    ),
    (
        "else",
        "The alternative branch of an `if`.",
        "tour/control-flow.html#if--else",
    ),
    (
        "match",
        "Matches a value against patterns, taking it apart by shape.",
        "tour/control-flow.html#match",
    ),
    (
        "is",
        "Tests whether a value matches a pattern, yielding a bool.",
        "tour/control-flow.html#match",
    ),
    (
        "for",
        "Iterates over the elements of a collection (`for x in xs`).",
        "tour/control-flow.html#loops",
    ),
    (
        "in",
        "Separates the binder from the iterated collection in a `for` loop.",
        "tour/control-flow.html#loops",
    ),
    (
        "jump",
        "Transfers control within a loop: `jump break` or `jump continue`.",
        "tour/control-flow.html#loops",
    ),
    (
        "ret",
        "Returns early from a function.",
        "tour/control-flow.html#early-return-ret",
    ),
    (
        "async",
        "Spawns work without waiting for it (`async expr` / `async { … }`), yielding a `Task<T>`; ordinary calls are awaited for you.",
        "tour/async.html#opting-out-of-waiting-async-and-await",
    ),
    (
        "await",
        "Collects a `Task<T>` spawned with `async`; ordinary calls need no `await`.",
        "tour/async.html#opting-out-of-waiting-async-and-await",
    ),
    (
        "true",
        "The boolean literal `true`.",
        "tour/values-and-types.html#primitives",
    ),
    (
        "false",
        "The boolean literal `false`.",
        "tour/values-and-types.html#primitives",
    ),
    (
        "null",
        "The null literal — the sole value of the `null` type.",
        "tour/values-and-types.html#wheres-null",
    ),
];

/// The scope-position construct snippets (E14) — the shape-heavy declarations
/// offered as fill-in-the-blanks templates *alongside* their bare keyword.
/// Each row is `(keyword, label, detail, body)`: `label` is the popup's
/// distinguishing display, `detail` its one-line description, `body` the
/// `${n:…}`-tabstopped snippet, and `keyword` both the lexer keyword this rides
/// and the plain-text fallback for a client without snippet support. The bodies
/// follow house style — tab indent, trailing comma, `i32` — verified against the
/// corpus. Growth is one row; each keyword stays a subset of the lexer's, pinned
/// by `construct_snippet_keywords_are_lexer_keywords`.
const CONSTRUCT_SNIPPETS: &[(&str, &str, &str, &str)] = &[
    (
        "for",
        "for … in { }",
        "iterate over a collection",
        "for ${1:item} in ${2:items} {\n\t$0\n}",
    ),
    (
        "fun",
        "fun … ( ) { }",
        "declare a function",
        "fun ${1:name}(${2}) {\n\t$0\n}",
    ),
    (
        "struct",
        "struct … { }",
        "declare a struct",
        "struct ${1:Name} {\n\t${2:field}: ${3:i32},\n}",
    ),
    (
        "match",
        "match … { }",
        "match on a value",
        "match ${1:subject} {\n\t${2:pattern} => $0,\n}",
    ),
];

/// The keyword lexeme a token spells, or `None` for non-keyword tokens
/// (identifiers, literals, operators, punctuation). Exhaustive over `Token`
/// deliberately: a new keyword variant must be classified here, which forces
/// the matching [`KEYWORD_DOCS`] entry it needs.
fn keyword_lexeme(token: &Token) -> Option<&'static str> {
    Some(match token {
        Token::Async => "async",
        Token::Await => "await",
        Token::Const => "const",
        Token::Else => "else",
        Token::Enum => "enum",
        Token::Export => "export",
        Token::External => "external",
        Token::Bool(true) => "true",
        Token::Bool(false) => "false",
        Token::For => "for",
        Token::Fun => "fun",
        Token::If => "if",
        Token::Impl => "impl",
        Token::Import => "import",
        Token::In => "in",
        Token::Is => "is",
        Token::Jump => "jump",
        Token::Let => "let",
        Token::Macro => "macro",
        Token::Match => "match",
        Token::Mod => "mod",
        Token::Mut => "mut",
        Token::Null => "null",
        Token::Own => "own",
        Token::Borrows => "borrows",
        Token::Ret => "ret",
        Token::Resource => "resource",
        Token::Struct => "struct",
        Token::Trait => "trait",
        Token::Type => "type",
        Token::Use => "use",
        Token::With => "with",
        Token::Ident(_)
        | Token::Ctrl(_)
        | Token::Number(_, _, _)
        | Token::Op(_)
        | Token::String(_)
        | Token::MultilineString(_) => return None,
    })
}

/// A parameter's signature fragment for hover, with its declared calling
/// convention: `own x: T`, `x: &T`, `x: &mut T`, or the plain `x: T`. The `&` /
/// `&mut` live on the convention (rule 3), not in `type_label`, so they are
/// prepended here; `self` renders in its convention-specific self form.
fn parameter_signature(parameter: &Parameter, type_label: &str) -> String {
    if parameter.name == "self" {
        return match parameter.convention {
            Convention::Bare => "self".to_string(),
            Convention::Own => "own self".to_string(),
            Convention::Ref => "&self".to_string(),
            Convention::RefMut => "&mut self".to_string(),
        };
    }
    match parameter.convention {
        Convention::Bare => format!("{}: {type_label}", parameter.name),
        Convention::Own => format!("own {}: {type_label}", parameter.name),
        Convention::Ref => format!("{}: &{type_label}", parameter.name),
        Convention::RefMut => format!("{}: &mut {type_label}", parameter.name),
    }
}

/// The pre-rendered signature of the function/external at `target` — the same
/// string hover fences — with the inferred `async` prepended, mirroring
/// [`Document::compose_hover`]. `target` is a function DEFINITION id (resolve a
/// use site through [`Document::function_target`] first). `None` when the id
/// names no declaration.
fn signature_label(program: &Program, target: Id) -> Option<String> {
    let declaration = program.declaration_labels.get(&target)?;
    if program.async_functions.contains(&target) && !declaration.starts_with("async ") {
        Some(format!("async {declaration}"))
    } else {
        Some(declaration.clone())
    }
}

/// The parameter names of the function/external at `target`, in order, with
/// `self` dropped (the receiver is not a call argument) — the tab-stop labels a
/// call-shaped completion fills. `Some(vec![])` for a zero-parameter callable;
/// `None` when the id is not a function or external. `target` is a DEFINITION
/// id (resolve through [`Document::function_target`] first).
fn call_parameter_names(program: &Program, target: Id) -> Option<Vec<String>> {
    let parameter_ids = if let Some(function) = program.functions.get(&target) {
        &function.parameters
    } else if let Some(external) = program.external_functions.get(&target) {
        &external.parameters
    } else {
        return None;
    };
    Some(
        parameter_ids
            .iter()
            .filter_map(|parameter_id| program.parameters.get(parameter_id))
            .filter(|parameter| parameter.name != "self")
            .map(|parameter| parameter.name.to_string())
            .collect(),
    )
}

/// Whether `offset` sits inside a `use`/`import` item — where a name is being
/// bound into scope, not called, so even a function completes to a bare name
/// (`use std::math::sqrt`, not `sqrt(…)`). Imports are single-line,
/// newline-terminated items, so this reads the current line's leading keyword
/// (a leading `export` prefix — `export import …` — is skipped). Multi-line
/// braced groups past their first line are not recognized; the corpus has none.
fn in_import_path(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map(|at| at + 1).unwrap_or(0);
    let mut words = text[line_start..offset].split_whitespace();
    let first = words.next().unwrap_or("");
    let keyword = if first == "export" {
        words.next().unwrap_or("")
    } else {
        first
    };
    keyword == "import" || keyword == "use"
}

pub struct Document {
    pub line_index: LineIndex,
    pub program: Option<Program<'static>>,
    /// Evaluated `const` results (E9: hover shows a constant's VALUE). The
    /// evaluation is fuel-capped and skips itself on any diagnostic, so a
    /// broken document costs nothing.
    pub const_results: std::collections::HashMap<Id, vilan_core::interpreter::ConstValue>,
    pub diagnostics: Vec<Error>,
    /// The source file each diagnostic belongs to, parallel to `diagnostics`
    /// (`SourceId(0)` = this document; imported modules publish to their own
    /// files — backlog E1).
    pub diagnostic_sources: Vec<SourceId>,
    /// Non-fatal diagnostics (`[must_use]` drops) — published at Warning severity.
    pub warnings: Vec<Error>,
    /// The buffer text as of the last edit — kept so a dependent re-analysis
    /// (another open file changed) can re-run this document without the editor
    /// resending its content.
    pub text: String,
    /// A hash of the source text this document was analyzed from, so an edit that
    /// leaves the buffer unchanged can skip re-analysis.
    pub text_hash: u64,
    /// `(start, end, id)` for every entry-file entity with a real span, used to
    /// find the innermost entity under a cursor.
    entity_spans: Vec<(usize, usize, Id)>,
    /// Per-function platform requirements (`platform_color::requirements`),
    /// rendered lines like ``requires the `process` layer of `std` (via `…`)``
    /// — appended to the hover of any function that carries one.
    platform_requirements: HashMap<Id, String>,
}

/// A semantic-token classification (E2): precision highlighting from the
/// ANALYZED program, over TextMate's regex approximations. The discriminant
/// order IS the LSP legend order (`TOKEN_TYPES`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    Namespace,
    Struct,
    Enum,
    Interface,
    TypeParameter,
    Parameter,
    Variable,
    Function,
    Method,
    Property,
    EnumMember,
    Macro,
}

/// Token-modifier bits, index-aligned with `TOKEN_MODIFIERS`.
pub const MODIFIER_DECLARATION: u32 = 1 << 0;
pub const MODIFIER_READONLY: u32 = 1 << 1;

/// The modifier legend.
pub const TOKEN_MODIFIERS: [&str; 2] = ["declaration", "readonly"];

/// The LSP legend, index-aligned with `TokenKind`.
pub const TOKEN_TYPES: [&str; 12] = [
    "namespace",
    "struct",
    "enum",
    "interface",
    "typeParameter",
    "parameter",
    "variable",
    "function",
    "method",
    "property",
    "enumMember",
    "macro",
];

/// One diagnostic as the language server publishes it: the file it belongs to
/// (`None` = the analyzed document itself), its span *in that file's text*, the
/// message, and the severity. LSP-type-free so the grouping is unit-testable.
pub struct PublishedDiagnostic {
    pub path: Option<PathBuf>,
    pub span: Span,
    pub message: String,
    pub warning: bool,
    /// The diagnostic's secondary note (diagnostics-standard.md C3): span,
    /// message, and the note's own file when it lives elsewhere (`None` =
    /// the diagnostic's file) — published as LSP related information.
    pub note: Option<(Span, String, Option<PathBuf>)>,
}

/// The span of an entity, flattened from the `&Span` stored in `span_map`.
fn span_of(program: &Program, id: Id) -> Option<Span> {
    program.span_map.get(&id).map(|span| **span)
}

/// Whether an expression is a value-position use of the definition `def_id` — the
/// forms the entity map records for a resolved name (a call subject, a bare value,
/// an enum variant). Used by the Organize Imports prune to keep a value-used
/// import. An enum-variant expression carries its enum's Id.
fn expr_references_definition(expr: &Expr, def_id: Id) -> bool {
    match expr {
        Expr::Local(id)
        | Expr::Function(id)
        | Expr::ExternalFunction(id)
        | Expr::Struct(id)
        | Expr::Enum(id)
        | Expr::Trait(id)
        | Expr::Module(id)
        | Expr::EnumVariant(id, _) => *id == def_id,
        _ => false,
    }
}

impl Document {
    pub fn analyze(text: &str, std_dir: &Path, entry_path: &Path) -> Self {
        // The pipeline recurses deeply (chumsky), and macro-world compiles NEST
        // a full analysis inside the analysis — run the whole thing on a
        // dedicated big-stack thread, like the CLI's compiler thread. Callers
        // stay synchronous (the LSP already wraps this in spawn_blocking).
        let text = text.to_string();
        let std_dir = std_dir.to_path_buf();
        let entry_path = entry_path.to_path_buf();
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(move || Self::analyze_on_this_thread(&text, &std_dir, &entry_path))
            .expect("spawn analysis thread")
            .join()
            .expect("analysis thread panicked")
    }

    fn analyze_on_this_thread(text: &str, std_dir: &Path, entry_path: &Path) -> Self {
        let line_index = LineIndex::new(text);
        let text_hash = hash_text(text);
        // The program borrows its source for `'static`, so leak a copy (the
        // editor re-analyzes on change; see the known leak tradeoff).
        let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
        vilan_core::leak_tally::record(
            vilan_core::leak_tally::LeakSite::LspEntryText,
            leaked.len(),
        );
        // Prefer the project's declared platform and source root (the file's role in
        // its `vilan.toml`); fall back to inferring the platform from imports and
        // rooting `pkg::` at the file's own directory.
        let context = resolve_project_context(entry_path);
        let pkg_root = context
            .pkg_root
            .unwrap_or_else(|| pkg_root_fallback(entry_path));
        // `std` is resolved as a library (its layered roots) from the std directory
        // — the manifest when present, else a bare base layer (L2).
        let std = vilan_core::manifest::resolve_std(std_dir);
        let (program, diagnostics) = analyze_source(
            leaked,
            &std,
            &pkg_root,
            entry_path,
            context.platform,
            &context.workspace,
        );

        let mut entity_spans = Vec::new();
        if let Some(program) = &program {
            for (id, span) in &program.span_map {
                if program.source_of(*id) != Some(SourceId(0)) {
                    continue;
                }
                let range = span.into_range();
                if range.start < range.end {
                    entity_spans.push((range.start, range.end, *id));
                }
            }
        }

        // `diagnostics` = the entry's own lex/parse errors, then the program's
        // (see `analyze_source`) — so the source list is an entry-attributed
        // prefix followed by the program's per-diagnostic attribution.
        let program_diagnostics = program
            .as_ref()
            .map(|program| program.diagnostics.len())
            .unwrap_or(0);
        let mut diagnostic_sources =
            vec![SourceId(0); diagnostics.len().saturating_sub(program_diagnostics)];
        if let Some(program) = &program {
            diagnostic_sources.extend(program.diagnostic_sources.iter().copied());
        }
        let warnings = program
            .as_ref()
            .map(|program| program.warnings.clone())
            .unwrap_or_default();
        let platform_requirements = program
            .as_ref()
            .map(vilan_core::platform_color::requirements)
            .unwrap_or_default();
        // Evaluate `const`s so hover can show their VALUES (E9). `evaluate`
        // skips itself on any diagnostic and is fuel-capped; its own errors
        // are already published by the build, so the editor drops them here
        // (squiggling them is the build's job — same wording, same spans).
        let const_results = program
            .as_ref()
            .map(|program| {
                vilan_core::const_eval::evaluate(
                    program,
                    &vilan_core::options::BuildOptions::default(),
                )
                .0
            })
            .unwrap_or_default();

        Document {
            line_index,
            program,
            const_results,
            diagnostics,
            diagnostic_sources,
            warnings,
            text: text.to_string(),
            text_hash,
            entity_spans,
            platform_requirements,
        }
    }

    /// The document's diagnostics grouped for publishing: errors attributed to
    /// the file they occurred in (`None` = this document), plus this document's
    /// warnings. Diagnostics from generated (derive) code carry template spans
    /// that map to no file — they attach to the entry at offset 0, labeled.
    pub fn published_diagnostics(&self) -> Vec<PublishedDiagnostic> {
        let mut published = Vec::new();
        for (index, error) in self.diagnostics.iter().enumerate() {
            let source = self
                .diagnostic_sources
                .get(index)
                .copied()
                .unwrap_or(SourceId(0));
            if source == SourceId(0) {
                published.push(PublishedDiagnostic {
                    path: None,
                    span: error.span,
                    message: error.msg.clone(),
                    warning: false,
                    note: error.note.as_ref().map(|note| {
                        let note_path = note
                            .source
                            .and_then(|source| self.program.as_ref()?.source_path(source))
                            .map(Path::to_path_buf);
                        (note.span, note.msg.clone(), note_path)
                    }),
                });
            } else if source == DERIVED_SOURCE {
                published.push(PublishedDiagnostic {
                    path: None,
                    span: Span::from(0..0),
                    message: format!("(in generated code) {}", error.msg),
                    warning: false,
                    note: None,
                });
            } else {
                let path = self
                    .program
                    .as_ref()
                    .and_then(|program| program.source_path(source))
                    .map(Path::to_path_buf);
                match path {
                    Some(path) => published.push(PublishedDiagnostic {
                        path: Some(path),
                        span: error.span,
                        message: error.msg.clone(),
                        warning: false,
                        note: None,
                    }),
                    // An unknown source (shouldn't happen): keep the error
                    // visible on the entry rather than dropping it.
                    None => published.push(PublishedDiagnostic {
                        path: None,
                        span: Span::from(0..0),
                        message: error.msg.clone(),
                        warning: false,
                        note: None,
                    }),
                }
            }
        }
        for warning in &self.warnings {
            published.push(PublishedDiagnostic {
                path: None,
                span: warning.span,
                message: warning.msg.clone(),
                warning: true,
                note: None,
            });
        }
        published
    }

    /// Updates the document's text (its line index) without re-analyzing — applied
    /// on every edit so position-based queries (notably completion's context scan)
    /// see the just-typed character immediately, while the heavier re-analysis
    /// stays debounced. `text_hash` is deliberately left at the last *analyzed*
    /// text so the pending re-analysis still fires.
    pub fn set_text(&mut self, text: &str) {
        self.line_index = LineIndex::new(text);
        self.text = text.to_string();
    }

    /// The innermost entry-file entity whose span contains `offset`.
    fn entity_at(&self, offset: usize) -> Option<Id> {
        self.entity_spans
            .iter()
            .filter(|(start, end, _)| *start <= offset && offset < *end)
            .min_by_key(|(start, end, _)| end - start)
            .map(|(_, _, id)| *id)
    }

    /// The hover for the entity under `offset` (E9): a fenced full
    /// declaration when the entity names one (function signature — with
    /// inferred `async` prepended — or a struct/enum block), the
    /// declaration's leading `//` comment as prose, and the platform
    /// requirement line where one is inferred. Anything else keeps its
    /// rendered type.
    pub fn hover(&self, offset: usize) -> Option<String> {
        // A keyword under the cursor: its one-line meaning + a book link. This
        // is purely lexical, so it works even when analysis produced no program
        // (a keyword hovers on a document that doesn't yet compile).
        if let Some(keyword) = self.keyword_hover(offset) {
            return Some(keyword);
        }
        let program = self.program.as_ref()?;
        // A type name in type position: the full declaration when known.
        if let Some((definition, label)) = self.type_reference_at(program, offset) {
            if let Some(definition) = definition {
                if let Some(declaration) = program.declaration_labels.get(&definition) {
                    return Some(self.compose_hover(program, definition, declaration, None));
                }
            }
            return Some(label);
        }
        let id = self.entity_at(offset)?;
        // A function (or requirement-carrying binding): the full signature.
        if let Some(target) = self.function_target(program, id) {
            let requirement = self.platform_requirements.get(&target).cloned();
            if let Some(declaration) = program.declaration_labels.get(&target) {
                return Some(self.compose_hover(program, target, declaration, requirement));
            }
        }
        // A struct/enum name in value position (a constructor, a variant).
        if let Some(definition) = self.type_declaration_target(program, id) {
            if let Some(declaration) = program.declaration_labels.get(&definition) {
                return Some(self.compose_hover(program, definition, declaration, None));
            }
        }
        // A variable (`let`/`mut`, local or module-level, or a destructured
        // binder) or a parameter: its typed declaration, else the bare type.
        let type_label = self.binding_hover(program, id).or_else(|| {
            self.hover_label(program, id).map(|label| {
                // A constant shows its VALUE beside its type (E9).
                match self.const_value_label(program, id) {
                    Some(value) => format!("{label} = {value}"),
                    None => label,
                }
            })
        });
        let requirement = self
            .function_target(program, id)
            .and_then(|function| self.platform_requirements.get(&function))
            .cloned();
        match (type_label, requirement) {
            // A blank markdown line, so the requirement renders as its own
            // paragraph under the type.
            (Some(type_label), Some(requirement)) => Some(format!("{type_label}\n\n{requirement}")),
            (Some(type_label), None) => Some(type_label),
            (None, requirement) => requirement,
        }
    }

    /// Assembles a declaration hover: the fenced declaration (with inferred
    /// `async` prepended to a function signature), its leading `//` doc
    /// block, and the platform requirement, each as its own paragraph.
    fn compose_hover(
        &self,
        program: &Program,
        declaration_id: Id,
        declaration: &str,
        requirement: Option<String>,
    ) -> String {
        let declaration = if program.async_functions.contains(&declaration_id)
            && !declaration.starts_with("async ")
        {
            format!("async {declaration}")
        } else {
            declaration.to_string()
        };
        let mut out = format!("```vilan\n{declaration}\n```");
        if let Some(docs) = self.doc_comment_of(program, declaration_id) {
            out.push_str("\n\n");
            out.push_str(&docs);
        }
        if let Some(requirement) = requirement {
            out.push_str("\n\n");
            out.push_str(&requirement);
        }
        out
    }

    /// The hover for a keyword under `offset`: a one-line meaning and a deep
    /// link into the book. Lexes the buffer (cheap, hover is a glance) and
    /// classifies the token whose span contains the cursor — only a keyword
    /// token yields a hover, so a string literal like `"fun"` never does.
    fn keyword_hover(&self, offset: usize) -> Option<String> {
        let (tokens, _errors) = tokenize(&self.text);
        let (token, _span) = tokens.iter().find(|(_, span)| {
            let range = span.into_range();
            range.start <= offset && offset < range.end
        })?;
        let lexeme = keyword_lexeme(token)?;
        let (_, sentence, path) = KEYWORD_DOCS
            .iter()
            .find(|(keyword, _, _)| *keyword == lexeme)?;
        Some(format!(
            "**`{lexeme}`** — {sentence}\n\n[The vilan book →]({BOOK_BASE}{path})"
        ))
    }

    /// The hover for a `let`/`mut` variable or a parameter under the cursor,
    /// rendered as a fenced declaration in the house style: `let name: T` /
    /// `mut name: T` for a variable (its `///` doc appended), and the
    /// convention-carrying `own x: T` / `x: &mut T` / `x: T` for a parameter
    /// (a function-typed parameter shows its `|A| R` closure shape). A use site
    /// resolves through to its binding, so both the declaration and every use
    /// hover the same. The type is the resolved label the analyzer pre-rendered
    /// (`expr_types`) — the element type for a destructured binder. Returns
    /// `None` for anything that is not a binding, leaving the bare-type path.
    fn binding_hover(&self, program: &Program, id: Id) -> Option<String> {
        let binding = match program.entity_map.get(&id) {
            Some(Expr::Local(inner) | Expr::Variable(inner) | Expr::Parameter(inner)) => *inner,
            _ => id,
        };
        if let Some(variable) = program.variables.get(&binding) {
            let type_label = program.expr_types.get(&binding)?;
            let keyword = if variable.mutable { "mut" } else { "let" };
            let mut signature = format!("{keyword} {}: {type_label}", variable.name);
            // A `const`-initialized binding shows its evaluated VALUE too (E9).
            if let Some(value) = self.const_value_label(program, binding) {
                signature.push_str(&format!(" = {value}"));
            }
            let mut out = format!("```vilan\n{signature}\n```");
            if let Some(docs) = self.doc_comment_of(program, binding) {
                out.push_str("\n\n");
                out.push_str(&docs);
            }
            return Some(out);
        }
        if let Some(parameter) = program.parameters.get(&binding) {
            let type_label = program.expr_types.get(&binding)?;
            return Some(format!(
                "```vilan\n{}\n```",
                parameter_signature(parameter, type_label)
            ));
        }
        None
    }

    /// The struct/enum definition an entity names in VALUE position — a
    /// constructor, a bare type reference, or an enum variant.
    fn type_declaration_target(&self, program: &Program, id: Id) -> Option<Id> {
        if program.structs.contains_key(&id) || program.enums.contains_key(&id) {
            return Some(id);
        }
        match program.entity_map.get(&id)? {
            Expr::Struct(struct_id) => Some(*struct_id),
            Expr::StructInitializer(initializer_id, _) => program
                .struct_initializer_to_def
                .get(initializer_id)
                .copied(),
            Expr::Enum(enum_id) | Expr::EnumVariant(enum_id, _) => Some(*enum_id),
            _ => None,
        }
    }

    /// The contiguous `//` block directly above a declaration's name line —
    /// its doc comment, with the comment markers stripped. Attribute lines
    /// (`[must_use]`, `[platform(…)]`) between the block and the name are
    /// skipped. The entry file reads from the open buffer; other sources
    /// read from disk on demand (hover-time, cheap).
    fn doc_comment_of(&self, program: &Program, declaration_id: Id) -> Option<String> {
        let source = program.source_of(declaration_id)?;
        let name_span = self.definition_name_span(program, declaration_id)?;
        let owned;
        let text: &str = if source == SourceId(0) {
            &self.text
        } else {
            let path = program.source_path(source)?;
            owned = std::fs::read_to_string(path).ok()?;
            &owned
        };
        let start = name_span.into_range().start.min(text.len());
        let head = &text[..start];
        let mut lines: Vec<&str> = head.lines().collect();
        // Drop the (partial) declaration line itself.
        lines.pop();
        // Skip attribute and modifier-only lines between docs and the name.
        while let Some(last) = lines.last() {
            let trimmed = last.trim();
            if trimmed.starts_with('[') || trimmed == "async" || trimmed == "external" {
                lines.pop();
            } else {
                break;
            }
        }
        // `///` is the doc-comment syntax (user decision, 2026-07-16); a
        // plain `//` block is an implementation note and never surfaces.
        let mut docs: Vec<String> = Vec::new();
        while let Some(last) = lines.last() {
            let trimmed = last.trim();
            let Some(comment) = trimmed.strip_prefix("///") else {
                break;
            };
            docs.push(comment.strip_prefix(' ').unwrap_or(comment).to_string());
            lines.pop();
        }
        if docs.is_empty() {
            return None;
        }
        docs.reverse();
        Some(docs.join("\n"))
    }

    /// The first paragraph of a declaration's `///` doc — up to the first blank
    /// line — for a completion item's brief documentation (WO-3). `None` when
    /// there is no doc.
    fn doc_first_paragraph(&self, program: &Program, declaration_id: Id) -> Option<String> {
        let docs = self.doc_comment_of(program, declaration_id)?;
        let paragraph = docs.split("\n\n").next().unwrap_or(&docs).trim();
        if paragraph.is_empty() {
            None
        } else {
            Some(paragraph.to_string())
        }
    }

    /// The requirement-carrying entity the cursor *names*, if any: a function
    /// declaration name, a binding that resolves to a function or to a
    /// module-level binding with a requirement (its initializer is code), or
    /// a call's callee (including method calls, whose wired subject is a
    /// `Local` pointing at the resolved method). Deliberately strict — a
    /// local holding a function's *result* names nothing; only ids the
    /// requirements map actually knows can surface a line.
    fn function_target(&self, program: &Program, id: Id) -> Option<Id> {
        let carries_requirement = |id: &Id| {
            program.functions.contains_key(id)
                || program.external_functions.contains_key(id)
                || self.platform_requirements.contains_key(id)
        };
        if carries_requirement(&id) {
            return Some(id);
        }
        match program.entity_map.get(&id)? {
            Expr::Local(binding) | Expr::Variable(binding) | Expr::Parameter(binding) => {
                carries_requirement(binding).then_some(*binding)
            }
            Expr::Function(function_id) | Expr::ExternalFunction(function_id) => Some(*function_id),
            Expr::Call(call_id) => {
                let subject = program.function_calls.get(call_id)?.subject_id;
                self.function_target(program, subject)
            }
            _ => None,
        }
    }

    /// A constant's evaluated value for hover (`= 42`), when `id` is (or
    /// names) a binding whose initializer is a `const` expression the
    /// evaluation resolved. Rendered compactly and truncated — hover is a
    /// glance, not a dump.
    fn const_value_label(&self, program: &Program, id: Id) -> Option<String> {
        use vilan_core::analyzer::Expr;
        let binding = match program.entity_map.get(&id)? {
            Expr::Local(binding) | Expr::Variable(binding) => *binding,
            _ => id,
        };
        let initial = program.variables.get(&binding)?.initial?;
        let value = self.const_results.get(&initial)?;
        fn render(value: &vilan_core::interpreter::ConstValue, out: &mut String) {
            use vilan_core::interpreter::ConstValue;
            match value {
                ConstValue::Undefined => out.push_str("undefined"),
                ConstValue::Null => out.push_str("null"),
                ConstValue::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
                ConstValue::Number(value) => out.push_str(&value.to_string()),
                ConstValue::BigInt(value) => {
                    out.push_str(&value.to_string());
                    out.push('n');
                }
                ConstValue::Str(value) => {
                    out.push('"');
                    out.push_str(value);
                    out.push('"');
                }
                ConstValue::Array(items) => {
                    out.push('[');
                    for (index, item) in items.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        render(item, out);
                        if out.len() > 120 {
                            out.push('…');
                            break;
                        }
                    }
                    out.push(']');
                }
                ConstValue::Set(items) => {
                    out.push_str("Set[");
                    for (index, item) in items.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        render(item, out);
                        if out.len() > 120 {
                            out.push('…');
                            break;
                        }
                    }
                    out.push(']');
                }
                ConstValue::Map(entries) => {
                    out.push_str("Map[");
                    for (index, (key, entry)) in entries.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        render(key, out);
                        out.push_str(": ");
                        render(entry, out);
                        if out.len() > 120 {
                            out.push('…');
                            break;
                        }
                    }
                    out.push(']');
                }
            }
        }
        let mut rendered = String::new();
        render(value, &mut rendered);
        if rendered.len() > 160 {
            rendered.truncate(160);
            rendered.push('…');
        }
        Some(rendered)
    }

    fn hover_label(&self, program: &Program, id: Id) -> Option<String> {
        if let Some(label) = program.expr_types.get(&id) {
            return Some(label.clone());
        }
        // A bare use carries no type on its own id; resolve through its binding
        // (and through that binding's own kind, e.g. an imported enum variant).
        match program.entity_map.get(&id)? {
            Expr::Local(binding) | Expr::Variable(binding) | Expr::Parameter(binding) => program
                .expr_types
                .get(binding)
                .cloned()
                .or_else(|| self.hover_label(program, *binding)),
            Expr::EnumVariant(enum_id, _) => program
                .enums
                .get(enum_id)
                .map(|e| format!("enum {}", e.name)),
            // A constructor / call: hover the thing being called (e.g. `Ok(x)`
            // shows the enum) when the call's own result type isn't recorded.
            Expr::Call(call_id) => {
                let subject = program.function_calls.get(call_id)?.subject_id;
                self.hover_label(program, subject)
            }
            _ => None,
        }
    }

    /// The definition location `(file, span)` for the entity under `offset`.
    pub fn definition(&self, offset: usize) -> Option<(SourceId, Span)> {
        let program = self.program.as_ref()?;
        // A type name in type position resolves straight to its definition (type
        // references aren't entities). Being inside one but with no navigable
        // target (a generic) yields nothing rather than falling through.
        if let Some((definition, _)) = self.type_reference_at(program, offset) {
            let definition = definition?;
            return Some((
                program.source_of(definition)?,
                self.definition_name_span(program, definition)?,
            ));
        }
        let id = self.entity_at(offset)?;
        self.definition_of(program, id)
    }

    /// The span to jump to for a definition id: the declaration's *name* for a
    /// type/function/variable (else its whole span, e.g. a module's file start).
    fn definition_name_span(&self, program: &Program, id: Id) -> Option<Span> {
        if let Some(structure) = program.structs.get(&id) {
            return Some(structure.name_span);
        }
        if let Some(enumeration) = program.enums.get(&id) {
            return Some(enumeration.name_span);
        }
        if let Some(trait_definition) = program.traits.get(&id) {
            return Some(trait_definition.name_span);
        }
        if let Some(function) = program.functions.get(&id) {
            return Some(function.name_span);
        }
        if let Some(function) = program.external_functions.get(&id) {
            return Some(function.name_span);
        }
        if let Some(variable) = program.variables.get(&id) {
            return Some(variable.name_span);
        }
        span_of(program, id)
    }

    /// The innermost type reference under `offset` in the open file, as
    /// `(definition id, label)`.
    /// Inlay type hints: `: T` after each UNANNOTATED binding whose type
    /// resolved — inference made a decision the source doesn't show, so the
    /// editor shows it in place. Sorted by position.
    pub fn inlay_hints(&self) -> Vec<(usize, String)> {
        let Some(program) = &self.program else {
            return Vec::new();
        };
        let mut hints: Vec<(usize, String)> = Vec::new();
        for (id, variable) in &program.variables {
            if variable.annotated || program.source_of(*id) != Some(SourceId(0)) {
                continue;
            }
            let Some(label) = program.expr_types.get(id) else {
                continue;
            };
            if label.is_empty() || label == "?" || label.contains("Unknown") {
                continue;
            }
            let range = variable.name_span.into_range();
            if range.is_empty() {
                continue;
            }
            hints.push((range.end, format!(": {label}")));
        }
        hints.sort();
        hints
    }

    /// The entry document's semantic tokens (E2), name-sized and
    /// non-overlapping, sorted by position. Classification comes from the
    /// ANALYZED program: declaration name spans, identifier-sized reference
    /// entities, method-call name spans, and type-position references (whose
    /// definitions also cover macro names — they share trait names by design,
    /// and only semantics can tell them apart).
    pub fn semantic_tokens(&self) -> Vec<(Span, TokenKind, u32)> {
        let Some(program) = &self.program else {
            return Vec::new();
        };
        let entry = |id: Id| program.source_of(id) == Some(SourceId(0));
        let mut tokens: Vec<(Span, TokenKind, u32)> = Vec::new();
        let classify_target = |target: Id| -> TokenKind {
            use vilan_core::analyzer::Expr;
            match program.entity_map.get(&target) {
                Some(Expr::Function(_)) | Some(Expr::ExternalFunction(_)) => TokenKind::Function,
                Some(Expr::Struct(_)) => TokenKind::Struct,
                Some(Expr::Enum(_)) => TokenKind::Enum,
                Some(Expr::EnumVariant(_, _)) => TokenKind::EnumMember,
                Some(Expr::Trait(_)) => TokenKind::Interface,
                Some(Expr::Module(_)) => TokenKind::Namespace,
                Some(Expr::Generic(_)) => TokenKind::TypeParameter,
                Some(Expr::Macro) => TokenKind::Macro,
                _ => {
                    if program.parameters.contains_key(&target) {
                        TokenKind::Parameter
                    } else {
                        TokenKind::Variable
                    }
                }
            }
        };
        // Declaration names.
        for (id, function) in &program.functions {
            if entry(*id) {
                tokens.push((
                    function.name_span,
                    TokenKind::Function,
                    MODIFIER_DECLARATION,
                ));
            }
        }
        for (id, struct_) in &program.structs {
            if entry(*id) {
                tokens.push((struct_.name_span, TokenKind::Struct, MODIFIER_DECLARATION));
            }
        }
        for (id, enum_) in &program.enums {
            if entry(*id) {
                tokens.push((enum_.name_span, TokenKind::Enum, MODIFIER_DECLARATION));
            }
        }
        for (id, trait_) in &program.traits {
            if entry(*id) {
                tokens.push((trait_.name_span, TokenKind::Interface, MODIFIER_DECLARATION));
            }
        }
        for (id, variable) in &program.variables {
            if entry(*id) {
                let readonly = if variable.mutable {
                    0
                } else {
                    MODIFIER_READONLY
                };
                tokens.push((
                    variable.name_span,
                    TokenKind::Variable,
                    MODIFIER_DECLARATION | readonly,
                ));
            }
        }
        for (id, _parameter) in &program.parameters {
            // A parameter entity's `span_map` entry IS its name.
            if entry(*id)
                && let Some(span) = span_of(program, *id)
            {
                tokens.push((span, TokenKind::Parameter, MODIFIER_DECLARATION));
            }
        }
        // Identifier-sized reference entities.
        {
            use vilan_core::analyzer::Expr;
            for (id, expr) in &program.entity_map {
                if !entry(*id) {
                    continue;
                }
                let Some(span) = span_of(program, *id) else {
                    continue;
                };
                let range = span.into_range();
                if range.start >= range.end {
                    continue;
                }
                match expr {
                    Expr::Local(target) => {
                        let readonly = match program.variables.get(target) {
                            Some(variable) if !variable.mutable => MODIFIER_READONLY,
                            _ => 0,
                        };
                        tokens.push((span, classify_target(*target), readonly));
                    }
                    Expr::Generic(_) => tokens.push((span, TokenKind::TypeParameter, 0)),
                    Expr::Module(_) => tokens.push((span, TokenKind::Namespace, 0)),
                    _ => {}
                }
            }
        }
        // Method-call names — a member with a call is a method, a plain
        // member read is a property (field).
        for (call_id, span) in &program.member_name_spans {
            if !entry(*call_id) {
                continue;
            }
            let kind = if program.function_calls.contains_key(call_id) {
                TokenKind::Method
            } else {
                TokenKind::Property
            };
            tokens.push((*span, kind, 0));
        }
        // Type-position references (macro names arrive here too).
        for (source, span, definition, _) in &program.type_references {
            if *source != SourceId(0) {
                continue;
            }
            // A reference with no resolved definition (an unresolved or
            // synthetic segment) stays untokenized — TextMate's base layer
            // keeps whatever it had.
            let Some(kind) = definition.map(classify_target) else {
                continue;
            };
            tokens.push((*span, kind, 0));
        }
        // Sort and drop overlaps: narrowest-first at each start, then keep
        // strictly non-overlapping tokens (the LSP requires it).
        tokens.sort_by_key(|(span, _, _)| {
            let range = span.into_range();
            (range.start, range.end - range.start)
        });
        let mut kept: Vec<(Span, TokenKind, u32)> = Vec::new();
        let mut last_end = 0usize;
        for (span, kind, modifiers) in tokens {
            let range = span.into_range();
            if range.start >= last_end && range.start < range.end {
                last_end = range.end;
                kept.push((span, kind, modifiers));
            }
        }
        kept
    }

    fn type_reference_at(&self, program: &Program, offset: usize) -> Option<(Option<Id>, String)> {
        program
            .type_references
            .iter()
            .filter(|(source, span, _, _)| {
                *source == SourceId(0) && {
                    let range = span.into_range();
                    range.start <= offset && offset < range.end
                }
            })
            .min_by_key(|(_, span, _, _)| {
                let range = span.into_range();
                range.end - range.start
            })
            .map(|(_, _, definition, label)| (*definition, label.clone()))
    }

    fn definition_of(&self, program: &Program, id: Id) -> Option<(SourceId, Span)> {
        match program.entity_map.get(&id)? {
            Expr::Local(binding) | Expr::Variable(binding) | Expr::Parameter(binding) => {
                // Resolve to the name span of the thing the binding actually is —
                // a function, a `let`/`mut` variable, or (parameters/generics,
                // whose `span_map` entry is already the name) the span itself.
                if let Some(function) = program.functions.get(binding) {
                    return Some((program.source_of(*binding)?, function.name_span));
                }
                if let Some(function) = program.external_functions.get(binding) {
                    return Some((program.source_of(*binding)?, function.name_span));
                }
                if let Some(variable) = program.variables.get(binding) {
                    return Some((program.source_of(*binding)?, variable.name_span));
                }
                Some((program.source_of(*binding)?, span_of(program, *binding)?))
            }
            Expr::Field(_, struct_id, index) => {
                let field = program.structs.get(struct_id)?.fields.get(*index)?;
                Some((program.source_of(*struct_id)?, field.name_span))
            }
            Expr::EnumVariant(enum_id, _) => {
                Some((program.source_of(*enum_id)?, span_of(program, *enum_id)?))
            }
            Expr::Call(call_id) => {
                let subject = program.function_calls.get(call_id)?.subject_id;
                self.definition_of(program, subject)
            }
            Expr::Function(function_id) => Some((
                program.source_of(*function_id)?,
                program.functions.get(function_id)?.name_span,
            )),
            Expr::ExternalFunction(function_id) => Some((
                program.source_of(*function_id)?,
                program.external_functions.get(function_id)?.name_span,
            )),
            Expr::Struct(struct_id) => Some((
                program.source_of(*struct_id)?,
                program.structs.get(struct_id)?.name_span,
            )),
            Expr::StructInitializer(initializer_id, _) => {
                let struct_id = program.struct_initializer_to_def.get(initializer_id)?;
                Some((
                    program.source_of(*struct_id)?,
                    program.structs.get(struct_id)?.name_span,
                ))
            }
            Expr::Enum(enum_id) => Some((
                program.source_of(*enum_id)?,
                program.enums.get(enum_id)?.name_span,
            )),
            Expr::Trait(trait_id) => Some((
                program.source_of(*trait_id)?,
                program.traits.get(trait_id)?.name_span,
            )),
            _ => None,
        }
    }

    /// All references to the symbol under `offset` (including its declaration).
    pub fn references(&self, offset: usize) -> Vec<(SourceId, Span)> {
        let Some(program) = self.program.as_ref() else {
            return Vec::new();
        };
        // Resolve a target from the entity under the cursor, falling back to a
        // type reference (a type *use* is not an entity) and then to a struct
        // field declaration (whose name has no entity of its own).
        let target = self
            .entity_at(offset)
            .and_then(|id| self.target_of(program, id))
            .or_else(|| self.type_reference_target(program, offset))
            .or_else(|| self.field_decl_at(program, offset));
        let Some(target) = target else {
            return Vec::new();
        };
        self.occurrences(program, target)
    }

    /// The "Organize Imports" edits (WO-2): the top-level import runs sorted into
    /// canonical order — the same order `vilan fmt` produces, through the shared
    /// `formatter::organize_import_runs` — and unused imports pruned. Returns one
    /// `(span, replacement)` per run whose canonical form differs from the source;
    /// empty when already organized (the action then offers nothing).
    ///
    /// Pruning is conservative. It happens only when the analyzed program matches
    /// the current buffer exactly and carries no diagnostics — a mid-edit
    /// unresolved name might be about to use an import, so a broken or stale
    /// document sorts but never prunes. Re-exports are never pruned (handled in
    /// the formatter — they are surface, not usage), and an import a macro
    /// expansion references is kept (see `unused_import_leaf_spans`).
    pub fn organize_import_edits(&self) -> Vec<(Span, String)> {
        let source = self.text.as_str();
        // Prune only against a fresh, diagnostic-free analysis of THIS buffer: a
        // stale or broken document (a mid-edit unresolved name might be about to
        // use an import) sorts but never prunes.
        let prunable_program = self
            .program
            .as_ref()
            .filter(|_| self.diagnostics.is_empty() && self.text_hash == hash_text(source));
        let edits = match prunable_program {
            Some(program) => {
                let keep = |leaf_span: Span| self.import_leaf_is_used(program, leaf_span);
                vilan_core::formatter::organize_import_runs(source, &keep)
            }
            None => vilan_core::formatter::organize_import_runs(source, &|_| true),
        };
        edits
            .map(|edits| {
                edits
                    .into_iter()
                    .map(|edit| (edit.span, edit.replacement))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the top-level import whose terminal name occupies `leaf_span` is
    /// used, so the organizer keeps it. Maps the leaf to the definition it binds
    /// (`resolve_import` records the leaf as a reference at its own span — see
    /// `flatten_namespace_branch`/`record_reference`), then asks whether that
    /// definition is referenced anywhere in this file beyond the import itself.
    /// An unmappable leaf (an import that didn't resolve — but then the file
    /// carries a diagnostic and pruning is off) is conservatively kept.
    ///
    /// Conservatism, per the surfaces a use can land in: a reference on ANY of
    /// them keeps the import.
    ///  - (A) Type/trait positions — `type_references`, filtered to this file.
    ///    This includes derive-GENERATED type/trait references, which the
    ///    analyzer attributes to the deriving (this) file, so a derive-only
    ///    import survives.
    ///  - (B) Value positions (call subject, bare value) — the entity map, whose
    ///    per-use source lets us filter to this file (or code generated from it).
    ///    `reference_count` is deliberately NOT used: an import binds its leaf
    ///    directly to the shared definition Id, so that tally aggregates uses
    ///    across every file and reads ~0 for type-only imports.
    ///  - (C) Struct constructors (`Point { .. }`) — the initializer map.
    fn import_leaf_is_used(&self, program: &Program, leaf_span: Span) -> bool {
        let entry = SourceId(0);
        let Some(def_id) = program
            .type_references
            .iter()
            .find_map(|(source, span, definition, _)| {
                (*source == entry && *span == leaf_span).then_some(*definition)
            })
            .flatten()
        else {
            return true;
        };
        // (A) Type / trait references in this file, beyond this import's own leaf.
        let referenced_as_type =
            program
                .type_references
                .iter()
                .any(|(source, span, definition, _)| {
                    *source == entry && *definition == Some(def_id) && *span != leaf_span
                });
        if referenced_as_type {
            return true;
        }
        // (B) Value references in this file (or generated from it).
        let referenced_as_value = program.entity_map.iter().any(|(use_id, expr)| {
            expr_references_definition(expr, def_id)
                && self.use_in_entry_or_generated(program, *use_id)
        });
        if referenced_as_value {
            return true;
        }
        // (C) Struct constructors reference the type through the initializer map.
        program
            .struct_initializer_to_def
            .iter()
            .any(|(initializer_id, struct_id)| {
                *struct_id == def_id && self.use_in_entry_or_generated(program, *initializer_id)
            })
    }

    /// Whether a use site belongs to the entry file or to code generated from it
    /// (a derive expansion) — the two sources whose references keep an import.
    fn use_in_entry_or_generated(&self, program: &Program, use_id: Id) -> bool {
        matches!(
            program.source_of(use_id),
            Some(SourceId(0)) | Some(DERIVED_SOURCE)
        )
    }

    /// A struct/enum/trait target when the cursor is on a type *use* (e.g.
    /// `Option` in `Option<T>`), which lives in `type_references` rather than as
    /// an entity.
    fn type_reference_target(&self, program: &Program, offset: usize) -> Option<Target> {
        let (definition, _) = self.type_reference_at(program, offset)?;
        let definition = definition?;
        if program.structs.contains_key(&definition)
            || program.enums.contains_key(&definition)
            || program.traits.contains_key(&definition)
        {
            Some(Target::Type(definition))
        } else {
            None
        }
    }

    /// The struct field whose declaration name contains `offset`, if any (field
    /// names in a declaration carry no entity, so they need a positional probe).
    fn field_decl_at(&self, program: &Program, offset: usize) -> Option<Target> {
        for (struct_id, struct_definition) in &program.structs {
            if program.source_of(*struct_id) != Some(SourceId(0)) {
                continue;
            }
            for (index, field) in struct_definition.fields.iter().enumerate() {
                let range = field.name_span.into_range();
                if range.start <= offset && offset < range.end {
                    return Some(Target::Field(*struct_id, index));
                }
            }
        }
        None
    }

    /// What a use site refers to, for find-references / rename.
    fn target_of(&self, program: &Program, id: Id) -> Option<Target> {
        match program.entity_map.get(&id)? {
            Expr::Local(binding) | Expr::Variable(binding) | Expr::Parameter(binding) => {
                Some(Target::Binding(*binding))
            }
            Expr::Field(_, struct_id, index) => Some(Target::Field(*struct_id, *index)),
            // The cursor is on a function/method declaration name.
            Expr::Function(function_id) => Some(Target::Method(*function_id)),
            // The cursor is on a type declaration name or a constructor.
            Expr::Struct(struct_id) => Some(Target::Type(*struct_id)),
            Expr::Enum(enum_id) => Some(Target::Type(*enum_id)),
            Expr::Trait(trait_id) => Some(Target::Type(*trait_id)),
            Expr::StructInitializer(initializer_id, _) => program
                .struct_initializer_to_def
                .get(initializer_id)
                .map(|struct_id| Target::Type(*struct_id)),
            Expr::Call(call_id) => {
                // A method call carries a member span, and its wired subject is a
                // `Local` pointing at the resolved method function (see
                // `wire_method_call`).
                if !program.member_name_spans.contains_key(&id) {
                    return None;
                }
                let subject = program.function_calls.get(call_id)?.subject_id;
                match program.entity_map.get(&subject)? {
                    Expr::Local(function_id) | Expr::Function(function_id)
                        if program.functions.contains_key(function_id) =>
                    {
                        Some(Target::Method(*function_id))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Every occurrence (declaration + uses) of a target, as `(file, span)`,
    /// each span covering exactly the identifier to rename.
    fn occurrences(&self, program: &Program, target: Target) -> Vec<(SourceId, Span)> {
        let mut spans: Vec<(SourceId, Span)> = Vec::new();
        let mut push = |id: Id, span: Span| {
            if let Some(source) = program.source_of(id) {
                spans.push((source, span));
            }
        };

        match target {
            Target::Binding(binding) => {
                // The declaration name span: a variable carries it explicitly; a
                // parameter/capture's `span_map` entry is already the name.
                let decl_span = program
                    .variables
                    .get(&binding)
                    .map(|variable| variable.name_span)
                    .or_else(|| span_of(program, binding));
                if let Some(span) = decl_span {
                    push(binding, span);
                }
                for (use_id, expr) in &program.entity_map {
                    let refers = matches!(
                        expr,
                        Expr::Local(other) | Expr::Variable(other) | Expr::Parameter(other)
                        if *other == binding
                    );
                    if refers && *use_id != binding {
                        if let Some(span) = span_of(program, *use_id) {
                            push(*use_id, span);
                        }
                    }
                }
            }
            Target::Field(struct_id, index) => {
                if let Some(field) = program
                    .structs
                    .get(&struct_id)
                    .and_then(|s| s.fields.get(index))
                {
                    push(struct_id, field.name_span);
                }
                for (use_id, expr) in &program.entity_map {
                    if let Expr::Field(_, other_struct, other_index) = expr {
                        if *other_struct == struct_id && *other_index == index {
                            if let Some(span) = program.member_name_spans.get(use_id) {
                                push(*use_id, *span);
                            }
                        }
                    }
                }
            }
            Target::Method(function_id) => {
                if let Some(function) = program.functions.get(&function_id) {
                    push(function_id, function.name_span);
                }
                for (use_id, expr) in &program.entity_map {
                    let Expr::Call(call_id) = expr else {
                        continue;
                    };
                    // A method call site carries a member span and a wired subject
                    // that is a `Local` pointing at the method function.
                    let Some(member_span) = program.member_name_spans.get(use_id) else {
                        continue;
                    };
                    let Some(call) = program.function_calls.get(call_id) else {
                        continue;
                    };
                    let refers = matches!(
                        program.entity_map.get(&call.subject_id),
                        Some(Expr::Local(other)) | Some(Expr::Function(other)) if *other == function_id
                    );
                    if refers {
                        push(*use_id, *member_span);
                    }
                }
            }
            Target::Type(type_id) => {
                // The declaration's name span.
                let name_span = program
                    .structs
                    .get(&type_id)
                    .map(|s| s.name_span)
                    .or_else(|| program.enums.get(&type_id).map(|e| e.name_span))
                    .or_else(|| program.traits.get(&type_id).map(|t| t.name_span));
                if let Some(span) = name_span {
                    push(type_id, span);
                }
                // Every type-position use is recorded in `type_references`.
                for (source, span, definition, _label) in &program.type_references {
                    if *definition == Some(type_id) {
                        spans.push((*source, *span));
                    }
                }
                // Constructor uses (`Point { .. }`) aren't type references; the
                // name leads the initializer span.
                if let Some(structure) = program.structs.get(&type_id) {
                    for (initializer_id, struct_id) in &program.struct_initializer_to_def {
                        if *struct_id != type_id {
                            continue;
                        }
                        if let (Some(initializer_span), Some(source)) = (
                            span_of(program, *initializer_id),
                            program.source_of(*initializer_id),
                        ) {
                            let start = initializer_span.into_range().start;
                            let name_span: Span = (start..start + structure.name.len()).into();
                            spans.push((source, name_span));
                        }
                    }
                }
            }
        }
        spans
    }

    /// The outline of the entry file: functions, structs (with their fields),
    /// enums, and traits, each with its declaration and name spans.
    pub fn document_symbols(&self) -> Vec<Symbol> {
        let Some(program) = self.program.as_ref() else {
            return Vec::new();
        };
        let in_entry = |id: Id| program.source_of(id) == Some(SourceId(0));
        let mut symbols = Vec::new();

        for (id, function) in &program.functions {
            if !in_entry(*id) {
                continue;
            }
            symbols.push(Symbol {
                name: function.name.to_string(),
                kind: SymbolKind::Function,
                full: span_of(program, *id).unwrap_or(function.name_span),
                selection: function.name_span,
                children: Vec::new(),
            });
        }
        for (id, structure) in &program.structs {
            if !in_entry(*id) {
                continue;
            }
            let Some(full) = span_of(program, *id) else {
                continue;
            };
            let children = structure
                .fields
                .iter()
                .map(|field| Symbol {
                    name: field.name.to_string(),
                    kind: SymbolKind::Field,
                    full: field.name_span,
                    selection: field.name_span,
                    children: Vec::new(),
                })
                .collect();
            symbols.push(Symbol {
                name: structure.name.to_string(),
                kind: SymbolKind::Struct,
                full,
                selection: full,
                children,
            });
        }
        for (id, enumeration) in &program.enums {
            if !in_entry(*id) {
                continue;
            }
            let Some(full) = span_of(program, *id) else {
                continue;
            };
            symbols.push(Symbol {
                name: enumeration.name.to_string(),
                kind: SymbolKind::Enum,
                full,
                selection: full,
                children: Vec::new(),
            });
        }
        for (id, trait_definition) in &program.traits {
            if !in_entry(*id) {
                continue;
            }
            let Some(full) = span_of(program, *id) else {
                continue;
            };
            symbols.push(Symbol {
                name: trait_definition.name.to_string(),
                kind: SymbolKind::Trait,
                full,
                selection: full,
                children: Vec::new(),
            });
        }
        symbols
    }

    /// Completion candidates at `offset`, dispatched by the syntax just before the
    /// cursor: members after `.`, path items after `::`, else names in scope plus
    /// keywords. The editor filters the list by whatever prefix is being typed.
    pub fn completion(&self, offset: usize) -> Vec<Completion> {
        let Some(program) = self.program.as_ref() else {
            return Vec::new();
        };
        let text = self.line_index.text();
        let bytes = text.as_bytes();
        // Scan back over the partial identifier the user is typing to reach the
        // syntactic context (`.`, `::`, or open scope) that drives the candidates.
        let mut start = offset.min(bytes.len());
        while start > 0 && is_identifier_byte(bytes[start - 1]) {
            start -= 1;
        }
        // `[Na|` at an item position (the line holds only the attribute so
        // far) and `[derive(Na|` complete registered macro names — the last
        // piece of the macro-LSP tail. Macro names are always bare, so they
        // bypass the call-suppression below.
        if start >= 1 && bytes[start - 1] == b'[' {
            let line_start = text[..start - 1].rfind('\n').map(|at| at + 1).unwrap_or(0);
            if text[line_start..start - 1].trim().is_empty() {
                return self.macro_name_completions(program);
            }
        }
        if start >= 1 && bytes[start - 1] == b'(' && text[..start - 1].ends_with("[derive") {
            return self.macro_name_completions(program);
        }
        let mut candidates = if start >= 1 && bytes[start - 1] == b'.' {
            // `a?.` completes on the LIFTED element (`Option<Profile>` offers
            // Profile's members — proposal/try-and-lift.md §5).
            if start >= 2 && bytes[start - 2] == b'?' {
                self.lifted_member_completions(program, start - 2)
            } else {
                self.member_completions(program, start - 1)
            }
        } else if start >= 2 && bytes[start - 1] == b':' && bytes[start - 2] == b':' {
            self.path_completions(program, text, start - 2)
        } else {
            self.scope_completions(program, offset)
        };
        // A call-shaped insertion is wrong when the callee is already
        // parenthesized — the char right after the cursor is `(`, so the user
        // pre-typed the parens or is retyping a call — or when a name is being
        // imported, not called (`use std::math::sqrt`). Fall back to a bare name
        // for every candidate; the signature and docs still show (WO-3 escape
        // hatches).
        let next_is_open_paren = bytes.get(offset).copied() == Some(b'(');
        let in_import = in_import_path(text, offset);
        // An import path takes names, so the construct snippets (`for …`,
        // `fun …`) have no business there — drop them entirely (E14). (Member
        // and path positions never produce snippets in the first place.)
        if in_import {
            candidates.retain(|candidate| !matches!(candidate.kind, CompletionKind::Snippet));
        }
        if next_is_open_paren || in_import {
            for candidate in &mut candidates {
                candidate.call_parameters = None;
            }
        }
        candidates
    }

    /// Every registered macro name, for attribute-position completion. The
    /// union over all scopes deliberately over-offers (visibility is
    /// file-scoped; the expansion engine still gates actual use) — the
    /// recorded refinement is filtering to this file's macro scope.
    fn macro_name_completions(&self, program: &Program) -> Vec<Completion> {
        let mut names: Vec<&str> = program
            .scopes
            .values()
            .flat_map(|scope| scope.macro_name_to_id.keys().copied())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .map(|name| Completion::bare(name.to_string(), CompletionKind::Macro))
            .collect()
    }

    /// Fields and methods of the receiver value ending just before the `.` at
    /// `dot_offset`.
    fn member_completions(&self, program: &Program, dot_offset: usize) -> Vec<Completion> {
        let Some(type_id) = self.receiver_nominal_id(program, dot_offset) else {
            return Vec::new();
        };
        self.nominal_member_completions(program, type_id)
    }

    /// The fields + methods of one nominal type — the member-completion list.
    fn nominal_member_completions(&self, program: &Program, type_id: Id) -> Vec<Completion> {
        let mut items = Vec::new();
        if let Some(structure) = program.structs.get(&type_id) {
            for field in &structure.fields {
                items.push(Completion::bare(
                    field.name.to_string(),
                    CompletionKind::Field,
                ));
            }
        }
        self.push_methods(program, type_id, true, &mut items);
        items
    }

    /// Members of the ELEMENT under a lifted chain (`a?.` on an
    /// `Option<Profile>` offers Profile's members): the receiver ends just
    /// before the `?` at `question_offset`; its container's first type
    /// argument is the element.
    fn lifted_member_completions(
        &self,
        program: &Program,
        question_offset: usize,
    ) -> Vec<Completion> {
        // A bare name (`p?.`): the binding's declared container type.
        if let Some(name) = identifier_ending_at(self.line_index.text(), question_offset) {
            let binding = self
                .binding_in_scope(program, name, question_offset)
                .or_else(|| self.same_file_variable(program, name, question_offset));
            let element = binding
                .and_then(|id| {
                    program
                        .variables
                        .get(&id)
                        .map(|variable| variable.type_id)
                        .or_else(|| {
                            program
                                .parameters
                                .get(&id)
                                .map(|parameter| parameter.type_id)
                        })
                })
                .and_then(|type_id| match program.type_id_to_type_map.get(&type_id) {
                    Some(Type::Enum(_, arguments)) | Some(Type::Struct(_, arguments)) => {
                        arguments.first().copied()
                    }
                    _ => None,
                })
                .and_then(
                    |element_id| match program.type_id_to_type_map.get(&element_id) {
                        Some(Type::Struct(id, _)) | Some(Type::Enum(id, _)) => Some(*id),
                        _ => None,
                    },
                );
            if let Some(element) = element {
                return self.nominal_member_completions(program, element);
            }
        }
        // A complex receiver (`find(x)?.`): its rendered type's first generic
        // argument names the element.
        question_offset
            .checked_sub(1)
            .and_then(|offset| self.entity_at(offset))
            .and_then(|receiver| self.hover_label(program, receiver))
            .and_then(|label| first_generic_argument(&label).map(str::to_string))
            .and_then(|element| self.nominal_id_by_name(program, base_type_name(&element)))
            .map(|type_id| self.nominal_member_completions(program, type_id))
            .unwrap_or_default()
    }

    /// The nominal struct/enum id of the receiver value ending just before the `.`
    /// at `dot_offset`.
    fn receiver_nominal_id(&self, program: &Program, dot_offset: usize) -> Option<Id> {
        // A bare name (`p.`): resolve through scope, or — when the cursor's own
        // statement failed to parse and dropped its local scope — the nearest
        // same-file binding of that name, then read its declared type. Robust while
        // the buffer is mid-edit, which is exactly when completion fires.
        if let Some(name) = identifier_ending_at(self.line_index.text(), dot_offset) {
            let binding = self
                .binding_in_scope(program, name, dot_offset)
                .or_else(|| self.same_file_variable(program, name, dot_offset));
            if let Some(nominal) = binding.and_then(|id| self.binding_nominal_id(program, id)) {
                return Some(nominal);
            }
        }
        // A complex receiver (`foo().`, `a.b.`): the parsed entity's rendered type.
        dot_offset
            .checked_sub(1)
            .and_then(|offset| self.entity_at(offset))
            .and_then(|receiver| self.hover_label(program, receiver))
            .and_then(|label| self.nominal_id_by_name(program, base_type_name(&label)))
    }

    /// The nominal struct/enum id a `let`/parameter binding's declared type names.
    fn binding_nominal_id(&self, program: &Program, binding: Id) -> Option<Id> {
        let type_id = program
            .variables
            .get(&binding)
            .map(|variable| variable.type_id)
            .or_else(|| {
                program
                    .parameters
                    .get(&binding)
                    .map(|parameter| parameter.type_id)
            })?;
        match program.type_id_to_type_map.get(&type_id)? {
            Type::Struct(id, _) | Type::Enum(id, _) => Some(*id),
            _ => None,
        }
    }

    /// The nearest same-file `let`/`mut` binding named `name` declared before
    /// `offset` — a fallback for when the cursor's statement failed to parse and so
    /// dropped its enclosing scope from the analysis.
    fn same_file_variable(&self, program: &Program, name: &str, offset: usize) -> Option<Id> {
        let mut best: Option<(usize, Id)> = None;
        for (id, variable) in &program.variables {
            let start = variable.name_span.into_range().start;
            if variable.name == name
                && start < offset
                && program.source_of(*id) == Some(SourceId(0))
                && best.is_none_or(|(best_start, _)| start > best_start)
            {
                best = Some((start, *id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Items reachable through `left::` — an enum's variants and methods, a
    /// struct's methods, or a module's members — where `left` is the identifier
    /// ending just before the `::` at `colon_offset`.
    fn path_completions(
        &self,
        program: &Program,
        text: &str,
        colon_offset: usize,
    ) -> Vec<Completion> {
        let Some(left) = identifier_ending_at(text, colon_offset) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for (enum_id, enumeration) in &program.enums {
            if enumeration.name == left {
                for variant in &enumeration.variants {
                    items.push(Completion::bare(
                        variant.name.to_string(),
                        CompletionKind::EnumVariant,
                    ));
                }
                self.push_methods(program, *enum_id, false, &mut items);
            }
        }
        for (struct_id, structure) in &program.structs {
            if structure.name == left {
                self.push_methods(program, *struct_id, false, &mut items);
            }
        }
        for module in program.modules.values() {
            if module.name == left {
                if let Some(scope) = program.scopes.get(&module.body.1) {
                    for (name, id) in &scope.name_to_id_map {
                        let kind = self.kind_of(program, *id);
                        items.push(self.entity_completion(program, name.to_string(), *id, kind));
                    }
                }
            }
        }
        items
    }

    /// Names visible at `offset` (the cursor's scope, then each enclosing scope up
    /// to global) plus the language keywords.
    fn scope_completions(&self, program: &Program, offset: usize) -> Vec<Completion> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut scope_id = self.scope_at(program, offset);
        while let Some(id) = scope_id {
            let Some(scope) = program.scopes.get(&id) else {
                break;
            };
            for (name, entity_id) in &scope.name_to_id_map {
                if seen.insert(*name) {
                    let kind = self.kind_of(program, *entity_id);
                    items.push(self.entity_completion(program, name.to_string(), *entity_id, kind));
                }
            }
            scope_id = scope.parent_id;
        }
        // The offered keywords are exactly the lexer's, drawn from the one
        // documented table [`KEYWORD_DOCS`] (kept in lockstep with the lexer by
        // [`keyword_lexeme`]) — no separate hand-list to drift (WO-3).
        for (keyword, _sentence, _link) in KEYWORD_DOCS {
            items.push(Completion::bare(
                keyword.to_string(),
                CompletionKind::Keyword,
            ));
        }
        // The shape-heavy constructs also complete as fill-in snippets, next to
        // the bare keyword (E14). Only scope positions reach here — member and
        // path completion never call this, and the import-path post-pass in
        // `completion` drops them — so the snippets stay out of `.`/`::`/import
        // contexts.
        for (keyword, label, detail, body) in CONSTRUCT_SNIPPETS {
            items.push(Completion::snippet(label, detail, body, keyword));
        }
        items
    }

    /// The scope of the entity at — or nearest before — the cursor, so the current
    /// function's locals are in scope even when the cursor sits in fresh text.
    fn scope_at(&self, program: &Program, offset: usize) -> Option<Id> {
        let entity = self.entity_at(offset).or_else(|| {
            self.entity_spans
                .iter()
                .filter(|(_, end, _)| *end <= offset)
                .max_by_key(|(_, end, _)| *end)
                .map(|(_, _, id)| *id)
        })?;
        program.entity_scope_map.get(&entity).copied()
    }

    /// The binding `name` resolves to in the scope at `offset` (searching the
    /// enclosing scopes up to global) — a local, parameter, or top-level item.
    fn binding_in_scope(&self, program: &Program, name: &str, offset: usize) -> Option<Id> {
        let mut scope_id = self.scope_at(program, offset);
        while let Some(id) = scope_id {
            let scope = program.scopes.get(&id)?;
            if let Some(binding) = scope.name_to_id_map.get(name) {
                return Some(*binding);
            }
            scope_id = scope.parent_id;
        }
        None
    }

    /// Appends `type_id`'s impl methods, restricted to either instance methods
    /// (`want_self`, for `value.`) or static/associated ones (for `Type::`). A
    /// `value.default()` (a static method with no `self`) would not type-check, so
    /// member completion must not offer it.
    fn push_methods(
        &self,
        program: &Program,
        type_id: Id,
        want_self: bool,
        items: &mut Vec<Completion>,
    ) {
        for implementation in &program.implementations {
            if self.impl_subject_id(program, implementation) == Some(type_id) {
                for (name, member_id) in &implementation.declarations {
                    if self.is_self_method(program, *member_id) == want_self {
                        items.push(self.entity_completion(
                            program,
                            name.to_string(),
                            *member_id,
                            CompletionKind::Method,
                        ));
                    }
                }
            }
        }
    }

    /// Whether a method's first parameter is `self` — i.e. it is called on a value
    /// (`v.method()`) rather than on the type (`Type::method()`).
    fn is_self_method(&self, program: &Program, member_id: Id) -> bool {
        let first_parameter = match program.entity_map.get(&member_id) {
            Some(Expr::Function(function_id)) => program
                .functions
                .get(function_id)
                .and_then(|function| function.parameters.first()),
            Some(Expr::ExternalFunction(external_id)) => program
                .external_functions
                .get(external_id)
                .and_then(|external| external.parameters.first()),
            _ => None,
        };
        first_parameter
            .and_then(|parameter_id| program.parameters.get(parameter_id))
            .is_some_and(|parameter| parameter.name == "self")
    }

    /// The nominal struct/enum id an impl's subject names, ignoring type arguments.
    fn impl_subject_id(&self, program: &Program, implementation: &Implementation) -> Option<Id> {
        match program.type_id_to_type_map.get(&implementation.subject)? {
            Type::Struct(id, _) | Type::Enum(id, _) => Some(*id),
            _ => None,
        }
    }

    /// The struct or enum named `name` (type arguments already stripped).
    fn nominal_id_by_name(&self, program: &Program, name: &str) -> Option<Id> {
        program
            .structs
            .iter()
            .find(|(_, structure)| structure.name == name)
            .map(|(id, _)| *id)
            .or_else(|| {
                program
                    .enums
                    .iter()
                    .find(|(_, enumeration)| enumeration.name == name)
                    .map(|(id, _)| *id)
            })
    }

    /// The completion category for a name bound in a scope.
    fn kind_of(&self, program: &Program, id: Id) -> CompletionKind {
        if program.functions.contains_key(&id) || program.external_functions.contains_key(&id) {
            CompletionKind::Function
        } else if program.structs.contains_key(&id) {
            CompletionKind::Struct
        } else if program.enums.contains_key(&id) {
            CompletionKind::Enum
        } else if program.traits.contains_key(&id) {
            CompletionKind::Trait
        } else if program.modules.contains_key(&id) {
            CompletionKind::Module
        } else {
            CompletionKind::Variable
        }
    }

    /// Builds a completion for a named entity, enriched for the popup and for
    /// call-shaped insertion (WO-3): a function/method carries its full
    /// signature (`detail`), its `///` first paragraph (`documentation`), and
    /// its parameter names (`call_parameters`, `self` dropped) so the server can
    /// insert `name(…)`; a variable carries its rendered type as `detail`.
    /// Everything else is a bare name. `id` is the entity id bound in scope (or
    /// an impl member id for a method), resolved to a definition through
    /// [`Self::function_target`].
    fn entity_completion(
        &self,
        program: &Program,
        label: String,
        id: Id,
        kind: CompletionKind,
    ) -> Completion {
        let mut completion = Completion::bare(label, kind);
        match completion.kind {
            CompletionKind::Function | CompletionKind::Method => {
                if let Some(target) = self.function_target(program, id) {
                    completion.detail = signature_label(program, target);
                    completion.documentation = self.doc_first_paragraph(program, target);
                    completion.call_parameters = call_parameter_names(program, target);
                }
            }
            CompletionKind::Variable => {
                completion.detail = self.hover_label(program, id);
            }
            _ => {}
        }
        completion
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The nominal name in a rendered type label: `struct Point` -> `Point`,
/// `enum Option<i32>` -> `Option` (drops the `struct`/`enum`/`trait` prefix the
/// type renderer adds, plus any type arguments and surrounding whitespace).
/// The first generic argument of a rendered type label — `Option<Profile>` →
/// `Profile`, `Result<User, str>` → `User` (nesting respected).
fn first_generic_argument(label: &str) -> Option<&str> {
    let open = label.find('<')?;
    let inner = &label[open + 1..];
    let mut depth = 0usize;
    for (index, character) in inner.char_indices() {
        match character {
            '<' => depth += 1,
            '>' if depth == 0 => return Some(inner[..index].trim()),
            '>' => depth -= 1,
            ',' if depth == 0 => return Some(inner[..index].trim()),
            _ => {}
        }
    }
    None
}

fn base_type_name(label: &str) -> &str {
    let label = label.trim();
    let label = ["struct ", "enum ", "trait "]
        .iter()
        .find_map(|prefix| label.strip_prefix(prefix))
        .unwrap_or(label);
    label.split('<').next().unwrap_or(label).trim()
}

/// The identifier ending at byte `end` in `text`, if any.
fn identifier_ending_at(text: &str, end: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = end.min(bytes.len());
    while start > 0 && is_identifier_byte(bytes[start - 1]) {
        start -= 1;
    }
    (start < end).then(|| &text[start..end])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    pub(crate) fn std_root() -> PathBuf {
        // The std PACKAGE directory (holding `vilan.toml`), like the server's
        // `discover_std_dir` — pointing at the bare source root instead would
        // drop the manifest's platform layers (no `std::fs`/`std::http`/…).
        std::env::var_os("VILAN_STD")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"))
    }

    /// A throwaway on-disk package: `files` written under a fresh temp dir,
    /// the first file analyzed as the open document. Returns the temp dir (for
    /// later edits + cleanup) and the analyzed document.
    pub(crate) fn analyze_workspace(files: &[(&str, &str)]) -> (PathBuf, Document) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("vilan_lsp_{}_{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (relative, contents) in files {
            let path = dir.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, contents).unwrap();
        }
        let entry = dir.join(files[0].0);
        let text = std::fs::read_to_string(&entry).unwrap();
        let document = Document::analyze(&text, &std_root(), &entry);
        (dir, document)
    }

    // An error INSIDE an imported module publishes to that module's path, with
    // a span that is correct in THAT file's text — the vanishing-diagnostics
    // bug (it used to map through the entry's line index and disappear).
    #[test]
    fn imported_file_error_groups_to_its_path_with_its_own_span() {
        let module = "fun answer(): i32 {\n\t\"not a number\"\n}\n";
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import std::print;\nimport pkg::broken::answer;\nfun main() { print(answer()); }\n",
            ),
            ("broken.vl", module),
        ]);
        let published = document.published_diagnostics();
        let item = published
            .iter()
            .find(|item| item.message.contains("Expected i32"))
            .expect("the module's type error should be published");
        let path = item.path.as_ref().expect("attributed to a file");
        assert!(path.ends_with("broken.vl"), "{path:?}");
        // The span must be an offset into broken.vl's own text — at the string
        // literal the error is about.
        let expected = module.find("\"not a number\"").unwrap();
        assert_eq!(
            item.span.into_range().start,
            expected,
            "span should locate the literal in the MODULE's text"
        );
        assert!(!item.warning);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Entry-file errors stay on the entry (path = None), even alongside module
    // errors in the same analysis.
    #[test]
    fn entry_errors_group_to_the_entry() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import pkg::helper::greet;\nfun main() {\n\tgreet();\n\tmissing_in_entry();\n}\n",
            ),
            ("helper.vl", "fun greet() {\n\tmissing_in_helper();\n}\n"),
        ]);
        let published = document.published_diagnostics();
        let entry_error = published
            .iter()
            .find(|item| item.message.contains("missing_in_entry"))
            .expect("the entry's error should be published");
        assert!(entry_error.path.is_none(), "entry errors carry no path");
        let helper_error = published
            .iter()
            .find(|item| item.message.contains("missing_in_helper"))
            .expect("the helper's error should be published");
        assert!(
            helper_error
                .path
                .as_ref()
                .is_some_and(|path| path.ends_with("helper.vl")),
            "{:?}",
            helper_error.path
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The staleness half: fixing the imported file on disk and re-analyzing the
    // SAME entry clears the module's diagnostics — what `reanalyze_dependents`
    // relies on (a dependent's re-analysis reads the dependency fresh).
    #[test]
    fn reanalysis_after_fixing_the_import_clears_its_diagnostics() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import std::print;\nimport pkg::broken::answer;\nfun main() { print(answer()); }\n",
            ),
            ("broken.vl", "fun answer(): i32 {\n\t\"not a number\"\n}\n"),
        ]);
        assert!(
            document
                .published_diagnostics()
                .iter()
                .any(|item| item.message.contains("Expected i32")),
            "the broken dependency should report first"
        );
        // Fix the module on disk; re-analyze the unchanged entry.
        std::fs::write(dir.join("broken.vl"), "fun answer(): i32 {\n\t42\n}\n").unwrap();
        let entry = dir.join("main.vl");
        let text = std::fs::read_to_string(&entry).unwrap();
        let reanalyzed = Document::analyze(&text, &std_root(), &entry);
        assert!(
            reanalyzed.published_diagnostics().is_empty(),
            "fixed dependency should publish clean: {:?}",
            reanalyzed
                .published_diagnostics()
                .iter()
                .map(|item| &item.message)
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // `[must_use]` drops surface as warnings on the entry.
    #[test]
    fn must_use_drops_publish_as_warnings() {
        let (dir, document) = analyze_workspace(&[(
            "main.vl",
            "[must_use]\nfun important(): i32 { 7 }\nfun main() {\n\timportant();\n}\n",
        )]);
        let published = document.published_diagnostics();
        let warning = published
            .iter()
            .find(|item| item.warning)
            .expect("the dropped result should warn");
        assert!(warning.path.is_none());
        assert!(
            warning.message.contains("must_use")
                || warning.message.contains("result")
                || warning.message.contains("unused"),
            "{}",
            warning.message
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── platform coloring in the editor (proposal/platform-coloring.md, phase 2) ──

    // A browser-target package whose entry REACHES `std::fs` publishes the
    // coloring violation live: chain-rendered with module-labeled library
    // frames, anchored at the offending call in the entry.
    #[test]
    fn coloring_violation_publishes_live_on_a_browser_target() {
        let entry = "import std::fs;\n\nfun main() {\n\tlet present = fs::exists(\"marker\");\n}\n";
        let (dir, document) = analyze_workspace(&[
            ("src/main.vl", entry),
            (
                "vilan.toml",
                "[package]\nname = \"app\"\ntarget = \"browser\"\n",
            ),
        ]);
        let published = document.published_diagnostics();
        let violation = published
            .iter()
            .find(|item| {
                item.message
                    .contains("requires the `process` layer of `std`")
            })
            .unwrap_or_else(|| {
                panic!(
                    "no coloring violation published: {:?}",
                    published
                        .iter()
                        .map(|item| &item.message)
                        .collect::<Vec<_>>()
                )
            });
        assert!(violation.path.is_none(), "anchored in the entry itself");
        assert!(!violation.warning);
        assert!(
            violation.message.contains("cannot run on `browser`"),
            "{}",
            violation.message
        );
        assert!(
            violation.message.contains("main → exists (std::fs)"),
            "{}",
            violation.message
        );
        // The anchor is the deepest user-code call site: the `fs::exists` call.
        let call = entry.find("exists(").unwrap();
        let range = violation.span.into_range();
        assert!(
            range.start <= call && call < range.end,
            "span {range:?} should cover the call at {call}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The same reach under the package's declared `node` target is admissible —
    // the manifest's `target` is what drives the editor's platform.
    #[test]
    fn the_manifest_target_admits_the_same_reach_on_node() {
        let entry = "import std::fs;\n\nfun main() {\n\tlet present = fs::exists(\"marker\");\n}\n";
        let (dir, document) = analyze_workspace(&[
            ("src/main.vl", entry),
            (
                "vilan.toml",
                "[package]\nname = \"app\"\ntarget = \"node\"\n",
            ),
        ]);
        let published = document.published_diagnostics();
        assert!(
            published.is_empty(),
            "{:?}",
            published
                .iter()
                .map(|item| &item.message)
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A manifest-less scratch file gets its platform INFERRED from its imports:
    // `std::dom` marks it a browser file, so reaching `std::fs` colors.
    #[test]
    fn an_inferred_browser_file_colors_without_a_manifest() {
        let document = Document::analyze(
            "import std::dom;\nimport std::fs;\n\nfun main() {\n\tlet present = fs::exists(\"marker\");\n}\n",
            &std_root(),
            Path::new("scratch.vl"),
        );
        let published = document.published_diagnostics();
        assert!(
            published.iter().any(|item| {
                item.message
                    .contains("requires the `process` layer of `std`")
                    && item.message.contains("cannot run on `browser`")
            }),
            "{:?}",
            published
                .iter()
                .map(|item| &item.message)
                .collect::<Vec<_>>()
        );
    }

    // A multi-entry package (proposal/platform-coloring.md §4.2): an entry
    // file analyzes under ITS entry's target — the browser entry colors on
    // reaching the store, the node entry running the same code doesn't — and
    // a shared module (no entry, no `main`) analyzes clean, its hover still
    // knowing the color.
    #[test]
    fn multi_entry_files_analyze_under_their_entry_targets() {
        let manifest =
            "[package]\nname = \"app\"\n\n[entry.client]\ntarget = \"browser\"\n\n[entry.server]\n";
        let store = "import std::fs;\n\nfun load(): bool {\n\tfs::exists(\"state\")\n}\n";
        let reach = "import std::print;\nimport pkg::store::load;\n\nfun main() {\n\tif load() { print(\"?\") }\n}\n";
        let (dir, client) = analyze_workspace(&[
            ("src/client.vl", reach),
            ("vilan.toml", manifest),
            ("src/store.vl", store),
            ("src/server.vl", reach),
        ]);
        assert!(
            client.published_diagnostics().iter().any(|item| {
                item.message
                    .contains("requires the `process` layer of `std`")
                    && item.message.contains("cannot run on `browser`")
            }),
            "the client entry should color: {:?}",
            client
                .published_diagnostics()
                .iter()
                .map(|item| &item.message)
                .collect::<Vec<_>>()
        );
        // The node entry, same code: admissible.
        let entry = dir.join("src/server.vl");
        let server = Document::analyze(
            &std::fs::read_to_string(&entry).unwrap(),
            &std_root(),
            &entry,
        );
        assert!(
            server.published_diagnostics().is_empty(),
            "{:?}",
            server
                .published_diagnostics()
                .iter()
                .map(|item| &item.message)
                .collect::<Vec<_>>()
        );
        // The shared module: no `main`, no admission walk — clean, but hover
        // on `load` still shows its requirement.
        let entry = dir.join("src/store.vl");
        let text = std::fs::read_to_string(&entry).unwrap();
        let module = Document::analyze(&text, &std_root(), &entry);
        assert!(module.published_diagnostics().is_empty());
        let hover = module
            .hover(text.find("load").unwrap())
            .expect("hover on `load` should produce a label");
        assert!(
            hover.contains("requires the `process` layer of `std`"),
            "{hover}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The hover text at the cursor marked `|` in `src` (a bare manifest-less
    /// file, like `completions_at_cursor` — keep the sources closure-free, the
    /// marker would collide with closure pipes).
    fn hover_at_cursor(src: &str) -> Option<String> {
        let offset = src
            .find('|')
            .expect("test source needs a `|` cursor marker");
        let text = src.replace('|', "");
        let document = Document::analyze(&text, &std_root(), Path::new("test.vl"));
        document.hover(offset)
    }

    // Hovering a function name appends its inferred platform requirement — the
    // coloring fixpoint surfaced in the editor, with the same via-chain
    // vocabulary the diagnostics use.
    #[test]
    fn hover_appends_a_functions_platform_requirement() {
        let hover = hover_at_cursor(
            "import std::fs;\n\nfun save() {\n\tfs::write_file(\"state\", \"data\");\n}\n\nfun main() {\n\tsa|ve();\n}\n",
        )
        .expect("hovering `save` should produce a label");
        assert!(
            hover.contains("requires the `process` layer of `std` (via `write_file (std::fs)`)"),
            "{hover}"
        );
    }

    // The declaration name carries the requirement too, not just call sites.
    #[test]
    fn hover_on_the_definition_name_carries_the_requirement() {
        let hover = hover_at_cursor(
            "import std::fs;\n\nfun sa|ve() {\n\tfs::write_file(\"state\", \"data\");\n}\n\nfun main() {\n\tsave();\n}\n",
        );
        assert!(
            hover
                .as_deref()
                .is_some_and(|hover| { hover.contains("requires the `process` layer of `std`") }),
            "hover on the declaration name should carry the requirement: {hover:?}"
        );
    }

    // A method call resolves through its wired subject to the method function,
    // whose requirement rides the hover alongside the call's type.
    #[test]
    fn hover_on_a_method_call_attributes_the_methods_requirement() {
        let hover = hover_at_cursor(
            "import std::fs;\n\nstruct Store { path: str }\n\nimpl Store {\n\tfun persist(self): bool {\n\t\tfs::write_file(self.path, \"state\");\n\t\ttrue\n\t}\n}\n\nfun main() {\n\tlet store = Store { path = \"s.txt\" };\n\tstore.per|sist();\n}\n",
        )
        .expect("hovering `persist` should produce a label");
        assert!(
            hover.contains("requires the `process` layer of `std` (via `write_file (std::fs)`)"),
            "{hover}"
        );
    }

    // A module-level binding's requirement rides hover like a function's —
    // its initializer is code, and the line says what running it needs.
    #[test]
    fn hover_on_a_global_reference_shows_the_initializers_requirement() {
        let hover = hover_at_cursor(
            "import std::fs::read_file_to_str;\n\nlet cache = read_file_to_str(\"cache.txt\");\n\nfun main() {\n\tlet content = ca|che;\n}\n",
        );
        assert!(
            hover.as_deref().is_some_and(|hover| hover.contains(
                "requires the `process` layer of `std` (via `read_file_to_str (std::fs)`)"
            )),
            "{hover:?}"
        );
    }

    // E2: semantic tokens classify from the ANALYZED program. The cases
    // TextMate cannot get right: a generic parameter at use, a macro name
    // (which deliberately shares its trait's name), method vs field on the
    // same `.name` shape, and module qualifiers.
    #[test]
    fn semantic_tokens_classify_the_ambiguous_cases() {
        let text = "import std::math;\n\nstruct Point {\n\tx: i32,\n}\n\nfun pick<T>(value: T): T {\n\tvalue\n}\n\nfun main() {\n\tlet p = Point { x = 1 };\n\tlet n = p.x;\n\tlet low = math::min(1, 2);\n\tlet chosen = pick(n);\n\tlet size = chosen.abs();\n}\n";
        let document = Document::analyze(text, &std_root(), Path::new("test.vl"));
        let tokens = document.semantic_tokens();
        let kind_of = |snippet: &str, occurrence: usize| -> Option<TokenKind> {
            let mut start = 0;
            let mut position = None;
            for _ in 0..=occurrence {
                position = text[start..].find(snippet).map(|at| start + at);
                start = position? + 1;
            }
            let at = position?;
            tokens
                .iter()
                .find(|(span, _, _)| {
                    let range = span.into_range();
                    range.start == at && range.end == at + snippet.len()
                })
                .map(|(_, kind, _)| *kind)
        };
        // The generic parameter at its USE site (T in `value: T`).
        assert_eq!(
            kind_of("T", 1),
            Some(TokenKind::TypeParameter),
            "{tokens:?}"
        );
        // A struct name in type/constructor position.
        assert_eq!(kind_of("Point", 1), Some(TokenKind::Struct), "{tokens:?}");
        // A field read is a property, not a method.
        assert_eq!(kind_of("x", 2), Some(TokenKind::Property), "{tokens:?}");
        // A module import name is a namespace.
        assert_eq!(kind_of("math", 0), Some(TokenKind::Namespace), "{tokens:?}");
        // Parameters and variables split.
        assert_eq!(
            kind_of("value", 0),
            Some(TokenKind::Parameter),
            "{tokens:?}"
        );
        assert_eq!(
            kind_of("chosen", 0),
            Some(TokenKind::Variable),
            "{tokens:?}"
        );
        // A member CALL is a method (the same `.name` shape as the property
        // read above — only semantics can split them).
        assert_eq!(kind_of("abs", 0), Some(TokenKind::Method), "{tokens:?}");
    }

    #[test]
    fn semantic_tokens_are_sorted_and_non_overlapping() {
        let text = "import std::option::Option::{ self, Some, None };\n\nfun main() {\n\tlet maybe = Some(2);\n\tlet doubled = maybe? * 2;\n}\n";
        let document = Document::analyze(text, &std_root(), Path::new("test.vl"));
        let tokens = document.semantic_tokens();
        assert!(!tokens.is_empty());
        let mut last_end = 0;
        for (span, _, _) in &tokens {
            let range = span.into_range();
            assert!(range.start >= last_end, "overlap at {range:?}: {tokens:?}");
            assert!(range.end > range.start);
            last_end = range.end;
        }
    }

    // E6: a dependent's analysis reads an OPEN document's buffer, not the
    // file on disk — the overlay seam in `load_package_module`. The disk
    // copy of the helper only defines `one`; the overlay renames it to
    // `two`, and the entry calling `two()` analyzes clean exactly when the
    // overlay is consulted.
    #[test]
    fn a_dependents_analysis_reads_the_open_buffer_not_the_disk() {
        let dir = std::env::temp_dir().join(format!("vilan-e6-overlay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let helper_path = dir.join("helper.vl");
        std::fs::write(&helper_path, "export fun one(): i32 {\n\t1\n}\n").expect("write helper");
        let entry_path = dir.join("main.vl");
        let entry_text =
            "import pkg::helper::two;\n\nfun main() {\n\tlet _x = two();\n}\n".to_string();
        std::fs::write(&entry_path, &entry_text).expect("write entry");

        // Disk truth: `two` does not exist — the entry has errors.
        let stale = Document::analyze(&entry_text, &std_root(), &entry_path);
        assert!(
            !stale.diagnostics.is_empty(),
            "expected the disk-backed analysis to fail on `two`"
        );

        // The helper is "open" with an edited, unsaved buffer defining `two`.
        vilan_core::analyzer::set_document_overlay(
            &helper_path,
            Some("export fun two(): i32 {\n\t2\n}\n".to_string()),
        );
        let live = Document::analyze(&entry_text, &std_root(), &entry_path);
        vilan_core::analyzer::set_document_overlay(&helper_path, None);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            live.diagnostics.is_empty(),
            "expected the overlay-backed analysis to be clean, got {:?}",
            live.diagnostics
        );
    }

    // Expression lifting (expression-lifting.md): hovering the RECEIVER of a
    // bare `?` shows the receiver's own container type — the region's binder
    // entity carries an empty span exactly so it cannot tie with the
    // receiver in the narrowest-span selection and leak the element type.
    #[test]
    fn hover_on_a_lift_receiver_shows_the_container_type() {
        let hover = hover_at_cursor(
            "import std::option::Option::{ self, Some, None };\n\nfun main() {\n\tlet count = Some(2);\n\tlet doubled = cou|nt? * 2;\n}\n",
        )
        .expect("hovering the receiver should produce a label");
        assert!(hover.contains("Option<i32>"), "{hover}");
    }

    // The binding a region initializes hovers as the lifted type.
    #[test]
    fn hover_on_a_region_initialized_binding_shows_the_lifted_type() {
        let hover = hover_at_cursor(
            "import std::option::Option::{ self, Some, None };\n\nfun main() {\n\tlet count = Some(2);\n\tlet dou|bled = count? * 2;\n}\n",
        )
        .expect("hovering the binding should produce a label");
        assert!(hover.contains("Option<i32>"), "{hover}");
    }

    // The applicative form analyzes and hovers without incident too — the
    // whole-document smoke for the region machinery under the LSP path.
    #[test]
    fn hover_across_an_applicative_region_document() {
        let hover = hover_at_cursor(
            "import std::option::Option::{ self, Some, None };\n\nfun main() {\n\tlet price = Some(40);\n\tlet tax = Some(2);\n\tlet tot|al = price? + tax?;\n}\n",
        )
        .expect("hovering the binding should produce a label");
        assert!(hover.contains("Option<i32>"), "{hover}");
    }

    // E9: hovering a function shows its FULL signature, fenced as code —
    // parameter names and types, the return type.
    #[test]
    fn hover_shows_the_full_function_signature() {
        let hover = hover_at_cursor(
            "import std::print;\n\nfun descr|ibe(count: i32, label: str): str {\n\tlabel\n}\n\nfun main() {\n\tprint(describe(1, \"x\"));\n}\n",
        )
        .expect("hovering the declaration should produce a label");
        assert!(
            hover.contains("```vilan\nfun describe(count: i32, label: str): str\n```"),
            "{hover}"
        );
    }

    // E9: INFERRED async (no `async` keyword written) prepends to the
    // signature — inference runs after the labels are built, so the server
    // adds it.
    #[test]
    fn hover_prepends_inferred_async() {
        let hover = hover_at_cursor(
            "import std::time::{ sleep_for, Duration };\n\nfun wa|rm() {\n\tsleep_for(Duration::millis(1));\n}\n\nfun main() {\n\twarm();\n}\n",
        )
        .expect("hover on the declaration");
        assert!(hover.contains("```vilan\nasync fun warm()\n```"), "{hover}");
    }

    // E9 (rule4-completion S1): the inferred `borrows` root-set surfaces in the
    // signature like the source clause. A `&mut self` method returning a
    // projection of `self` renders `borrows self` though no clause was written.
    #[test]
    fn hover_shows_an_inferred_single_borrows_position() {
        let hover = hover_at_cursor(
            "import std::print;\n\nstruct Wrapper { value: i32 }\n\nimpl Wrapper {\n\tfun sl|ot(&mut self): &mut i32 {\n\t\t&mut self.value\n\t}\n}\n\nfun main() {\n\tmut w = Wrapper { value = 1 };\n\tw.slot() = 2;\n\tprint(w.value);\n}\n",
        )
        .expect("hovering `slot` should produce a label");
        assert!(hover.contains("borrows self"), "{hover}");
    }

    // A wrapped view projecting a different parameter per branch unions both
    // positions; the clause names them in order — `borrows a, b`.
    #[test]
    fn hover_shows_an_inferred_multi_borrows_position() {
        let hover = hover_at_cursor(
            "import std::option::Option::{ self, Some, None };\n\nstruct Box { x: i32 }\n\nfun pi|ck(a: &mut Box, b: &mut Box, first: bool): Option<&mut i32> {\n\tif first { Some(&mut a.x) } else { Some(&mut b.x) }\n}\n\nfun main() {\n\tmut p = Box { x = 1 };\n\tmut q = Box { x = 2 };\n\tmatch pick(&mut p, &mut q, true) {\n\t\tSome(let v) => { v = 9; }\n\t\tNone => {}\n\t}\n}\n",
        )
        .expect("hovering `pick` should produce a label");
        assert!(hover.contains("borrows a, b"), "{hover}");
    }

    // The rendered position is the one the chain projects, not always the
    // receiver: `pick` returns `grow(b)`, so it borrows `b` — `borrows b`.
    #[test]
    fn hover_shows_a_chained_non_receiver_borrows_position() {
        let hover = hover_at_cursor(
            "fun grow(x: &mut i32): &mut i32 borrows x {\n\tx\n}\n\nfun pi|ck(a: &mut i32, b: &mut i32): &mut i32 {\n\tgrow(b)\n}\n\nfun main() {\n\tmut p = 1;\n\tmut q = 2;\n\tpick(&mut p, &mut q) = 9;\n}\n",
        )
        .expect("hovering `pick` should produce a label");
        assert!(hover.contains("borrows b"), "{hover}");
        assert!(!hover.contains("borrows a"), "{hover}");
    }

    // The inferred `bumps` effect renders after `borrows` (rule-4 S2, C6): a
    // geometry-advancing mutator names its bumping parameter.
    #[test]
    fn hover_shows_an_inferred_bumps_clause() {
        let hover = hover_at_cursor(
            "fun to|uch(xs: &mut List<i32>) {\n\txs.push(1);\n}\n\nfun main() {\n\tmut xs = [ 1 ];\n\ttouch(&mut xs);\n}\n",
        )
        .expect("hovering `touch` should produce a label");
        assert!(hover.contains("bumps xs"), "{hover}");
    }

    // A content-stable `&mut` mutator (field writes only) carries NO bumps
    // clause — the absence is the verdict.
    #[test]
    fn hover_omits_bumps_for_a_content_stable_mutator() {
        let hover = hover_at_cursor(
            "struct Point { x: i32, y: i32 }\n\nfun re|tag(p: &mut Point) {\n\tp.x = 1;\n}\n\nfun main() {\n\tmut p = Point { x = 0, y = 0 };\n\tretag(&mut p);\n}\n",
        )
        .expect("hovering `retag` should produce a label");
        assert!(!hover.contains("bumps"), "{hover}");
    }

    // E9: the declaration's leading `///` block surfaces as prose, and
    // attribute lines between it and the name don't break the chain.
    #[test]
    fn hover_surfaces_the_leading_doc_comment() {
        let hover = hover_at_cursor(
            "import std::print;\n\n/// Renders the badge label.\n/// Two lines of docs.\n[must_use]\nfun bad|ge(count: i32): str {\n\t\"b\"\n}\n\nfun main() {\n\tlet _b = badge(1);\n\tprint(\"x\");\n}\n",
        )
        .expect("hover on the declaration");
        assert!(
            hover.contains("Renders the badge label.\nTwo lines of docs."),
            "{hover}"
        );
        assert!(hover.contains("fun badge(count: i32): str"), "{hover}");
    }

    // WO-4 variables: a local `let` hovers as its typed binding — `let name: T`,
    // fenced like a declaration, the type resolved by inference.
    #[test]
    fn hover_on_a_local_let_shows_its_typed_binding() {
        let hover = hover_at_cursor("fun main() {\n\tlet cou|nt = 5;\n\tlet _ = count;\n}\n")
            .expect("hover on the let binding");
        assert!(hover.contains("```vilan\nlet count: i32\n```"), "{hover}");
    }

    // A `mut` binding hovers with the `mut` keyword — it can be reassigned.
    #[test]
    fn hover_on_a_mut_binding_shows_mut() {
        let hover = hover_at_cursor("fun main() {\n\tmut tot|al = 0;\n\ttotal = 1;\n}\n")
            .expect("hover on the mut binding");
        assert!(hover.contains("```vilan\nmut total: i32\n```"), "{hover}");
    }

    // A module-level binding hovers as a `let` too, not just locals.
    #[test]
    fn hover_on_a_module_binding_shows_its_typed_binding() {
        let hover =
            hover_at_cursor("let cap|acity = 100;\n\nfun main() {\n\tlet _ = capacity;\n}\n")
                .expect("hover on the module binding");
        assert!(
            hover.contains("```vilan\nlet capacity: i32\n```"),
            "{hover}"
        );
    }

    // A destructured binder hovers as `let name: T` with its ELEMENT type.
    #[test]
    fn hover_on_a_destructured_binder_shows_its_element_type() {
        let hover = hover_at_cursor(
            "fun main() {\n\tlet (a|a, bb) = (1, 2);\n\tlet _ = aa;\n\tlet _ = bb;\n}\n",
        )
        .expect("hover on the destructured binder");
        assert!(hover.contains("```vilan\nlet aa: i32\n```"), "{hover}");
    }

    // A use site hovers identically to the declaration it resolves to.
    #[test]
    fn hover_on_a_binding_use_site_matches_the_declaration() {
        let hover = hover_at_cursor("fun main() {\n\tlet count = 5;\n\tlet _ = cou|nt;\n}\n")
            .expect("hover on the use site");
        assert!(hover.contains("```vilan\nlet count: i32\n```"), "{hover}");
    }

    // A binding's leading `///` doc rides its hover, like a declaration's.
    #[test]
    fn hover_on_a_binding_surfaces_its_doc_comment() {
        let hover = hover_at_cursor(
            "fun main() {\n\t/// how many things\n\tlet cou|nt = 5;\n\tlet _ = count;\n}\n",
        )
        .expect("hover on the documented binding");
        assert!(hover.contains("```vilan\nlet count: i32\n```"), "{hover}");
        assert!(hover.contains("how many things"), "{hover}");
    }

    // WO-4 parameters: a plain parameter hovers as `name: T`.
    #[test]
    fn hover_on_a_plain_parameter_shows_name_and_type() {
        let hover = hover_at_cursor(
            "fun f(coun|t: i32): i32 {\n\tcount\n}\n\nfun main() {\n\tlet _ = f(1);\n}\n",
        )
        .expect("hover on the plain parameter");
        assert!(hover.contains("```vilan\ncount: i32\n```"), "{hover}");
    }

    // An `own` parameter carries its convention: `own name: T`.
    #[test]
    fn hover_on_an_own_parameter_shows_the_own_convention() {
        let hover = hover_at_cursor(
            "struct Box { n: i32 }\n\nfun consume(own |b: Box): i32 {\n\tb.n\n}\n\nfun main() {\n\tlet _ = consume(Box { n = 1 });\n}\n",
        )
        .expect("hover on the own parameter");
        assert!(hover.contains("```vilan\nown b: Box\n```"), "{hover}");
    }

    // A `&` (readonly view) parameter — the `&` lives on the convention, not the
    // type, so hover reconstructs `name: &T`.
    #[test]
    fn hover_on_a_ref_parameter_shows_the_ref_convention() {
        let hover = hover_at_cursor("fun peek(|x: &i32): i32 {\n\tx\n}\n\nfun main() {\n\tlet a = 1;\n\tlet _ = peek(&a);\n}\n")
            .expect("hover on the ref parameter");
        assert!(hover.contains("```vilan\nx: &i32\n```"), "{hover}");
    }

    // A `&mut` (writable view) parameter, hovered at a USE site — the convention
    // is not in the pre-rendered type, so hover adds `&mut` back.
    #[test]
    fn hover_on_a_mut_ref_parameter_use_shows_the_mut_ref_convention() {
        let hover = hover_at_cursor(
            "fun f(xs: &mut i32) {\n\tx|s = 1;\n}\n\nfun main() {\n\tmut a = 0;\n\tf(&mut a);\n}\n",
        )
        .expect("hover on the &mut parameter use");
        assert!(hover.contains("```vilan\nxs: &mut i32\n```"), "{hover}");
    }

    // A function-typed parameter shows its closure shape (`|A| R`). The source
    // carries closure pipes, so the `|` cursor marker can't be used — the offset
    // is computed straight onto the parameter name.
    #[test]
    fn hover_on_a_closure_parameter_shows_its_shape() {
        let text = "fun apply(g: |i32| i32): i32 {\n\tg(1)\n}\n\nfun main() {\n\tlet _ = apply(fun(x: i32): i32 { x });\n}\n";
        let document = Document::analyze(text, &std_root(), Path::new("test.vl"));
        let offset = text.find("(g:").unwrap() + 1;
        let hover = document
            .hover(offset)
            .expect("hover on the closure parameter");
        assert!(hover.contains("```vilan\ng: |i32| i32\n```"), "{hover}");
    }

    // WO-4 keywords: a keyword hovers as one crisp sentence + a book deep link.
    // Covers the flagship memory-model word `resource` (spec link), a second
    // memory-model word `own` (spec link), and a control-flow word `for` (tour
    // link) — sentence AND URL asserted per case.
    #[test]
    fn hover_on_a_keyword_shows_its_meaning_and_book_link() {
        let hover = hover_at_cursor("resou|rce struct File { fd: i32 }\n\nfun main() {}\n")
            .expect("hover on `resource`");
        assert!(
            hover.contains("An owned value with exactly one owner, moved rather than copied"),
            "{hover}"
        );
        assert!(
            hover.contains(
                "https://reedsyllas.github.io/vilan/spec/memory.html#68-resources-and-destruction"
            ),
            "{hover}"
        );

        let hover = hover_at_cursor(
            "struct Box { n: i32 }\n\nfun consume(o|wn b: Box): i32 {\n\tb.n\n}\n\nfun main() {\n\tlet _ = consume(Box { n = 1 });\n}\n",
        )
        .expect("hover on `own`");
        assert!(hover.contains("moves ownership into the callee"), "{hover}");
        assert!(
            hover.contains("https://reedsyllas.github.io/vilan/spec/memory.html#63-rule-3"),
            "{hover}"
        );

        let hover =
            hover_at_cursor("fun main() {\n\tfo|r x in [ 1, 2 ] {\n\t\tlet _ = x;\n\t}\n}\n")
                .expect("hover on `for`");
        assert!(hover.contains("Iterates over the elements"), "{hover}");
        assert!(
            hover.contains("https://reedsyllas.github.io/vilan/tour/control-flow.html#loops"),
            "{hover}"
        );
    }

    // A keyword hovers even on a document that does not compile — the lookup is
    // purely lexical, ahead of any analysis.
    #[test]
    fn hover_on_a_keyword_works_without_a_program() {
        let text = "fun main() {\n\tresource\n}\n"; // `resource` misused — analysis fails.
        let document = Document::analyze(text, &std_root(), Path::new("test.vl"));
        let offset = text.find("resource").unwrap() + 1;
        let hover = document
            .hover(offset)
            .expect("keyword hover without a program");
        assert!(
            hover.contains("An owned value with exactly one owner"),
            "{hover}"
        );
    }

    // The keyword table stays in lockstep with the lexer: every documented
    // keyword lexes to exactly one keyword token that classifies back to
    // itself. If a new keyword lands in the lexer, `keyword_lexeme` (exhaustive
    // over `Token`) forces a new arm, and this pin forces its `KEYWORD_DOCS`
    // entry — so no keyword ships without a hover.
    #[test]
    fn every_documented_keyword_round_trips_through_the_lexer() {
        for (keyword, _sentence, _link) in KEYWORD_DOCS {
            let (tokens, errors) = tokenize(keyword);
            assert!(errors.is_empty(), "{keyword} lexed with errors: {errors:?}");
            assert_eq!(tokens.len(), 1, "{keyword} should lex to one token");
            assert_eq!(
                keyword_lexeme(&tokens[0].0),
                Some(*keyword),
                "{keyword} must classify back to itself"
            );
        }
    }

    // A no-hover case that must STAY silent: whitespace between items and a
    // comment name no entity and are no keyword.
    #[test]
    fn hover_stays_silent_on_whitespace_and_comments() {
        let hover = hover_at_cursor("fun a() {}\n|\nfun main() {}\n");
        assert!(hover.is_none(), "whitespace should not hover: {hover:?}");
        let text = "fun a() {}\n// just a note\nfun main() {}\n";
        let document = Document::analyze(text, &std_root(), Path::new("test.vl"));
        let offset = text.find("just").unwrap();
        assert!(
            document.hover(offset).is_none(),
            "a comment should not hover: {:?}",
            document.hover(offset)
        );
    }

    // The macro-LSP tail's last piece: `[` at an item position offers the
    // registered macro names — derives included — and `[derive(` offers
    // them for the derive list.
    #[test]
    fn attribute_position_completes_macro_names() {
        let completions = completions_at_cursor(
            "import std::print;\n\n[Hash|]\nstruct Point { x: i32 }\n\nfun main() {\n\tprint(1);\n}\n",
        );
        assert!(
            completions.iter().any(|label| label == "Hashable"),
            "expected the derive prelude: {completions:?}"
        );
        assert!(
            completions.iter().any(|label| label == "Json"),
            "{completions:?}"
        );
        let derive_completions = completions_at_cursor(
            "import std::print;\n\n[derive(Pa|)]\nstruct Point { x: i32 }\n\nfun main() {\n\tprint(1);\n}\n",
        );
        assert!(
            derive_completions.iter().any(|label| label == "PartialEq"),
            "{derive_completions:?}"
        );
    }

    // Inlay hints: an UNANNOTATED binding shows its inferred type in
    // place; an annotated one shows nothing (the source already says it).
    #[test]
    fn inlay_hints_show_inferred_types_only() {
        let text = "import std::option::Option::{ self, Some, None };\n\nfun main() {\n\tlet count = Some(2);\n\tlet doubled = count? * 2;\n\tlet named: i32 = 4;\n}\n";
        let document = Document::analyze(text, &std_root(), Path::new("test.vl"));
        let hints = document.inlay_hints();
        let hint_after = |name: &str| {
            let at = text.find(name).unwrap() + name.len();
            hints
                .iter()
                .find(|(offset, _)| *offset == at)
                .map(|(_, label)| label.clone())
        };
        assert_eq!(
            hint_after("doubled"),
            Some(": Option<i32>".to_string()),
            "{hints:?}"
        );
        assert!(hint_after("count").is_some(), "{hints:?}");
        assert_eq!(hint_after("named"), None, "{hints:?}");
    }

    // Token modifiers: declarations carry `declaration`; an immutable
    // binding and its uses carry `readonly`, a `mut` one does not.
    #[test]
    fn semantic_token_modifiers_split_readonly_and_declarations() {
        let text = "import std::print;\n\nfun main() {\n\tlet fixed = 1;\n\tmut counter = 2;\n\tprint(fixed + counter);\n}\n";
        let document = Document::analyze(text, &std_root(), Path::new("test.vl"));
        let tokens = document.semantic_tokens();
        let modifiers_at = |at: usize, len: usize| {
            tokens
                .iter()
                .find(|(span, _, _)| {
                    let range = span.into_range();
                    range.start == at && range.end == at + len
                })
                .map(|(_, _, modifiers)| *modifiers)
        };
        let fixed_declaration = text.find("fixed").unwrap();
        let counter_declaration = text.find("counter").unwrap();
        let fixed_use = text.rfind("fixed").unwrap();
        let counter_use = text.rfind("counter").unwrap();
        assert_eq!(
            modifiers_at(fixed_declaration, 5),
            Some(MODIFIER_DECLARATION | MODIFIER_READONLY),
            "{tokens:?}"
        );
        assert_eq!(
            modifiers_at(counter_declaration, 7),
            Some(MODIFIER_DECLARATION),
            "{tokens:?}"
        );
        assert_eq!(
            modifiers_at(fixed_use, 5),
            Some(MODIFIER_READONLY),
            "{tokens:?}"
        );
        assert_eq!(modifiers_at(counter_use, 7), Some(0), "{tokens:?}");
    }

    // E9: hover on a `const` binding shows its evaluated VALUE beside the
    // type — the LSP evaluation is fuel-capped and skips broken documents.
    #[test]
    fn hover_shows_a_constants_value() {
        let hover = hover_at_cursor(
            "import std::print;\n\nlet SIZE = const 8 * 8;\n\nfun main() {\n\tprint(SI|ZE);\n}\n",
        )
        .expect("hover on the constant");
        assert!(hover.contains("= 64"), "{hover}");
    }

    // E9: a parameter's `context` clause renders in the hovered signature.
    #[test]
    fn hover_renders_a_parameters_context_clause() {
        let hover = hover_at_cursor(
            "import std::reactive::{ owner_scope, Owner };\n\nfun with_o|wner(body: (|| void) context owner_scope) {\n\tlet _b = body;\n}\n\nfun main() {}\n",
        )
        .expect("hover on the declaration");
        assert!(hover.contains("context owner_scope"), "{hover}");
    }

    // std is documented with `///` (user decision): hovering a std function
    // from user code surfaces its doc line, read cross-file from the std
    // source.
    #[test]
    fn hover_surfaces_std_docs_cross_file() {
        let hover = hover_at_cursor(
            "import std::time::{ now, Instant };\n\nfun main() {\n\tlet started = no|w();\n}\n",
        )
        .expect("hover on the std function");
        assert!(hover.contains("The current moment, typed."), "{hover}");
    }

    // `///` is the doc syntax — a plain `//` block is an implementation note
    // and must NOT surface (user decision, 2026-07-16).
    #[test]
    fn hover_ignores_plain_comment_blocks() {
        let hover = hover_at_cursor(
            "import std::print;\n\n// An internal note, not docs.\nfun bad|ge(count: i32): str {\n\t\"b\"\n}\n\nfun main() {\n\tlet _b = badge(1);\n\tprint(\"x\");\n}\n",
        )
        .expect("hover on the declaration");
        assert!(
            !hover.contains("An internal note"),
            "plain `//` must not surface: {hover}"
        );
        assert!(hover.contains("fun badge(count: i32): str"), "{hover}");
    }

    // E9: struct hovers show the declaration block with fields; enum hovers
    // show variants with payloads.
    #[test]
    fn hover_shows_struct_fields_and_enum_variants() {
        let hover = hover_at_cursor(
            "import std::print;\n\nstruct Point { x: i32, name: str }\n\nfun main() {\n\tlet p = Po|int { x = 1, name = \"a\" };\n\tprint(p.name);\n}\n",
        )
        .expect("hover on the constructor");
        assert!(
            hover.contains("```vilan\nstruct Point {\n\tx: i32,\n\tname: str,\n}\n```"),
            "{hover}"
        );
        let hover = hover_at_cursor(
            "import std::print;\n\nenum Shape {\n\tDot,\n\tBox2(i32, i32),\n}\n\nfun main() {\n\tlet s = Sha|pe::Dot;\n\tmatch s {\n\t\tShape::Dot => print(\"dot\"),\n\t\tShape::Box2(let _w, let _h) => print(\"box\"),\n\t}\n}\n",
        )
        .expect("hover on the enum reference");
        assert!(
            hover.contains("Dot,") && hover.contains("Box2(i32, i32),"),
            "{hover}"
        );
    }

    // E9: a std function's docs come from its source file on disk (the
    // non-entry read path) alongside the signature.
    #[test]
    fn hover_reads_imported_declarations_from_their_files() {
        let hover = hover_at_cursor(
            "import std::fs::exists;\n\nfun main() {\n\tlet _e = exi|sts(\"x\");\n}\n",
        )
        .expect("hover on the std call");
        assert!(
            hover.contains("exists(") && hover.contains("```vilan"),
            "{hover}"
        );
    }

    // Colorless functions hover exactly as before — no requirement line.
    #[test]
    fn hover_stays_clean_on_a_colorless_function() {
        let hover = hover_at_cursor(
            "import std::print;\n\nfun greet() {\n\tprint(\"hi\");\n}\n\nfun main() {\n\tgre|et();\n}\n",
        );
        assert!(
            hover
                .as_deref()
                .is_none_or(|hover| !hover.contains("requires")),
            "{hover:?}"
        );
    }

    /// The completion labels offered at the cursor marked `|` in `src`.
    fn completions_at_cursor(src: &str) -> Vec<String> {
        completion_items_at_cursor(src)
            .into_iter()
            .map(|completion| completion.label)
            .collect()
    }

    /// The full completion candidates offered at the `|` cursor in `src` —
    /// carrying `detail`, `documentation`, and `call_parameters` (WO-3).
    fn completion_items_at_cursor(src: &str) -> Vec<Completion> {
        let offset = src
            .find('|')
            .expect("test source needs a `|` cursor marker");
        let text = src.replace('|', "");
        let document = Document::analyze(&text, &std_root(), Path::new("test.vl"));
        document.completion(offset)
    }

    /// The one candidate labelled `label` at the `|` cursor in `src` (the pins
    /// probe a specific function/method/keyword by name).
    fn completion_named(src: &str, label: &str) -> Completion {
        completion_items_at_cursor(src)
            .into_iter()
            .find(|completion| completion.label == label)
            .unwrap_or_else(|| panic!("no `{label}` completion offered"))
    }

    #[test]
    fn lifted_member_completion_offers_the_element() {
        let labels = completions_at_cursor(
            "import std::option::Option::{ self, Some, None };\n\
             struct Profile { name: str, age: i32 }\n\
             impl Profile { fun greeting(self): str { self.name } }\n\
             fun find(): Option<Profile> { None }\n\
             fun main() {\n\tlet p: Option<Profile> = find();\n\tp?.|\n}\n",
        );
        assert!(labels.contains(&"name".to_string()), "fields: {labels:?}");
        assert!(
            labels.contains(&"greeting".to_string()),
            "methods: {labels:?}"
        );
        assert!(
            !labels.contains(&"unwrap_or".to_string()),
            "the ELEMENT's members, not Option's: {labels:?}"
        );
    }

    #[test]
    fn member_completion_lists_fields_and_methods() {
        let labels = completions_at_cursor(
            "struct Point { x: i32, y: i32 }\n\
             impl Point { fun sum(self): i32 { self.x + self.y } }\n\
             fun main() {\n\tlet p = Point { x = 1, y = 2 };\n\tp.|x\n}\n",
        );
        assert!(labels.contains(&"x".to_string()), "fields: {labels:?}");
        assert!(labels.contains(&"y".to_string()), "fields: {labels:?}");
        assert!(labels.contains(&"sum".to_string()), "methods: {labels:?}");
    }

    #[test]
    fn member_completion_on_incomplete_receiver() {
        // The realistic moment: `p.` typed with nothing after it yet.
        let labels = completions_at_cursor(
            "struct Point { x: i32, y: i32 }\n\
             fun main() {\n\tlet p = Point { x = 1, y = 2 };\n\tp.|\n}\n",
        );
        assert!(
            labels.contains(&"x".to_string()),
            "incomplete member: {labels:?}"
        );
    }

    // WO-3: a function completion in a call position carries its full
    // signature (the same string hover fences), the first paragraph of its
    // `///` doc, and its parameter names (for the call-shaped insertion) — a
    // multi-parameter case, with the second doc paragraph correctly dropped.
    #[test]
    fn function_completion_carries_signature_parameters_and_doc() {
        let add = completion_named(
            "/// Adds two numbers.\n\
             ///\n\
             /// A second paragraph, not shown.\n\
             fun add(a: i32, b: i32): i32 { a + b }\n\
             fun main() {\n\tad|\n}\n",
            "add",
        );
        assert_eq!(
            add.call_parameters,
            Some(vec!["a".to_string(), "b".to_string()]),
            "parameter names for the placeholder insertion"
        );
        let detail = add
            .detail
            .expect("a function completion carries a signature");
        assert!(
            detail.contains("a: i32, b: i32") && detail.contains("): i32"),
            "signature must show the parameter list and return type: {detail:?}"
        );
        assert_eq!(
            add.documentation.as_deref(),
            Some("Adds two numbers."),
            "only the first `///` paragraph"
        );
    }

    // WO-3: a method drops the `self` receiver from the call placeholders (it
    // is supplied by the `value.` receiver, not typed as an argument), while
    // the signature detail still renders `self` in full.
    #[test]
    fn method_completion_skips_self_in_call_parameters() {
        let scale = completion_named(
            "struct Point { x: i32, y: i32 }\n\
             impl Point {\n\tfun scale(self, factor: i32): i32 { self.x * factor }\n}\n\
             fun main() {\n\tlet p = Point { x = 1, y = 2 };\n\tp.sc|\n}\n",
            "scale",
        );
        assert_eq!(
            scale.call_parameters,
            Some(vec!["factor".to_string()]),
            "`self` must not be a call placeholder"
        );
        let detail = scale
            .detail
            .expect("a method completion carries a signature");
        assert!(
            detail.contains("self") && detail.contains("factor: i32") && detail.contains("): i32"),
            "the method signature keeps `self`: {detail:?}"
        );
    }

    // WO-3: a zero-parameter callable carries an EMPTY parameter list (distinct
    // from a non-callable's `None`) — the server inserts `name()`.
    #[test]
    fn zero_parameter_function_has_empty_call_parameters() {
        let tick = completion_named("fun tick() { }\nfun main() {\n\tti|\n}\n", "tick");
        assert_eq!(tick.call_parameters, Some(Vec::new()));
    }

    // WO-3 escape hatch: when the callee is already followed by `(` — the user
    // pre-typed the parens, or is retyping a call — the completion inserts a
    // bare name (no duplicated parens), yet still shows the signature.
    #[test]
    fn callee_before_open_paren_suppresses_call_shape() {
        let add = completion_named(
            "fun add(a: i32, b: i32): i32 { a + b }\nfun main() {\n\tadd|(1, 2)\n}\n",
            "add",
        );
        assert_eq!(
            add.call_parameters, None,
            "no parens when `(` already follows"
        );
        assert!(add.detail.is_some(), "the signature still shows");
    }

    // WO-3 escape hatch: inside a `use`/`import` path a callable is being bound
    // into scope, not called, so it inserts a bare name — while the SAME
    // function in expression position keeps its call shape.
    #[test]
    fn import_path_suppresses_call_shape_but_expression_keeps_it() {
        let imported = completion_named(
            "mod geometry {\n\tfun area(w: i32, h: i32): i32 { w * h }\n}\n\
             import geometry::ar|\n",
            "area",
        );
        assert_eq!(
            imported.call_parameters, None,
            "a name in an import path inserts bare"
        );
        let called = completion_named(
            "mod geometry {\n\tfun area(w: i32, h: i32): i32 { w * h }\n}\n\
             fun main() {\n\tlet a = geometry::ar|\n}\n",
            "area",
        );
        assert_eq!(
            called.call_parameters,
            Some(vec!["w".to_string(), "h".to_string()]),
            "the same function in expression position keeps its call shape"
        );
    }

    // WO-3: a type name never grows parens — a struct is not call-shaped
    // regardless of position (its kind, not the cursor, decides).
    #[test]
    fn type_name_completion_never_call_shapes() {
        let point = completion_named(
            "struct Point { x: i32, y: i32 }\nfun main() {\n\tlet p = Poi|\n}\n",
            "Point",
        );
        assert_eq!(point.call_parameters, None, "a struct name inserts bare");
    }

    // WO-3 (the WO-4 finding): the offered keywords are EXACTLY the lexer's set,
    // drawn from the one documented table — no stale hand-list. Guards the two
    // concrete bugs it replaced: `return` (spelled `ret`) is gone, and
    // `const`/`borrows`/`resource`/`macro` are now present.
    #[test]
    fn keyword_completions_are_exactly_the_lexer_keywords() {
        let items = completion_items_at_cursor("fun main() {\n\t|\n}\n");
        let mut offered: Vec<String> = items
            .iter()
            .filter(|completion| matches!(completion.kind, CompletionKind::Keyword))
            .map(|completion| completion.label.clone())
            .collect();
        offered.sort();
        let mut expected: Vec<String> = KEYWORD_DOCS
            .iter()
            .map(|(keyword, _, _)| keyword.to_string())
            .collect();
        expected.sort();
        assert_eq!(
            offered, expected,
            "keyword completions == the documented set"
        );
        // Each offered keyword really lexes to that keyword (offered ⊆ lexer);
        // combined with the documented set == every `keyword_lexeme` arm (pinned
        // by `every_documented_keyword_round_trips_through_the_lexer`), the
        // offered set is exactly the lexer's.
        for keyword in &offered {
            let (tokens, errors) = tokenize(keyword);
            assert!(errors.is_empty(), "{keyword} lexed with errors: {errors:?}");
            assert_eq!(tokens.len(), 1, "{keyword} should lex to one token");
            assert_eq!(keyword_lexeme(&tokens[0].0), Some(keyword.as_str()));
        }
        assert!(
            !offered.iter().any(|keyword| keyword == "return"),
            "`return` is not a vilan keyword — it is `ret`"
        );
        for added in ["const", "borrows", "resource", "macro"] {
            assert!(
                offered.iter().any(|keyword| keyword == added),
                "the `{added}` keyword must be offered (it was missing from the old hand-list)"
            );
        }
    }

    // WO-3: `in_import_path` reads the current line's leading keyword, skipping
    // an `export` prefix, and does not confuse an identifier that merely starts
    // with `import`/`use`.
    #[test]
    fn in_import_path_recognizes_import_and_use_lines() {
        assert!(in_import_path("import std::math::sqrt", 22));
        assert!(in_import_path("use pkg::option::Option", 23));
        assert!(in_import_path("export import pkg::x::y", 23));
        assert!(in_import_path("\tuse a::b", 9));
        assert!(!in_import_path("fun main() { sqrt", 17));
        assert!(
            !in_import_path("imported = 5", 12),
            "a word starting with `import`"
        );
        assert!(!in_import_path("used = 5", 8), "a word starting with `use`");
    }

    // E14: at a scope position (an open function body) each shape-heavy
    // construct completes as a SNIPPET-kind template carrying its exact
    // tab-stopped body. The bodies are pinned verbatim — house style (tab
    // indent, trailing comma, `i32`) is part of the contract.
    #[test]
    fn construct_snippets_are_offered_at_a_scope_position() {
        let source = "fun main() {\n\t|\n}\n";
        for (label, body) in [
            ("for … in { }", "for ${1:item} in ${2:items} {\n\t$0\n}"),
            ("fun … ( ) { }", "fun ${1:name}(${2}) {\n\t$0\n}"),
            (
                "struct … { }",
                "struct ${1:Name} {\n\t${2:field}: ${3:i32},\n}",
            ),
            (
                "match … { }",
                "match ${1:subject} {\n\t${2:pattern} => $0,\n}",
            ),
        ] {
            let completion = completion_named(source, label);
            assert!(
                matches!(completion.kind, CompletionKind::Snippet),
                "`{label}` should be a snippet"
            );
            let snippet = completion.snippet.expect("a snippet carries its body");
            assert_eq!(snippet.body, body, "`{label}` body");
            // The fallback is the bare keyword (the label's first word).
            assert_eq!(snippet.fallback, label.split(' ').next().unwrap());
        }
    }

    // E14: the snippet is offered ALONGSIDE the bare keyword, not instead of it —
    // typing `for` still surfaces the plain keyword AND the distinctly-labelled
    // template, each with its own kind.
    #[test]
    fn scope_completion_offers_the_bare_keyword_alongside_the_snippet() {
        let items = completion_items_at_cursor("fun main() {\n\t|\n}\n");
        for keyword in ["for", "fun", "struct", "match"] {
            assert!(
                items
                    .iter()
                    .any(|c| c.label == keyword && matches!(c.kind, CompletionKind::Keyword)),
                "the bare `{keyword}` keyword is still offered"
            );
        }
        assert!(
            items
                .iter()
                .any(|c| c.label == "for … in { }" && matches!(c.kind, CompletionKind::Snippet)),
            "and the `for` snippet, distinctly labelled"
        );
    }

    // E14: construct snippets are a scope-position feature — a member list
    // (after `.`) offers none. The list is non-empty (the receiver's fields), so
    // this is a real member completion, not a vacuously empty one.
    #[test]
    fn construct_snippets_are_absent_in_member_completion() {
        let items = completion_items_at_cursor(
            "struct Point { x: i32, y: i32 }\n\
             fun main() {\n\tlet p = Point { x = 1, y = 2 };\n\tp.|\n}\n",
        );
        assert!(
            items.iter().any(|c| c.label == "x"),
            "the member list has the receiver's fields: {:?}",
            items.iter().map(|c| &c.label).collect::<Vec<_>>()
        );
        assert!(
            !items
                .iter()
                .any(|c| matches!(c.kind, CompletionKind::Snippet)),
            "member completion offers no construct snippets: {:?}",
            items.iter().map(|c| &c.label).collect::<Vec<_>>()
        );
    }

    // E14: an import path (`import st|`, which reaches scope completion — the
    // char before `st` is a space) offers no construct snippets; the post-pass
    // drops them. Bare keywords survive (so the list is non-vacuous), proving
    // the drop is targeted at snippets, not the whole list.
    #[test]
    fn construct_snippets_are_absent_in_import_path() {
        let items = completion_items_at_cursor("import st|\nfun main() {}\n");
        assert!(
            items
                .iter()
                .any(|c| matches!(c.kind, CompletionKind::Keyword)),
            "the import-path completion still ran (keywords present): {:?}",
            items.iter().map(|c| &c.label).collect::<Vec<_>>()
        );
        assert!(
            !items
                .iter()
                .any(|c| matches!(c.kind, CompletionKind::Snippet)),
            "import path offers no construct snippets: {:?}",
            items.iter().map(|c| &c.label).collect::<Vec<_>>()
        );
    }

    // E14: the snippet table stays a subset of the lexer's keywords, in lockstep
    // with `KEYWORD_DOCS` (WO-4's round-trip guard pattern) — every snippet rides
    // a real keyword that classifies back to itself and carries a doc entry. The
    // four named constructs are pinned in their exact order.
    #[test]
    fn construct_snippet_keywords_are_lexer_keywords() {
        let keywords: Vec<&str> = CONSTRUCT_SNIPPETS.iter().map(|(k, _, _, _)| *k).collect();
        assert_eq!(
            keywords,
            ["for", "fun", "struct", "match"],
            "the four named constructs, in order"
        );
        for (keyword, _label, _detail, _body) in CONSTRUCT_SNIPPETS {
            let (tokens, errors) = tokenize(keyword);
            assert!(errors.is_empty(), "{keyword} lexed with errors: {errors:?}");
            assert_eq!(tokens.len(), 1, "{keyword} should lex to one token");
            assert_eq!(
                keyword_lexeme(&tokens[0].0),
                Some(*keyword),
                "{keyword} must classify back to itself (subset of the lexer)"
            );
            assert!(
                KEYWORD_DOCS
                    .iter()
                    .any(|(documented, _, _)| documented == keyword),
                "{keyword} must have a KEYWORD_DOCS entry (lockstep)"
            );
        }
    }

    /// The shipped example projects must analyze cleanly through the *LSP* path
    /// (`Document::analyze` — project-context + `pkg::` + `std` resolution), not
    /// just the CLI. Guards against a regression where the editor surfaces errors
    /// the CLI doesn't, and pins that the RPC example's cross-file object-stub form
    /// stays diagnostic-free. Reads the real files, so an example edit that breaks
    /// analysis fails here.
    fn assert_example_analyzes_clean(relative: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let document = Document::analyze(&text, &std_root(), &path);
        let messages: Vec<String> = document
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.msg.clone())
            .collect();
        assert!(
            messages.is_empty(),
            "{relative}: expected no LSP diagnostics, got {messages:#?}"
        );
    }

    #[test]
    fn rpc_example_analyzes_without_diagnostics() {
        // The entry: the generated `[service(Client)]` paradigm over `std::rpc`
        // (the runtime module itself now lives in std).
        assert_example_analyzes_clean("../../vilan/examples/rpc/src/main.vl");
    }

    #[test]
    fn todo_example_analyzes_without_diagnostics() {
        // The realtime workspace: both entries import the shared `common`
        // library (`[derive(Wire)]` + a generated `[service(TodoClient)]`), and
        // the non-entry files (a package module, a `[library]` module — neither
        // has a `main`) must analyze via project context, not be rejected the
        // way a bare `vilan check <file>` would.
        assert_example_analyzes_clean("../../vilan/examples/todo/server/src/main.vl");
        assert_example_analyzes_clean("../../vilan/examples/todo/client/src/main.vl");
        assert_example_analyzes_clean("../../vilan/examples/todo/client/src/todos.vl");
        assert_example_analyzes_clean("../../vilan/examples/todo/common/src/lib.vl");
    }

    #[test]
    fn span_to_range_conversions_never_panic_on_multibyte_source() {
        // The RPC example's leading comment contains em-dashes (3-byte chars).
        // Converting an entity/symbol span whose byte boundary lands inside one
        // (documentSymbol, go-to-definition, diagnostics) used to panic the server
        // on a non-char-boundary string slice (`line_index.rs`). Drive the whole
        // span→range path the editor exercises on open, on the real file.
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/examples/rpc/src/main.vl");
        let text = std::fs::read_to_string(&path).unwrap();
        let document = Document::analyze(&text, &std_root(), &path);
        for symbol in document.document_symbols() {
            let _ = document.line_index.range(&symbol.full);
            let _ = document.line_index.range(&symbol.selection);
        }
        for (start, end, _) in &document.entity_spans {
            let _ = document.line_index.position(*start);
            let _ = document.line_index.position(*end);
        }
    }

    #[test]
    fn derive_synthesized_entities_are_excluded_from_the_user_file() {
        // `[derive(Json)] struct User` synthesizes `to_json`/`from_json` impls whose
        // spans are offsets into a *generated template*, not this file. They used to
        // be bundled into the entry's `SourceId(0)` range, so `source_of` reported
        // them as user-file entities and the editor placed them at those bogus
        // offsets — landing inside the leading comment (and, on the em-dash, crashing
        // position conversion). The fix attributes them to `DERIVED_SOURCE`, so they
        // are excluded from `entity_spans`/`document_symbols`. Pin that: the file's
        // first real token is `import` on line 9, so no user-file span may begin in
        // the comment block before it.
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/examples/rpc/src/main.vl");
        let text = std::fs::read_to_string(&path).unwrap();
        let first_code = text.find("import std::print").expect("first import");
        let document = Document::analyze(&text, &std_root(), &path);

        for (start, _end, id) in &document.entity_spans {
            assert!(
                *start >= first_code,
                "entity {id:?} span starts at {start}, inside the leading comment \
                 (a derive-synthesized entity leaking into the user file)"
            );
        }
        for symbol in document.document_symbols() {
            let start = symbol.selection.into_range().start;
            assert!(
                start >= first_code,
                "symbol {:?} selection starts at {start}, inside the leading comment",
                symbol.name
            );
        }
    }

    #[test]
    fn scope_completion_includes_top_level_and_keywords() {
        let labels = completions_at_cursor(
            "fun helper(): i32 { 42 }\nfun main() {\n\tlet value = hel|\n}\n",
        );
        assert!(
            labels.contains(&"helper".to_string()),
            "top-level: {labels:?}"
        );
        assert!(labels.contains(&"fun".to_string()), "keyword: {labels:?}");
    }

    #[test]
    fn path_completion_lists_enum_variants() {
        let labels = completions_at_cursor(
            "enum Color { Red, Green, Blue }\nfun main() {\n\tlet c = Color::|\n}\n",
        );
        assert!(labels.contains(&"Red".to_string()), "variants: {labels:?}");
        assert!(
            labels.contains(&"Green".to_string()),
            "variants: {labels:?}"
        );
        assert!(labels.contains(&"Blue".to_string()), "variants: {labels:?}");
    }

    const COUNTER: &str = "struct Counter { n: i32 }\n\
         impl Counter {\n\
         \tfun new(): Counter { Counter { n = 0 } }\n\
         \tfun bump(self): i32 { self.n + 1 }\n\
         }\n";

    #[test]
    fn member_completion_excludes_static_methods() {
        // `b.new()` would not type-check (`new` has no `self`), so it must not be
        // offered on `b.` — only `bump` (a `self` method) and the field `n`.
        let labels = completions_at_cursor(&format!(
            "{COUNTER}fun main() {{\n\tlet b = Counter {{ n = 0 }};\n\tb.|\n}}\n"
        ));
        assert!(
            labels.contains(&"bump".to_string()),
            "instance method: {labels:?}"
        );
        assert!(labels.contains(&"n".to_string()), "field: {labels:?}");
        assert!(
            !labels.contains(&"new".to_string()),
            "static excluded: {labels:?}"
        );
    }

    #[test]
    fn path_completion_lists_static_methods_not_instance() {
        let labels = completions_at_cursor(&format!(
            "{COUNTER}fun main() {{\n\tlet c = Counter::|\n}}\n"
        ));
        assert!(
            labels.contains(&"new".to_string()),
            "static method: {labels:?}"
        );
        assert!(
            !labels.contains(&"bump".to_string()),
            "instance excluded: {labels:?}"
        );
    }

    // --- E8: editor support for macros ---

    // Hover on a macro attribute shows the macro's signature; definition
    // jumps to the `macro fun` (same file here).
    #[test]
    fn macro_attribute_hover_and_definition() {
        let source = "macro fun derive_tag(item: Item): Source {\n\timport macro_std::source;\n\timport macro_std::meta::{ Item, Source };\n\tsource(\"\")\n}\n\n[derive_tag]\nstruct Point {\n\tx: i32,\n}\n\nfun main() {}\n\nmain();\n";
        let (_dir, document) = analyze_workspace(&[("main.vl", source)]);
        // The attribute site is the SECOND occurrence of the name.
        let definition_at = source.find("derive_tag").unwrap();
        let use_at = source[definition_at + 1..].find("derive_tag").unwrap() + definition_at + 1;
        let hover = document
            .hover(use_at + 2)
            .expect("hover on the attribute name");
        assert!(
            hover.contains("macro fun derive_tag(item: Item): Source"),
            "hover should show the signature, got: {hover}"
        );
        let (source_id, span) = document
            .definition(use_at + 2)
            .expect("definition on the attribute name");
        assert_eq!(source_id, vilan_core::analyzer::SourceId(0));
        assert_eq!(
            span.into_range().start,
            definition_at,
            "definition should land on the macro fun's name"
        );
    }

    // A prelude derive navigates CROSS-FILE into std (compare.vl).
    #[test]
    fn prelude_derive_definition_reaches_std() {
        let source =
            "[derive(PartialEq)]\nstruct Point {\n\tx: i32,\n}\n\nfun main() {}\n\nmain();\n";
        let (_dir, document) = analyze_workspace(&[("main.vl", source)]);
        let use_at = source.find("PartialEq").unwrap();
        let hover = document
            .hover(use_at + 2)
            .expect("hover on the derive name");
        assert!(
            hover.contains("macro fun PartialEq(item: Item): Source"),
            "hover should show the prelude macro's signature, got: {hover}"
        );
        let (source_id, _span) = document
            .definition(use_at + 2)
            .expect("definition on the derive name");
        assert_ne!(
            source_id,
            vilan_core::analyzer::SourceId(0),
            "the definition lives in std's compare.vl, not the entry"
        );
    }

    // --- WO-2: Organize Imports (sort + conservative prune) ----------------
    //
    // A helper package for the mechanics pins: two free functions and a struct,
    // so imports resolve without depending on std's exact surface. The
    // derive-survival pin needs a real derive, so it uses `std::json`.
    const ORGANIZE_HELPER: &str = "fun alpha() {}\nfun beta() {}\nstruct Widget {}\n";

    /// Applies the Organize Imports edits to the document's text (back-to-front,
    /// so earlier offsets stay valid), or `None` when the action offers no edit.
    fn organized(document: &Document) -> Option<String> {
        let mut edits = document.organize_import_edits();
        if edits.is_empty() {
            return None;
        }
        edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.into_range().start));
        let mut text = document.text.clone();
        for (span, replacement) in edits {
            text.replace_range(span.into_range(), &replacement);
        }
        Some(text)
    }

    // A shuffled top-level run sorts into canonical order; both imports are used,
    // so nothing is pruned.
    #[test]
    fn organize_sorts_a_shuffled_run() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import pkg::helper::beta;\nimport pkg::helper::alpha;\nfun main() {\n\talpha();\n\tbeta();\n}\n",
            ),
            ("helper.vl", ORGANIZE_HELPER),
        ]);
        assert!(
            document.diagnostics.is_empty(),
            "{:?}",
            document.diagnostics
        );
        let result = organized(&document).expect("a shuffled run offers a sort edit");
        assert_eq!(
            result,
            "import pkg::helper::alpha;\nimport pkg::helper::beta;\nfun main() {\n\talpha();\n\tbeta();\n}\n",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // An import referenced nowhere is pruned; the used one stays.
    #[test]
    fn organize_prunes_an_unused_import() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import pkg::helper::alpha;\nimport pkg::helper::beta;\nfun main() {\n\talpha();\n}\n",
            ),
            ("helper.vl", ORGANIZE_HELPER),
        ]);
        assert!(
            document.diagnostics.is_empty(),
            "{:?}",
            document.diagnostics
        );
        let result = organized(&document).expect("an unused import offers a prune edit");
        assert_eq!(
            result,
            "import pkg::helper::alpha;\nfun main() {\n\talpha();\n}\n",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A brace set with one dead branch shrinks to the live branch; the whole
    // import survives because a live branch remains.
    #[test]
    fn organize_shrinks_a_brace_set_to_its_used_branch() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import pkg::helper::{ alpha, beta };\nfun main() {\n\talpha();\n}\n",
            ),
            ("helper.vl", ORGANIZE_HELPER),
        ]);
        assert!(
            document.diagnostics.is_empty(),
            "{:?}",
            document.diagnostics
        );
        let result = organized(&document).expect("a dead branch offers a shrink edit");
        assert_eq!(
            result,
            "import pkg::helper::{ alpha };\nfun main() {\n\talpha();\n}\n",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // An import referenced ONLY by derive-generated code survives: the code
    // `[derive(Json)]` synthesizes (the `impl Json for Point`) references `Json`,
    // and the analyzer attributes that reference to this file. The empty-diags
    // assert guards against a vacuous pass (a diagnostic would disable pruning).
    #[test]
    fn organize_keeps_an_import_used_only_by_a_derive() {
        let (dir, document) = analyze_workspace(&[(
            "main.vl",
            "import std::json::Json;\n[derive(Json)]\nstruct Point {\n\tx: i32,\n\ty: i32,\n}\nfun make(): Point {\n\tPoint { x = 1, y = 2 }\n}\n",
        )]);
        assert!(
            document.diagnostics.is_empty(),
            "{:?}",
            document.diagnostics
        );
        assert_eq!(
            organized(&document),
            None,
            "an import used only by a derive was pruned",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A re-export is public surface, not local usage — never pruned, even when
    // its name is used nowhere in this file.
    #[test]
    fn organize_never_prunes_a_reexport() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "export import pkg::helper::alpha;\nfun main() {}\n",
            ),
            ("helper.vl", ORGANIZE_HELPER),
        ]);
        assert!(
            document.diagnostics.is_empty(),
            "{:?}",
            document.diagnostics
        );
        assert_eq!(organized(&document), None, "a re-export was pruned");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A file with diagnostics still sorts (a mid-edit error disables pruning, not
    // sorting): the run reorders but the unused import is NOT pruned. Both halves
    // asserted.
    #[test]
    fn organize_with_diagnostics_sorts_but_does_not_prune() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import pkg::helper::beta;\nimport pkg::helper::alpha;\nfun main() {\n\talpha();\n\tundefined_name();\n}\n",
            ),
            ("helper.vl", ORGANIZE_HELPER),
        ]);
        assert!(
            !document.diagnostics.is_empty(),
            "the entry must carry the unresolved-name error",
        );
        let result = organized(&document).expect("sorting is still offered under diagnostics");
        assert!(
            result.contains("import pkg::helper::beta;"),
            "beta was pruned despite diagnostics:\n{result}",
        );
        let alpha_at = result.find("helper::alpha").unwrap();
        let beta_at = result.find("helper::beta").unwrap();
        assert!(alpha_at < beta_at, "the run was not sorted:\n{result}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Block-scoped imports (inside a fn body) are deliberate placements the
    // organizer never touches: they live in a block body, not the top-level item
    // list it walks. The file still parses (the shuffled block `use`s are valid
    // syntax — they don't resolve, which is backlog H2, but the organizer skips
    // them structurally either way), so a no-op here proves the organizer never
    // reached into the block to reorder them.
    #[test]
    fn organize_leaves_block_scoped_imports_alone() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "fun main() {\n\tuse std::collections::Map;\n\tuse std::collections::Set;\n}\n",
            ),
            ("helper.vl", ORGANIZE_HELPER),
        ]);
        assert_eq!(
            organized(&document),
            None,
            "the organizer reached into a block and reordered its imports",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // An already-organized file (sorted, nothing dead) offers no edit — the
    // no-op the action relies on to stay quiet under `codeActionsOnSave`.
    #[test]
    fn organize_is_a_no_op_when_already_organized() {
        let (dir, document) = analyze_workspace(&[
            (
                "main.vl",
                "import pkg::helper::alpha;\nimport pkg::helper::beta;\nfun main() {\n\talpha();\n\tbeta();\n}\n",
            ),
            ("helper.vl", ORGANIZE_HELPER),
        ]);
        assert!(
            document.diagnostics.is_empty(),
            "{:?}",
            document.diagnostics
        );
        assert_eq!(
            organized(&document),
            None,
            "an already-organized file offered an edit",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- WO-5: LSP features survive recoverable errors ---------------------
    //
    // Since the handwritten frontend cut over (H6 S5), `parsing::parse` salvages
    // the parsed prefix of a broken file and `analyze_source` runs the analyzer
    // on that partial tree, so `Document::program` is `Some` for everything that
    // parsed — a syntax error no longer blanks the file. These pins prove every
    // position-based feature keeps serving the salvaged program, per input class.
    //
    // A mid-file error inside a `{}` body: the delimiter recovery closes the
    // broken region and parsing continues, so the items above AND below the
    // error all survive. `let x = ;` is the syntax error; nothing else is wrong.
    const RECOVERABLE_INBODY: &str = "struct Widget { size: i32 }\n\nfun above(w: Widget): i32 {\n\tw.size\n}\n\nfun broken() {\n\tlet x = ;\n}\n\nfun below(): i32 {\n\thelper()\n}\n\nfun helper(): i32 {\n\t7\n}\n";
    // A stray token at file scope: the top-level statement loop declines and
    // stops, so only the PREFIX (everything before the token) is salvaged; the
    // tail after it is not recovered. This is the other salvage regime.
    const RECOVERABLE_TOPLEVEL: &str =
        "fun above(): i32 {\n\t42\n}\n\n$ garbage here $\n\nfun below(): i32 {\n\t7\n}\n";
    // A clean parse with an analyzer error in the middle (`no_such_name` is
    // unresolved): the whole program is present, so every feature works on both
    // sides of the erroring expression.
    const ANALYZER_ONLY: &str = "struct Point { x: i32 }\n\nfun before(): i32 {\n\tlet p = Point { x = 1 };\n\tp.x\n}\n\nfun uses_undefined(): i32 {\n\tno_such_name()\n}\n\nfun after(): i32 {\n\tlet q = Point { x = 2 };\n\tq.x\n}\n";

    fn analyze_text(text: &str) -> Document {
        Document::analyze(text, &std_root(), Path::new("test.vl"))
    }

    /// The byte offset of `needle` in `text`, plus `delta` (to land inside the
    /// matched identifier). Panics if the needle is absent, so a source edit that
    /// invalidates a pin fails loudly rather than silently probing offset 0.
    fn offset_at(text: &str, needle: &str, delta: usize) -> usize {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not found in the pin source"))
            + delta
    }

    // A syntax error mid-file leaves `program` present (salvage) and the
    // diagnostics non-empty — the precondition every pin below relies on. Guards
    // against a source that accidentally became clean (a vacuous pass).
    #[test]
    fn a_recoverable_source_still_yields_a_program() {
        let document = analyze_text(RECOVERABLE_INBODY);
        assert!(
            document.program.is_some(),
            "the salvaged tree must still analyze to a program",
        );
        assert!(
            !document.diagnostics.is_empty(),
            "the syntax error must be reported",
        );
    }

    // Hover on a function name ABOVE a mid-file syntax error shows its full
    // signature (the salvaged program still carries the declaration).
    #[test]
    fn hover_above_a_syntax_error_shows_the_signature() {
        let document = analyze_text(RECOVERABLE_INBODY);
        let hover = document
            .hover(offset_at(RECOVERABLE_INBODY, "fun above", 4))
            .expect("hovering `above` above the error");
        assert!(hover.contains("fun above(w: Widget): i32"), "{hover}",);
    }

    // Hover on a TYPE reference above the error resolves to its declaration —
    // the type_references surface survives salvage.
    #[test]
    fn hover_on_a_type_above_a_syntax_error_shows_its_declaration() {
        let document = analyze_text(RECOVERABLE_INBODY);
        let hover = document
            .hover(offset_at(RECOVERABLE_INBODY, "w: Widget", 3))
            .expect("hovering the `Widget` annotation above the error");
        assert!(hover.contains("struct Widget"), "{hover}");
    }

    // Go-to-definition from a use-site above the error jumps to the binding it
    // resolves to (here the parameter `w`).
    #[test]
    fn goto_definition_above_a_syntax_error_resolves() {
        let document = analyze_text(RECOVERABLE_INBODY);
        let (source, span) = document
            .definition(offset_at(RECOVERABLE_INBODY, "w.size", 0))
            .expect("go-to-def on `w` above the error");
        assert_eq!(source, SourceId(0));
        assert_eq!(
            &RECOVERABLE_INBODY[span.into_range()],
            "w",
            "the jump target is the parameter's name",
        );
    }

    // Scope completion inside a function above the error offers the scope
    // entities: the local parameter (`w`) and a top-level sibling (`helper`),
    // proving both the local and the global scope survive salvage.
    #[test]
    fn completion_above_a_syntax_error_offers_scope_entities() {
        let document = analyze_text(RECOVERABLE_INBODY);
        let labels: Vec<String> = document
            .completion(offset_at(RECOVERABLE_INBODY, "w.size", 0))
            .into_iter()
            .map(|completion| completion.label)
            .collect();
        assert!(labels.contains(&"w".to_string()), "local param: {labels:?}");
        assert!(
            labels.contains(&"helper".to_string()),
            "top-level sibling: {labels:?}",
        );
    }

    // Member completion on a receiver above the error lists the receiver's
    // fields — the receiver's type still resolves in the salvaged program.
    #[test]
    fn member_completion_above_a_syntax_error_lists_fields() {
        let document = analyze_text(RECOVERABLE_INBODY);
        let labels: Vec<String> = document
            .completion(offset_at(RECOVERABLE_INBODY, "w.size", 2))
            .into_iter()
            .map(|completion| completion.label)
            .collect();
        assert!(labels.contains(&"size".to_string()), "{labels:?}");
    }

    // Semantic tokens cover the salvaged region (non-empty) and the pass never
    // panics on a partial program.
    #[test]
    fn semantic_tokens_cover_a_salvaged_region() {
        let document = analyze_text(RECOVERABLE_INBODY);
        assert!(
            !document.semantic_tokens().is_empty(),
            "the salvaged declarations must still tokenize",
        );
    }

    // Document symbols list every salvaged item — the outline survives a mid-file
    // error (an in-body error recovers, so the tail items are here too).
    #[test]
    fn document_symbols_list_the_salvaged_items() {
        let document = analyze_text(RECOVERABLE_INBODY);
        let names: Vec<String> = document
            .document_symbols()
            .into_iter()
            .map(|symbol| symbol.name)
            .collect();
        for expected in ["Widget", "above", "broken", "below", "helper"] {
            assert!(
                names.contains(&expected.to_string()),
                "missing `{expected}` from the outline: {names:?}",
            );
        }
    }

    // The reality of the in-body regime: an error inside a `{}` body is
    // delimiter-recovered, so the items AFTER it survive too — hover and
    // go-to-def work on a function declared below the broken one.
    #[test]
    fn an_in_body_error_keeps_the_items_after_it() {
        let document = analyze_text(RECOVERABLE_INBODY);
        let hover = document
            .hover(offset_at(RECOVERABLE_INBODY, "fun below", 4))
            .expect("hovering `below`, declared after the error");
        assert!(hover.contains("fun below(): i32"), "{hover}");
        let (source, span) = document
            .definition(offset_at(RECOVERABLE_INBODY, "helper()", 0))
            .expect("go-to-def on `helper`, called and declared after the error");
        assert_eq!(source, SourceId(0));
        assert_eq!(&RECOVERABLE_INBODY[span.into_range()], "helper");
    }

    // The reality of the top-level regime: a stray token at file scope stops the
    // statement loop, so only the prefix is salvaged. `above` (before) works;
    // `below` (after) is not in the program at all. Contrast the in-body case.
    #[test]
    fn a_top_level_error_salvages_the_prefix_and_drops_the_tail() {
        let document = analyze_text(RECOVERABLE_TOPLEVEL);
        assert!(document.program.is_some());
        assert!(!document.diagnostics.is_empty());
        let names: Vec<String> = document
            .document_symbols()
            .into_iter()
            .map(|symbol| symbol.name)
            .collect();
        assert!(
            names.contains(&"above".to_string()),
            "prefix kept: {names:?}"
        );
        assert!(
            !names.contains(&"below".to_string()),
            "the tail after a top-level stray token is not recovered: {names:?}",
        );
        assert!(
            document
                .hover(offset_at(RECOVERABLE_TOPLEVEL, "fun above", 4))
                .is_some_and(|hover| hover.contains("fun above(): i32")),
            "the prefix item still hovers",
        );
        assert_eq!(
            document.hover(offset_at(RECOVERABLE_TOPLEVEL, "fun below", 4)),
            None,
            "the dropped tail item has nothing to hover",
        );
    }

    // A clean parse with an analyzer error in the middle: hover and go-to-def
    // work on BOTH sides of the erroring expression (the whole program is
    // present — a diagnostic never blanks a feature).
    #[test]
    fn hover_and_goto_work_on_both_sides_of_an_analyzer_error() {
        let document = analyze_text(ANALYZER_ONLY);
        assert!(
            document
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.msg.contains("cannot find 'no_such_name'")),
            "the analyzer error must be present: {:?}",
            document.diagnostics,
        );
        // Before the error.
        assert!(
            document
                .hover(offset_at(ANALYZER_ONLY, "Point { x = 1 }", 0))
                .is_some_and(|hover| hover.contains("struct Point")),
            "hover before the error",
        );
        let (source, span) = document
            .definition(offset_at(ANALYZER_ONLY, "Point { x = 1 }", 0))
            .expect("go-to-def before the error");
        assert_eq!(
            (source, &ANALYZER_ONLY[span.into_range()]),
            (SourceId(0), "Point")
        );
        // After the error.
        assert!(
            document
                .hover(offset_at(ANALYZER_ONLY, "Point { x = 2 }", 0))
                .is_some_and(|hover| hover.contains("struct Point")),
            "hover after the error",
        );
        let (source, span) = document
            .definition(offset_at(ANALYZER_ONLY, "Point { x = 2 }", 0))
            .expect("go-to-def after the error");
        assert_eq!(
            (source, &ANALYZER_ONLY[span.into_range()]),
            (SourceId(0), "Point")
        );
        // The full outline and tokens are present (nothing degraded).
        assert_eq!(document.document_symbols().len(), 4);
        assert!(!document.semantic_tokens().is_empty());
    }

    // The graceful-empty case: a hopeless file panics nowhere and every feature
    // returns cleanly. The test completing IS the no-panic proof; the empty
    // assertions pin that a program with no salvageable items answers with
    // nothing rather than garbage.
    #[test]
    fn a_hopeless_file_answers_every_feature_without_panicking() {
        for source in [
            "@@@@ !!!! $$$$ %%%%\n",
            "",
            "))))]]]]}}}}\n",
            "12 34 fun fun",
        ] {
            let document = analyze_text(source);
            // Position queries at several offsets, all in bounds.
            for offset in [0, source.len() / 2, source.len()] {
                let _ = document.hover(offset);
                let _ = document.definition(offset);
                let _ = document.completion(offset);
                let _ = document.references(offset);
            }
            assert!(
                document.document_symbols().is_empty(),
                "no items to outline in {source:?}",
            );
            assert!(
                document.semantic_tokens().is_empty(),
                "no tokens in {source:?}",
            );
            assert!(document.inlay_hints().is_empty(), "no hints in {source:?}");
            assert!(
                document.organize_import_edits().is_empty(),
                "no imports to organize in {source:?}",
            );
        }
    }

    // Formatting a source that does not parse cleanly degrades to no edit: the
    // formatter's net requires a clean parse, so `format` returns the input
    // verbatim, and the LSP `formatting` handler turns `formatted == source`
    // into `Ok(None)` — no edit, no error popup.
    #[test]
    fn a_broken_source_formats_to_no_edit() {
        for source in [RECOVERABLE_INBODY, RECOVERABLE_TOPLEVEL] {
            assert_eq!(
                vilan_core::formatter::format(source),
                source,
                "a non-clean source must format to itself (the handler then emits no edit)",
            );
        }
    }
}

#[cfg(test)]
mod leak_measurement {
    use super::*;
    use crate::document::tests::std_root;
    use vilan_core::leak_tally::{self, LeakSite};

    /// Resident set size in KiB, from /proc/self/statm (Linux pages × 4).
    fn rss_kib() -> usize {
        let statm = std::fs::read_to_string("/proc/self/statm").expect("statm");
        let pages: usize = statm
            .split_whitespace()
            .nth(1)
            .expect("resident field")
            .parse()
            .expect("resident pages");
        pages * 4
    }

    /// The macro-expansion leak sites. analysis-reuse.md §2's fix routes the
    /// stamped `parse_generated` calls through the content cache, so after an
    /// unchanged program's expansions are cached these must PLATEAU — leak zero
    /// further bytes per analysis. (Before the fix, a gensym-stamped expansion
    /// re-leaked its parse every analysis, the true per-keystroke leak.)
    const MACRO_SITES: &[LeakSite] = &[
        LeakSite::MacroParseText,
        LeakSite::MacroParseAst,
        LeakSite::MacroExpansion,
        LeakSite::MacroWorldText,
        LeakSite::MacroWorldProgram,
        LeakSite::MacroPreludeText,
        LeakSite::MacroBlockEntryName,
    ];

    fn macro_bytes() -> usize {
        MACRO_SITES.iter().copied().map(leak_tally::bytes).sum()
    }

    /// The per-analysis leak counted over the `measured` window, plus the RSS
    /// growth (a noisy report, never asserted on). Built on the analysis thread
    /// — the counters are thread-local, so a snapshot read after the loop on the
    /// same thread tallies exactly these analyses and nothing a parallel test
    /// leaked (the E12 flaky-global-counter lesson).
    struct LeakReport {
        rss_grown: usize,
        entry_text: usize,
        entry_ast: usize,
        display: usize,
        /// The two sites analysis-reuse.md §2 fixes: `parse_generated`'s leaked
        /// source and AST, reached from the stamped expansion paths.
        stamped_parse: usize,
        macro_bytes: usize,
        total: usize,
        measured: usize,
    }

    impl LeakReport {
        fn print(&self, label: &str) {
            println!(
                "[{label}] RSS +{} KiB ≈ {:.1} KiB/analysis over {} analyses (report only)",
                self.rss_grown,
                self.rss_grown as f64 / self.measured as f64,
                self.measured,
            );
            println!(
                "[{label}] counted leak over {} analyses: entry-text {} B, entry-AST {} B, \
                 display {} B, macro {} B, total {} B ≈ {:.0} B/analysis",
                self.measured,
                self.entry_text,
                self.entry_ast,
                self.display,
                self.macro_bytes,
                self.total,
                self.total as f64 / self.measured as f64,
            );
        }
    }

    /// Runs `warmup` then `measured` analyses of `text_at(i)` **on the current
    /// thread** (via `analyze_on_this_thread`, so the leaks land in this
    /// thread's `leak_tally`), zeroing the counters after warmup. Callers must
    /// invoke this on a big-stack thread — the pipeline nests a full analysis
    /// inside macro-world compiles.
    fn measure(text_at: impl Fn(usize) -> String, warmup: usize, measured: usize) -> LeakReport {
        let dir = std::env::temp_dir().join(format!("vilan_leak_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("main.vl");
        let std_dir = std_root();
        // Warmup fills every content-addressed cache (the reachable std, the
        // module parses, the macro worlds and their stamped expansions) so the
        // measured window sees only the genuinely per-analysis leaks.
        for i in 0..warmup {
            let _ = Document::analyze_on_this_thread(&text_at(i), &std_dir, &entry);
        }
        leak_tally::reset();
        let before_rss = rss_kib();
        for i in warmup..warmup + measured {
            let _ = Document::analyze_on_this_thread(&text_at(i), &std_dir, &entry);
        }
        let report = LeakReport {
            rss_grown: rss_kib().saturating_sub(before_rss),
            entry_text: leak_tally::bytes(LeakSite::LspEntryText),
            entry_ast: leak_tally::bytes(LeakSite::EntryAst),
            display: leak_tally::bytes(LeakSite::DisplayName),
            stamped_parse: leak_tally::bytes(LeakSite::MacroParseText)
                + leak_tally::bytes(LeakSite::MacroParseAst),
            macro_bytes: macro_bytes(),
            total: leak_tally::total(),
            measured,
        };
        let _ = std::fs::remove_dir_all(&dir);
        report
    }

    fn on_big_stack(work: impl FnOnce() -> LeakReport + Send + 'static) -> LeakReport {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(work)
            .expect("spawn measurement thread")
            .join()
            .expect("measurement thread panicked")
    }

    // A changing, std-using document with no macros. Each `i` differs (a
    // keystroke), so every analysis re-parses and re-analyzes.
    fn no_macro_text(i: usize) -> String {
        format!(
            "import std::print;\nimport std::option::Option::{{ self, Some, None }};\n\n\
             fun describe(value: Option<i32>): str {{\n\
             \tmatch value {{\n\t\tSome(let n) => int_to_string(n),\n\t\tNone => \"empty {i}\",\n\t}}\n}}\n\n\
             fun int_to_string(n: i32): str {{\n\t\"n\"\n}}\n\n\
             fun main() {{\n\tlet value = Some({i});\n\tprint(describe(value));\n\tprint(describe(None));\n}}\n"
        )
    }

    // The Phase-1 pin (analysis-reuse.md §2): a changing, std-using document
    // that uses NO macros. After warmup the ONLY per-analysis leaks must be the
    // named, file-size-proportional ones — entry source text and entry AST (no
    // dependency packages, so no display names) — and nothing on the macro path
    // or any other site. RSS is far noisier (allocator retention from rebuilding
    // the reachable `Program`); it is printed, never asserted.
    #[test]
    fn per_analysis_leak_is_bounded_by_named_sites() {
        let warmup = 20;
        let measured = 200;
        let report = on_big_stack(move || measure(no_macro_text, warmup, measured));
        report.print("no-macro");

        // The counted per-analysis leak is EXACTLY the named sites — every other
        // leak site (macro path, the content-keyed module parses, the loader's
        // error path) contributed zero over the measured window.
        assert_eq!(
            report.total,
            report.entry_text + report.entry_ast + report.display,
            "an unnamed leak site grew per analysis: total {} B, named {} B",
            report.total,
            report.entry_text + report.entry_ast + report.display,
        );
        assert_eq!(
            report.macro_bytes, 0,
            "a non-macro document leaked {} macro bytes over {} analyses",
            report.macro_bytes, report.measured,
        );
        // The entry source is the dominant named leak and is file-proportional:
        // it is exactly the bytes of every analyzed text (each keystroke leaks
        // its own source copy — the recorded, still-open refinement).
        let expected_entry_text: usize = (warmup..warmup + measured)
            .map(|i| no_macro_text(i).len())
            .sum();
        assert_eq!(
            report.entry_text, expected_entry_text,
            "entry-text leak {} B is not the sum of analyzed source lengths {} B",
            report.entry_text, expected_entry_text,
        );
    }

    // A document that defines and invokes an expression-position macro emitting
    // a `fresh()` gensym: its output is `__s<site>_m<N>`-stamped, the path that
    // used to `parse_generated` uncached (analysis-reuse.md §2). `tail` changes
    // per analysis (a keystroke that does not touch the macro), but always four
    // digits — so the length-preserving world blanking maps every analysis to a
    // byte-identical blanked source, and the macro world (which the macro
    // definition living in this file would otherwise recompile on every edit)
    // stays cached. The changing invocation is thus isolated: the only thing
    // that could re-leak on the macro path is the stamped expansion's parse.
    fn gensym_text(tail: usize) -> String {
        format!(
            "import std::print;\n\n\
             macro fun unroll(arguments: Arguments): Source {{\n\
             \timport macro_std::source;\n\
             \timport macro_std::fresh;\n\
             \timport macro_std::meta::{{ Arguments, Source }};\n\
             \timport macro_std::option::Option::{{ self, Some, None }};\n\
             \tlet count = match arguments.as_i32(0) {{\n\t\tSome(let n) => n,\n\t\tNone => 0,\n\t}};\n\
             \tlet binder = fresh();\n\
             \tmut sum = \"0\";\n\
             \tmut index = 0;\n\
             \tfor index < count {{\n\t\tsum = sum + i\" + {{binder}}({{index}})\";\n\t\tindex = index + 1;\n\t}}\n\
             \tsource(i\"\\{{ let {{binder}} = (|x: i32| x + 1); {{sum}} \\}}\")\n\
             }}\n\n\
             fun main() {{\n\tlet unrolled = macro unroll(4);\n\tprint(unrolled);\n\tlet tail = {tail:04};\n\tprint(tail);\n}}\n\n\
             main();\n"
        )
    }

    // The gensym plateau — the leak analysis-reuse.md §2 actually fixes. Before
    // the fix the stamped expression parse re-leaked every analysis; after, the
    // content cache (keyed on the site-stamped text) makes it plateau to zero.
    #[test]
    fn gensym_expansion_leak_plateaus() {
        // `tail` stays four digits so the blanked world source is byte-stable.
        let warmup = 8;
        let measured = 40;
        let report = on_big_stack(move || {
            let report = measure(|i| gensym_text(1000 + i), warmup, measured);
            for site in MACRO_SITES {
                println!("[gensym] {:?} = {} B", site, leak_tally::bytes(*site));
            }
            report
        });
        report.print("gensym");

        // The §2-fixed sites: the stamped expression parse is content-cached, so
        // the warm, unchanged invocation re-leaks nothing.
        assert_eq!(
            report.stamped_parse, 0,
            "the gensym expansion's stamped parse re-leaked {} B over {} analyses — \
             `parse_generated` is not being content-cached (analysis-reuse.md §2)",
            report.stamped_parse, report.measured,
        );
        // With the world cached (fixed-width tail) the WHOLE macro path plateaus.
        assert_eq!(
            report.macro_bytes, 0,
            "the macro path re-leaked {} B over {} analyses (see the per-site breakdown above)",
            report.macro_bytes, report.measured,
        );
        // The entry still leaks per analysis (the changing tail), so the fixture
        // genuinely re-analyzes each round rather than short-circuiting.
        assert!(
            report.entry_text > 0,
            "the changing gensym fixture leaked no entry text — it may not be re-analyzing",
        );
    }
}
