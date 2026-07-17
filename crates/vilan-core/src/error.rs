use crate::analyzer::SourceId;
use crate::span::Span;

/// A diagnostic's secondary location + label (diagnostics-standard.md C3):
/// "first call here", "the trait declares it here". One, not a list —
/// diagnostics stay terse. `source` names the note's file when it differs
/// from the primary span's (`None` = the same file); the CLI renders it as
/// an ariadne sub-label, the language server as related information.
#[derive(Debug, Clone)]
pub struct Note {
    pub span: Span,
    pub msg: String,
    pub source: Option<SourceId>,
}

impl Note {
    /// A note in the SAME file as the diagnostic's primary span.
    pub fn here(span: Span, msg: String) -> Self {
        Note {
            span,
            msg,
            source: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Error {
    pub span: Span,
    pub msg: String,
    pub note: Option<Note>,
}
