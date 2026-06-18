//! The source formatter behind `vilan fmt`: it reparses a file and reprints the
//! AST in canonical style (tab indentation, normalized spacing and blank lines),
//! reattaching the comments the lexer drops as trivia.
//!
//! Safety: reprinting from the AST could, given a bug, silently change a program.
//! So `format` re-lexes its own output and checks the token stream matches the
//! input's (ignoring spans, whitespace, and comments); on any mismatch it returns
//! the source unchanged rather than risk corrupting the file.

use chumsky::prelude::*;

use crate::lexer::lexer;
use crate::node::{ImportBranch, Node, NodeList};
use crate::parser::parser;
use crate::span::{Span, Spanned};
use crate::token::Token;

/// Extracts `//` line comments from `source` as `(span, text)` in source order.
/// `text` keeps the leading `//` and is trimmed of trailing whitespace. String
/// literals are skipped so a `//` inside a string isn't taken for a comment.
pub fn extract_comments(source: &str) -> Vec<(Span, &str)> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                let text = source[start..index].trim_end();
                comments.push((Span::new((), start..start + text.len()), text));
            }
            _ => index += 1,
        }
    }
    comments
}

/// The lexer's token stream with spans stripped — the formatter's notion of "the
/// same code", used to check a reprint didn't change anything but trivia.
fn code_tokens(source: &str) -> Option<Vec<Token<'_>>> {
    lexer()
        .parse(source)
        .into_output()
        .map(|tokens| tokens.into_iter().map(|(token, _)| token).collect())
}

/// Parses `source` into its top-level item list, or `None` if it doesn't parse.
fn parse(source: &str) -> Option<NodeList<'_>> {
    let tokens = lexer().parse(source).into_output()?;
    let end = source.len();
    let stream = tokens
        .as_slice()
        .map((end..end).into(), |(token, span)| (token, span));
    parser().parse(stream).into_output().map(|(items, _)| items)
}

/// Formats `source`, returning the reprinted text. Returns the input unchanged if
/// it doesn't lex/parse, if the printer hits a construct it doesn't yet handle, or
/// if the reprint would change the code (see the safety note).
pub fn format(source: &str) -> String {
    let Some(original) = code_tokens(source) else {
        return source.to_string();
    };
    let Some(items) = parse(source) else {
        return source.to_string();
    };
    let mut printer = Printer {
        out: String::new(),
        indent: 0,
        comments: extract_comments(source),
        cursor: 0,
        source,
        bailed: false,
    };
    let prev_end = printer.print_items(&items, 0);
    // Comments after the last item (trailing end-of-file comments).
    printer.flush_comments_before(source.len(), prev_end);
    printer.out.push('\n');
    if printer.bailed {
        return source.to_string();
    }
    match code_tokens(&printer.out) {
        Some(reprinted) if reprinted == original => printer.out,
        _ => source.to_string(),
    }
}

struct Printer<'src> {
    out: String,
    indent: usize,
    comments: Vec<(Span, &'src str)>,
    cursor: usize,
    source: &'src str,
    bailed: bool,
}

impl<'src> Printer<'src> {
    /// Whether the source between `from` and `to` contains a blank line (a run of
    /// only-whitespace with two or more newlines), used to preserve paragraph gaps.
    fn has_blank_between(&self, from: usize, to: usize) -> bool {
        from < to
            && self
                .source
                .get(from..to)
                .is_some_and(|gap| gap.bytes().filter(|byte| *byte == b'\n').count() >= 2)
    }

    /// Starts a fresh line at the current indentation (no leading newline at the
    /// very start of the output).
    fn line(&mut self) {
        if !self.out.is_empty() {
            self.out.push('\n');
            for _ in 0..self.indent {
                self.out.push('\t');
            }
        }
    }

    /// Emits a blank line (used to preserve a paragraph gap before the next item).
    fn blank_line(&mut self) {
        if !self.out.is_empty() {
            self.out.push('\n');
        }
    }

    /// Emits the standalone comments that appear before `pos`, each on its own
    /// line, preserving a blank line before a comment that the source had one
    /// before. Returns the source offset just past the last comment emitted (or
    /// `start_from` if none), so the caller can judge the gap before the item.
    fn flush_comments_before(&mut self, pos: usize, start_from: usize) -> usize {
        let mut at = start_from;
        while self.cursor < self.comments.len() {
            let (span, text) = self.comments[self.cursor];
            let range = span.into_range();
            if range.start >= pos {
                break;
            }
            if self.has_blank_between(at, range.start) {
                self.blank_line();
            }
            self.line();
            self.out.push_str(text);
            at = range.end;
            self.cursor += 1;
        }
        at
    }

    /// Prints a list of items (top level or a block body), interleaving standalone
    /// comments and preserved blank lines. Returns the source offset past the last
    /// item, for any trailing comments.
    fn print_items(&mut self, items: &[Spanned<Node<'src>>], start_from: usize) -> usize {
        let mut prev_end = start_from;
        for (node, span) in items {
            let range = span.into_range();
            let after_comments = self.flush_comments_before(range.start, prev_end);
            if self.has_blank_between(after_comments, range.start) {
                self.blank_line();
            }
            self.line();
            self.print_item(node);
            prev_end = range.end;
        }
        prev_end
    }

    /// Prints one top-level / block item. Sets `bailed` for anything not yet
    /// handled, so `format` falls back to the original source.
    fn print_item(&mut self, node: &Node<'src>) {
        match node {
            // `[external ]struct Name;` — only the bodyless form for now.
            Node::Struct(name, None, external, None) => {
                if *external {
                    self.out.push_str("external ");
                }
                self.out.push_str("struct ");
                self.out.push_str(name.0);
                self.out.push(';');
            }
            // `enum Name { variants }` — simple (no generics, no payload/discriminant).
            Node::Enum(name, None, variants)
                if variants
                    .0
                    .iter()
                    .all(|(variant, _)| variant.1.is_empty() && variant.2.is_none()) =>
            {
                self.out.push_str("enum ");
                self.out.push_str(name.0);
                self.out.push_str(" {");
                self.indent += 1;
                for (variant, _) in &variants.0 {
                    self.line();
                    self.out.push_str(variant.0);
                    self.out.push(',');
                }
                self.indent -= 1;
                self.line();
                self.out.push('}');
            }
            Node::Use(branch) => {
                self.out.push_str("use ");
                self.print_import_branch(branch);
                self.out.push(';');
            }
            Node::Import(branch) => {
                self.out.push_str("import ");
                self.print_import_branch(branch);
                self.out.push(';');
            }
            _ => self.bailed = true,
        }
    }

    /// Prints an import/use path: `a::b::{ c, d }`.
    fn print_import_branch(&mut self, branch: &ImportBranch<'src>) {
        match branch {
            ImportBranch::Path(name, _, child) => {
                self.out.push_str(name);
                if let Some(child) = child {
                    self.out.push_str("::");
                    self.print_import_branch(child);
                }
            }
            ImportBranch::Set(branches) => {
                self.out.push_str("{ ");
                for (index, child) in branches.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_import_branch(child);
                }
                self.out.push_str(" }");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_comments_skipping_strings() {
        let source = "let url = \"http://x\"; // a note\n// lead\nfun f() {}\n";
        let comments: Vec<&str> = extract_comments(source)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(comments, vec!["// a note", "// lead"]);
    }

    #[test]
    fn comment_spans_are_forward_and_trimmed() {
        let source = "fun f() {} // trailing  \n";
        let (span, text) = extract_comments(source)[0];
        assert_eq!(text, "// trailing");
        let range = span.into_range();
        assert!(range.start <= range.end);
        assert_eq!(&source[range.start..range.end], "// trailing");
    }
}

#[cfg(test)]
mod idempotency {
    use super::format;

    fn assert_idempotent(name: &str, source: &str) {
        let formatted = format(source);
        assert_eq!(formatted, source, "formatting changed {name}");
    }

    #[test]
    fn null_vl() {
        assert_idempotent("null.vl", include_str!("../../../vilan/std/src/null.vl"));
    }

    #[test]
    fn boolean_vl() {
        assert_idempotent(
            "boolean.vl",
            include_str!("../../../vilan/std/src/boolean.vl"),
        );
    }
}
