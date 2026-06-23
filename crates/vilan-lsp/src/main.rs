//! The Vilan language server: a thin tower-lsp front-end over `vilan-core`.
//! Analyzes each open document on change and answers diagnostics, hover,
//! go-to-definition, find-references, and rename — across files into `std`.

mod document;
mod line_index;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server, jsonrpc::Result};
use vilan_core::Span;
use vilan_core::analyzer::SourceId;

use crate::document::{
    Completion, CompletionKind as VilanCompletionKind, Document, Symbol,
    SymbolKind as VilanSymbolKind, hash_text,
};
use crate::line_index::LineIndex;

/// How long to wait after the last edit before re-analyzing, so a burst of
/// keystrokes collapses to a single analysis instead of one per character.
const DEBOUNCE_MS: u64 = 150;

/// Convert a Vilan completion candidate to an LSP `CompletionItem`.
fn to_completion_item(completion: Completion) -> CompletionItem {
    let kind = match completion.kind {
        VilanCompletionKind::Function => CompletionItemKind::FUNCTION,
        VilanCompletionKind::Method => CompletionItemKind::METHOD,
        VilanCompletionKind::Field => CompletionItemKind::FIELD,
        VilanCompletionKind::Struct => CompletionItemKind::STRUCT,
        VilanCompletionKind::Enum => CompletionItemKind::ENUM,
        VilanCompletionKind::EnumVariant => CompletionItemKind::ENUM_MEMBER,
        VilanCompletionKind::Trait => CompletionItemKind::INTERFACE,
        VilanCompletionKind::Variable => CompletionItemKind::VARIABLE,
        VilanCompletionKind::Module => CompletionItemKind::MODULE,
        VilanCompletionKind::Keyword => CompletionItemKind::KEYWORD,
    };
    CompletionItem {
        label: completion.label,
        kind: Some(kind),
        ..Default::default()
    }
}

/// Convert a Vilan outline node to an LSP `DocumentSymbol`.
#[allow(deprecated)]
fn to_lsp_symbol(symbol: Symbol, line_index: &LineIndex) -> DocumentSymbol {
    let kind = match symbol.kind {
        VilanSymbolKind::Function => SymbolKind::FUNCTION,
        VilanSymbolKind::Struct => SymbolKind::STRUCT,
        VilanSymbolKind::Field => SymbolKind::FIELD,
        VilanSymbolKind::Enum => SymbolKind::ENUM,
        VilanSymbolKind::Trait => SymbolKind::INTERFACE,
    };
    let children = symbol
        .children
        .into_iter()
        .map(|child| to_lsp_symbol(child, line_index))
        .collect::<Vec<_>>();
    DocumentSymbol {
        name: symbol.name,
        detail: None,
        kind,
        tags: None,
        deprecated: None,
        range: line_index.range(&symbol.full),
        selection_range: line_index.range(&symbol.selection),
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
    }
}

struct Backend {
    client: Client,
    documents: Arc<DashMap<Url, Document>>,
    /// The latest edit generation per document, so a debounced analysis can tell
    /// whether a newer edit (or a close) has superseded it before it runs.
    pending: Arc<DashMap<Url, u64>>,
    /// `std` files don't change during a session, so cache their line indices
    /// rather than re-reading the file on every cross-file definition/reference.
    line_indices: Arc<DashMap<PathBuf, Arc<LineIndex>>>,
}

/// Locate the `std` library directory: `$VILAN_STD`, else the nearest ancestor of
/// the document containing `vilan/std`. `resolve_std` reads its `[library]`
/// manifest (or falls back to the layer convention if the path is a bare source
/// root).
fn discover_std_dir(start: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("VILAN_STD") {
        return PathBuf::from(path);
    }
    let mut directory = start.parent();
    while let Some(current) = directory {
        let candidate = current.join("vilan").join("std");
        if candidate.is_dir() {
            return candidate;
        }
        directory = current.parent();
    }
    PathBuf::from("vilan/std")
}

/// Analyze `text` as the document at `uri`, store the result, and publish its
/// diagnostics. The analysis is CPU-bound, so it runs on a blocking thread to
/// keep the async runtime responsive.
async fn analyze_and_publish(
    documents: &DashMap<Url, Document>,
    client: &Client,
    uri: Url,
    text: String,
) {
    let path = uri.to_file_path().unwrap_or_default();
    let std_dir = discover_std_dir(&path);
    let document = match tokio::task::spawn_blocking(move || {
        Document::analyze(&text, &std_dir, &path)
    })
    .await
    {
        Ok(document) => document,
        Err(_) => return,
    };
    let diagnostics = build_diagnostics(&document);
    documents.insert(uri.clone(), document);
    client.publish_diagnostics(uri, diagnostics, None).await;
}

/// Render a document's analyzer diagnostics as LSP diagnostics.
fn build_diagnostics(document: &Document) -> Vec<Diagnostic> {
    document
        .diagnostics
        .iter()
        .map(|error| Diagnostic {
            range: document.line_index.range(&error.span),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("vilan".to_string()),
            message: error.msg.clone(),
            ..Default::default()
        })
        .collect()
}

impl Backend {
    /// Schedule a debounced re-analysis. A burst of edits collapses to a single
    /// analysis once typing pauses, and an edit that leaves the buffer unchanged
    /// is skipped entirely.
    fn on_change(&self, uri: Url, text: String) {
        let generation = {
            let mut entry = self.pending.entry(uri.clone()).or_insert(0);
            *entry += 1;
            *entry
        };
        let documents = Arc::clone(&self.documents);
        let pending = Arc::clone(&self.pending);
        let client = self.client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
            // A newer edit (or a close) superseded this one.
            if pending.get(&uri).map(|current| *current) != Some(generation) {
                return;
            }
            // The buffer is byte-for-byte what we last analyzed — nothing to do.
            if documents.get(&uri).map(|document| document.text_hash) == Some(hash_text(&text)) {
                return;
            }
            analyze_and_publish(&documents, &client, uri, text).await;
        });
    }

    /// The line index for a `std` file, cached by path so a cross-file query
    /// doesn't re-read and re-index the file on every lookup.
    fn line_index_for(&self, path: &Path) -> Option<Arc<LineIndex>> {
        if let Some(cached) = self.line_indices.get(path) {
            return Some(Arc::clone(cached.value()));
        }
        let text = std::fs::read_to_string(path).ok()?;
        let line_index = Arc::new(LineIndex::new(&text));
        self.line_indices
            .insert(path.to_path_buf(), Arc::clone(&line_index));
        Some(line_index)
    }

    /// Convert a `(source, span)` from analysis into an LSP `Location`. The entry
    /// file uses the open document's line index; a `std` file uses its cached one.
    fn location_for(
        &self,
        document: &Document,
        doc_uri: &Url,
        source: SourceId,
        span: Span,
    ) -> Option<Location> {
        if source == SourceId(0) {
            return Some(Location {
                uri: doc_uri.clone(),
                range: document.line_index.range(&span),
            });
        }
        let program = document.program.as_ref()?;
        let path = program.source_path(source)?;
        let line_index = self.line_index_for(path)?;
        let uri = Url::from_file_path(path).ok()?;
        Some(Location {
            uri,
            range: line_index.range(&span),
        })
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    // `.` and `:` (the second `:` of `::`) re-trigger completion so
                    // member/path candidates appear without a manual invoke.
                    trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "vilan-lsp".to_string(),
                version: None,
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "vilan-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        // Analyze inline and insert the document before the first `.await`, so a
        // query that arrives right after open — before diagnostics are published
        // — still finds it. (The debounced change path runs off the async thread,
        // but there a previous analysis is always already in place.)
        let uri = params.text_document.uri;
        let path = uri.to_file_path().unwrap_or_default();
        let std_dir = discover_std_dir(&path);
        let document = Document::analyze(&params.text_document.text, &std_dir, &path);
        let diagnostics = build_diagnostics(&document);
        self.documents.insert(uri.clone(), document);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.pop() {
            let uri = params.text_document.uri;
            // Apply the new text to the open document immediately so a completion
            // request arriving before the debounced re-analysis still sees the
            // just-typed character (e.g. the `.` that selects member completion).
            if let Some(mut document) = self.documents.get_mut(&uri) {
                document.set_text(&change.text);
            }
            self.on_change(uri, change.text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.remove(&params.text_document.uri);
        // Drop the edit generation so any in-flight debounced analysis bails.
        self.pending.remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let offset = document.line_index.offset(position);
        Ok(document.hover(offset).map(|label| Hover {
            contents: HoverContents::Scalar(MarkedString::String(label)),
            range: None,
        }))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let offset = document.line_index.offset(position);
        let items = document
            .completion(offset)
            .into_iter()
            .map(to_completion_item)
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let offset = document.line_index.offset(position);
        let Some((source, span)) = document.definition(offset) else {
            return Ok(None);
        };
        Ok(self
            .location_for(&document, &uri, source, span)
            .map(GotoDefinitionResponse::Scalar))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let offset = document.line_index.offset(position);
        let locations = document
            .references(offset)
            .into_iter()
            .filter_map(|(source, span)| self.location_for(&document, &uri, source, span))
            .collect();
        Ok(Some(locations))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let offset = document.line_index.offset(position);
        let occurrences = document.references(offset);
        if occurrences.is_empty() {
            return Ok(None);
        }
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for (source, span) in occurrences {
            if let Some(location) = self.location_for(&document, &uri, source, span) {
                changes.entry(location.uri).or_default().push(TextEdit {
                    range: location.range,
                    new_text: new_name.clone(),
                });
            }
        }
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let symbols = document
            .document_symbols()
            .into_iter()
            .map(|symbol| to_lsp_symbol(symbol, &document.line_index))
            .collect::<Vec<_>>();
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let source = document.line_index.text();
        let formatted = vilan_core::formatter::format(source);
        // `format` returns the input unchanged when the file is already canonical
        // or hits a construct it can't print (it never produces non-round-tripping
        // output) — either way there is nothing to edit.
        if formatted == source {
            return Ok(None);
        }
        // Replace the whole document in one edit, from the start to the end
        // position the line index reports for the final byte.
        let end = document.line_index.position(source.len());
        Ok(Some(vec![TextEdit {
            range: Range::new(Position::new(0, 0), end),
            new_text: formatted,
        }]))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: Arc::new(DashMap::new()),
        pending: Arc::new(DashMap::new()),
        line_indices: Arc::new(DashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
