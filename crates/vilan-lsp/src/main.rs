//! The Vilan language server: a thin tower-lsp front-end over `vilan-core`.
//! Analyzes each open document on change and answers diagnostics, hover,
//! go-to-definition, find-references, and rename — across files into `std`.

mod document;
mod line_index;
mod publish;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// How completion inserts a function or method call — the `vilan.completion.functionCall`
/// setting, consumed by [`to_completion_item`]: `Full` fills named parameter
/// tab-stops, `ParensOnly` inserts the parentheses, `None` inserts the bare name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CompletionFunctionCall {
    /// Insert the name only.
    None,
    /// Insert `name()` (empty parentheses).
    ParensOnly,
    /// Insert `name(…)` with a placeholder argument list.
    Full,
}

/// The client's feature settings (VS Code `contributes.configuration`), received
/// as `initializationOptions` at startup and refreshed live by
/// `workspace/didChangeConfiguration`. Defaults preserve today's behavior: every
/// provider on, full function-call completion. (`organizeImports.onSave` is a
/// client-only concern — `editor.codeActionsOnSave` — so the server never reads
/// it.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Config {
    inlay_hints_enabled: bool,
    semantic_tokens_enabled: bool,
    completion_function_call: CompletionFunctionCall,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            inlay_hints_enabled: true,
            semantic_tokens_enabled: true,
            completion_function_call: CompletionFunctionCall::Full,
        }
    }
}

impl Config {
    /// Parses the settings object the client sends. Accepts either the bare
    /// `vilan` config (as `initializationOptions`) or a `{ "vilan": { … } }`
    /// wrapper (as `didChangeConfiguration`'s `settings`). Every field falls back
    /// to its default when absent or the wrong type, so a partial or malformed
    /// payload never silently flips a provider off.
    fn from_settings(settings: &serde_json::Value) -> Self {
        let root = settings.get("vilan").unwrap_or(settings);
        let mut config = Config::default();
        if let Some(enabled) = root
            .pointer("/inlayHints/enabled")
            .and_then(|v| v.as_bool())
        {
            config.inlay_hints_enabled = enabled;
        }
        if let Some(enabled) = root
            .pointer("/semanticTokens/enabled")
            .and_then(|v| v.as_bool())
        {
            config.semantic_tokens_enabled = enabled;
        }
        if let Some(mode) = root
            .pointer("/completion/functionCall")
            .and_then(|v| v.as_str())
        {
            config.completion_function_call = match mode {
                "none" => CompletionFunctionCall::None,
                "parensOnly" => CompletionFunctionCall::ParensOnly,
                // `full` and any unrecognized value keep the default.
                _ => CompletionFunctionCall::Full,
            };
        }
        config
    }
}

/// Convert a Vilan completion candidate to an LSP `CompletionItem`, applying the
/// `vilan.completion.functionCall` setting and the client's snippet capability
/// to shape a function/method insertion (WO-3). The popup always carries the
/// candidate's signature/type `detail` and `///` documentation.
fn to_completion_item(
    completion: Completion,
    mode: CompletionFunctionCall,
    snippet_support: bool,
) -> CompletionItem {
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
    let mut item = CompletionItem {
        label: completion.label.clone(),
        kind: Some(kind),
        detail: completion.detail,
        documentation: completion.documentation.map(Documentation::String),
        ..Default::default()
    };
    // A call-shaped insertion applies only to a callable in a call position
    // (`call_parameters` is `Some`) and only when the setting asks for it. `none`
    // keeps today's bare-name insertion. With parameters, a signature-help popup
    // is triggered so the user sees what to fill.
    if let Some(parameters) = completion.call_parameters {
        let call = call_insertion(&completion.label, &parameters, mode, snippet_support);
        if let Some((insert_text, format)) = call {
            item.insert_text = Some(insert_text);
            item.insert_text_format = Some(format);
            if !parameters.is_empty() {
                item.command = Some(Command {
                    title: "Trigger Parameter Hints".to_string(),
                    command: "editor.action.triggerParameterHints".to_string(),
                    arguments: None,
                });
            }
        }
    }
    item
}

/// The insert text (and its format) for a call-shaped completion, or `None` when
/// the setting is `none` — leaving the bare label. `full` fills each parameter
/// as a named tab-stop (`name(${1:a}, ${2:b})$0`); `parensOnly` positions the
/// cursor between the parens (`name($0)`); both write `name()$0` for a
/// zero-parameter callable. Without client snippet support every shape degrades
/// to the plain `name()` (cursor after) — a snippet's tab-stops would otherwise
/// surface as literal text.
fn call_insertion(
    label: &str,
    parameters: &[String],
    mode: CompletionFunctionCall,
    snippet_support: bool,
) -> Option<(String, InsertTextFormat)> {
    if matches!(mode, CompletionFunctionCall::None) {
        return None;
    }
    if !snippet_support {
        return Some((format!("{label}()"), InsertTextFormat::PLAIN_TEXT));
    }
    let snippet = if parameters.is_empty() {
        format!("{label}()$0")
    } else {
        match mode {
            CompletionFunctionCall::Full => {
                let placeholders: Vec<String> = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, name)| format!("${{{}:{name}}}", index + 1))
                    .collect();
                format!("{label}({})$0", placeholders.join(", "))
            }
            // `parensOnly` (with parameters): cursor inside the parens.
            _ => format!("{label}($0)"),
        }
    };
    Some((snippet, InsertTextFormat::SNIPPET))
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
    /// The client's feature settings, seeded from `initializationOptions` and
    /// updated live by `workspace/didChangeConfiguration`. Read per request
    /// (`inlay_hint`, `semantic_tokens_full`, …) so a toggle takes effect without
    /// re-registering capabilities.
    config: Arc<std::sync::RwLock<Config>>,
    /// Whether the client can render snippet completions (`$1`/`${1:name}`
    /// tab-stops). Captured from `ClientCapabilities` at `initialize` (fixed for
    /// the session); when absent, call-shaped completions degrade to plain text
    /// (WO-3).
    snippet_support: Arc<AtomicBool>,
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

#[cfg(test)]
mod config_tests {
    use super::{CompletionFunctionCall, Config};
    use serde_json::json;

    // Defaults preserve today's behavior: every provider on, full completion.
    #[test]
    fn defaults_preserve_todays_behavior() {
        let config = Config::default();
        assert!(config.inlay_hints_enabled);
        assert!(config.semantic_tokens_enabled);
        assert_eq!(
            config.completion_function_call,
            CompletionFunctionCall::Full
        );
    }

    // `initializationOptions` sends the bare `vilan` config object.
    #[test]
    fn parses_the_bare_vilan_object() {
        let config = Config::from_settings(&json!({
            "inlayHints": { "enabled": false },
            "semanticTokens": { "enabled": false },
            "completion": { "functionCall": "parensOnly" },
        }));
        assert!(!config.inlay_hints_enabled);
        assert!(!config.semantic_tokens_enabled);
        assert_eq!(
            config.completion_function_call,
            CompletionFunctionCall::ParensOnly
        );
    }

    // `didChangeConfiguration` wraps it as `{ "vilan": { … } }`; unspecified
    // fields keep their defaults.
    #[test]
    fn parses_the_wrapped_settings_and_keeps_unset_defaults() {
        let config = Config::from_settings(&json!({
            "vilan": {
                "inlayHints": { "enabled": false },
                "completion": { "functionCall": "none" },
            },
        }));
        assert!(!config.inlay_hints_enabled);
        assert!(config.semantic_tokens_enabled);
        assert_eq!(
            config.completion_function_call,
            CompletionFunctionCall::None
        );
    }

    // A partial, empty, or malformed payload never silently flips a provider off:
    // wrong types and unknown enum values fall back to the default.
    #[test]
    fn a_malformed_payload_keeps_defaults() {
        assert_eq!(Config::from_settings(&json!({})), Config::default());
        let config = Config::from_settings(&json!({
            "inlayHints": { "enabled": "yes" },
            "completion": { "functionCall": 3 },
        }));
        assert!(config.inlay_hints_enabled);
        assert_eq!(
            config.completion_function_call,
            CompletionFunctionCall::Full
        );
        let config = Config::from_settings(&json!({ "completion": { "functionCall": "wat" } }));
        assert_eq!(
            config.completion_function_call,
            CompletionFunctionCall::Full
        );
    }
}

#[cfg(test)]
mod completion_item_tests {
    use super::{CompletionFunctionCall, to_completion_item};
    use crate::document::{Completion, CompletionKind};
    use tower_lsp::lsp_types::{Documentation, InsertTextFormat};

    /// A function candidate as `Document` would hand it over: a full signature,
    /// a doc, and `call_parameters` naming the arguments (`None` = a bare name).
    fn function(call_parameters: Option<Vec<&str>>) -> Completion {
        Completion {
            label: "connect".to_string(),
            kind: CompletionKind::Function,
            detail: Some("fun connect(host: str, port: i32): Socket".to_string()),
            documentation: Some("Opens a connection.".to_string()),
            call_parameters: call_parameters
                .map(|names| names.into_iter().map(str::to_string).collect()),
        }
    }

    // WO-3 `full`: each parameter becomes a named tab-stop, the cursor lands
    // after the call, and the signature-help popup is triggered.
    #[test]
    fn full_mode_inserts_named_parameter_placeholders() {
        let item = to_completion_item(
            function(Some(vec!["host", "port"])),
            CompletionFunctionCall::Full,
            true,
        );
        assert_eq!(
            item.insert_text.as_deref(),
            Some("connect(${1:host}, ${2:port})$0")
        );
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert_eq!(
            item.command
                .as_ref()
                .map(|command| command.command.as_str()),
            Some("editor.action.triggerParameterHints"),
            "parameters present ⇒ trigger the hints popup"
        );
    }

    // WO-3 `parensOnly`: the parentheses are inserted with the cursor between
    // them, no named placeholders.
    #[test]
    fn parens_only_mode_positions_cursor_inside_parens() {
        let item = to_completion_item(
            function(Some(vec!["host", "port"])),
            CompletionFunctionCall::ParensOnly,
            true,
        );
        assert_eq!(item.insert_text.as_deref(), Some("connect($0)"));
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert!(item.command.is_some(), "parameters present ⇒ trigger hints");
    }

    // WO-3 `none`: today's behavior — a bare name (no `insert_text`, so the
    // client inserts the label), and no hints command.
    #[test]
    fn none_mode_leaves_a_bare_name() {
        let item = to_completion_item(
            function(Some(vec!["host"])),
            CompletionFunctionCall::None,
            true,
        );
        assert!(item.insert_text.is_none(), "the bare label is inserted");
        assert!(item.insert_text_format.is_none());
        assert!(item.command.is_none());
    }

    // WO-3: a zero-parameter callable inserts `name()$0` in BOTH call modes —
    // and, having no parameters, triggers no hints popup.
    #[test]
    fn zero_parameter_call_inserts_empty_parens_and_no_hints() {
        for mode in [
            CompletionFunctionCall::Full,
            CompletionFunctionCall::ParensOnly,
        ] {
            let item = to_completion_item(function(Some(vec![])), mode, true);
            assert_eq!(item.insert_text.as_deref(), Some("connect()$0"), "{mode:?}");
            assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
            assert!(item.command.is_none(), "no parameters ⇒ no hints: {mode:?}");
        }
    }

    // WO-3: without client snippet support, a call-shaped insertion degrades to
    // plain `name()` (a snippet's tab-stops would otherwise show as literals).
    #[test]
    fn without_snippet_support_degrades_to_plain_parens() {
        let item = to_completion_item(
            function(Some(vec!["host", "port"])),
            CompletionFunctionCall::Full,
            false,
        );
        assert_eq!(item.insert_text.as_deref(), Some("connect()"));
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
    }

    // WO-3: a candidate with `call_parameters == None` (a non-callable, or one
    // the escape hatches suppressed) stays a bare name even in `full` mode.
    #[test]
    fn non_callable_stays_bare_in_full_mode() {
        let mut candidate = function(None);
        candidate.kind = CompletionKind::Struct;
        let item = to_completion_item(candidate, CompletionFunctionCall::Full, true);
        assert!(item.insert_text.is_none());
        assert!(item.command.is_none());
    }

    // WO-3: the popup always carries the signature `detail` and the `///`
    // documentation, independent of the insertion mode.
    #[test]
    fn detail_and_documentation_reach_the_item() {
        let item = to_completion_item(
            function(Some(vec!["host"])),
            CompletionFunctionCall::Full,
            true,
        );
        assert_eq!(
            item.detail.as_deref(),
            Some("fun connect(host: str, port: i32): Socket")
        );
        assert!(
            matches!(item.documentation, Some(Documentation::String(doc)) if doc == "Opens a connection."),
            "the doc paragraph is attached"
        );
    }
}

#[cfg(test)]
mod code_action_tests {
    use super::organize_imports_requested;
    use tower_lsp::lsp_types::CodeActionKind;

    // Organize Imports is offered when unfiltered (the Source Action menu), for
    // its exact kind, and for the ancestor `source` kind (what
    // `codeActionsOnSave` requests).
    #[test]
    fn organize_is_offered_for_matching_and_ancestor_kinds() {
        assert!(organize_imports_requested(&None));
        assert!(organize_imports_requested(&Some(vec![
            CodeActionKind::SOURCE_ORGANIZE_IMPORTS
        ])));
        assert!(organize_imports_requested(&Some(vec![
            CodeActionKind::SOURCE
        ])));
    }

    // It is NOT offered for unrelated kinds, a sibling `source.*` kind, or an
    // empty filter.
    #[test]
    fn organize_is_not_offered_for_unrelated_kinds() {
        assert!(!organize_imports_requested(&Some(vec![
            CodeActionKind::QUICKFIX
        ])));
        assert!(!organize_imports_requested(&Some(vec![
            CodeActionKind::SOURCE_FIX_ALL
        ])));
        assert!(!organize_imports_requested(&Some(vec![])));
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
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Seed the feature settings from the client's `initializationOptions`
        // (the extension sends the `vilan` config object); later changes arrive
        // via `did_change_configuration`.
        if let Some(options) = &params.initialization_options {
            *self.config.write().unwrap() = Config::from_settings(options);
        }
        // Snippet completions (tab-stop placeholders) need the client to opt in
        // via `completionItem.snippetSupport`; without it, a call-shaped
        // completion degrades to plain text. This is fixed for the session.
        let snippet_support = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|text_document| text_document.completion.as_ref())
            .and_then(|completion| completion.completion_item.as_ref())
            .and_then(|completion_item| completion_item.snippet_support)
            .unwrap_or(false);
        self.snippet_support
            .store(snippet_support, Ordering::Relaxed);
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
                // WO-2: the "Organize Imports" source action (sort + prune).
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
                        ..Default::default()
                    },
                )),
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

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        // Our client pushes `{ "vilan": { … } }` on a relevant change; re-parse
        // and replace (providers read the config per request, so a toggle is live
        // without re-registration). Ignore a payload without the `vilan` section
        // — the language client also emits a bare `{ settings: null }` on any
        // config change, which must NOT reset our settings to their defaults.
        if params.settings.get("vilan").is_some() {
            *self.config.write().unwrap() = Config::from_settings(&params.settings);
        }
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
        // `vilan.inlayHints.enabled` gates the provider server-side.
        if !self.config.read().unwrap().inlay_hints_enabled {
            return Ok(None);
        }
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
        // `vilan.semanticTokens.enabled` gates the provider server-side; when off,
        // the editor falls back to its TextMate grammar.
        if !self.config.read().unwrap().semantic_tokens_enabled {
            return Ok(None);
        }
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
        let mode = self.config.read().unwrap().completion_function_call;
        let snippet_support = self.snippet_support.load(Ordering::Relaxed);
        let items = document
            .completion(offset)
            .into_iter()
            .map(|completion| to_completion_item(completion, mode, snippet_support))
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

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        // The only source action we offer is Organize Imports; skip the work
        // entirely when the client asked for a different kind (e.g. quickfix).
        if !organize_imports_requested(&params.context.only) {
            return Ok(None);
        }
        let uri = params.text_document.uri;
        let Some(document) = self.documents.get(&uri) else {
            return Ok(None);
        };
        let edits = document.organize_import_edits();
        // No edits = already organized (or nothing to do): offer no action, so
        // `codeActionsOnSave` is a clean no-op.
        if edits.is_empty() {
            return Ok(None);
        }
        let text_edits: Vec<TextEdit> = edits
            .into_iter()
            .map(|(span, new_text)| TextEdit {
                range: document.line_index.range(&span),
                new_text,
            })
            .collect();
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        changes.insert(uri, text_edits);
        let action = CodeAction {
            title: "Organize Imports".to_string(),
            kind: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            ..Default::default()
        };
        Ok(Some(vec![CodeActionOrCommand::CodeAction(action)]))
    }
}

/// Whether a code-action request wants the Organize Imports source action: an
/// unfiltered request (no `only`) does, and a filtered one does when it lists
/// `source.organizeImports` or an ancestor kind (`source`). The `.`-delimited
/// kind hierarchy means a requested `source` matches `source.organizeImports`.
fn organize_imports_requested(only: &Option<Vec<CodeActionKind>>) -> bool {
    let Some(kinds) = only else {
        return true;
    };
    let organize = CodeActionKind::SOURCE_ORGANIZE_IMPORTS;
    kinds.iter().any(|requested| {
        organize == *requested
            || organize
                .as_str()
                .strip_prefix(requested.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
    })
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
        config: Arc::new(std::sync::RwLock::new(Config::default())),
        snippet_support: Arc::new(AtomicBool::new(false)),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
