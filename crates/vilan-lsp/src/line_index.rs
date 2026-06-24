//! Converts between Vilan's byte-offset spans and LSP line/character positions.
//! LSP positions count UTF-16 code units within a line.

use tower_lsp::lsp_types::{Position, Range};
use vilan_core::Span;

pub struct LineIndex {
    /// Byte offset at which each line begins (line 0 starts at 0).
    line_starts: Vec<usize>,
    text: String,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        LineIndex {
            line_starts,
            text: text.to_string(),
        }
    }

    /// The LSP position for a byte offset.
    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        };
        let line_start = self.line_starts[line];
        // Count UTF-16 units from the line start up to `offset` by iterating
        // characters (a line start is always on a char boundary, so the open-ended
        // slice is safe). A `text[line_start..offset]` slice would instead *panic*
        // if `offset` fell inside a multi-byte character — which a malformed span
        // boundary can be, and which must never crash the language server.
        let mut character = 0usize;
        let mut byte = line_start;
        for c in self.text[line_start..].chars() {
            if byte >= offset {
                break;
            }
            character += c.len_utf16();
            byte += c.len_utf8();
        }
        Position {
            line: line as u32,
            character: character as u32,
        }
    }

    /// The document's source text (for completion's backward scan over the
    /// characters preceding the cursor).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The byte offset for an LSP position.
    pub fn offset(&self, position: Position) -> usize {
        let line_start = self
            .line_starts
            .get(position.line as usize)
            .copied()
            .unwrap_or(self.text.len());
        let mut utf16 = 0usize;
        let mut offset = line_start;
        for c in self.text[line_start..].chars() {
            if utf16 >= position.character as usize || c == '\n' {
                break;
            }
            utf16 += c.len_utf16();
            offset += c.len_utf8();
        }
        offset
    }

    /// The LSP range for a span.
    pub fn range(&self, span: &Span) -> Range {
        let range = span.into_range();
        Range {
            start: self.position(range.start),
            end: self.position(range.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_counts_utf16_units_at_char_boundaries() {
        // `—` (em-dash) is 3 bytes but 1 UTF-16 unit; `😀` is 4 bytes, 2 UTF-16 units.
        let text = "// — 😀 x\n";
        let index = LineIndex::new(text);
        assert_eq!(index.position(0).character, 0);
        assert_eq!(index.position(3).character, 3); // "// " = 3
        assert_eq!(index.position(6).character, 4); // "// —" = 4
        assert_eq!(index.position(7).character, 5); // "// — " = 5
        assert_eq!(index.position(11).character, 7); // + 😀 (2) = 7
    }

    #[test]
    fn position_inside_a_multibyte_char_does_not_panic() {
        // A byte offset that lands *inside* a multi-byte character — which a
        // malformed span boundary can be — must never panic the server (it used to
        // slice `text[line_start..offset]` and abort). It resolves to a stable,
        // non-decreasing position instead.
        let text = "// plain — NO\n"; // `—` occupies bytes 9..12
        let index = LineIndex::new(text);
        let before = index.position(9).character;
        let mid = index.position(10).character; // inside the em-dash — must not panic
        let after = index.position(12).character;
        assert!(before <= mid && mid <= after, "{before} {mid} {after}");
    }

    #[test]
    fn position_clamps_past_end_of_text() {
        let index = LineIndex::new("abc\n");
        // An out-of-range offset clamps to the text length rather than panicking.
        let _ = index.position(1000);
    }
}
