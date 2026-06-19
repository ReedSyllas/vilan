//! Per-document analysis state and the navigation queries the language-server
//! handlers run against it: position→entity lookup, hover, go-to-definition,
//! find-references, and rename.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use vilan_core::analyzer::{Expr, Implementation, SourceId};
use vilan_core::id::Id;
use vilan_core::type_::Type;
use vilan_core::{Error, Program, Span, Target as BuildTarget, analyze_source};

use crate::line_index::LineIndex;

/// The build target for a file, resolved from its project's `vilan.toml` when the
/// file is a declared entry: the `[client]` entry is a browser build, a
/// `[server]`/`[package]` entry a Node build. `None` if there's no project or the
/// file isn't an entry (a module / shared file) — analysis then infers the target
/// from the file's own imports.
fn resolve_manifest_target(entry_path: &Path) -> Option<BuildTarget> {
    // The nearest ancestor directory holding a `vilan.toml`.
    let mut directory = entry_path.parent();
    let (manifest, root) = loop {
        let current = directory?;
        let candidate = current.join("vilan.toml");
        if candidate.is_file() {
            break (candidate, current);
        }
        directory = current.parent();
    };
    let table: toml::Table = std::fs::read_to_string(&manifest).ok()?.parse().ok()?;
    let entry = |section: &str, default: Option<&str>| -> Option<PathBuf> {
        table
            .get(section)
            .and_then(|section| section.get("entry"))
            .and_then(|entry| entry.as_str())
            .or(default)
            .map(|entry| root.join(entry))
    };
    let is = |entry: Option<PathBuf>| {
        entry.is_some_and(|entry| {
            match (
                std::fs::canonicalize(&entry),
                std::fs::canonicalize(entry_path),
            ) {
                (Ok(a), Ok(b)) => a == b,
                _ => entry == entry_path,
            }
        })
    };
    if is(entry("client", None)) {
        Some(BuildTarget::Browser)
    } else if is(entry("server", None)) || is(entry("package", Some("main.vl"))) {
        Some(BuildTarget::Node)
    } else {
        None
    }
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
    /// A hash of the source text this document was analyzed from, so an edit that
    /// leaves the buffer unchanged can skip re-analysis.
    pub text_hash: u64,
    /// `(start, end, id)` for every entry-file entity with a real span, used to
    /// find the innermost entity under a cursor.
    entity_spans: Vec<(usize, usize, Id)>,
}

/// The span of an entity, flattened from the `&Span` stored in `span_map`.
fn span_of(program: &Program, id: Id) -> Option<Span> {
    program.span_map.get(&id).map(|span| **span)
}

impl Document {
    pub fn analyze(text: &str, std_root: &Path, entry_path: &Path) -> Self {
        let line_index = LineIndex::new(text);
        let text_hash = hash_text(text);
        // The program borrows its source for `'static`, so leak a copy (the
        // editor re-analyzes on change; see the known leak tradeoff).
        let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
        // Prefer the project's declared target (the file's role in its
        // `vilan.toml`); fall back to inferring from imports for a non-entry file.
        let target = resolve_manifest_target(entry_path);
        let (program, diagnostics) = analyze_source(leaked, std_root, entry_path, target);

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

        Document {
            line_index,
            program,
            diagnostics,
            text_hash,
            entity_spans,
        }
    }

    /// Updates the document's text (its line index) without re-analyzing — applied
    /// on every edit so position-based queries (notably completion's context scan)
    /// see the just-typed character immediately, while the heavier re-analysis
    /// stays debounced. `text_hash` is deliberately left at the last *analyzed*
    /// text so the pending re-analysis still fires.
    pub fn set_text(&mut self, text: &str) {
        self.line_index = LineIndex::new(text);
    }

    /// The innermost entry-file entity whose span contains `offset`.
    fn entity_at(&self, offset: usize) -> Option<Id> {
        self.entity_spans
            .iter()
            .filter(|(start, end, _)| *start <= offset && offset < *end)
            .min_by_key(|(start, end, _)| end - start)
            .map(|(_, _, id)| *id)
    }

    /// The hover label (a rendered type) for the entity under `offset`.
    pub fn hover(&self, offset: usize) -> Option<String> {
        let program = self.program.as_ref()?;
        // A type name in type position renders its type directly.
        if let Some((_, label)) = self.type_reference_at(program, offset) {
            return Some(label);
        }
        self.hover_label(program, self.entity_at(offset)?)
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
        std::env::var_os("VILAN_STD")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std/src"))
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
}
