//! The compiler's source-span type: a half-open byte range `[start, end)` into a
//! source string.
//!
//! Owned outright — no chumsky dependency (the whole point of the handwritten
//! frontend, H6). Offsets are `usize` byte offsets, matching every consumer's
//! arithmetic (string slicing, `char::len_utf8`, the parser's `position`/`eoi`,
//! the LSP's line/column conversion), so the ~15 modules that read spans read
//! them exactly as they did under chumsky's `SimpleSpan<usize>`.

use std::ops::Range;

/// A half-open byte range `[start, end)` into a source string.
///
/// `Debug` renders as `start..end` — identical to chumsky's `SimpleSpan` — which
/// the span-inclusive AST differential and every span-bearing snapshot rely on.
#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    /// The start byte offset.
    pub start: usize,
    /// The end (exclusive) byte offset.
    pub end: usize,
}

impl Span {
    /// Construct a span from a byte range. The unit `context` parameter matches
    /// the `Span::new((), range)` call shape used across the pipeline — a holdover
    /// from chumsky's context-carrying constructor; vilan spans carry no context.
    pub fn new(_context: (), range: Range<usize>) -> Self {
        Span {
            start: range.start,
            end: range.end,
        }
    }

    /// This span as a `Range<usize>` — for string slicing, ariadne, and the LSP's
    /// byte-offset conversions.
    pub fn into_range(self) -> Range<usize> {
        self.start..self.end
    }

    /// A zero-width span at this span's end (`3..7` becomes `7..7`).
    pub fn to_end(self) -> Self {
        Span {
            start: self.end,
            end: self.end,
        }
    }
}

impl From<Range<usize>> for Span {
    fn from(range: Range<usize>) -> Self {
        Span {
            start: range.start,
            end: range.end,
        }
    }
}

impl std::fmt::Debug for Span {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}..{:?}", self.start, self.end)
    }
}

/// A value paired with the source span it came from.
pub type Spanned<T> = (T, Span);
