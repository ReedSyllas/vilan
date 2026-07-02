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
use crate::node::{
    BinaryOp, Convention, ExternBinding, Func, GenericParameters, ImportBranch, Node, NodeIfBranch,
    NodeList, Pattern,
};
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

    /// Emits a trailing (same-line) comment if the next pending comment starts on
    /// the same source line as `after` — i.e. it sat at the end of the item just
    /// printed (`foo(); // note`) rather than on its own line. Spacing collapses to
    /// a single space.
    fn flush_trailing_comment(&mut self, after: usize) {
        if let Some((span, text)) = self.comments.get(self.cursor).copied() {
            let start = span.into_range().start;
            if start >= after
                && self
                    .source
                    .get(after..start)
                    .is_some_and(|gap| !gap.contains('\n'))
            {
                self.out.push(' ');
                self.out.push_str(text);
                self.cursor += 1;
            }
        }
    }

    /// Prints a list of items (top level or a block body), interleaving standalone
    /// comments and preserved blank lines. Returns the source offset past the last
    /// item, for any trailing comments.
    fn print_items(&mut self, items: &[Spanned<Node<'src>>], start_from: usize) -> usize {
        let mut prev_end = start_from;
        for item in items {
            let range = item.1.into_range();
            let after_comments = self.flush_comments_before(range.start, prev_end);
            if self.has_blank_between(after_comments, range.start) {
                self.blank_line();
            }
            self.line();
            self.print_item(item);
            if Self::needs_semicolon(&item.0) {
                self.out.push(';');
            }
            self.flush_trailing_comment(range.end);
            prev_end = range.end;
        }
        prev_end
    }

    /// Whether `node`, printed as a statement, takes a terminating `;`. Expression
    /// statements (`let`, assignments, calls, …) do; control-flow forms
    /// (`if`/`for`/`match`/block), declarations, and `use`/`import` (which already
    /// emit their own `;`) do not.
    fn needs_semicolon(node: &Node<'src>) -> bool {
        !matches!(
            node,
            Node::If(_)
                | Node::For(_, _)
                | Node::ForIn(_, _, _)
                | Node::Match(_, _)
                | Node::Block(_)
                | Node::Func(_)
                | Node::Struct(_, _, _, _)
                | Node::Enum(_, _, _)
                | Node::Impl(_, _, _)
                | Node::Trait(_, _, _, _)
                | Node::Module(_, _)
                | Node::Derive(_, _)
                | Node::Export(_)
                | Node::Use(_)
                | Node::Import(_)
        )
    }

    /// Prints one top-level / block item. Sets `bailed` for anything not yet
    /// handled, so `format` falls back to the original source.
    fn print_item(&mut self, item: &Spanned<Node<'src>>) {
        match &item.0 {
            // `[external ]struct Name[<…>][;|{ fields }]`.
            Node::Struct(name, generics, external, body) => {
                if *external {
                    self.out.push_str("external ");
                }
                self.out.push_str("struct ");
                self.out.push_str(name.0);
                self.print_generic_parameters(generics);
                match body {
                    None => self.out.push(';'),
                    Some(fields) if fields.0.is_empty() => self.out.push_str(" {}"),
                    Some(fields) => {
                        self.out.push_str(" {");
                        self.indent += 1;
                        let mut prev_end = fields.1.into_range().start + 1;
                        for ((field_name, field_type, exposed), span) in &fields.0 {
                            let range = span.into_range();
                            let after_comments = self.flush_comments_before(range.start, prev_end);
                            if self.has_blank_between(after_comments, range.start) {
                                self.blank_line();
                            }
                            self.line();
                            if *exposed {
                                self.out.push_str("[expose] ");
                            }
                            self.out.push_str(field_name.0);
                            if let Some(field_type) = field_type {
                                self.out.push_str(": ");
                                self.print_type(&field_type.0);
                            }
                            self.out.push(',');
                            self.flush_trailing_comment(range.end);
                            prev_end = range.end;
                        }
                        self.flush_comments_before(fields.1.into_range().end, prev_end);
                        self.indent -= 1;
                        self.line();
                        self.out.push('}');
                    }
                }
            }
            // `enum Name[<…>] { Variant[(payload)][ = discriminant], … }`.
            Node::Enum(name, generics, variants) => {
                self.out.push_str("enum ");
                self.out.push_str(name.0);
                self.print_generic_parameters(generics);
                if variants.0.is_empty() {
                    self.out.push_str(" {}");
                } else {
                    self.out.push_str(" {");
                    self.indent += 1;
                    let mut prev_end = variants.1.into_range().start + 1;
                    for ((variant_name, payload, discriminant), span) in &variants.0 {
                        let range = span.into_range();
                        let after_comments = self.flush_comments_before(range.start, prev_end);
                        if self.has_blank_between(after_comments, range.start) {
                            self.blank_line();
                        }
                        self.line();
                        self.out.push_str(variant_name);
                        if !payload.is_empty() {
                            self.out.push('(');
                            for (index, (payload_type, _)) in payload.iter().enumerate() {
                                if index > 0 {
                                    self.out.push_str(", ");
                                }
                                self.print_type(payload_type);
                            }
                            self.out.push(')');
                        }
                        if let Some(discriminant) = discriminant {
                            self.out.push_str(" = ");
                            self.out.push_str(&discriminant.to_string());
                        }
                        self.out.push(',');
                        self.flush_trailing_comment(range.end);
                        prev_end = range.end;
                    }
                    self.flush_comments_before(variants.1.into_range().end, prev_end);
                    self.indent -= 1;
                    self.line();
                    self.out.push('}');
                }
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
            Node::Func(func) => self.print_func(func),
            // `impl Subject[ with A + B] { items }`.
            Node::Impl(subject, traits, body) => {
                self.out.push_str("impl ");
                self.print_type(&subject.0);
                self.print_with_clause(traits);
                self.out.push(' ');
                self.print_braced_items(body);
            }
            // `trait Name[ with A + B] { items }`.
            Node::Trait(name, generics, supertraits, body) => {
                self.out.push_str("trait ");
                self.out.push_str(name.0);
                self.print_generic_parameters(generics);
                self.print_with_clause(supertraits);
                self.out.push(' ');
                self.print_braced_items(body);
            }
            // `[derive(A, B)]` sits on its own line above the item it annotates.
            Node::Derive(names, derived) => {
                self.out.push_str("[derive(");
                self.out.push_str(&names.join(", "));
                self.out.push_str(")]");
                self.line();
                self.print_item(derived);
            }
            Node::Export(exported) => {
                self.out.push_str("export ");
                self.print_item(exported);
            }
            // `mod name { items }`.
            Node::Module(name, body) => {
                self.out.push_str("mod ");
                self.out.push_str(name);
                self.out.push(' ');
                self.print_braced_items(body);
            }
            // Anything else is an expression appearing as a statement.
            _ => self.print_expr(item),
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
            // `|A, B| Ret` (or `||` for no parameters) — a closure type.
            Node::ClosureType(parameters, return_type) => {
                if parameters.0.is_empty() {
                    self.out.push_str("||");
                } else {
                    self.out.push('|');
                    for (index, (name, parameter_type)) in parameters.0.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        if let Some(name) = name {
                            self.out.push_str(name);
                            self.out.push_str(": ");
                        }
                        self.print_type(&parameter_type.0);
                    }
                    self.out.push('|');
                }
                if let Some(return_type) = return_type {
                    self.out.push(' ');
                    self.print_type(&return_type.0);
                }
            }
            // `type T[: A + B]` — a generic binder inside an impl subject pattern.
            Node::TypeBinder(name, bounds) => {
                self.out.push_str("type ");
                self.out.push_str(name);
                self.print_bounds(bounds);
            }
            // `(A, B)` — a tuple type.
            Node::Tuple(elements) => {
                self.out.push('(');
                for (index, (element, _)) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_type(element);
                }
                self.out.push(')');
            }
            _ => self.bailed = true,
        }
    }

    /// Prints a `: A + B` trait-bound list, or nothing when `bounds` is empty.
    fn print_bounds(&mut self, bounds: &[Spanned<Node<'src>>]) {
        if bounds.is_empty() {
            return;
        }
        self.out.push_str(": ");
        for (index, (bound, _)) in bounds.iter().enumerate() {
            if index > 0 {
                self.out.push_str(" + ");
            }
            self.print_type(bound);
        }
    }

    /// Prints a `with A + B` clause (the traits of an `impl`/`trait`), or nothing
    /// when there are none.
    fn print_with_clause(&mut self, traits: &[Spanned<Node<'src>>]) {
        if traits.is_empty() {
            return;
        }
        self.out.push_str(" with ");
        for (index, (trait_, _)) in traits.iter().enumerate() {
            if index > 0 {
                self.out.push_str(" + ");
            }
            self.print_type(trait_);
        }
    }

    /// Prints the `<T, U: Bound = Default>` parameter list of a generic item, or
    /// nothing when there are none.
    fn print_generic_parameters(&mut self, parameters: &Option<GenericParameters<'src>>) {
        let Some((parameters, _)) = parameters else {
            return;
        };
        self.out.push('<');
        for (index, parameter) in parameters.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            if parameter.is_type {
                self.out.push_str("type ");
            }
            self.out.push_str(parameter.name);
            self.print_bounds(&parameter.bounds);
            if let Some(default) = &parameter.default {
                self.out.push_str(" = ");
                self.print_type(&default.0);
            }
        }
        self.out.push('>');
    }

    /// Prints a `{ items }` block of declarations (an `impl`/`trait`/`mod` body),
    /// each item on its own line, preserving interior comments and blank lines.
    fn print_braced_items(&mut self, body: &Spanned<NodeList<'src>>) {
        let range = body.1.into_range();
        if body.0.is_empty() && !self.has_comment_in(range.start, range.end) {
            self.out.push_str("{}");
            return;
        }
        self.out.push('{');
        self.indent += 1;
        let prev_end = self.print_items(&body.0, range.start + 1);
        self.flush_comments_before(range.end, prev_end);
        self.indent -= 1;
        self.line();
        self.out.push('}');
    }

    /// Prints a function declaration: its `[extern]`/`[must_use]`/`[rpc]`
    /// attributes (if any) each on their own line, then
    /// `[async ][external ]fun name[<…>](…)[: T][ borrows p]` followed by the
    /// body block, or a `;` for a signature with no body.
    fn print_func(&mut self, func: &Func<'src>) {
        if let Some(binding) = &func.extern_binding {
            self.print_extern_attribute(binding);
            self.line();
        }
        if func.must_use {
            self.out.push_str("[must_use]");
            self.line();
        }
        if func.rpc {
            self.out.push_str("[rpc]");
            self.line();
        }
        if func.trait_only {
            self.out.push_str("[trait_only]");
            self.line();
        }
        if func.doc_hidden {
            self.out.push_str("[doc(hidden)]");
            self.line();
        }
        if func.is_async {
            self.out.push_str("async ");
        }
        if func.external {
            self.out.push_str("external ");
        }
        self.out.push_str("fun ");
        self.out.push_str(func.name.0);
        self.print_generic_parameters(&func.generic_parameters);
        self.print_parameters(&func.parameters.0);
        if let Some(return_type) = &func.return_type {
            self.out.push_str(": ");
            self.print_type(&return_type.0);
        }
        if let Some(borrows) = func.borrows {
            self.out.push_str(" borrows ");
            self.out.push_str(borrows);
        }
        match &func.body {
            Some(body) => {
                self.out.push(' ');
                self.print_block(body);
            }
            None => self.out.push(';'),
        }
    }

    /// Prints a `[extern(..)]` host-binding attribute in its canonical form.
    fn print_extern_attribute(&mut self, binding: &ExternBinding<'src>) {
        self.out.push_str("[extern(");
        match binding {
            ExternBinding::Function {
                module: None,
                symbol,
            } => {
                self.out.push('"');
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Function {
                module: Some(module),
                symbol,
            } => {
                self.out.push('"');
                self.out.push_str(module);
                self.out.push_str("\", \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Method { symbol: None } => self.out.push_str("method"),
            ExternBinding::Method {
                symbol: Some(symbol),
            } => {
                self.out.push_str("method, \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Get { symbol } => {
                self.out.push_str("get, \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
            ExternBinding::Set { symbol } => {
                self.out.push_str("set, \"");
                self.out.push_str(symbol);
                self.out.push('"');
            }
        }
        self.out.push_str(")]");
    }

    /// Prints a `(name: T, &mut self, …)` parameter list. The `&`/`&mut`/`own`
    /// convention prefix is reprinted only when it came from a prefix rather than
    /// the parameter's reference type (which already carries it).
    fn print_parameters(&mut self, parameters: &[crate::node::Parameter<'src>]) {
        self.out.push('(');
        self.print_parameters_inner(parameters);
        self.out.push(')');
    }

    /// Prints the comma-separated parameters themselves, without the surrounding
    /// delimiters (shared by function `(…)` and closure `|…|` lists).
    fn print_parameters_inner(&mut self, parameters: &[crate::node::Parameter<'src>]) {
        for (index, (binder, parameter_type, convention, _)) in parameters.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            let type_is_reference = matches!(
                parameter_type.as_deref().map(|spanned| &spanned.0),
                Some(Node::Reference(..))
            );
            match convention {
                Convention::Own => self.out.push_str("own "),
                Convention::Ref if !type_is_reference => self.out.push('&'),
                Convention::RefMut if !type_is_reference => self.out.push_str("&mut "),
                _ => {}
            }
            self.print_binder(binder);
            if let Some(parameter_type) = parameter_type {
                self.out.push_str(": ");
                self.print_type(&parameter_type.0);
            }
        }
    }

    /// Prints a braced statement block `{ … }` — the body of a function, loop,
    /// `if`, or a block expression. An empty block (no statements, no tail, no
    /// interior comment) stays inline as `{}`.
    fn print_block(&mut self, block: &Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>) {
        let range = block.1.into_range();
        let (statements, tail) = &block.0;
        let empty = statements.is_empty() && matches!(tail.0, Node::Void);
        if empty && !self.has_comment_in(range.start, range.end) {
            self.out.push_str("{}");
            return;
        }
        self.out.push('{');
        self.indent += 1;
        let mut prev_end = self.print_items(statements, range.start + 1);
        if !matches!(tail.0, Node::Void) {
            let tail_range = tail.1.into_range();
            let after_comments = self.flush_comments_before(tail_range.start, prev_end);
            if self.has_blank_between(after_comments, tail_range.start) {
                self.blank_line();
            }
            self.line();
            self.print_expr(tail);
            self.flush_trailing_comment(tail_range.end);
            prev_end = tail_range.end;
        }
        self.flush_comments_before(range.end, prev_end);
        self.indent -= 1;
        self.line();
        self.out.push('}');
    }

    /// Whether the next non-trivia character at or after `from` in the source is a
    /// comma — used to preserve a match arm's optional separator comma (which the
    /// AST drops) so either corpus style round-trips.
    fn source_has_comma_at(&self, from: usize) -> bool {
        let bytes = self.source.as_bytes();
        let mut index = from;
        while index < bytes.len() {
            match bytes[index] {
                b' ' | b'\t' | b'\n' | b'\r' => index += 1,
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                b',' => return true,
                _ => return false,
            }
        }
        false
    }

    /// Whether any extracted comment falls within `[from, to)` — used to decide
    /// whether an otherwise-empty block must expand to carry its comments.
    fn has_comment_in(&self, from: usize, to: usize) -> bool {
        self.comments.iter().any(|(span, _)| {
            let range = span.into_range();
            range.start >= from && range.start < to
        })
    }

    /// The binding precedence of a binary operator (higher binds tighter), used to
    /// decide where operands need parentheses.
    fn binary_precedence(operator: BinaryOp) -> u8 {
        match operator {
            BinaryOp::Or => 0,
            BinaryOp::And => 1,
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Lt
            | BinaryOp::Gt
            | BinaryOp::LtEq
            | BinaryOp::GtEq => 3,
            BinaryOp::Add | BinaryOp::Sub => 4,
            BinaryOp::Mul | BinaryOp::Div => 5,
        }
    }

    /// The precedence of an expression as an operand — `100` for atoms/postfix
    /// (never need wrapping), `0` for statement-like forms (always wrapped as an
    /// operand). Mirrors the parser's expression layering.
    fn expression_precedence(node: &Node<'src>) -> u8 {
        match node {
            Node::Binary(operator, _, _) => Self::binary_precedence(*operator),
            Node::Is(_, _) => 2,
            Node::Unary(_, _)
            | Node::Reference(_, _)
            | Node::Dereference(_)
            | Node::Await(_)
            | Node::Async(_) => 6,
            Node::Assign(_, _, _)
            | Node::Let(_, _, _, _)
            | Node::Closure(_)
            | Node::If(_)
            | Node::For(_, _)
            | Node::ForIn(_, _, _)
            | Node::Match(_, _)
            | Node::Jump(_)
            | Node::FuncReturn(_) => 0,
            _ => 100,
        }
    }

    /// Prints `expr` as an operand, wrapping it in parentheses when its precedence
    /// is below `minimum` (so the reprint reparses to the same tree). An
    /// interpolated string is reprinted verbatim and never wrapped — it already
    /// carries its own parentheses in the expanded token stream.
    fn print_operand(&mut self, expr: &Spanned<Node<'src>>, minimum: u8) {
        if self.interpolated_source(expr).is_some() {
            self.print_expr(expr);
        } else if Self::expression_precedence(&expr.0) < minimum {
            self.out.push('(');
            self.print_expr(expr);
            self.out.push(')');
        } else {
            self.print_expr(expr);
        }
    }

    /// Prints a comma-separated expression list (call arguments, list/tuple
    /// elements) inline.
    fn print_expression_list(&mut self, elements: &[Spanned<Node<'src>>]) {
        for (index, element) in elements.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            self.print_expr(element);
        }
    }

    /// If `expr`'s source span is an `i"..."` interpolated string — which the
    /// lexer rewrites into a parenthesized `("" + parts..)` concatenation before
    /// parsing, with every produced token sharing the literal's span — return the
    /// literal's original source text. Reprinting that verbatim is exact;
    /// rebuilding it from the expanded AST would have to re-derive the lexer's
    /// brace/quote escaping.
    fn interpolated_source(&self, expr: &Spanned<Node<'src>>) -> Option<&'src str> {
        let range = expr.1.into_range();
        if self.source.get(range.start..range.start + 2) != Some("i\"") {
            return None;
        }
        // The lexer reports an i-string's span ending *at* its closing quote
        // rather than after it, so the recovered slice would drop the quote (and
        // swallow the rest of the file into one string). Include it when present.
        let end = if self.source.as_bytes().get(range.end) == Some(&b'"') {
            range.end + 1
        } else {
            range.end
        };
        self.source.get(range.start..end)
    }

    /// Prints any expression. Sets `bailed` for forms not yet handled.
    fn print_expr(&mut self, expr: &Spanned<Node<'src>>) {
        if let Some(interpolated) = self.interpolated_source(expr) {
            self.out.push_str(interpolated);
            return;
        }
        match &expr.0 {
            Node::Number(whole, fraction, suffix) => {
                self.out.push_str(whole);
                if let Some(fraction) = fraction {
                    self.out.push('.');
                    self.out.push_str(fraction);
                }
                if let Some(suffix) = suffix {
                    self.out.push_str(suffix);
                }
            }
            Node::String(text) => {
                self.out.push('"');
                self.out.push_str(text);
                self.out.push('"');
            }
            Node::Bool(value) => self.out.push_str(if *value { "true" } else { "false" }),
            Node::Null => self.out.push_str("null"),
            Node::Void => {}
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
            Node::MemberAccessor(subject, member) => {
                self.print_operand(subject, 100);
                self.out.push('.');
                self.print_expr(member);
            }
            Node::StaticAccessor(subject, member) => {
                self.print_operand(subject, 100);
                self.out.push_str("::");
                self.out.push_str(member);
            }
            Node::Index(subject, index) => {
                self.print_operand(subject, 100);
                self.out.push('[');
                self.print_expr(index);
                self.out.push(']');
            }
            Node::Call(callee, generic_arguments, arguments) => {
                // A call binds tighter than `.`/`[]`, so `a.b(c)` parses as
                // `a.(b(c))`. To call the *result* of a member/index access the
                // callee must be parenthesized — `(a.b)(c)` — or it reparses wrong.
                if matches!(callee.0, Node::MemberAccessor(_, _) | Node::Index(_, _)) {
                    self.out.push('(');
                    self.print_expr(callee);
                    self.out.push(')');
                } else {
                    self.print_operand(callee, 100);
                }
                if let Some((generic_arguments, _)) = generic_arguments {
                    self.out.push('<');
                    for (index, (argument, _)) in generic_arguments.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.print_type(argument);
                    }
                    self.out.push('>');
                }
                self.out.push('(');
                self.print_expression_list(&arguments.0);
                self.out.push(')');
            }
            Node::Binary(operator, left, right) => {
                let precedence = Self::binary_precedence(*operator);
                self.print_operand(left, precedence);
                self.out.push(' ');
                self.out.push_str(binary_operator_symbol(*operator));
                self.out.push(' ');
                self.print_operand(right, precedence + 1);
            }
            Node::Unary(operator, operand) => {
                self.out.push(*operator);
                self.print_operand(operand, 6);
            }
            Node::Reference(mutable, operand) => {
                self.out.push('&');
                if *mutable {
                    self.out.push_str("mut ");
                }
                self.print_operand(operand, 6);
            }
            Node::Dereference(operand) => {
                self.out.push('*');
                self.print_operand(operand, 6);
            }
            Node::Await(operand) => {
                self.out.push_str("await ");
                self.print_operand(operand, 6);
            }
            Node::Async(operand) => {
                self.out.push_str("async ");
                self.print_expr(operand);
            }
            Node::Let(name, declared_type, value, mutable) => {
                self.out.push_str(if *mutable { "mut " } else { "let " });
                self.out.push_str(name.0);
                if let Some(declared_type) = declared_type {
                    self.out.push_str(": ");
                    self.print_type(&declared_type.0);
                }
                if let Some(value) = value {
                    self.out.push_str(" = ");
                    self.print_expr(value);
                }
            }
            Node::Assign(target, operator, value) => {
                self.print_expr(target);
                self.out.push(' ');
                if let Some(operator) = operator {
                    self.out.push_str(binary_operator_symbol(*operator));
                }
                self.out.push_str("= ");
                self.print_expr(value);
            }
            Node::If(branch) => self.print_if_branch(branch),
            Node::Match(subject, legs) => {
                self.out.push_str("match ");
                self.print_expr(subject);
                self.out.push_str(" {");
                self.indent += 1;
                let mut prev_end = legs.1.into_range().start + 1;
                for leg in &legs.0 {
                    let (patterns, _, body) = leg;
                    let start = patterns
                        .first()
                        .map(|(_, span)| span.into_range().start)
                        .unwrap_or(prev_end);
                    let after_comments = self.flush_comments_before(start, prev_end);
                    if self.has_blank_between(after_comments, start) {
                        self.blank_line();
                    }
                    self.line();
                    self.print_match_leg(leg);
                    // The arm separator comma is optional and not kept in the AST
                    // (the corpus mixes `=> { .. },` and `=> { .. }`), so preserve
                    // whatever the source had to round-trip either style faithfully.
                    let body_end = body.1.into_range().end;
                    if self.source_has_comma_at(body_end) {
                        self.out.push(',');
                    }
                    self.flush_trailing_comment(body_end);
                    prev_end = body_end;
                }
                self.flush_comments_before(legs.1.into_range().end, prev_end);
                self.indent -= 1;
                self.line();
                self.out.push('}');
            }
            Node::For(condition, body) => {
                self.out.push_str("for");
                if let Some(condition) = condition {
                    self.out.push(' ');
                    self.print_expr(condition);
                }
                self.out.push(' ');
                self.print_block(body);
            }
            Node::ForIn(variable, iterable, body) => {
                self.out.push_str("for ");
                self.out.push_str(variable);
                self.out.push_str(" in ");
                self.print_expr(iterable);
                self.out.push(' ');
                self.print_block(body);
            }
            Node::FuncReturn(value) => {
                self.out.push_str("ret ");
                self.print_expr(value);
            }
            Node::Jump(target) => {
                self.out.push_str("jump ");
                self.out.push_str(target);
            }
            Node::Block(block) => self.print_block(block),
            Node::StructInitializer(name, generic_arguments, fields) => {
                self.out.push_str(name);
                if let Some((generic_arguments, _)) = generic_arguments {
                    self.out.push('<');
                    for (index, (argument, _)) in generic_arguments.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.print_type(argument);
                    }
                    self.out.push('>');
                }
                if fields.0.is_empty() {
                    self.out.push_str(" {}");
                } else {
                    self.out.push_str(" { ");
                    for (index, ((field_name, value), _)) in fields.0.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.out.push_str(field_name);
                        if let Some(value) = value {
                            self.out.push_str(" = ");
                            self.print_expr(value);
                        }
                    }
                    self.out.push_str(" }");
                }
            }
            Node::List(elements) => {
                self.out.push('[');
                self.print_expression_list(elements);
                self.out.push(']');
            }
            Node::Tuple(elements) => {
                self.out.push('(');
                self.print_expression_list(elements);
                self.out.push(')');
            }
            Node::Closure(closure) => {
                self.print_closure_parameters(&closure.parameters.0);
                if let Some(return_type) = &closure.return_type {
                    self.out.push_str(": ");
                    self.print_type(&return_type.0);
                }
                self.out.push(' ');
                self.print_expr(&closure.return_value);
            }
            Node::Is(subject, pattern) => {
                self.print_operand(subject, 3);
                self.out.push_str(" is ");
                self.print_pattern(&pattern.0);
            }
            _ => self.bailed = true,
        }
    }

    /// Prints the closure parameter list `|a, b|` (or `||`) for a closure value.
    /// Closures share the function parameter syntax, but with `|` delimiters and a
    /// single-token `||` for the empty list.
    fn print_closure_parameters(&mut self, parameters: &[crate::node::Parameter<'src>]) {
        if parameters.is_empty() {
            self.out.push_str("||");
            return;
        }
        self.out.push('|');
        self.print_parameters_inner(parameters);
        self.out.push('|');
    }

    /// Prints an `if`/`else if`/`else` chain.
    fn print_if_branch(&mut self, branch: &NodeIfBranch<'src>) {
        match branch {
            NodeIfBranch::If(if_) => {
                self.out.push_str("if ");
                self.print_expr(&if_.condition);
                self.out.push(' ');
                self.print_block(&if_.then);
                if let Some((else_branch, _)) = &if_.else_ {
                    self.out.push_str(" else ");
                    match else_branch {
                        NodeIfBranch::If(_) => self.print_if_branch(else_branch),
                        NodeIfBranch::Else(block) => self.print_block(block),
                    }
                }
            }
            NodeIfBranch::Else(block) => self.print_block(block),
        }
    }

    /// Prints one `match` leg: `pattern[, pattern][ if guard] => body`.
    fn print_match_leg(&mut self, leg: &crate::node::MatchLeg<'src>) {
        let (patterns, guard, body) = leg;
        for (index, (pattern, _)) in patterns.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            self.print_pattern(pattern);
        }
        if let Some(guard) = guard {
            self.out.push_str(" if ");
            self.print_expr(guard);
        }
        self.out.push_str(" => ");
        self.print_expr(body);
    }

    /// Prints a match pattern.
    /// Prints a binder in `let`/parameter position: a bare name (no `let `
    /// keyword) or a tuple of binders (`(a, b)`). Distinct from a match pattern.
    fn print_binder(&mut self, binder: &Pattern<'src>) {
        match binder {
            Pattern::Binding(name, _) => self.out.push_str(name),
            Pattern::Tuple(elements) => {
                self.out.push('(');
                for (index, (element, _)) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_binder(element);
                }
                self.out.push(')');
            }
            // A binder is only ever a name or a tuple of names; other shapes
            // can't reach here from the parser.
            other => self.print_pattern(other),
        }
    }

    fn print_pattern(&mut self, pattern: &Pattern<'src>) {
        match pattern {
            Pattern::Wildcard => self.out.push('_'),
            Pattern::Binding(name, mutable) => {
                self.out.push_str(if *mutable { "mut " } else { "let " });
                self.out.push_str(name);
            }
            Pattern::Variant(path, payload) => {
                for (index, segment) in path.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str("::");
                    }
                    self.out.push_str(segment);
                }
                if let Some(payload) = payload {
                    self.out.push('(');
                    for (index, (sub_pattern, _)) in payload.iter().enumerate() {
                        if index > 0 {
                            self.out.push_str(", ");
                        }
                        self.print_pattern(sub_pattern);
                    }
                    self.out.push(')');
                }
            }
            Pattern::Tuple(elements) => {
                self.out.push('(');
                for (index, (element, _)) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.print_pattern(element);
                }
                self.out.push(')');
            }
            Pattern::Literal(literal) => self.print_expr(literal),
        }
    }
}

/// The source spelling of a binary operator.
fn binary_operator_symbol(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Gt => ">",
        BinaryOp::LtEq => "<=",
        BinaryOp::GtEq => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
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

    #[test]
    fn generic_enum_with_payloads() {
        assert_formats(
            "enum Option<T>{Some(T),None}\n",
            "enum Option<T> {\n\tSome(T),\n\tNone,\n}\n",
        );
    }

    #[test]
    fn function_signature_and_body() {
        assert_formats(
            "fun add(a:i32,b:i32):i32{a+b}\n",
            "fun add(a: i32, b: i32): i32 {\n\ta + b\n}\n",
        );
    }

    #[test]
    fn statements_take_semicolons_tail_does_not() {
        assert_formats(
            "fun f(){let x=1;print(x);x}\n",
            "fun f() {\n\tlet x = 1;\n\tprint(x);\n\tx\n}\n",
        );
    }

    #[test]
    fn precedence_parentheses_are_minimal() {
        assert_formats("fun f(){(a+b)*c}\n", "fun f() {\n\t(a + b) * c\n}\n");
        assert_formats("fun f(){a+b*c}\n", "fun f() {\n\ta + b * c\n}\n");
    }

    #[test]
    fn call_through_a_member_keeps_its_parentheses() {
        // `(self.fn)()` calls a field-closure; `self.fn()` is a method call.
        assert_formats(
            "fun f(self){(self.fn)()}\n",
            "fun f(self) {\n\t(self.fn)()\n}\n",
        );
    }

    #[test]
    fn trailing_comment_stays_on_its_line() {
        assert_formats(
            "fun f(){print(1);    // note\n}\n",
            "fun f() {\n\tprint(1); // note\n}\n",
        );
    }

    #[test]
    fn interpolated_string_is_reprinted_verbatim() {
        // The lexer expands `i"..."` to `("" + ..)` before parsing; the printer
        // recovers the original literal from the source rather than the AST.
        assert_formats(
            "fun f(self){print(i\"hi {self.name}!\")}\n",
            "fun f(self) {\n\tprint(i\"hi {self.name}!\")\n}\n",
        );
    }

    #[test]
    fn interpolated_string_with_escaped_braces() {
        assert_formats(
            "fun f(){let x=i\"a \\{b\\} c\";x}\n",
            "fun f() {\n\tlet x = i\"a \\{b\\} c\";\n\tx\n}\n",
        );
    }

    #[test]
    fn impl_with_match_and_closure() {
        // `fn: |T| U` keeps the space after `:` — `:|` would lex as one operator.
        // The last arm has no source comma, so the faithful output keeps none.
        assert_formats(
            "impl Option<type T> { fun map<U>(self, fn: |T| U): Option<U> { match self { Some(let x)=>Some(fn(x)), None=>None } } }\n",
            "impl Option<type T> {\n\tfun map<U>(self, fn: |T| U): Option<U> {\n\t\tmatch self {\n\t\t\tSome(let x) => Some(fn(x)),\n\t\t\tNone => None\n\t\t}\n\t}\n}\n",
        );
    }
}

#[cfg(test)]
mod idempotency {
    use super::format;

    /// The real invariant: formatting is a fixed point. `format(x)` may tidy `x`,
    /// but formatting the result again must change nothing.
    fn assert_fixed_point(name: &str, source: &str) {
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice, "formatting {name} is not a fixed point");
    }

    macro_rules! fixed_point_tests {
        ($($name:ident => $path:literal),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    assert_fixed_point($path, include_str!(concat!("../../../vilan/std/src/", $path)));
                }
            )*
        };
    }

    // A spread of std modules exercising functions, impls, traits, generics,
    // enums with payloads, matches, closures, and `[extern]` bindings —
    // `reactive.vl` also exercises `[must_use]` (which the formatter once
    // dropped, tripping its safety check into a silent no-op).
    fixed_point_tests! {
        null_vl => "null.vl",
        boolean_vl => "boolean.vl",
        option_vl => "option.vl",
        result_vl => "result.vl",
        list_vl => "list.vl",
        string_vl => "string.vl",
        set_vl => "set.vl",
        iterator_vl => "iterator.vl",
        arena_vl => "arena.vl",
        shared_vl => "shared.vl",
        display_vl => "display.vl",
        reactive_vl => "reactive.vl",
    }

    /// Attributes must be *retained*, not just idempotent — a formatter that
    /// deterministically deleted an attribute would still be a fixed point, so
    /// the retention is asserted directly. (Dropping one used to trip the
    /// safety check, silently leaving the whole file unformatted.)
    #[test]
    fn attributes_round_trip() {
        let source = "trait Source {\n\t[must_use]\n\tfun sub(self): i32;\n\
                      \t[trait_only]\n\tfun tag(self): str;\n\
                      \t[doc(hidden)]\n\tfun internal(self): i32;\n}\n\
                      struct Sess {\n\t[expose] status: Signal<str>,\n\thidden: i32,\n}\n\
                      impl Sess {\n\t[rpc]\n\tfun login(self, name: str): bool {\n\t\ttrue\n\t}\n}\n";
        let formatted = format(source);
        assert!(
            formatted.contains("[must_use]"),
            "must_use attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[expose] status"),
            "expose attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[rpc]"),
            "rpc attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[trait_only]"),
            "trait_only attribute lost:\n{formatted}"
        );
        assert!(
            formatted.contains("[doc(hidden)]"),
            "doc(hidden) attribute lost:\n{formatted}"
        );
        assert_fixed_point("attributes", source);
    }
}
