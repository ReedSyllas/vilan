//! Per-document analysis state and the navigation queries the language-server
//! handlers run against it: position→entity lookup, hover, go-to-definition,
//! find-references, and rename.

use std::path::Path;

use vilan_core::analyzer::{Expr, SourceId};
use vilan_core::id::Id;
use vilan_core::{Error, Program, Span, analyze_source};

use crate::line_index::LineIndex;

/// What a use site ultimately refers to — the key for find-references / rename.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    /// A `let`/`mut` local or a parameter, by its binding id.
    Binding(Id),
    /// A struct field, by owning struct id and field index.
    Field(Id, usize),
    /// A method, by its function id (call sites carry a precise member span).
    Method(Id),
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

pub struct Document {
    pub line_index: LineIndex,
    pub program: Option<Program<'static>>,
    pub diagnostics: Vec<Error>,
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
        // The program borrows its source for `'static`, so leak a copy (the
        // editor re-analyzes on change; see the known leak tradeoff).
        let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
        let (program, diagnostics) = analyze_source(leaked, std_root, entry_path);

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
            entity_spans,
        }
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
                span_of(program, definition)?,
            ));
        }
        let id = self.entity_at(offset)?;
        self.definition_of(program, id)
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
                // A binding that names a function resolves to its name, not the
                // whole declaration.
                if let Some(function) = program.functions.get(binding) {
                    return Some((program.source_of(*binding)?, function.name_span));
                }
                if let Some(function) = program.external_functions.get(binding) {
                    return Some((program.source_of(*binding)?, function.name_span));
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
                span_of(program, *struct_id)?,
            )),
            Expr::StructInitializer(initializer_id, _) => {
                let struct_id = program.struct_initializer_to_def.get(initializer_id)?;
                Some((
                    program.source_of(*struct_id)?,
                    span_of(program, *struct_id)?,
                ))
            }
            Expr::Enum(enum_id) => {
                Some((program.source_of(*enum_id)?, span_of(program, *enum_id)?))
            }
            Expr::Trait(trait_id) => {
                Some((program.source_of(*trait_id)?, span_of(program, *trait_id)?))
            }
            _ => None,
        }
    }

    /// All references to the symbol under `offset` (including its declaration).
    pub fn references(&self, offset: usize) -> Vec<(SourceId, Span)> {
        let Some(program) = self.program.as_ref() else {
            return Vec::new();
        };
        // Resolve a target from the entity under the cursor, falling back to a
        // struct field declaration (whose name has no entity of its own).
        let target = self
            .entity_at(offset)
            .and_then(|id| self.target_of(program, id))
            .or_else(|| self.field_decl_at(program, offset));
        let Some(target) = target else {
            return Vec::new();
        };
        self.occurrences(program, target)
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

    /// What a use site refers to, for find-references / rename. `None` for
    /// symbols whose rename isn't supported yet (e.g. free functions, whose
    /// declaration and call-site spans aren't cleanly separable).
    fn target_of(&self, program: &Program, id: Id) -> Option<Target> {
        match program.entity_map.get(&id)? {
            Expr::Local(binding) | Expr::Variable(binding) | Expr::Parameter(binding) => {
                Some(Target::Binding(*binding))
            }
            Expr::Field(_, struct_id, index) => Some(Target::Field(*struct_id, *index)),
            Expr::Call(call_id) => {
                // A method call resolves to a method whose call sites carry a
                // precise member span (unlike a free function).
                let subject = program.function_calls.get(call_id)?.subject_id;
                match program.entity_map.get(&subject)? {
                    Expr::Function(function_id) if program.member_name_spans.contains_key(&id) => {
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
                // The declaration's own span is the binding name.
                if let Some(span) = span_of(program, binding) {
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
                    if let Expr::Call(call_id) = expr {
                        let resolves = program
                            .function_calls
                            .get(call_id)
                            .map(|call| call.subject_id)
                            .and_then(|subject| program.entity_map.get(&subject))
                            .is_some_and(|subject| {
                                matches!(subject, Expr::Function(other) if *other == function_id)
                            });
                        if resolves {
                            if let Some(span) = program.member_name_spans.get(use_id) {
                                push(*use_id, *span);
                            }
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
}
