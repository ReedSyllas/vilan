pub fn plural(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 { singular } else { plural }.to_string()
}

use std::cell::Cell;

thread_local! {
    static RECURSION_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// A safety net for the recursive type operations (`reconcile_type`,
/// `substitute_type`, the transformer's `resolve_type_id`). A self-mapping or
/// otherwise pathological generic graph that slips past the explicit guards must
/// degrade to a graceful bail rather than overflow the stack — a compiler should
/// never crash on user input. The limit is far above any real type's nesting.
pub struct RecursionGuard;

impl RecursionGuard {
    /// Enters one level of recursion; `None` once the depth limit is reached, so
    /// the caller can return a graceful fallback instead of recursing.
    pub fn enter() -> Option<RecursionGuard> {
        RECURSION_DEPTH.with(|depth| {
            let current = depth.get();
            if current >= 2048 {
                None
            } else {
                depth.set(current + 1);
                Some(RecursionGuard)
            }
        })
    }
}

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        RECURSION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Trims a triple-quoted string literal's raw inner text — everything between
/// the `"""` delimiters — to its content (backlog H4; Swift's multiline rule):
///
/// - The opening `"""` is followed by a newline (after optional whitespace):
///   content starts on the next line.
/// - The closing `"""` sits alone on its line; the whitespace before it is the
///   INDENTATION PREFIX, stripped from every content line.
/// - A content line must start with that exact prefix (the same characters, so
///   a tab never satisfies a space prefix) — unless it is whitespace-only, in
///   which case it may be shorter and becomes empty.
/// - The newlines adjoining the delimiters belong to the syntax, not the
///   string. The body is RAW: no escape processing at all (the appeal is
///   pasting code verbatim), so `\n` is a backslash and an `n`.
///
/// A `\r` before any line-ending `\n` is dropped (CRLF tolerance).
///
/// An error carries the offending byte range RELATIVE TO `raw`, so the caller
/// can span the diagnostic at the exact offender rather than the whole literal.
pub fn trim_multiline_string(raw: &str) -> Result<String, (String, std::ops::Range<usize>)> {
    let Some(first_newline) = raw.find('\n') else {
        return Err((
            "a triple-quoted string spans lines: the opening \"\"\" must be followed by a newline"
                .to_string(),
            0..raw.len(),
        ));
    };
    let opener_rest = raw[..first_newline].trim_end_matches('\r');
    if !opener_rest.trim().is_empty() {
        let start = opener_rest.len() - opener_rest.trim_start().len();
        return Err((
            format!(
                "nothing may follow the opening \"\"\" on its line (found `{}`)",
                opener_rest.trim()
            ),
            start..opener_rest.trim_end().len(),
        ));
    }
    let last_newline = raw.rfind('\n').expect("found above");
    let prefix = &raw[last_newline + 1..];
    if !prefix.chars().all(|c| c == ' ' || c == '\t') {
        return Err((
            "the closing \"\"\" must sit alone on its line, preceded only by indentation"
                .to_string(),
            last_newline + 1..raw.len(),
        ));
    }
    if first_newline == last_newline {
        // `"""` directly followed by the closing line: zero content lines.
        return Ok(String::new());
    }
    let body = &raw[first_newline + 1..last_newline];
    let mut content_lines = Vec::new();
    let mut line_start = first_newline + 1;
    for (index, line) in body.split('\n').enumerate() {
        let raw_line_length = line.len();
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix(prefix) {
            content_lines.push(rest);
        } else if line.chars().all(|c| c == ' ' || c == '\t') {
            // A whitespace-only line may fall short of the prefix.
            content_lines.push("");
        } else {
            return Err((
                format!(
                    "line {} of the triple-quoted string is not indented to its closing \"\"\" \
                     (every line must start with the whitespace that precedes the closing delimiter)",
                    index + 1
                ),
                line_start..line_start + line.len(),
            ));
        }
        line_start += raw_line_length + 1;
    }
    Ok(content_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::trim_multiline_string;

    #[test]
    fn trims_each_line_by_the_closing_indentation() {
        // The motivating example (H4): opener/closer indented 4 spaces.
        let raw = "\n        line 1\n    line 2\n\n      line 3\n        \n    ";
        assert_eq!(
            trim_multiline_string(raw).unwrap(),
            "    line 1\nline 2\n\n  line 3\n    "
        );
    }

    #[test]
    fn a_tab_prefix_strips_tabs() {
        let raw = "\n\t\thello\n\tworld\n\t";
        assert_eq!(trim_multiline_string(raw).unwrap(), "\thello\nworld");
    }

    #[test]
    fn a_column_zero_closer_strips_nothing() {
        let raw = "\n  a\nb\n";
        assert_eq!(trim_multiline_string(raw).unwrap(), "  a\nb");
    }

    #[test]
    fn a_short_whitespace_only_line_becomes_empty() {
        let raw = "\n    a\n  \n    b\n    ";
        assert_eq!(trim_multiline_string(raw).unwrap(), "a\n\nb");
    }

    #[test]
    fn zero_content_lines_is_the_empty_string() {
        assert_eq!(trim_multiline_string("\n    ").unwrap(), "");
        assert_eq!(trim_multiline_string("\n").unwrap(), "");
    }

    #[test]
    fn crlf_line_endings_are_tolerated() {
        let raw = "\r\n    a\r\n    b\r\n    ";
        assert_eq!(trim_multiline_string(raw).unwrap(), "a\nb");
    }

    #[test]
    fn trailing_whitespace_after_the_prefix_is_kept() {
        let raw = "\n    a   \n    ";
        assert_eq!(trim_multiline_string(raw).unwrap(), "a   ");
    }

    #[test]
    fn no_newline_at_all_is_an_error() {
        let (error, _) = trim_multiline_string("one line").unwrap_err();
        assert!(error.contains("followed by a newline"), "{error}");
    }

    #[test]
    fn content_after_the_opener_is_an_error() {
        let (error, range) = trim_multiline_string("oops\n    a\n    ").unwrap_err();
        assert!(error.contains("nothing may follow the opening"), "{error}");
        assert!(error.contains("oops"), "{error}");
        assert_eq!(range, 0..4, "the range covers the offending text");
    }

    #[test]
    fn content_before_the_closer_is_an_error() {
        let (error, _) = trim_multiline_string("\n    a\n    b: ").unwrap_err();
        assert!(error.contains("alone on its line"), "{error}");
    }

    #[test]
    fn insufficient_indentation_is_an_error_naming_the_line() {
        let (error, range) = trim_multiline_string("\n    a\n  b\n    ").unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        assert!(error.contains("not indented"), "{error}");
        // raw = "\n    a\n  b\n    ": line 2 ("  b") starts at byte 7.
        assert_eq!(range, 7..10, "the range covers the offending line");
    }

    #[test]
    fn a_tab_never_satisfies_a_space_prefix() {
        let (error, _) = trim_multiline_string("\n\ta\n    ").unwrap_err();
        assert!(error.contains("line 1"), "{error}");
    }
}
