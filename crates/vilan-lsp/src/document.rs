//! Per-document analysis state and the navigation queries the language-server
//! handlers run against it: position→entity lookup, hover, go-to-definition,
//! find-references, and rename.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use vilan_core::analyzer::{DERIVED_SOURCE, Expr, Implementation, SourceId};
use vilan_core::id::Id;
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

/// A completion candidate offered at the cursor (mapped to an LSP `CompletionItem`
/// by the server).
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
}

/// The category of a completion, for its editor icon.
pub enum CompletionKind {
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
}

/// The language keywords offered in scope-position completion.
const KEYWORDS: &[&str] = &[
    "fun", "struct", "enum", "impl", "trait", "let", "mut", "own", "import", "use", "mod", "for",
    "in", "is", "match", "if", "else", "async", "await", "return", "ret", "jump", "type", "with",
    "export", "external", "true", "false", "null",
];

pub struct Document {
    pub line_index: LineIndex,
    pub program: Option<Program<'static>>,
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

/// One diagnostic as the language server publishes it: the file it belongs to
/// (`None` = the analyzed document itself), its span *in that file's text*, the
/// message, and the severity. LSP-type-free so the grouping is unit-testable.
pub struct PublishedDiagnostic {
    pub path: Option<PathBuf>,
    pub span: Span,
    pub message: String,
    pub warning: bool,
    /// The diagnostic's secondary note (diagnostics-standard.md C3), same
    /// source file as the primary span in v1 — published as LSP related
    /// information for entry-attributed diagnostics.
    pub note: Option<(Span, String)>,
}

/// The span of an entity, flattened from the `&Span` stored in `span_map`.
fn span_of(program: &Program, id: Id) -> Option<Span> {
    program.span_map.get(&id).map(|span| **span)
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

        Document {
            line_index,
            program,
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
                    note: error.note.clone(),
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
        let type_label = self.hover_label(program, id);
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
        let mut docs: Vec<String> = Vec::new();
        while let Some(last) = lines.last() {
            let trimmed = last.trim();
            let Some(comment) = trimmed.strip_prefix("//") else {
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
        if start >= 1 && bytes[start - 1] == b'.' {
            // `a?.` completes on the LIFTED element (`Option<Profile>` offers
            // Profile's members — proposal/try-and-lift.md §5).
            if start >= 2 && bytes[start - 2] == b'?' {
                return self.lifted_member_completions(program, start - 2);
            }
            return self.member_completions(program, start - 1);
        }
        if start >= 2 && bytes[start - 1] == b':' && bytes[start - 2] == b':' {
            return self.path_completions(program, text, start - 2);
        }
        self.scope_completions(program, offset)
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
                items.push(Completion {
                    label: field.name.to_string(),
                    kind: CompletionKind::Field,
                });
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
                    items.push(Completion {
                        label: variant.name.to_string(),
                        kind: CompletionKind::EnumVariant,
                    });
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
                        items.push(Completion {
                            label: name.to_string(),
                            kind: self.kind_of(program, *id),
                        });
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
                    items.push(Completion {
                        label: name.to_string(),
                        kind: self.kind_of(program, *entity_id),
                    });
                }
            }
            scope_id = scope.parent_id;
        }
        for keyword in KEYWORDS {
            items.push(Completion {
                label: keyword.to_string(),
                kind: CompletionKind::Keyword,
            });
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
                        items.push(Completion {
                            label: name.to_string(),
                            kind: CompletionKind::Method,
                        });
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn std_root() -> PathBuf {
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
    fn analyze_workspace(files: &[(&str, &str)]) -> (PathBuf, Document) {
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

    // E9: the declaration's leading `//` block surfaces as prose, and
    // attribute lines between it and the name don't break the chain.
    #[test]
    fn hover_surfaces_the_leading_doc_comment() {
        let hover = hover_at_cursor(
            "import std::print;\n\n// Renders the badge label.\n// Two lines of docs.\n[must_use]\nfun bad|ge(count: i32): str {\n\t\"b\"\n}\n\nfun main() {\n\tlet _b = badge(1);\n\tprint(\"x\");\n}\n",
        )
        .expect("hover on the declaration");
        assert!(
            hover.contains("Renders the badge label.\nTwo lines of docs."),
            "{hover}"
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
        let offset = src
            .find('|')
            .expect("test source needs a `|` cursor marker");
        let text = src.replace('|', "");
        let document = Document::analyze(&text, &std_root(), Path::new("test.vl"));
        document
            .completion(offset)
            .into_iter()
            .map(|completion| completion.label)
            .collect()
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
}
