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
        let character = self.text[line_start..offset]
            .chars()
            .map(|c| c.len_utf16())
            .sum::<usize>();
        Position {
            line: line as u32,
            character: character as u32,
        }
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
