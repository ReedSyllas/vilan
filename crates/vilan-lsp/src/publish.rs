//! The publish planner (backlog E6): which diagnostics land at which URI,
//! computed as data before anything is sent. The server's `Client` only
//! transmits the returned actions, so the whole lifecycle — open, edit,
//! close, shared dependencies — is testable synchronously, without a
//! language-server connection.
//!
//! Each open document is an *owner*: its analysis produces diagnostic groups
//! for one or more *targets* (its own URI, plus each imported file with
//! diagnostics). A target's published list is the union of every owner's
//! group for it, so two open documents importing the same broken module
//! cannot overwrite each other's view — closing or fixing one leaves the
//! other's diagnostics standing.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Range, Url,
};

use crate::document::Document;
use crate::line_index::LineIndex;

/// The bookkeeping for everything published so far: each owner's last
/// diagnostic groups, keyed by owner then target. `BTreeMap` so merged
/// unions list owners in a stable order (diagnostics-standard.md C1 —
/// republishing without a change must not reorder).
pub struct PublishState {
    owned: BTreeMap<Url, Vec<(Url, Vec<Diagnostic>)>>,
}

impl PublishState {
    pub fn new() -> Self {
        PublishState {
            owned: BTreeMap::new(),
        }
    }

    /// Re-plan after `owner` was (re)analyzed: recompute its groups and
    /// return one `(target, merged diagnostics)` action per target the
    /// change touches — including targets the owner dropped since last
    /// time, which get the remaining owners' merged view (possibly empty),
    /// so nothing goes stale.
    pub fn plan_publish(
        &mut self,
        owner: &Url,
        document: &Document,
    ) -> Vec<(Url, Vec<Diagnostic>)> {
        let groups = diagnostic_groups(document, owner);
        let mut affected: Vec<Url> = groups.iter().map(|(target, _)| target.clone()).collect();
        if let Some(previous) = self.owned.get(owner) {
            for (target, _) in previous {
                if !affected.contains(target) {
                    affected.push(target.clone());
                }
            }
        }
        self.owned.insert(owner.clone(), groups);
        affected
            .into_iter()
            .map(|target| {
                let merged = self.merged(&target);
                (target, merged)
            })
            .collect()
    }

    /// Remove `owner` (the document closed) and return the republish
    /// actions for every target it contributed to — each now the remaining
    /// owners' merged view, empty where it was the only contributor.
    pub fn plan_close(&mut self, owner: &Url) -> Vec<(Url, Vec<Diagnostic>)> {
        let Some(previous) = self.owned.remove(owner) else {
            return Vec::new();
        };
        previous
            .into_iter()
            .map(|(target, _)| {
                let merged = self.merged(&target);
                (target, merged)
            })
            .collect()
    }

    /// The union of every owner's group for `target`, deduplicated — two
    /// owners that see the same error in a shared module contribute it
    /// once.
    fn merged(&self, target: &Url) -> Vec<Diagnostic> {
        let mut merged: Vec<Diagnostic> = Vec::new();
        for groups in self.owned.values() {
            for (candidate, group) in groups {
                if candidate != target {
                    continue;
                }
                for diagnostic in group {
                    if !merged.contains(diagnostic) {
                        merged.push(diagnostic.clone());
                    }
                }
            }
        }
        merged
    }
}

/// One analyzed document's diagnostics as per-target groups: the entry's own
/// (always present, even when empty, so the owner's URI is always brought
/// current) plus each imported file's, with spans converted through a fresh
/// read of *that* file — the analysis read it from disk too, so they agree.
fn diagnostic_groups(document: &Document, owner: &Url) -> Vec<(Url, Vec<Diagnostic>)> {
    let mut entry_group: Vec<Diagnostic> = Vec::new();
    let mut extra_groups: Vec<(Url, Vec<Diagnostic>)> = Vec::new();
    let mut extra_indices: HashMap<PathBuf, Option<Arc<LineIndex>>> = HashMap::new();
    for item in document.published_diagnostics() {
        let severity = if item.warning {
            DiagnosticSeverity::WARNING
        } else {
            DiagnosticSeverity::ERROR
        };
        let diagnostic = |range| Diagnostic {
            range,
            severity: Some(severity),
            source: Some("vilan".to_string()),
            message: item.message.clone(),
            ..Default::default()
        };
        match &item.path {
            None => {
                let mut converted = diagnostic(document.line_index.range(&item.span));
                // A secondary note becomes related information — "first
                // call here"-style anchors.
                if let Some((note_span, note_msg, note_path)) = &item.note {
                    // A cross-source note points into ITS file, with a
                    // fresh index for that file's positions.
                    let located = match note_path {
                        None => Some((owner.clone(), document.line_index.range(note_span))),
                        Some(path) => std::fs::read_to_string(path)
                            .ok()
                            .map(|text| LineIndex::new(&text).range(note_span))
                            .and_then(|range| {
                                Url::from_file_path(path).ok().map(|target| (target, range))
                            }),
                    };
                    if let Some((target, range)) = located {
                        converted.related_information = Some(vec![DiagnosticRelatedInformation {
                            location: Location { uri: target, range },
                            message: note_msg.clone(),
                        }]);
                    }
                }
                entry_group.push(converted);
            }
            Some(path) => {
                // A fresh (uncached) read: module files change across saves,
                // so a session-cached index would misplace ranges.
                let index = extra_indices
                    .entry(path.clone())
                    .or_insert_with(|| {
                        std::fs::read_to_string(path)
                            .ok()
                            .map(|text| Arc::new(LineIndex::new(&text)))
                    })
                    .clone();
                match (index, Url::from_file_path(path)) {
                    (Some(index), Ok(target)) => {
                        let converted = diagnostic(index.range(&item.span));
                        match extra_groups
                            .iter_mut()
                            .find(|(existing, _)| *existing == target)
                        {
                            Some((_, group)) => group.push(converted),
                            None => extra_groups.push((target, vec![converted])),
                        }
                    }
                    // Unreadable file: keep the error visible on the entry.
                    _ => entry_group.push(Diagnostic {
                        range: Range::default(),
                        severity: Some(severity),
                        source: Some("vilan".to_string()),
                        message: format!("(in {}) {}", path.display(), item.message),
                        ..Default::default()
                    }),
                }
            }
        }
    }
    let mut groups = vec![(owner.clone(), entry_group)];
    groups.extend(extra_groups);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::{analyze_workspace, std_root};
    use std::path::Path;

    /// Analyze `relative` under `dir` as an open document (its own entry,
    /// like the server does for every open file).
    fn open(dir: &Path, relative: &str) -> (Url, Document) {
        let path = dir.join(relative);
        let text = std::fs::read_to_string(&path).unwrap();
        let document = Document::analyze(&text, &std_root(), &path);
        (Url::from_file_path(&path).unwrap(), document)
    }

    fn apply(editor: &mut BTreeMap<Url, Vec<Diagnostic>>, actions: Vec<(Url, Vec<Diagnostic>)>) {
        for (target, group) in actions {
            editor.insert(target, group);
        }
    }

    /// What the editor should show for exactly `open_documents`: a fresh
    /// planner replayed from scratch, empty targets dropped.
    fn fresh_view(open_documents: &[(&Url, &Document)]) -> BTreeMap<Url, Vec<Diagnostic>> {
        let mut state = PublishState::new();
        let mut editor: BTreeMap<Url, Vec<Diagnostic>> = BTreeMap::new();
        for (uri, document) in open_documents {
            apply(&mut editor, state.plan_publish(uri, document));
        }
        editor.retain(|_, group| !group.is_empty());
        editor
    }

    fn visible(editor: &BTreeMap<Url, Vec<Diagnostic>>) -> BTreeMap<Url, Vec<Diagnostic>> {
        let mut visible = editor.clone();
        visible.retain(|_, group| !group.is_empty());
        visible
    }

    // The E6 lifecycle property: after every open/edit/close, what the
    // editor shows (the applied actions) equals a fresh analysis of the
    // currently-open documents — nothing stale, nothing lost. The scenario
    // exercises the shared-dependency union: two open documents import the
    // same broken module, then one drops it, then each closes.
    #[test]
    fn published_equals_fresh_analysis_across_the_lifecycle() {
        let broken = "fun answer(): i32 {\n\t\"not a number\"\n}\n";
        let (dir, _) = analyze_workspace(&[
            (
                "main.vl",
                "import std::print;\nimport pkg::broken::answer;\nfun main() { print(answer()); }\n",
            ),
            (
                "other.vl",
                "import std::print;\nimport pkg::broken::answer;\nfun main() { print(answer() + 1); }\n",
            ),
            ("broken.vl", broken),
        ]);
        let mut state = PublishState::new();
        let mut editor: BTreeMap<Url, Vec<Diagnostic>> = BTreeMap::new();

        // Open main: the module error shows at broken.vl.
        let (main_uri, main_document) = open(&dir, "main.vl");
        apply(&mut editor, state.plan_publish(&main_uri, &main_document));
        assert_eq!(
            visible(&editor),
            fresh_view(&[(&main_uri, &main_document)]),
            "after opening main"
        );
        let broken_uri = visible(&editor)
            .keys()
            .find(|target| target.path().ends_with("broken.vl"))
            .cloned()
            .expect("the module error publishes at broken.vl");

        // Open other: both owners see the same module error — the union
        // holds ONE copy, not last-writer's.
        let (other_uri, other_document) = open(&dir, "other.vl");
        apply(&mut editor, state.plan_publish(&other_uri, &other_document));
        assert_eq!(
            visible(&editor),
            fresh_view(&[(&main_uri, &main_document), (&other_uri, &other_document)]),
            "after opening other"
        );
        assert_eq!(
            editor.get(&broken_uri).map(Vec::len),
            Some(1),
            "identical views of the shared module deduplicate"
        );

        // Edit main to drop the import: broken.vl must KEEP other's view —
        // the last-writer-wins case the union exists for.
        std::fs::write(
            dir.join("main.vl"),
            "import std::print;\nfun main() { print(1); }\n",
        )
        .unwrap();
        let (_, main_edited) = open(&dir, "main.vl");
        apply(&mut editor, state.plan_publish(&main_uri, &main_edited));
        assert_eq!(
            visible(&editor),
            fresh_view(&[(&main_uri, &main_edited), (&other_uri, &other_document)]),
            "after editing main"
        );
        assert_eq!(
            editor.get(&broken_uri).map(Vec::len),
            Some(1),
            "the remaining owner's view of the shared module survives"
        );

        // Close other: no owner sees broken.vl any more — explicit empty.
        apply(&mut editor, state.plan_close(&other_uri));
        assert_eq!(
            visible(&editor),
            fresh_view(&[(&main_uri, &main_edited)]),
            "after closing other"
        );
        assert_eq!(
            editor.get(&broken_uri).map(Vec::len),
            Some(0),
            "the dropped module target clears explicitly"
        );

        // Close main: everything clears.
        apply(&mut editor, state.plan_close(&main_uri));
        assert_eq!(visible(&editor), BTreeMap::new(), "after closing main");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // An entry's OWN diagnostics publish at its URI, update in place on an
    // edit that fixes them, and clear on close — the single-document
    // lifecycle (explicit empties included).
    #[test]
    fn own_diagnostics_update_and_clear_with_the_document() {
        let (dir, _) =
            analyze_workspace(&[("solo.vl", "fun main() {\n\tlet wrong: i32 = \"text\";\n}\n")]);
        let mut state = PublishState::new();
        let mut editor: BTreeMap<Url, Vec<Diagnostic>> = BTreeMap::new();

        let (uri, document) = open(&dir, "solo.vl");
        apply(&mut editor, state.plan_publish(&uri, &document));
        assert!(
            editor.get(&uri).is_some_and(|group| !group.is_empty()),
            "the type error publishes at the entry"
        );
        assert_eq!(visible(&editor), fresh_view(&[(&uri, &document)]));

        std::fs::write(
            dir.join("solo.vl"),
            "fun main() {\n\tlet right: i32 = 1;\n}\n",
        )
        .unwrap();
        let (_, fixed) = open(&dir, "solo.vl");
        apply(&mut editor, state.plan_publish(&uri, &fixed));
        assert_eq!(
            editor.get(&uri).map(Vec::len),
            Some(0),
            "fixing the error publishes an explicit empty"
        );

        apply(&mut editor, state.plan_close(&uri));
        assert_eq!(visible(&editor), BTreeMap::new());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
