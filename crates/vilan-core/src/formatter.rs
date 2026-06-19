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

/// Drops every comma that sits immediately before a closing `}`, `)`, or `]`.
/// Vilan treats such a trailing comma as insignificant (tuples need two or more
/// elements, so there is no `(a,)` one-tuple to confuse it with), which lets the
/// safety check accept the formatter normalizing trailing commas in or out.
fn normalize(tokens: Vec<Token<'_>>) -> Vec<Token<'_>> {
    let mut result: Vec<Token<'_>> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if matches!(
            token,
            Token::Ctrl('}') | Token::Ctrl(')') | Token::Ctrl(']')
        ) {
            while let Some(Token::Ctrl(',')) = result.last() {
                result.pop();
            }
        }
        result.push(token);
    }
    result
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
    let matches = code_tokens(&printer.out)
        .is_some_and(|reprinted| normalize(reprinted) == normalize(original));
    if matches {
        printer.out
    } else {
        source.to_string()
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
            // `struct Name { field: Type, ... }` (no generics yet).
            Node::Struct(name, None, external, Some(fields)) => {
                if *external {
                    self.out.push_str("external ");
                }
                self.out.push_str("struct ");
                self.out.push_str(name.0);
                if fields.0.is_empty() {
                    self.out.push_str(" {}");
                } else {
                    self.out.push_str(" {");
                    self.indent += 1;
                    for ((field_name, field_type), _) in &fields.0 {
                        self.line();
                        self.out.push_str(field_name);
                        if let Some(field_type) = field_type {
                            self.out.push_str(": ");
                            self.print_type(&field_type.0);
                        }
                        self.out.push(',');
                    }
                    self.indent -= 1;
                    self.line();
                    self.out.push('}');
                }
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

    /// Prints a type expression: `i32`, `List<T>`, `Map<str, i32>`, `&mut T`.
    /// Bails (falling `format` back to the source) on any type form not yet handled.
    fn print_type(&mut self, node: &Node<'src>) {
        match node {
            Node::Accessor(name) => self.out.push_str(name),
            Node::AccessorWithGenerics(name, arguments) => {
                self.out.push_str(name);
                self.out.push('<');
                for (index, (argument, _)) in arguments.0.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_type(argument);
                }
                self.out.push('>');
            }
            Node::Reference(mutable, inner) => {
                self.out.push('&');
                if *mutable {
                    self.out.push_str("mut ");
                }
                self.print_type(&inner.0);
            }
            _ => self.bailed = true,
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
mod reformats {
    use super::format;

    fn assert_formats(source: &str, expected: &str) {
        assert_eq!(format(source), expected);
        // The output must be a fixed point — formatting it again is a no-op.
        assert_eq!(format(expected), expected, "output is not idempotent");
    }

    #[test]
    fn struct_fields_onto_their_own_lines() {
        assert_formats(
            "struct Point{x:i32,y:i32}\n",
            "struct Point {\n\tx: i32,\n\ty: i32,\n}\n",
        );
    }

    #[test]
    fn generic_and_reference_field_types() {
        assert_formats(
            "struct Boxed { item :  List<i32> , next : &mut Node }\n",
            "struct Boxed {\n\titem: List<i32>,\n\tnext: &mut Node,\n}\n",
        );
    }

    #[test]
    fn empty_struct_body_stays_inline() {
        assert_formats("struct Unit{\n}\n", "struct Unit {}\n");
    }

    #[test]
    fn enum_variants_onto_their_own_lines() {
        assert_formats("enum E{A,B}\n", "enum E {\n\tA,\n\tB,\n}\n");
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
