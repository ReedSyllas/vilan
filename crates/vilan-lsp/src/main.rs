//! The Vilan language server: a thin tower-lsp front-end over `vilan-core`.
//! Analyzes each open document on change and answers diagnostics, hover,
//! go-to-definition, find-references, and rename — across files into `std`.

mod document;
mod line_index;
mod publish;

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
use crate::publish::PublishState;

/// How long to wait after the last edit before re-analyzing, so a burst of
/// keystrokes collapses to a single analysis instead of one per character.
const DEBOUNCE_MS: u64 = 150;

/// Convert a Vilan completion candidate to an LSP `CompletionItem`.
fn to_completion_item(completion: Completion) -> CompletionItem {
    let kind = match completion.kind {
        // The LSP kind set has no macro entry; functions render closest.
        VilanCompletionKind::Macro => CompletionItemKind::FUNCTION,
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
    /// The publish planner (backlog E6): every open document's last
    /// diagnostic groups, merged per target URI so shared dependencies show
    /// the union of their importers' views, and stale targets get explicit
    /// empties. Locked only around synchronous planning, never across an
    /// await.
    publish_state: Arc<std::sync::Mutex<PublishState>>,
    /// `std` files don't change during a session, so cache their line indices
    /// rather than re-reading the file on every cross-file definition/reference.
    line_indices: Arc<DashMap<PathBuf, Arc<LineIndex>>>,
}

/// Locate the `std` package directory: `$VILAN_STD`, else the nearest ancestor
/// of the document containing `vilan/std/vilan.toml` (a checkout — documents in
/// this repo resolve the working tree). `resolve_std` reads its `[library]`
/// manifest (or falls back to the layer convention if the path is a bare source
/// root).
fn discover_std_dir(start: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("VILAN_STD") {
        return PathBuf::from(path);
    }
    let mut directory = start.parent();
    while let Some(current) = directory {
        let candidate = current.join("vilan").join("std");
        if candidate.join("vilan.toml").is_file() {
            return candidate;
        }
        directory = current.parent();
    }
    // No ancestor carries a checkout — a project OUTSIDE the vilan repo (the
    // kolt shape, and every installed binary). Materialize the server's own
    // embedded std (real files, so definitions into std keep resolving); the
    // CLI does the same, so both tools see the identical std from any
    // directory. On a materialization failure (no writable home OR temp dir)
    // the path is left nonexistent and imports diagnose it.
    vilan_embedded_std::materialize()
        .unwrap_or_else(|_| PathBuf::from("<the embedded std could not be materialized>"))
}

#[cfg(test)]
mod std_discovery_tests {
    use super::discover_std_dir;
    use std::path::Path;

    #[test]
    fn a_document_outside_any_checkout_falls_back_to_the_embedded_std() {
        // A kolt-shaped path: no ancestor contains `vilan/std`. The fallback
        // must be the server's own materialized std — a real, complete package
        // directory that resolves from anywhere — not a compile-time path into
        // the machine the server happened to be built on.
        let discovered = discover_std_dir(Path::new("/tmp/definitely/not/a/checkout/main.vl"));
        assert!(
            discovered.is_absolute()
                && discovered.join("vilan.toml").is_file()
                && discovered.join("src/lib.vl").is_file(),
            "expected the materialized embedded std, got {discovered:?}"
        );
    }
}

/// Analyze `text` as the document at `uri`, store the result, and publish its
/// diagnostics (grouped per file — backlog E1). The analysis is CPU-bound, so
/// it runs on a blocking thread to keep the async runtime responsive.
async fn analyze_and_publish(
    documents: &DashMap<Url, Document>,
    client: &Client,
    publish_state: &std::sync::Mutex<PublishState>,
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
    documents.insert(uri.clone(), document);
    publish_document(documents, client, publish_state, &uri).await;
}

/// Publish the stored document's diagnostics: the planner computes every
/// `(target, merged diagnostics)` action synchronously (the entry's own to
/// `uri`, each imported file's to *that file's* URI, stale targets cleared,
/// shared targets merged across owners — see `publish.rs`), and this sends
/// them.
async fn publish_document(
    documents: &DashMap<Url, Document>,
    client: &Client,
    publish_state: &std::sync::Mutex<PublishState>,
    uri: &Url,
) {
    // Plan before the first await (neither the map guard nor the planner
    // lock may be held across one).
    let actions = {
        let Some(document) = documents.get(uri) else {
            return;
        };
        publish_state.lock().unwrap().plan_publish(uri, &document)
    };
    for (target, group) in actions {
        client.publish_diagnostics(target, group, None).await;
    }
}

/// Re-analyze every OTHER open document: an edit (or save) of one file changes
/// what its dependents see, so their diagnostics must be recomputed — the
/// stale-diagnostics half of backlog E1. Their buffers didn't change, so this
/// bypasses the unchanged-text short-circuit deliberately.
async fn reanalyze_dependents(
    documents: &DashMap<Url, Document>,
    client: &Client,
    publish_state: &std::sync::Mutex<PublishState>,
    changed: &Url,
) {
    let others: Vec<(Url, String)> = documents
        .iter()
        .filter(|entry| entry.key() != changed)
        .map(|entry| (entry.key().clone(), entry.value().text.clone()))
        .collect();
    for (uri, text) in others {
        analyze_and_publish(documents, client, publish_state, uri, text).await;
    }
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
        let publish_state = Arc::clone(&self.publish_state);
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
            analyze_and_publish(&documents, &client, &publish_state, uri.clone(), text).await;
            // The edit may change what other open files see (they import this
            // one, or a file it re-exports) — bring their diagnostics up to date.
            reanalyze_dependents(&documents, &client, &publish_state, &uri).await;
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
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        ..Default::default()
                    },
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
                inlay_hint_provider: Some(OneOf::Left(true)),
                // E2: precision highlighting from the analyzed program. The
                // legend is index-aligned with `document::TokenKind`.
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: crate::document::TOKEN_TYPES
                                    .iter()
                                    .map(|name| SemanticTokenType::new(name))
                                    .collect(),
                                token_modifiers: crate::document::TOKEN_MODIFIERS
                                    .iter()
                                    .map(|name| SemanticTokenModifier::new(name))
                                    .collect(),
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            ..Default::default()
                        },
                    ),
                ),
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
        // Register the buffer so OTHER documents' analyses load this one's
        // live content instead of the file on disk (backlog E6).
        vilan_core::analyzer::set_document_overlay(&path, Some(params.text_document.text.clone()));
        let std_dir = discover_std_dir(&path);
        let document = Document::analyze(&params.text_document.text, &std_dir, &path);
        self.documents.insert(uri.clone(), document);
        publish_document(&self.documents, &self.client, &self.publish_state, &uri).await;
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
            // The overlay updates immediately (pre-debounce), so any analysis
            // that runs meanwhile — a dependent's, this one's — sees the edit.
            if let Ok(path) = uri.to_file_path() {
                vilan_core::analyzer::set_document_overlay(&path, Some(change.text.clone()));
            }
            self.on_change(uri, change.text);
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // A save changes what OTHER documents' analyses read from disk (module
        // loading is disk-backed), so re-analyze every open document.
        let saved = params.text_document.uri;
        if let Some((uri, text)) = self
            .documents
            .get(&saved)
            .map(|document| (saved.clone(), document.text.clone()))
        {
            analyze_and_publish(
                &self.documents,
                &self.client,
                &self.publish_state,
                uri,
                text,
            )
            .await;
        }
        reanalyze_dependents(&self.documents, &self.client, &self.publish_state, &saved).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        // Disk truth returns for other documents' analyses.
        if let Ok(path) = uri.to_file_path() {
            vilan_core::analyzer::set_document_overlay(&path, None);
        }
        self.documents.remove(&uri);
        // Drop the edit generation so any in-flight debounced analysis bails.
        self.pending.remove(&uri);
        // Clear this document's diagnostics AND the ones it published onto
        // other files — each target republishes as the remaining owners'
        // merged view (empty where this was the only contributor).
        let actions = self.publish_state.lock().unwrap().plan_close(&uri);
        for (target, group) in actions {
            self.client.publish_diagnostics(target, group, None).await;
        }
        // A document that never analyzed (open failed) still clears.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let range = params.range;
        let hints = document
            .inlay_hints()
            .into_iter()
            .filter_map(|(offset, label)| {
                let span = vilan_core::Span::from(offset..offset);
                let position = document.line_index.range(&span).start;
                (position >= range.start && position <= range.end).then(|| InlayHint {
                    position,
                    label: InlayHintLabel::String(label),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(false),
                    padding_right: Some(false),
                    data: None,
                })
            })
            .collect::<Vec<_>>();
        Ok(Some(hints))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        // Delta-encode (line delta, char delta, length, type, modifiers) in
        // position order; identifiers never span lines, so the length is the
        // span's width.
        let mut data: Vec<SemanticToken> = Vec::new();
        let mut previous_line = 0u32;
        let mut previous_start = 0u32;
        for (span, kind, modifiers) in document.semantic_tokens() {
            let range = document.line_index.range(&span);
            let line = range.start.line;
            let start = range.start.character;
            let length = span.into_range().len() as u32;
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                start - previous_start
            } else {
                start
            };
            data.push(SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type: kind as u32,
                token_modifiers_bitset: modifiers,
            });
            previous_line = line;
            previous_start = start;
        }
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
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
        publish_state: Arc::new(std::sync::Mutex::new(PublishState::new())),
        pending: Arc::new(DashMap::new()),
        line_indices: Arc::new(DashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
