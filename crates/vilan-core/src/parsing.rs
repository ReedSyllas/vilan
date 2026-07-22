//! The handwritten parser (H6 S3, `proposal/frontend.md` §2 "Parser" + §3 S3).
//!
//! A dependency-free, single-pass recursive-descent + precedence-climbing parser
//! over `&[(Token, Span)]` (the output of [`crate::lexing::tokenize`], which S1
//! proved byte-identical to the chumsky lexer). It produces the same
//! `Spanned<Node>` tree — spans included — that the chumsky grammar in `parser.rs`
//! produces, which stays in-tree as the oracle for the whole H6 arc (deleted at
//! S5). Nothing in the pipeline calls this yet; it is exercised by the corpus-
//! scale differential (`tests/parse_differential.rs`, which S3 repoints at this
//! module), the corpus-through-the-new-frontend byte gate
//! (`tests/corpus_new_frontend.rs`), and this module's own pins.
//!
//! With S3 the grammar is COMPLETE — the whole file, not just its expressions.
//! S2 covered the *expression* and *type* grammar plus the block-bearing forms
//! (`closure`/`if`/`for`/`match`/block/`let`/assignment/`ret`/`jump`). S3 fills the
//! seam: top-level *items* (`fun`/`struct`/`enum`/`impl`/`trait`/`mod`/`import`/
//! `use`/`export`), the bracket-attribute grammar (`[derive(..)]`/`[service(..)]`/
//! `[extern(..)]`/`[platform(..)]`/`[must_use]`/`[rpc]`/`[trait_only]`/`[doc(..)]`/
//! `[expose]` and user macro attributes), the *macro forms* (`macro fun`/`macro
//! { }`/`macro name(..)`), and the deferred `tuple_comprehension` atom. The
//! statement/item interleaving reproduces the chumsky `statement` choice ORDER
//! exactly — the attribute/macro/export forms lead, then `expression ;` (and the
//! block-bearing statement forms), then the declaration items — because that
//! order decides which reading of an ambiguous `[`- or `async`-led head wins.
//!
//! Faithfulness over improvement: every shape here reproduces the chumsky grammar,
//! quirks included (the split-shift reassembly, the H.1 struct-literal-free
//! condition mode as a boolean rather than a parallel grammar, the collect-then-
//! group `?.` continuation, the arrow-less closure *type*, the paren-dissolving
//! atom that keeps the inner expression's own span, `apply_binding_mutability`
//! leaving array binders untouched, the FIXED attribute order on a function, the
//! optional `;` on a `macro`-statement vs the mandatory one on an expression, the
//! `resource`-misplaced steer). Ugly-but-reproduced behaviours are recorded for
//! the S4/S5 error-quality pass, not fixed. The differential is the referee.

use crate::lexing;
use crate::node::{
    BinaryOp, Closure, Convention, EnumVariant, ExternBinding, Func, GenericArguments,
    GenericParameter, GenericParameters, If, ImportBranch, MatchLeg, Node, NodeIfBranch, NodeList,
    Parameter, Pattern, StructField, TupleBound,
};
use crate::span::{Span, Spanned};
use crate::token::Token;

/// A parse error value: where it was detected and a description of what the parser
/// wanted. S2 accumulates these but does not yet reproduce chumsky's structured
/// expected/found sets or their rendering — that is S4's concern (proposal §6a
/// allows improved, not byte-identical, diagnostics at cutover). The expression
/// differential compares only *clean* parses, so this content is not yet gated;
/// its shape is deliberately minimal for S4 to build the real diagnostics on.
#[derive(Clone, Debug)]
pub struct ParseError {
    /// Where the error was detected (a token span, or a zero-width span at end of
    /// input).
    pub span: Span,
    /// A human description of what was expected at [`ParseError::span`].
    pub expected: &'static str,
}

/// Parse `source` into its statement list (with spans) and any parse errors. The
/// tree is the same `Spanned<NodeList>` the chumsky parser produces for every
/// source the H6 differential covers within S2's implemented subset; on any lex
/// error, a source that reaches an unimplemented item/macro form, or any other
/// parse failure, the error list is non-empty and the tree is `None` — the
/// clean-or-decline contract the differential's candidate relies on.
///
/// The returned tree borrows `source` (identifiers, string bodies, and numeric
/// slices are `&'src str` copied out of the tokens), exactly like the chumsky
/// parser; the intermediate token vector does not outlive this call.
pub fn parse(source: &str) -> (Option<Spanned<NodeList<'_>>>, Vec<ParseError>) {
    let (tokens, lex_errors) = lexing::tokenize(source);
    if !lex_errors.is_empty() {
        let errors = lex_errors
            .iter()
            .map(|error| ParseError {
                span: (error.position..error.position + error.character.len_utf8()).into(),
                expected: "a valid token",
            })
            .collect();
        return (None, errors);
    }

    let mut parser = Parser::new(&tokens, source.len());
    let root = parser.parse_program();
    if parser.position != tokens.len() {
        // Statements were consumed greedily but tokens remain — an unparseable
        // statement (an unimplemented item, or a genuine error). The differential
        // candidate declines on a non-empty error list.
        let span = parser.here_span();
        parser.errors.push(ParseError {
            span,
            expected: "an item or end of input",
        });
    }

    if parser.errors.is_empty() {
        (Some(root), Vec::new())
    } else {
        (None, parser.errors)
    }
}

struct Parser<'a, 'src> {
    tokens: &'a [Spanned<Token<'src>>],
    position: usize,
    /// The end-of-input offset (`source.len()`), the span the chumsky parser reports
    /// at EOI — `.map((end..end).into(), …)` in every call site.
    eoi: usize,
    errors: Vec<ParseError>,
}

// --- A postfix suffix in the member/call chain, collected then grouped ----------

/// One postfix operator in the member chain (`proposal/try-and-lift.md` §3): a
/// faithful copy of `chain_expr_parser`'s local `Postfix` enum. Collected in source
/// order, then grouped so the segment from one `?.` to the next `?.`/`!`/chain end
/// forms that lifted link's continuation.
enum Postfix<'src> {
    Member(Spanned<Node<'src>>),
    Index(Spanned<Node<'src>>),
    TryAssert,
    LiftMember(Spanned<Node<'src>>),
    /// A bare `?` (one NOT followed by `.`): an expression-lifting mark.
    LiftBare,
    /// `subject(args)` where the subject is itself a postfix result.
    DirectCall(Spanned<NodeList<'src>>),
}

/// Stamps a binder pattern's bindings mutable (or not). A faithful copy of
/// `parser.rs::apply_binding_mutability` — note that a `Pattern::Array` binder is
/// left UNTOUCHED (`other => other`), so `mut [a, b]` keeps `a`/`b` immutable: a
/// reproduced quirk, recorded for S4, not fixed here. Used only by the match/`is`
/// pattern grammar (the `let`/`mut` binding arm); `let`/parameter binders carry
/// mutability in a separate field instead.
fn apply_binding_mutability(pattern: Pattern<'_>, mutable: bool) -> Pattern<'_> {
    match pattern {
        Pattern::Binding(name, _) => Pattern::Binding(name, mutable),
        Pattern::Tuple(patterns) => Pattern::Tuple(
            patterns
                .into_iter()
                .map(|(pattern, span)| (apply_binding_mutability(pattern, mutable), span))
                .collect(),
        ),
        other => other,
    }
}

/// One argument inside a `[extern(..)]` attribute — a bare word (`method`/`get`/
/// `set`/`new`) or a quoted string (a module path or host symbol). A faithful copy
/// of `parser.rs::ExternArg`.
enum ExternArg<'src> {
    Word(&'src str),
    Text(&'src str),
}

/// Interprets a `[extern(..)]` attribute's arguments into a host binding. A
/// faithful copy of `parser.rs::extern_binding_from_args` — a malformed attribute
/// (author error) lowers to an empty global symbol, exactly as the oracle does.
fn extern_binding_from_args<'src>(args: &[ExternArg<'src>]) -> ExternBinding<'src> {
    use ExternArg::{Text, Word};
    match args {
        [Text(symbol)] => ExternBinding::Function {
            module: None,
            symbol,
        },
        [Text(module), Text(symbol)] => ExternBinding::Function {
            module: Some(module),
            symbol,
        },
        [Word("method")] => ExternBinding::Method { symbol: None },
        [Word("method"), Text(symbol)] => ExternBinding::Method {
            symbol: Some(symbol),
        },
        [Word("get"), Text(symbol)] => ExternBinding::Get { symbol },
        [Word("set"), Text(symbol)] => ExternBinding::Set { symbol },
        [Word("new"), Text(symbol)] => ExternBinding::New {
            module: None,
            symbol,
        },
        [Word("new"), Text(module), Text(symbol)] => ExternBinding::New {
            module: Some(module),
            symbol,
        },
        _ => ExternBinding::Function {
            module: None,
            symbol: "",
        },
    }
}

/// The built-in attribute-marker names, excluded from a *user* macro attribute's
/// name (they keep their own parsers, fused into `function`/`struct` or earlier in
/// the statement choice). Mirrors the chumsky `macro_attribute_name` guard.
fn is_known_attribute_marker(name: &str) -> bool {
    matches!(
        name,
        "derive"
            | "service"
            | "extern"
            | "must_use"
            | "rpc"
            | "trait_only"
            | "doc"
            | "expose"
            | "platform"
    )
}

impl<'a, 'src> Parser<'a, 'src> {
    fn new(tokens: &'a [Spanned<Token<'src>>], eoi: usize) -> Self {
        Parser {
            tokens,
            position: 0,
            eoi,
            errors: Vec::new(),
        }
    }

    // --- Cursor primitives ---------------------------------------------------

    fn peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.position).map(|(token, _)| token)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token<'src>> {
        self.tokens
            .get(self.position + offset)
            .map(|(token, _)| token)
    }

    fn at_end(&self) -> bool {
        self.position >= self.tokens.len()
    }

    fn bump(&mut self) {
        self.position += 1;
    }

    fn peek_is_ctrl(&self, character: char) -> bool {
        matches!(self.peek(), Some(Token::Ctrl(found)) if *found == character)
    }

    fn peek_at_is_ctrl(&self, offset: usize, character: char) -> bool {
        matches!(self.peek_at(offset), Some(Token::Ctrl(found)) if *found == character)
    }

    fn peek_is_op(&self, symbol: &str) -> bool {
        matches!(self.peek(), Some(Token::Op(found)) if *found == symbol)
    }

    fn peek_is(&self, token: &Token<'src>) -> bool {
        self.peek() == Some(token)
    }

    /// The `&str` of the current `Op` token, if any (for the arithmetic/comparison
    /// operator tables).
    fn peek_op(&self) -> Option<&'src str> {
        if let Some(Token::Op(symbol)) = self.peek() {
            Some(symbol)
        } else {
            None
        }
    }

    fn eat_ctrl(&mut self, character: char) -> bool {
        if self.peek_is_ctrl(character) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_op(&mut self, symbol: &str) -> bool {
        if self.peek_is_op(symbol) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat(&mut self, token: &Token<'src>) -> bool {
        if self.peek_is(token) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_ctrl(&mut self, character: char) -> Option<()> {
        self.eat_ctrl(character).then_some(())
    }

    fn expect_op(&mut self, symbol: &str) -> Option<()> {
        self.eat_op(symbol).then_some(())
    }

    fn expect(&mut self, token: &Token<'src>) -> Option<()> {
        self.eat(token).then_some(())
    }

    fn eat_ident(&mut self) -> Option<&'src str> {
        if let Some(Token::Ident(name)) = self.peek() {
            let name = *name;
            self.bump();
            Some(name)
        } else {
            None
        }
    }

    /// A name in path / variant position: an identifier, or the boolean-literal
    /// keywords (`true`/`false`) so the bootstrap `bool` enum and its variants can
    /// be written. Mirrors the chumsky `name` production.
    fn eat_name(&mut self) -> Option<&'src str> {
        match self.peek() {
            Some(Token::Ident(name)) => {
                let name = *name;
                self.bump();
                Some(name)
            }
            Some(Token::Bool(true)) => {
                self.bump();
                Some("true")
            }
            Some(Token::Bool(false)) => {
                self.bump();
                Some("false")
            }
            _ => None,
        }
    }

    // --- Span assembly -------------------------------------------------------

    /// The span of a parse that began at token index `start` and ends at the
    /// current position — reproducing chumsky's `MappedInput::span` (input.rs):
    /// `first_token.start .. last_consumed_token.end` for a non-empty parse, and
    /// the EOI span (`eoi..eoi`) when there is no token at `start`. The empty-parse
    /// case (position == start with a token present) reproduces chumsky's
    /// `first_token.start .. previous_token.end`, which never surfaces in a produced
    /// node here but is kept faithful.
    fn span_from(&self, start: usize) -> Span {
        if start >= self.tokens.len() {
            return (self.eoi..self.eoi).into();
        }
        let start_offset = self.tokens[start].1.start;
        let end_offset = if self.position > start {
            self.tokens[self.position - 1].1.end
        } else if start > 0 {
            self.tokens[start - 1].1.end
        } else {
            self.eoi
        };
        (start_offset..end_offset).into()
    }

    /// The span of the current token (a single-token span), or a zero-width span at
    /// end of input.
    fn here_span(&self) -> Span {
        if let Some((_, span)) = self.tokens.get(self.position) {
            *span
        } else {
            (self.eoi..self.eoi).into()
        }
    }

    /// Run `body`; if it declines (`None`), restore the cursor to where it started.
    /// The recursive-descent equivalent of chumsky's backtracking on a failed
    /// alternative.
    fn attempt<T>(&mut self, body: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let start = self.position;
        let result = body(self);
        if result.is_none() {
            self.position = start;
        }
        result
    }

    /// `item (',' item)* ','?` up to (but not consuming) the closer `is_close`
    /// reports — the `allow_trailing` comma-list shared by argument lists, generic
    /// arguments, list literals, tuple types, and closure parameters. An empty list
    /// (the closer immediately) is allowed; the caller enforces any minimum.
    fn comma_list<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> Option<T>,
        is_close: impl Fn(&Self) -> bool,
    ) -> Option<Vec<T>> {
        let mut items = Vec::new();
        if is_close(self) {
            return Some(items);
        }
        loop {
            items.push(item(self)?);
            if self.eat_ctrl(',') {
                if is_close(self) {
                    break;
                }
                continue;
            }
            break;
        }
        Some(items)
    }

    // --- Program / statements ------------------------------------------------

    /// The whole source: a sequence of statements. Stops at the first token that
    /// begins no S2 statement (an unimplemented item, or an error); [`parse`] then
    /// sees leftover tokens and declines.
    fn parse_program(&mut self) -> Spanned<NodeList<'src>> {
        let mut statements = Vec::new();
        loop {
            if self.at_end() {
                break;
            }
            match self.parse_statement() {
                Some(statement) => statements.push(statement),
                None => break,
            }
        }
        (statements, self.span_from(0))
    }

    /// One statement, reproducing the chumsky `statement` choice in ORDER — the
    /// ordering is load-bearing (it decides which reading of an ambiguous `[`- or
    /// `async`-led head wins). Each alternative backtracks cleanly on a mismatch,
    /// so the first that matches wins, exactly as chumsky's ordered `choice`.
    /// Returns `None` (restoring) when no statement starts here, so the caller's
    /// loop can stop and a trailing block value can be taken instead.
    ///
    /// The chumsky order (parser.rs `statement.define`):
    /// 1. `[derive(..)] struct|enum`, 2. `[service(..)] struct`,
    /// 3. `[<user>(..)] struct|enum|fun`, 4. `macro fun`, 5. `macro { } ;?`,
    /// 6. `macro name(..) ;?`, 7. `export <stmt>`, 8. `expression ;`,
    /// 9-11. `if`/`for`/`match` without `;` (not-block-end), 12. `fun`,
    /// 13. `struct`, 14. `enum`, 15. misplaced-`resource` steer, 16. `impl`,
    /// 17. `trait`, 18. `mod`, 19. `import ;`, 20. `use ;`, 21. `{ } block`
    /// without `;` (not-block-end). Items 8-11 and 21 are fused into one
    /// expression attempt (an expression carries the block-bearing forms and the
    /// bare block already), exactly as S2 did.
    fn parse_statement(&mut self) -> Option<Spanned<Node<'src>>> {
        if let Some(item) = self.attempt(Self::parse_derived_item) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_service_item) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_macro_attributed_item) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_macro_fun) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_macro_block_statement) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_macro_invocation_statement) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_export) {
            return Some(item);
        }
        // Items 8-11 & 21: `expression ;`, or a block-bearing form
        // (`if`/`for`/`match`/`{ }`) used as a statement — which needs no `;` but
        // must not be the last thing in its block (chumsky's `not_block_end`).
        if let Some(statement) = self.attempt(|parser| {
            let expression = parser.parse_expression()?;
            if parser.eat_ctrl(';') {
                return Some(expression);
            }
            if is_block_like(&expression.0) && !parser.peek_is_ctrl('}') {
                return Some(expression);
            }
            None
        }) {
            return Some(statement);
        }
        if let Some(item) = self.attempt(Self::parse_function) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_struct) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_enum) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_misplaced_resource) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_impl) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_trait) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_module) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_import_statement) {
            return Some(item);
        }
        if let Some(item) = self.attempt(Self::parse_use_statement) {
            return Some(item);
        }
        None
    }

    // --- Expressions ---------------------------------------------------------

    /// A full expression: the weak-precedence `const` prefix, else the secondary
    /// grammar (with struct literals admitted as operands). `condition_expression`
    /// is the sibling for condition positions and has NO `const` (§H.1).
    fn parse_expression(&mut self) -> Option<Spanned<Node<'src>>> {
        if self.peek_is(&Token::Const) {
            let start = self.position;
            self.bump();
            let inner = self.parse_expression()?;
            return Some((Node::Const(Box::new(inner)), self.span_from(start)));
        }
        self.parse_secondary(false)
    }

    /// The condition-position expression (`if`/`for` conditions, a `for … in`
    /// iterable, a `match` subject): the secondary grammar with struct literals
    /// excluded as operands, so the `{` after `if Foo` is the block, not a literal
    /// (§H.1). No `const` prefix.
    fn parse_condition(&mut self) -> Option<Spanned<Node<'src>>> {
        self.parse_secondary(true)
    }

    /// `secondary_expression` / `condition_expression`: the block-bearing and
    /// statement-shaped forms, then assignment, then the operator tower. The only
    /// difference between the two grammars is `no_struct`, which reaches ONLY the
    /// operator tower's chain (and its atom head); the block-bearing sub-parsers
    /// each recurse with their own mode (full expressions for values/bodies,
    /// conditions for nested heads), so it is not threaded into them.
    fn parse_secondary(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        match self.peek() {
            // A closure literal (`|params| body`, `|| body`) — always tried before
            // the tower, so a leading `||` is never a logical-or (which needs a left
            // operand). Nothing else in the grammar leads with `|`/`||` here.
            Some(Token::Op("|") | Token::Op("||")) => return self.parse_closure(),
            Some(Token::Ctrl('{')) => return self.parse_block_as_expression(),
            Some(Token::If) => return self.parse_if(),
            Some(Token::For) => return self.parse_for(),
            Some(Token::Match) => return self.parse_match(),
            Some(Token::Jump) => return self.parse_jump(),
            Some(Token::Let | Token::Mut) => return self.parse_let(),
            Some(Token::Ret) => return self.parse_return(),
            _ => {}
        }
        // Assignment (an lvalue then `=`/`+=`/…) is tried before the tower; it
        // backtracks when no assignment operator follows the place.
        if let Some(assignment) = self.parse_assignment() {
            return Some(assignment);
        }
        self.parse_operators(no_struct)
    }

    /// The operator tower above the postfix/precedence chain: the `is` pattern test,
    /// then `&&`, then `||` (each looser than the last). Built over the chain in the
    /// selected struct-literal mode.
    fn parse_operators(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_logical_or(no_struct)
    }

    fn parse_logical_or(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let mut left = self.parse_logical_and(no_struct)?;
        loop {
            let save = self.position;
            if !self.eat_op("||") {
                break;
            }
            match self.parse_logical_and(no_struct) {
                Some(right) => {
                    left = (
                        Node::Binary(BinaryOp::Or, Box::new(left), Box::new(right)),
                        self.span_from(start),
                    );
                }
                None => {
                    self.position = save;
                    break;
                }
            }
        }
        Some(left)
    }

    fn parse_logical_and(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let mut left = self.parse_is_expression(no_struct)?;
        loop {
            let save = self.position;
            if !self.eat_op("&&") {
                break;
            }
            match self.parse_is_expression(no_struct) {
                Some(right) => {
                    left = (
                        Node::Binary(BinaryOp::And, Box::new(left), Box::new(right)),
                        self.span_from(start),
                    );
                }
                None => {
                    self.position = save;
                    break;
                }
            }
        }
        Some(left)
    }

    /// `subject is pattern` — a single, optional pattern test (binds tighter than
    /// `&&`). Backtracks the `is` if no pattern follows.
    fn parse_is_expression(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let subject = self.parse_chain(no_struct)?;
        let save = self.position;
        if self.eat(&Token::Is) {
            match self.parse_pattern() {
                Some(pattern) => {
                    return Some((
                        Node::Is(Box::new(subject), Box::new(pattern)),
                        self.span_from(start),
                    ));
                }
                None => self.position = save,
            }
        }
        Some(subject)
    }

    /// The precedence chain (`chain_expr_parser`): the postfix/call/static-access
    /// expression, then the arithmetic/bitwise/comparison tower up to (and
    /// including) comparison — everything below the `is`/`&&`/`||` tier.
    fn parse_chain(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_compare(no_struct)
    }

    /// A left-associative binary level: `operand (op operand)*`, folding with a span
    /// from the chain's start. `operator` returns the `BinaryOp` for the current
    /// token and consumes it, or `None` to stop. On a right-operand failure the
    /// operator is backtracked (chumsky's `op.then(operand)` is atomic).
    fn parse_binary_level(
        &mut self,
        no_struct: bool,
        mut operator: impl FnMut(&mut Self) -> Option<BinaryOp>,
        mut operand: impl FnMut(&mut Self, bool) -> Option<Spanned<Node<'src>>>,
    ) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let mut left = operand(self, no_struct)?;
        loop {
            let save = self.position;
            let Some(op) = operator(self) else {
                break;
            };
            match operand(self, no_struct) {
                Some(right) => {
                    left = (
                        Node::Binary(op, Box::new(left), Box::new(right)),
                        self.span_from(start),
                    );
                }
                None => {
                    self.position = save;
                    break;
                }
            }
        }
        Some(left)
    }

    fn parse_compare(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_binary_level(
            no_struct,
            |parser| {
                // `==` / `!=` are `Op`s; `<` / `>` are `Ctrl`, and `<=` / `>=` lex as
                // `<` / `>` then `=`.
                if parser.eat_op("==") {
                    Some(BinaryOp::Eq)
                } else if parser.eat_op("!=") {
                    Some(BinaryOp::NotEq)
                } else if parser.eat_ctrl('<') {
                    Some(if parser.eat_op("=") {
                        BinaryOp::LtEq
                    } else {
                        BinaryOp::Lt
                    })
                } else if parser.eat_ctrl('>') {
                    Some(if parser.eat_op("=") {
                        BinaryOp::GtEq
                    } else {
                        BinaryOp::Gt
                    })
                } else {
                    None
                }
            },
            Self::parse_bit_or,
        )
    }

    fn parse_bit_or(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_binary_level(
            no_struct,
            |parser| parser.eat_op("|").then_some(BinaryOp::BitOr),
            Self::parse_bit_xor,
        )
    }

    fn parse_bit_xor(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_binary_level(
            no_struct,
            |parser| parser.eat_op("^").then_some(BinaryOp::BitXor),
            Self::parse_bit_and,
        )
    }

    fn parse_bit_and(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_binary_level(
            no_struct,
            |parser| parser.eat_op("&").then_some(BinaryOp::BitAnd),
            Self::parse_shift,
        )
    }

    fn parse_shift(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_binary_level(no_struct, Self::eat_shift_operator, Self::parse_sum)
    }

    /// `<<` / `>>` — reassembled from two SPAN-ADJACENT `Ctrl` tokens (there is no
    /// dedicated shift token; `<`/`>` are control characters). A lone `<`/`>`, or a
    /// non-adjacent pair (`a < < b`), is not a shift — the second token is left for
    /// the comparison level.
    fn eat_shift_operator(&mut self) -> Option<BinaryOp> {
        for (character, op) in [('<', BinaryOp::Shl), ('>', BinaryOp::Shr)] {
            if self.peek_is_ctrl(character) && self.peek_at_is_ctrl(1, character) {
                let first = self.tokens[self.position].1;
                let second = self.tokens[self.position + 1].1;
                if first.end == second.start {
                    self.position += 2;
                    return Some(op);
                }
            }
        }
        None
    }

    fn parse_sum(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_binary_level(
            no_struct,
            |parser| match parser.peek_op() {
                Some("+") => {
                    parser.bump();
                    Some(BinaryOp::Add)
                }
                Some("-") => {
                    parser.bump();
                    Some(BinaryOp::Sub)
                }
                _ => None,
            },
            Self::parse_product,
        )
    }

    fn parse_product(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        self.parse_binary_level(
            no_struct,
            |parser| match parser.peek_op() {
                Some("*") => {
                    parser.bump();
                    Some(BinaryOp::Mul)
                }
                Some("/") => {
                    parser.bump();
                    Some(BinaryOp::Div)
                }
                Some("%") => {
                    parser.bump();
                    Some(BinaryOp::Rem)
                }
                _ => None,
            },
            Self::parse_unary,
        )
    }

    /// Unary prefixes, binding tighter than the binary ops and recursing on
    /// themselves: `!`, prefix `-`, `await`, `async` (a block or any unary), `&` /
    /// `&mut` (take a view), `*` (deref). Falls through to the postfix chain.
    fn parse_unary(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        if self.eat_op("!") {
            let inner = self.parse_unary(no_struct)?;
            return Some((Node::Unary('!', Box::new(inner)), self.span_from(start)));
        }
        if self.eat_op("-") {
            let inner = self.parse_unary(no_struct)?;
            return Some((Node::Unary('-', Box::new(inner)), self.span_from(start)));
        }
        if self.eat(&Token::Await) {
            let inner = self.parse_unary(no_struct)?;
            return Some((Node::Await(Box::new(inner)), self.span_from(start)));
        }
        if self.eat(&Token::Async) {
            // `async { .. }` takes the block; `async expr` any unary.
            let inner = if self.peek_is_ctrl('{') {
                self.parse_block_as_expression()?
            } else {
                self.parse_unary(no_struct)?
            };
            return Some((Node::Async(Box::new(inner)), self.span_from(start)));
        }
        if self.eat_op("&") {
            let mutable = self.eat(&Token::Mut);
            let inner = self.parse_unary(no_struct)?;
            return Some((
                Node::Reference(mutable, Box::new(inner)),
                self.span_from(start),
            ));
        }
        if self.eat_op("*") {
            let inner = self.parse_unary(no_struct)?;
            return Some((Node::Dereference(Box::new(inner)), self.span_from(start)));
        }
        self.parse_member_accessor(no_struct)
    }

    /// The postfix chain over a call/static-access base: `.member`, `[index]`, `!`,
    /// a direct call `(args)`, `?.member`, and a bare `?`. Collected in order, then
    /// grouped so a `?.` link absorbs the following plain postfixes into its
    /// continuation (up to the next `?.`/`!`/chain end).
    fn parse_member_accessor(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        let base = self.parse_call(no_struct)?;
        let mut postfixes: Vec<(Postfix<'src>, Span)> = Vec::new();
        loop {
            let start = self.position;
            match self.parse_one_postfix()? {
                Some(postfix) => postfixes.push((postfix, self.span_from(start))),
                None => break,
            }
        }
        Some(group_postfixes(base, postfixes))
    }

    /// One postfix suffix, or `Some(None)` when none starts here (stop the loop).
    /// The outer `Option` is the clean-or-decline signal (a required inner element
    /// failed); the inner `Option` distinguishes "no postfix here".
    fn parse_one_postfix(&mut self) -> Option<Option<Postfix<'src>>> {
        // `.member` — with the mid-edit recovery to an `Error` member (silent at
        // parse; the receiver still analyzes, for LSP completion).
        if self.peek_is_ctrl('.') {
            let dot_span = self.here_span();
            self.bump();
            let member = self.parse_member_call();
            return Some(Some(Postfix::Member(
                member.unwrap_or((Node::Error, dot_span)),
            )));
        }
        // `[index]`.
        if self.peek_is_ctrl('[') {
            self.bump();
            let index = self.parse_expression()?;
            self.expect_ctrl(']')?;
            return Some(Some(Postfix::Index(index)));
        }
        // `expr!` — assert-or-return. `!=` is one token, so this never eats a
        // comparison's `!`.
        if self.eat_op("!") {
            return Some(Some(Postfix::TryAssert));
        }
        // `(args)` on a postfix result — calling a closure-typed value.
        if self.peek_is_ctrl('(') {
            let arguments = self.parse_argument_list()?;
            return Some(Some(Postfix::DirectCall(arguments)));
        }
        // `?.member` — a lifted link (tried before the bare `?`).
        if self.peek_is_op("?") && self.peek_at_is_ctrl(1, '.') {
            let start = self.position;
            self.bump(); // `?`
            self.bump(); // `.`
            let dot_span = self.span_from(start);
            let member = self.parse_member_call();
            return Some(Some(Postfix::LiftMember(
                member.unwrap_or((Node::Error, dot_span)),
            )));
        }
        // A bare `?` — an expression-lifting mark.
        if self.eat_op("?") {
            return Some(Some(Postfix::LiftBare));
        }
        Some(None)
    }

    /// A member after `.`/`?.`: a tuple index (`.0`), or a name with at most ONE
    /// fused call (`.method(args)`, optionally `.method<T>(args)`). Further
    /// `(args)` suffixes are left to the `DirectCall` postfix (the `.read()(a)`
    /// case). Returns `None` when no member follows (the recovery site).
    fn parse_member_call(&mut self) -> Option<Spanned<Node<'src>>> {
        if let Some(Token::Number(whole, fraction, suffix)) = self.peek() {
            let node = Node::Number(*whole, *fraction, *suffix);
            let span = self.here_span();
            self.bump();
            return Some((node, span));
        }
        let start = self.position;
        let name = self.eat_ident()?;
        let accessor = (Node::Accessor(name), self.span_from(start));
        // An optional single fused call: `<generics>? ( args )`. If generics parse
        // but no `(` follows, they are backtracked and the bare accessor is kept.
        let save = self.position;
        let generic_arguments = self.parse_generic_arguments();
        if self.peek_is_ctrl('(') {
            let arguments = self.parse_argument_list()?;
            Some((
                Node::Call(Box::new(accessor), generic_arguments, arguments),
                self.span_from(start),
            ))
        } else {
            self.position = save;
            Some(accessor)
        }
    }

    /// `f(args)` / `f<T>(args)` folded over the static-access base. Generic
    /// arguments stick only when a `(` follows (so `a < b` stays a comparison); on
    /// no `(` the whole generic attempt is backtracked.
    fn parse_call(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let mut callee = self.parse_static_accessor(no_struct)?;
        loop {
            let save = self.position;
            let generic_arguments = self.parse_generic_arguments();
            if self.peek_is_ctrl('(') {
                let arguments = self.parse_argument_list()?;
                callee = (
                    Node::Call(Box::new(callee), generic_arguments, arguments),
                    self.span_from(start),
                );
            } else {
                self.position = save;
                break;
            }
        }
        Some(callee)
    }

    /// `( expr, … )` argument list (allow-trailing), carrying its own `(`..`)` span.
    fn parse_argument_list(&mut self) -> Option<Spanned<NodeList<'src>>> {
        let start = self.position;
        self.expect_ctrl('(')?;
        let arguments =
            self.comma_list(Self::parse_expression, |parser| parser.peek_is_ctrl(')'))?;
        self.expect_ctrl(')')?;
        Some((arguments, self.span_from(start)))
    }

    /// `head (:: member)*` — a `::` path. The head is a generic static head
    /// (`List<str>::…`) when a `::` follows generics, else the chain head atom.
    fn parse_static_accessor(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let mut current = match self.parse_generic_static_head() {
            Some(head) => head,
            None => self.parse_chain_head(no_struct)?,
        };
        loop {
            let save = self.position;
            if !self.eat_op("::") {
                break;
            }
            match self.eat_ident() {
                Some(member) => {
                    current = (
                        Node::StaticAccessor(Box::new(current), member),
                        self.span_from(start),
                    );
                }
                None => {
                    self.position = save;
                    break;
                }
            }
        }
        Some(current)
    }

    /// `Name<Args>` as a `::`-path head — only when a `::` actually follows (matched
    /// with a lookahead, not consumed), so a generic *call* `default<Id>()` is left
    /// for the call level.
    fn parse_generic_static_head(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            let name = parser.eat_ident()?;
            let generic_arguments = parser.parse_generic_arguments()?;
            if !parser.peek_is_op("::") {
                return None;
            }
            Some((
                Node::AccessorWithGenerics(name, generic_arguments),
                parser.span_from(start),
            ))
        })
    }

    /// The chain head: in expression mode a struct initializer is tried first, then
    /// the plain atom; in condition mode (`no_struct`) only the plain atom, so a `{`
    /// after a bare name is a block, not a literal (§H.1).
    fn parse_chain_head(&mut self, no_struct: bool) -> Option<Spanned<Node<'src>>> {
        if !no_struct {
            if let Some(initializer) = self.parse_struct_initializer() {
                return Some(initializer);
            }
        }
        self.parse_atom()
    }

    /// `Name<Args>? { field, … }` — a struct initializer (expression mode only).
    /// Backtracks when no `{` follows the name (+ optional generics), so a bare name
    /// falls through to the atom.
    fn parse_struct_initializer(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            let name = parser.eat_ident()?;
            let generic_arguments = parser.parse_generic_arguments();
            if !parser.peek_is_ctrl('{') {
                return None;
            }
            let fields_start = parser.position;
            parser.expect_ctrl('{')?;
            let fields = parser.comma_list(Self::parse_struct_initializer_field, |parser| {
                parser.peek_is_ctrl('}')
            })?;
            parser.expect_ctrl('}')?;
            let fields = (fields, parser.span_from(fields_start));
            Some((
                Node::StructInitializer(name, generic_arguments, fields),
                parser.span_from(start),
            ))
        })
    }

    /// `name` or `name = value` — one struct-initializer field.
    fn parse_struct_initializer_field(
        &mut self,
    ) -> Option<Spanned<(&'src str, Option<Spanned<Node<'src>>>)>> {
        let start = self.position;
        let name = self.eat_ident()?;
        let value = if self.eat_op("=") {
            Some(self.parse_expression()?)
        } else {
            None
        };
        Some(((name, value), self.span_from(start)))
    }

    /// An atom (containing no ambiguity), in the chumsky `atom` choice order: a
    /// literal, a `tuple_comprehension` (`(x in xs => e)`), a `macro name(..)`
    /// invocation, a `macro { }` block, a bare name (`Accessor`), a `[..]` (repeat
    /// or list), or a `(..)` (tuple or parenthesised group). `local_type` is dead
    /// in expression position (the bare-name alternative always wins), matching the
    /// chumsky choice, so it is omitted.
    fn parse_atom(&mut self) -> Option<Spanned<Node<'src>>> {
        if let Some(literal) = self.parse_literal() {
            return Some(literal);
        }
        if let Some(comprehension) = self.parse_tuple_comprehension() {
            return Some(comprehension);
        }
        if let Some(invocation) = self.parse_macro_invocation() {
            return Some(invocation);
        }
        if let Some(macro_block) = self.parse_macro_block() {
            return Some(macro_block);
        }
        if let Some(Token::Ident(name)) = self.peek() {
            let node = Node::Accessor(name);
            let span = self.here_span();
            self.bump();
            return Some((node, span));
        }
        if self.peek_is_ctrl('[') {
            return self.parse_bracket_atom();
        }
        if self.peek_is_ctrl('(') {
            return self.parse_paren_atom();
        }
        None
    }

    /// A single-token literal, including `void` (the unit value — a bare `void`
    /// identifier in expression position).
    fn parse_literal(&mut self) -> Option<Spanned<Node<'src>>> {
        let node = match self.peek()? {
            Token::Null => Node::Null,
            Token::Bool(value) => Node::Bool(*value),
            Token::Number(whole, fraction, suffix) => Node::Number(*whole, *fraction, *suffix),
            Token::String(text) => Node::String(text),
            Token::MultilineString(text) => Node::MultilineString(text),
            Token::Ident("void") => Node::Void,
            _ => return None,
        };
        let span = self.here_span();
        self.bump();
        Some((node, span))
    }

    /// `[value; length]` (repeat) or `[a, b, …]` (list). The `;` after the first
    /// element is the fork.
    fn parse_bracket_atom(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('[')?;
            if parser.eat_ctrl(']') {
                return Some((Node::List(Vec::new()), parser.span_from(start)));
            }
            let first = parser.parse_expression()?;
            if parser.eat_ctrl(';') {
                let length = parser.parse_expression()?;
                parser.expect_ctrl(']')?;
                return Some((
                    Node::Repeat(Box::new(first), Box::new(length)),
                    parser.span_from(start),
                ));
            }
            let mut items = vec![first];
            while parser.eat_ctrl(',') {
                if parser.peek_is_ctrl(']') {
                    break;
                }
                items.push(parser.parse_expression()?);
            }
            parser.expect_ctrl(']')?;
            Some((Node::List(items), parser.span_from(start)))
        })
    }

    /// `(a, b, …)` (a tuple, ≥2 elements) or `(expr)` (a group that dissolves to its
    /// inner expression — keeping the inner's own span — unless it contains a
    /// bare-`?` mark, when it becomes a region-delimiting `LiftGroup`).
    fn parse_paren_atom(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('(')?;
            let first = parser.parse_expression()?;
            if parser.peek_is_ctrl(',') {
                // A tuple is `expr (',' expr)*` (≥2 elements) with NO trailing comma
                // — unlike a list literal, the chumsky `tuple` atom has no
                // `allow_trailing`, so `(a, b,)` declines (a group can't hold a comma
                // either). Every `,` here must be followed by an expression.
                let mut items = vec![first];
                while parser.eat_ctrl(',') {
                    items.push(parser.parse_expression()?);
                }
                parser.expect_ctrl(')')?;
                Some((Node::Tuple(items), parser.span_from(start)))
            } else {
                parser.expect_ctrl(')')?;
                if first.0.contains_lift_mark() {
                    Some((Node::LiftGroup(Box::new(first)), parser.span_from(start)))
                } else {
                    // A group dissolves to its inner expression, keeping the inner's
                    // own span (the parens contribute nothing).
                    Some(first)
                }
            }
        })
    }

    // --- Generic arguments ---------------------------------------------------

    /// `<Type, …>` — generic arguments (allow-trailing), or `None` (backtracking)
    /// when no well-formed `<…>` is present, which is how `a < b` stays a
    /// comparison. Recovery of a malformed `<…>` is deferred to S4.
    fn parse_generic_arguments(&mut self) -> Option<GenericArguments<'src>> {
        self.attempt(|parser| {
            if !parser.peek_is_ctrl('<') {
                return None;
            }
            let start = parser.position;
            parser.expect_ctrl('<')?;
            let arguments =
                parser.comma_list(Self::parse_type, |parser| parser.peek_is_ctrl('>'))?;
            parser.expect_ctrl('>')?;
            Some((arguments, parser.span_from(start)))
        })
    }

    // --- Block-bearing forms -------------------------------------------------

    /// A brace-delimited block: `statement* trailing_expression?`. The trailing
    /// expression is the block's value; with none, the value is `void` at the
    /// closing brace.
    fn parse_block(&mut self) -> Option<Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>> {
        let start = self.position;
        self.expect_ctrl('{')?;
        let mut statements = Vec::new();
        loop {
            if self.peek_is_ctrl('}') || self.at_end() {
                break;
            }
            match self.parse_statement() {
                Some(statement) => statements.push(statement),
                None => break,
            }
        }
        let tail = if self.peek_is_ctrl('}') {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect_ctrl('}')?;
        let span = self.span_from(start);
        let tail = tail
            .map(Box::new)
            .unwrap_or_else(|| Box::new((Node::Void, (span.end..span.end).into())));
        Some(((statements, tail), span))
    }

    /// A block used as an expression (`Node::Block`) — the secondary-expression `{`
    /// alternative and the `async { .. }` body.
    fn parse_block_as_expression(&mut self) -> Option<Spanned<Node<'src>>> {
        let (body, span) = self.parse_block()?;
        Some((Node::Block((body, span)), span))
    }

    /// `if condition { .. } (else ({ .. } | if …))?`.
    fn parse_if(&mut self) -> Option<Spanned<Node<'src>>> {
        let (branch, span) = self.parse_if_branch()?;
        Some((Node::If(branch), span))
    }

    /// The recursive core of `if`, yielding a `NodeIfBranch::If`. `else if` recurses
    /// into another branch; `else { .. }` is a `NodeIfBranch::Else`.
    fn parse_if_branch(&mut self) -> Option<Spanned<NodeIfBranch<'src>>> {
        let start = self.position;
        self.expect(&Token::If)?;
        let condition = self.parse_condition()?;
        let then = self.parse_block()?;
        let else_ = if self.eat(&Token::Else) {
            if self.peek_is(&Token::If) {
                Some(self.parse_if_branch()?)
            } else {
                let block = self.parse_block()?;
                let block_span = block.1;
                Some((NodeIfBranch::Else(block), block_span))
            }
        } else {
            None
        };
        Some((
            NodeIfBranch::If(Box::new(If {
                condition: Box::new(condition),
                then,
                else_,
            })),
            self.span_from(start),
        ))
    }

    /// Every loop form: `for item in iterable { .. }`, `for { .. }` (infinite), or
    /// `for condition { .. }` (while). The `item in` form is distinguished by a
    /// bare loop variable followed by `in`.
    fn parse_for(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::For)?;
        // `for IDENT in …` — a bare identifier immediately followed by `in`.
        if matches!(self.peek(), Some(Token::Ident(_))) && self.peek_at(1) == Some(&Token::In) {
            let variable = self.eat_ident()?;
            self.expect(&Token::In)?;
            let iterable = self.parse_condition()?;
            let body = self.parse_block()?;
            return Some((
                Node::ForIn(variable, Box::new(iterable), body),
                self.span_from(start),
            ));
        }
        // `for { .. }` (infinite) — the block is tried before a condition so its
        // brace is not read as a condition.
        if self.peek_is_ctrl('{') {
            let body = self.parse_block()?;
            return Some((Node::For(None, body), self.span_from(start)));
        }
        // `for condition { .. }` (while).
        let condition = self.parse_condition()?;
        let body = self.parse_block()?;
        Some((
            Node::For(Some(Box::new(condition)), body),
            self.span_from(start),
        ))
    }

    /// `match subject { leg, … }`.
    fn parse_match(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Match)?;
        let subject = self.parse_condition()?;
        let legs_start = self.position;
        self.expect_ctrl('{')?;
        let mut legs = Vec::new();
        loop {
            if self.peek_is_ctrl('}') || self.at_end() {
                break;
            }
            legs.push(self.parse_match_leg()?);
            // A comma after a leg is optional.
            self.eat_ctrl(',');
        }
        self.expect_ctrl('}')?;
        let legs = (legs, self.span_from(legs_start));
        Some((Node::Match(Box::new(subject), legs), self.span_from(start)))
    }

    /// One leg: `pattern (, pattern)* (if guard)? => body`.
    fn parse_match_leg(&mut self) -> Option<MatchLeg<'src>> {
        let mut patterns = vec![self.parse_pattern()?];
        loop {
            let save = self.position;
            if !self.eat_ctrl(',') {
                break;
            }
            match self.parse_pattern() {
                Some(pattern) => patterns.push(pattern),
                None => {
                    // A trailing `,` before the guard/`=>` (no pattern list is
                    // allow-trailing) is backtracked.
                    self.position = save;
                    break;
                }
            }
        }
        let guard = if self.eat(&Token::If) {
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };
        self.expect_op("=>")?;
        let body = self.parse_expression()?;
        Some((patterns, guard, body))
    }

    /// `jump target` — a loop-control keyword.
    fn parse_jump(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Jump)?;
        let target = self.eat_ident()?;
        Some((Node::Jump(target), self.span_from(start)))
    }

    /// `ret expr?` — return a value, or a bare `ret` for an early void return.
    fn parse_return(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Ret)?;
        let value = self.parse_expression();
        Some((Node::FuncReturn(value.map(Box::new)), self.span_from(start)))
    }

    /// `let`/`mut` binding: `(let|mut) binder (: type)? (= value)?`, lowering a bare
    /// name to `Let` and a destructuring binder to `LetDestructure`.
    fn parse_let(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let mutable = if self.eat(&Token::Let) {
            false
        } else if self.eat(&Token::Mut) {
            true
        } else {
            return None;
        };
        let (pattern, pattern_span) = self.parse_binder()?;
        let type_ = if self.eat_op(":") {
            Some(Box::new(self.parse_type()?))
        } else {
            None
        };
        let value = if self.eat_op("=") {
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };
        let node = match pattern {
            Pattern::Binding(name, _) => Node::Let((name, pattern_span), type_, value, mutable),
            pattern => Node::LetDestructure((pattern, pattern_span), type_, value, mutable),
        };
        Some((node, self.span_from(start)))
    }

    /// An assignment: `(*)? place op value`, where `place` is the struct-free
    /// precedence chain and `op` is `=`/`+=`/`-=`/`*=`/`/=`/`%=`. Backtracks when no
    /// assignment operator follows the place, so an ordinary expression is left to
    /// the operator tower.
    fn parse_assignment(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            let deref = parser.eat_op("*");
            let place = parser.parse_chain(true)?;
            let target = if deref {
                (Node::Dereference(Box::new(place)), parser.span_from(start))
            } else {
                place
            };
            let op = if parser.eat_op("=") {
                None
            } else if parser.eat_op("+=") {
                Some(BinaryOp::Add)
            } else if parser.eat_op("-=") {
                Some(BinaryOp::Sub)
            } else if parser.eat_op("*=") {
                Some(BinaryOp::Mul)
            } else if parser.eat_op("/=") {
                Some(BinaryOp::Div)
            } else if parser.eat_op("%=") {
                Some(BinaryOp::Rem)
            } else {
                return None;
            };
            let value = parser.parse_expression()?;
            Some((
                Node::Assign(Box::new(target), op, Box::new(value)),
                parser.span_from(start),
            ))
        })
    }

    /// A closure literal: `|param, …| : return_type? body` or `|| : return_type?
    /// body`. Parameters are `binder (: type)?` with the bare view convention.
    fn parse_closure(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            let parameters = if parser.eat_op("||") {
                Vec::new()
            } else if parser.eat_op("|") {
                let parameters = parser.comma_list(Self::parse_closure_parameter, |parser| {
                    parser.peek_is_op("|")
                })?;
                parser.expect_op("|")?;
                parameters
            } else {
                return None;
            };
            let parameters = (parameters, parser.span_from(start));
            let return_type = if parser.eat_op(":") {
                Some(Box::new(parser.parse_type()?))
            } else {
                None
            };
            let return_value = parser.parse_expression()?;
            Some((
                Node::Closure(Closure {
                    parameters,
                    return_type,
                    return_value: Box::new(return_value),
                }),
                parser.span_from(start),
            ))
        })
    }

    /// One closure parameter: `binder (: type)?`, carrying the bare convention and
    /// the binder's span.
    fn parse_closure_parameter(&mut self) -> Option<Parameter<'src>> {
        let (pattern, pattern_span) = self.parse_binder()?;
        let parameter_type = if self.eat_op(":") {
            Some(Box::new(self.parse_type()?))
        } else {
            None
        };
        Some((pattern, parameter_type, Convention::Bare, pattern_span))
    }

    // --- Binders and patterns ------------------------------------------------

    /// A binder in `let`/parameter position: a plain name, a tuple of binders
    /// (`(a, b)`, ≥2), or a fixed-array binder (`[a, b, c]`, ≥1). Nests recursively.
    /// Bindings are parsed immutable; `let`/`mut` stamps mutability separately.
    fn parse_binder(&mut self) -> Option<Spanned<Pattern<'src>>> {
        let start = self.position;
        if self.peek_is_ctrl('(') {
            return self.attempt(|parser| {
                parser.expect_ctrl('(')?;
                let patterns =
                    parser.comma_list(Self::parse_binder, |parser| parser.peek_is_ctrl(')'))?;
                parser.expect_ctrl(')')?;
                if patterns.len() < 2 {
                    return None;
                }
                Some((Pattern::Tuple(patterns), parser.span_from(start)))
            });
        }
        if self.peek_is_ctrl('[') {
            return self.attempt(|parser| {
                parser.expect_ctrl('[')?;
                let patterns =
                    parser.comma_list(Self::parse_binder, |parser| parser.peek_is_ctrl(']'))?;
                parser.expect_ctrl(']')?;
                if patterns.is_empty() {
                    return None;
                }
                Some((Pattern::Array(patterns), parser.span_from(start)))
            });
        }
        let name = self.eat_ident()?;
        Some((Pattern::Binding(name, false), self.span_from(start)))
    }

    /// A match/`is` pattern: `_`, `let x` / `mut x` (a binder), a literal (`"quit"`,
    /// `42`), a tuple (`(a, b)`, ≥2), or a variant (`Some(let x)`, `Enum::Variant`).
    fn parse_pattern(&mut self) -> Option<Spanned<Pattern<'src>>> {
        let start = self.position;
        // `let x` / `mut x` — a binder, stamped mutable per the keyword.
        if self.peek_is(&Token::Let) || self.peek_is(&Token::Mut) {
            let mutable = self.eat(&Token::Mut);
            if !mutable {
                self.bump(); // `let`
            }
            let (binder, _) = self.parse_binder()?;
            let pattern = apply_binding_mutability(binder, mutable);
            return Some((pattern, self.span_from(start)));
        }
        // A tuple pattern `(a, b, …)` (≥2, keeping a single parenthesised pattern
        // unambiguous — there is no grouping arm).
        if self.peek_is_ctrl('(') {
            return self.attempt(|parser| {
                parser.expect_ctrl('(')?;
                let patterns =
                    parser.comma_list(Self::parse_pattern, |parser| parser.peek_is_ctrl(')'))?;
                parser.expect_ctrl(')')?;
                if patterns.len() < 2 {
                    return None;
                }
                Some((Pattern::Tuple(patterns), parser.span_from(start)))
            });
        }
        // A literal pattern — matched by equality (`bool`/`null` stay variant/keyword
        // patterns).
        let literal_node = match self.peek() {
            Some(Token::String(text)) => Some(Node::String(text)),
            Some(Token::MultilineString(text)) => Some(Node::MultilineString(text)),
            Some(Token::Number(whole, fraction, suffix)) => {
                Some(Node::Number(*whole, *fraction, *suffix))
            }
            _ => None,
        };
        if let Some(node) = literal_node {
            let span = self.here_span();
            self.bump();
            return Some((
                Pattern::Literal(Box::new((node, span))),
                self.span_from(start),
            ));
        }
        // A variant path `Name (:: member)* (payload)?` — a bare `_` with no path
        // and no payload is the wildcard.
        if let Some(head) = self.eat_name() {
            let mut path = vec![head];
            loop {
                let save = self.position;
                if !self.eat_op("::") {
                    break;
                }
                match self.eat_ident() {
                    Some(member) => path.push(member),
                    None => {
                        self.position = save;
                        break;
                    }
                }
            }
            let payload = if self.peek_is_ctrl('(') {
                self.attempt(|parser| {
                    parser.expect_ctrl('(')?;
                    let patterns = parser
                        .comma_list(Self::parse_pattern, |parser| parser.peek_is_ctrl(')'))?;
                    parser.expect_ctrl(')')?;
                    Some(patterns)
                })
            } else {
                None
            };
            let pattern = if path.len() == 1 && path[0] == "_" && payload.is_none() {
                Pattern::Wildcard
            } else {
                Pattern::Variant(path, payload)
            };
            return Some((pattern, self.span_from(start)));
        }
        None
    }

    // --- Types ---------------------------------------------------------------

    /// A type, with the optional `context` clause suffix (`Type context name` /
    /// `context (a, b)`).
    fn parse_type(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let inner = self.parse_type_atom()?;
        if let Some(contexts) = self.parse_context_clause() {
            Some((
                Node::TypeWithContexts(Box::new(inner), contexts),
                self.span_from(start),
            ))
        } else {
            Some(inner)
        }
    }

    /// A type without the context suffix: `[T; n]`, `&T`/`&mut T`, a `type` binder,
    /// a closure type (with the optional `async`/`sync` marker), a generic-applied
    /// local (`List<T>`), a plain local, a mapped tuple type (`(U in T: F<U>)`), or a
    /// tuple type. Tried in the chumsky order.
    fn parse_type_atom(&mut self) -> Option<Spanned<Node<'src>>> {
        if self.peek_is_ctrl('[') {
            if let Some(array) = self.parse_array_type() {
                return Some(array);
            }
        }
        if self.peek_is_op("&") {
            return self.parse_reference_type();
        }
        if self.peek_is(&Token::Type) {
            return self.parse_type_binder();
        }
        if let Some(closure) = self.parse_closure_type() {
            return Some(closure);
        }
        if let Some(local) = self.parse_local_type() {
            return Some(local);
        }
        if let Some(name) = self.eat_ident() {
            let span = self.span_from(self.position - 1);
            return Some((Node::Accessor(name), span));
        }
        if self.peek_is_ctrl('(') {
            if let Some(mapped) = self.parse_mapped_type() {
                return Some(mapped);
            }
            return self.parse_tuple_type();
        }
        None
    }

    /// `[T; length]` — a fixed-length array type; `length` is an integer literal.
    fn parse_array_type(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('[')?;
            let element = parser.parse_type()?;
            parser.expect_ctrl(';')?;
            let length = parser.parse_array_length()?;
            parser.expect_ctrl(']')?;
            Some((
                Node::ArrayType(Box::new(element), Box::new(length)),
                parser.span_from(start),
            ))
        })
    }

    /// An array-type length: an integer (numeric) literal.
    fn parse_array_length(&mut self) -> Option<Spanned<Node<'src>>> {
        if let Some(Token::Number(whole, fraction, suffix)) = self.peek() {
            let node = Node::Number(*whole, *fraction, *suffix);
            let span = self.here_span();
            self.bump();
            Some((node, span))
        } else {
            None
        }
    }

    /// `&T` / `&mut T` — a view type.
    fn parse_reference_type(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect_op("&")?;
        let mutable = self.eat(&Token::Mut);
        let inner = self.parse_type()?;
        Some((
            Node::Reference(mutable, Box::new(inner)),
            self.span_from(start),
        ))
    }

    /// `type X (: A + B)?` — a generic binder in type position (impl subject
    /// patterns).
    fn parse_type_binder(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Type)?;
        let name = self.eat_ident()?;
        let bounds = if self.eat_op(":") {
            self.parse_type_bounds()?
        } else {
            Vec::new()
        };
        Some((Node::TypeBinder(name, bounds), self.span_from(start)))
    }

    /// `A + B + …` — a `+`-separated bound list (≥1).
    fn parse_type_bounds(&mut self) -> Option<Vec<Spanned<Node<'src>>>> {
        let mut bounds = vec![self.parse_type()?];
        while self.eat_op("+") {
            bounds.push(self.parse_type()?);
        }
        Some(bounds)
    }

    /// A closure type: an optional `async`/`sync` (contextual) marker, then
    /// `|param, …| return?` or `|| return?`. A closure parameter is `(name :)? type`.
    /// The whole thing backtracks (falling through to a plain local) if no `|`/`||`
    /// follows the marker — so a bare `sync` is a type name.
    fn parse_closure_type(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            enum Marker {
                Async,
                Sync,
            }
            let marker = if parser.eat(&Token::Async) {
                Some(Marker::Async)
            } else if matches!(parser.peek(), Some(Token::Ident("sync"))) {
                parser.bump();
                Some(Marker::Sync)
            } else {
                None
            };
            let inner = parser.parse_closure_type_inner()?;
            Some(match marker {
                Some(Marker::Async) => (Node::AsyncType(Box::new(inner)), parser.span_from(start)),
                Some(Marker::Sync) => (Node::SyncType(Box::new(inner)), parser.span_from(start)),
                None => inner,
            })
        })
    }

    /// The `|params| return?` core of a closure type (no marker). Note there is NO
    /// arrow: the return type follows the parameter delimiters directly.
    fn parse_closure_type_inner(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let parameters = if self.eat_op("||") {
            Vec::new()
        } else if self.eat_op("|") {
            let parameters = self.comma_list(Self::parse_closure_type_parameter, |parser| {
                parser.peek_is_op("|")
            })?;
            self.expect_op("|")?;
            parameters
        } else {
            return None;
        };
        let parameters = (parameters, self.span_from(start));
        let return_type = self.attempt(|parser| parser.parse_type()).map(Box::new);
        Some((
            Node::ClosureType(parameters, return_type),
            self.span_from(start),
        ))
    }

    /// One closure-type parameter: `(name :)? type`.
    fn parse_closure_type_parameter(
        &mut self,
    ) -> Option<(Option<&'src str>, Box<Spanned<Node<'src>>>)> {
        let name = if matches!(self.peek(), Some(Token::Ident(_)))
            && matches!(self.peek_at(1), Some(Token::Op(":")))
        {
            let name = self.eat_ident();
            self.bump(); // `:`
            name
        } else {
            None
        };
        let type_ = self.parse_type()?;
        Some((name, Box::new(type_)))
    }

    /// `Name<Args>` in type position — a generic-applied local (before the plain
    /// local so the generics are consumed as part of the type).
    fn parse_local_type(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            let name = parser.eat_ident()?;
            let generic_arguments = parser.parse_generic_arguments()?;
            Some((
                Node::AccessorWithGenerics(name, generic_arguments),
                parser.span_from(start),
            ))
        })
    }

    /// `(U in T: F<U>)` — a mapped tuple type (tried before the plain tuple type,
    /// distinguished by the `in`).
    fn parse_mapped_type(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('(')?;
            let binder_start = parser.position;
            let binder = parser.eat_ident()?;
            let binder_span = parser.span_from(binder_start);
            parser.expect(&Token::In)?;
            let source = parser.parse_type()?;
            parser.expect_op(":")?;
            let template = parser.parse_type()?;
            parser.expect_ctrl(')')?;
            Some((
                Node::MappedType {
                    binder,
                    binder_span,
                    source: Box::new(source),
                    template: Box::new(template),
                },
                parser.span_from(start),
            ))
        })
    }

    /// `(A, B, …)` — a tuple type (allow-trailing, no minimum: `()` is the empty
    /// tuple and `(A)` a one-tuple, unlike a parenthesised expression).
    fn parse_tuple_type(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('(')?;
            let elements =
                parser.comma_list(Self::parse_type, |parser| parser.peek_is_ctrl(')'))?;
            parser.expect_ctrl(')')?;
            Some((Node::Tuple(elements), parser.span_from(start)))
        })
    }

    /// `context name` / `context (a, b)` — the optional context clause on a type
    /// (`context` is contextual: an `Ident`, so `std::context` paths stay legal).
    fn parse_context_clause(&mut self) -> Option<Vec<(&'src str, Span)>> {
        if !matches!(self.peek(), Some(Token::Ident("context"))) {
            return None;
        }
        self.attempt(|parser| {
            parser.bump(); // `context`
            if parser.peek_is_ctrl('(') {
                parser.expect_ctrl('(')?;
                let names = parser
                    .comma_list(Self::parse_context_name, |parser| parser.peek_is_ctrl(')'))?;
                parser.expect_ctrl(')')?;
                if names.is_empty() {
                    return None;
                }
                Some(names)
            } else {
                let start = parser.position;
                let name = parser.eat_ident()?;
                Some(vec![(name, parser.span_from(start))])
            }
        })
    }

    fn parse_context_name(&mut self) -> Option<(&'src str, Span)> {
        let start = self.position;
        let name = self.eat_ident()?;
        Some((name, self.span_from(start)))
    }

    // --- Item bodies ---------------------------------------------------------

    /// `{ statement* }` — an item body (an `impl` or `mod` block): a bare statement
    /// list with NO trailing expression (unlike [`Parser::parse_block`]), carrying
    /// the `{ .. }` span. Reproduces the chumsky `statement.repeated().collect()
    /// .delimited_by('{','}')`.
    fn parse_item_body(&mut self) -> Option<Spanned<NodeList<'src>>> {
        let start = self.position;
        self.expect_ctrl('{')?;
        let mut statements = Vec::new();
        loop {
            if self.peek_is_ctrl('}') || self.at_end() {
                break;
            }
            match self.parse_statement() {
                Some(statement) => statements.push(statement),
                None => break,
            }
        }
        self.expect_ctrl('}')?;
        Some((statements, self.span_from(start)))
    }

    /// `{ function* }` — a trait body: a list of function declarations ONLY (not
    /// arbitrary statements), carrying the `{ .. }` span. Reproduces the chumsky
    /// `function.repeated().collect().delimited_by('{','}')`.
    fn parse_trait_body(&mut self) -> Option<Spanned<NodeList<'src>>> {
        let start = self.position;
        self.expect_ctrl('{')?;
        let mut functions = Vec::new();
        loop {
            if self.peek_is_ctrl('}') || self.at_end() {
                break;
            }
            match self.attempt(Self::parse_function) {
                Some(function) => functions.push(function),
                None => break,
            }
        }
        self.expect_ctrl('}')?;
        Some((functions, self.span_from(start)))
    }

    // --- Generic parameters --------------------------------------------------

    /// `<param, ...>` — generic PARAMETERS in declaration position (allow-trailing),
    /// or `None` (backtracking) when no well-formed `<...>` is present. Distinct
    /// from [`Parser::parse_generic_arguments`] (types). Recovery of a malformed
    /// `<...>` is deferred to S4.
    fn parse_generic_parameters(&mut self) -> Option<GenericParameters<'src>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('<')?;
            let parameters = parser.comma_list(Self::parse_generic_parameter, |parser| {
                parser.peek_is_ctrl('>')
            })?;
            parser.expect_ctrl('>')?;
            Some((parameters, parser.span_from(start)))
        })
    }

    /// One generic parameter: `type? name (: (tuple_bound | A + B))? (= default)?`.
    /// The `type` marker makes it a binder (impl subject patterns); the bound is a
    /// tuple bound (`(2..)`) tried before the `+`-separated trait-bound list.
    fn parse_generic_parameter(&mut self) -> Option<GenericParameter<'src>> {
        let is_type = self.eat(&Token::Type);
        let name_start = self.position;
        let name = self.eat_ident()?;
        let name_span = self.span_from(name_start);
        let (bounds, tuple_bound) = if self.eat_op(":") {
            match self.parse_tuple_bound() {
                Some(bound) => (Vec::new(), Some(bound)),
                None => (self.parse_type_bounds()?, None),
            }
        } else {
            (Vec::new(), None)
        };
        let default = if self.eat_op("=") {
            Some(Box::new(self.parse_type()?))
        } else {
            None
        };
        Some(GenericParameter {
            name,
            name_span,
            is_type,
            bounds,
            tuple_bound,
            default,
        })
    }

    /// `(lo?..hi? (: element)?)` — a tuple-arity bound (`T: (2..)`, `(..: Display)`).
    /// The `..` is two `.` control tokens (NO adjacency check, matching the chumsky
    /// `dot_dot`, unlike the shift operator). Backtracks when the `(` does not open
    /// an `int? .. …` shape (so a tuple-type bound `(A, B)` falls through to the
    /// trait-bound list). Endpoints that do not parse as `u32` become `None`.
    fn parse_tuple_bound(&mut self) -> Option<TupleBound<'src>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('(')?;
            let lo = parser.eat_integer();
            parser.expect_ctrl('.')?;
            parser.expect_ctrl('.')?;
            let hi = parser.eat_integer();
            let element = if parser.eat_op(":") {
                Some(Box::new(parser.parse_type()?))
            } else {
                None
            };
            parser.expect_ctrl(')')?;
            Some(TupleBound {
                lo: lo.and_then(|whole| whole.parse::<u32>().ok()),
                hi: hi.and_then(|whole| whole.parse::<u32>().ok()),
                element,
                span: parser.span_from(start),
            })
        })
    }

    // --- Functions -----------------------------------------------------------

    /// A function declaration: the ORDERED attribute prefix (`[extern(..)]`,
    /// `[must_use]`, `[rpc]`, `[trait_only]`, `[doc(hidden)]`, `[platform(..)]` —
    /// each optional but IN THIS ORDER, a faithful quirk), then `async? external?
    /// fun name generics? (params) (: return)? (borrows param)? (block | ;)`.
    fn parse_function(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let extern_binding = self.parse_extern_attribute();
        let must_use = self.eat_marker_attribute("must_use");
        let rpc = self.eat_marker_attribute("rpc");
        let trait_only = self.eat_marker_attribute("trait_only");
        let doc_hidden = self.parse_doc_hidden_attribute();
        let platform_fence = self.parse_platform_attribute().unwrap_or_default();
        let is_async = self.eat(&Token::Async);
        let external = self.eat(&Token::External);
        self.expect(&Token::Fun)?;
        let name_start = self.position;
        let name = self.eat_ident()?;
        let name = (name, self.span_from(name_start));
        let generic_parameters = self.parse_generic_parameters();
        let parameters = self.parse_function_parameters()?;
        let return_type = if self.eat_op(":") {
            Some(Box::new(self.parse_type()?))
        } else {
            None
        };
        // `borrows <param>` — the returned view is a projection of that parameter.
        let borrows = if self.eat(&Token::Borrows) {
            Some(self.eat_ident()?)
        } else {
            None
        };
        // A block body, or `;` for a signature-only declaration (a required trait
        // method or an `external` intrinsic). The block is tried first (chumsky
        // `block.map(Some).or(';'.map(|_| None))`), but the two lead on disjoint
        // tokens (`{` vs `;`) so a bare `;` short-circuits equivalently.
        let body = if self.eat_ctrl(';') {
            None
        } else {
            Some(self.parse_block()?)
        };
        Some((
            Node::Func(Func {
                name,
                is_async,
                external,
                extern_binding,
                must_use,
                rpc,
                trait_only,
                doc_hidden,
                platform_fence,
                generic_parameters,
                parameters,
                return_type,
                borrows,
                body,
            }),
            self.span_from(start),
        ))
    }

    /// `(param, ...)` — a function parameter list (allow-trailing), carrying the
    /// `( .. )` span.
    fn parse_function_parameters(&mut self) -> Option<Spanned<Vec<Parameter<'src>>>> {
        let start = self.position;
        self.expect_ctrl('(')?;
        let parameters = self.comma_list(Self::parse_function_parameter, |parser| {
            parser.peek_is_ctrl(')')
        })?;
        self.expect_ctrl(')')?;
        Some((parameters, self.span_from(start)))
    }

    /// One function parameter: `(own | & mut?)? binder (: type)?`. The convention is
    /// the explicit prefix, else inferred from a `&T` / `&mut T` type, else `Bare`.
    /// (Distinct from a closure parameter, which carries no convention.)
    fn parse_function_parameter(&mut self) -> Option<Parameter<'src>> {
        let prefix = if self.eat(&Token::Own) {
            Some(Convention::Own)
        } else if self.eat_op("&") {
            Some(if self.eat(&Token::Mut) {
                Convention::RefMut
            } else {
                Convention::Ref
            })
        } else {
            None
        };
        let (pattern, pattern_span) = self.parse_binder()?;
        let parameter_type = if self.eat_op(":") {
            Some(Box::new(self.parse_type()?))
        } else {
            None
        };
        let convention =
            prefix.unwrap_or_else(
                || match parameter_type.as_deref().map(|spanned| &spanned.0) {
                    Some(Node::Reference(true, _)) => Convention::RefMut,
                    Some(Node::Reference(false, _)) => Convention::Ref,
                    _ => Convention::Bare,
                },
            );
        Some((pattern, parameter_type, convention, pattern_span))
    }

    // --- Structs / enums -----------------------------------------------------

    /// `resource? external? struct (name | null) generics? ({ fields } | ;)`. The
    /// `resource` modifier sits in `external`'s position (canonical order `resource
    /// external struct`); the name may be the `null` keyword (the built-in `external
    /// struct null`); a bodyless `;` form is valid only for an `external` struct
    /// (checked past the parser).
    fn parse_struct(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let resource = self.eat(&Token::Resource);
        let external = self.eat(&Token::External);
        self.expect(&Token::Struct)?;
        let name_start = self.position;
        let name = if let Some(name) = self.eat_ident() {
            name
        } else if self.eat(&Token::Null) {
            "null"
        } else {
            return None;
        };
        let name = (name, self.span_from(name_start));
        let generic_parameters = self.parse_generic_parameters();
        let body = if self.peek_is_ctrl('{') {
            let fields_start = self.position;
            self.expect_ctrl('{')?;
            let fields =
                self.comma_list(Self::parse_struct_field, |parser| parser.peek_is_ctrl('}'))?;
            self.expect_ctrl('}')?;
            Some((fields, self.span_from(fields_start)))
        } else if self.eat_ctrl(';') {
            None
        } else {
            return None;
        };
        Some((
            Node::Struct(name, generic_parameters, external, resource, body),
            self.span_from(start),
        ))
    }

    /// `[expose]? name (: type)?` — one struct field, carrying the whole-field span
    /// (the inner name keeps its own span).
    fn parse_struct_field(&mut self) -> Option<Spanned<StructField<'src>>> {
        let start = self.position;
        let exposed = self.eat_marker_attribute("expose");
        let name_start = self.position;
        let name = self.eat_ident()?;
        let name = (name, self.span_from(name_start));
        let type_ = if self.eat_op(":") {
            Some(self.parse_type()?)
        } else {
            None
        };
        Some(((name, type_, exposed), self.span_from(start)))
    }

    /// `resource? enum name generics? { variants }`. There is no `external enum`, so
    /// `resource` is the only leading modifier.
    fn parse_enum(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let resource = self.eat(&Token::Resource);
        self.expect(&Token::Enum)?;
        let name_start = self.position;
        let name = self.eat_ident()?;
        let name = (name, self.span_from(name_start));
        let generic_parameters = self.parse_generic_parameters();
        let variants_start = self.position;
        self.expect_ctrl('{')?;
        let variants =
            self.comma_list(Self::parse_enum_variant, |parser| parser.peek_is_ctrl('}'))?;
        self.expect_ctrl('}')?;
        let variants = (variants, self.span_from(variants_start));
        Some((
            Node::Enum(name, generic_parameters, resource, variants),
            self.span_from(start),
        ))
    }

    /// One enum variant: `name (payload types)? (= discriminant)?`, carrying the
    /// whole-variant span.
    fn parse_enum_variant(&mut self) -> Option<Spanned<EnumVariant<'src>>> {
        let start = self.position;
        let name = self.eat_name()?;
        let data = self.attempt(|parser| {
            parser.expect_ctrl('(')?;
            let types = parser.comma_list(Self::parse_type, |parser| parser.peek_is_ctrl(')'))?;
            parser.expect_ctrl(')')?;
            Some(types)
        });
        let discriminant = self.parse_discriminant();
        Some((
            (name, data.unwrap_or_default(), discriminant),
            self.span_from(start),
        ))
    }

    /// `= (-)? integer` — an explicit enum discriminant, or `None` (backtracking)
    /// when no `=` follows. The magnitude is parsed as `i64` (0 on overflow,
    /// matching chumsky's `unwrap_or(0)`).
    fn parse_discriminant(&mut self) -> Option<i64> {
        self.attempt(|parser| {
            parser.expect_op("=")?;
            let negative = parser.eat_op("-");
            let whole = parser.eat_integer()?;
            let magnitude = whole.parse::<i64>().unwrap_or(0);
            Some(if negative { -magnitude } else { magnitude })
        })
    }

    // --- impl / trait / mod --------------------------------------------------

    /// `impl <subject> (with A + B)? { statements }`. The subject is a type (its
    /// `type X` binders declare the impl's generics); the optional `with` clause is
    /// the `+`-separated list of implemented traits.
    fn parse_impl(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Impl)?;
        let subject = self.parse_type()?;
        let traits = if self.eat(&Token::With) {
            self.parse_type_bounds()?
        } else {
            Vec::new()
        };
        let body = self.parse_item_body()?;
        Some((
            Node::Impl(Box::new(subject), traits, body),
            self.span_from(start),
        ))
    }

    /// `trait name generics? (with A + B)? { functions }`. The body is a list of
    /// function declarations only.
    fn parse_trait(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Trait)?;
        let name_start = self.position;
        let name = self.eat_ident()?;
        let name = (name, self.span_from(name_start));
        let generic_parameters = self.parse_generic_parameters();
        let supertraits = if self.eat(&Token::With) {
            self.parse_type_bounds()?
        } else {
            Vec::new()
        };
        let body = self.parse_trait_body()?;
        Some((
            Node::Trait(name, generic_parameters, supertraits, body),
            self.span_from(start),
        ))
    }

    /// `mod name { statements }` — a nested module.
    fn parse_module(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Mod)?;
        let name = self.eat_ident()?;
        let body = self.parse_item_body()?;
        Some((Node::Module(name, body), self.span_from(start)))
    }

    // --- import / use / export -----------------------------------------------

    /// `import <namespace_path>` (the node's span covers only `import <path>`; the
    /// statement-level `;` is consumed separately).
    fn parse_import(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Import)?;
        let path = self.parse_namespace_path()?;
        Some((Node::Import(path), self.span_from(start)))
    }

    /// `import <namespace_path> ;` — an import used as a statement.
    fn parse_import_statement(&mut self) -> Option<Spanned<Node<'src>>> {
        let import = self.parse_import()?;
        self.expect_ctrl(';')?;
        Some(import)
    }

    /// `use <namespace_path>` (the statement-level `;` is consumed separately).
    fn parse_use(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Use)?;
        let path = self.parse_namespace_path()?;
        Some((Node::Use(path), self.span_from(start)))
    }

    /// `use <namespace_path> ;` — a use used as a statement.
    fn parse_use_statement(&mut self) -> Option<Spanned<Node<'src>>> {
        let use_ = self.parse_use()?;
        self.expect_ctrl(';')?;
        Some(use_)
    }

    /// `export <statement>` — re-export an import or expose a declaration. The
    /// inner statement consumes its own terminator (so `export import a::b;`'s
    /// `Export` span includes the `;`, while the inner `Import` span does not).
    fn parse_export(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Export)?;
        let inner = self.parse_statement()?;
        Some((Node::Export(Box::new(inner)), self.span_from(start)))
    }

    /// A `::`-separated namespace path ending in a name or a `{ a, b }` set (H2) —
    /// the recursive `import`/`use` path grammar. A single path (a name with an
    /// optional `:: continuation`) is tried before a brace set, matching the chumsky
    /// `path.clone().or(set)`.
    fn parse_namespace_path(&mut self) -> Option<ImportBranch<'src>> {
        if let Some(path) = self.attempt(Self::parse_namespace_single_path) {
            return Some(path);
        }
        self.parse_namespace_set()
    }

    /// `name (:: branch)?` — one path in a namespace path (the chumsky `path`). The
    /// name's `ImportBranch::Path` span is the name token only; the continuation is
    /// a full [`Parser::parse_namespace_path`] (a further path or a set).
    fn parse_namespace_single_path(&mut self) -> Option<ImportBranch<'src>> {
        let start = self.position;
        let name = self.eat_name()?;
        let name_span = self.span_from(start);
        let continuation = if self.eat_op("::") {
            Some(Box::new(self.parse_namespace_path()?))
        } else {
            None
        };
        Some(ImportBranch::Path(name, name_span, continuation))
    }

    /// `{ path, ... }` — a brace-delimited set of paths (allow-trailing). Each
    /// element is a single path (chumsky's `path`, which must start with a name),
    /// so a nested bare set is not a legal element.
    fn parse_namespace_set(&mut self) -> Option<ImportBranch<'src>> {
        self.attempt(|parser| {
            parser.expect_ctrl('{')?;
            let paths = parser.comma_list(Self::parse_namespace_single_path, |parser| {
                parser.peek_is_ctrl('}')
            })?;
            parser.expect_ctrl('}')?;
            Some(ImportBranch::Set(paths))
        })
    }

    // --- Derive / service / macro-attribute items ----------------------------

    /// `[derive(A, B)] (struct | enum)` — a derive attribute wrapping a struct/enum.
    fn parse_derived_item(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let derives = self.parse_derive_attribute()?;
        let item = match self.attempt(Self::parse_struct) {
            Some(item) => item,
            None => self.attempt(Self::parse_enum)?,
        };
        Some((Node::Derive(derives, Box::new(item)), self.span_from(start)))
    }

    /// `[derive(A, B)]` — the derive trait names (with spans), allow-trailing; or
    /// `None` when no derive attribute leads.
    fn parse_derive_attribute(&mut self) -> Option<Vec<(&'src str, Span)>> {
        self.attempt(|parser| {
            parser.expect_ctrl('[')?;
            if parser.peek() != Some(&Token::Ident("derive")) {
                return None;
            }
            parser.bump();
            parser.expect_ctrl('(')?;
            let names =
                parser.comma_list(Self::parse_spanned_ident, |parser| parser.peek_is_ctrl(')'))?;
            parser.expect_ctrl(')')?;
            parser.expect_ctrl(']')?;
            Some(names)
        })
    }

    /// An identifier with its own span, for derive names.
    fn parse_spanned_ident(&mut self) -> Option<(&'src str, Span)> {
        let start = self.position;
        let name = self.eat_ident()?;
        Some((name, self.span_from(start)))
    }

    /// `[service(Client)?] struct …` — a service struct; the argument names the
    /// generated client type (default `<Struct>Client`).
    fn parse_service_item(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        let client_name = self.parse_service_attribute()?;
        let item = self.parse_struct()?;
        Some((
            Node::Service(client_name, Box::new(item)),
            self.span_from(start),
        ))
    }

    /// `[service(Name)?]` — a service attribute. The outer `Option` is whether this
    /// is a service attribute at all (`None` ⇒ not one); the inner `Option<&str>` is
    /// the optional `(Name)` client name.
    fn parse_service_attribute(&mut self) -> Option<Option<&'src str>> {
        self.attempt(|parser| {
            parser.expect_ctrl('[')?;
            if parser.peek() != Some(&Token::Ident("service")) {
                return None;
            }
            parser.bump();
            let client_name = parser.attempt(|parser| {
                parser.expect_ctrl('(')?;
                let name = parser.eat_ident()?;
                parser.expect_ctrl(')')?;
                Some(name)
            });
            parser.expect_ctrl(']')?;
            Some(client_name)
        })
    }

    /// `[<user-name>(args)?] (struct | enum | fun)` — a user macro attribute. The
    /// name must NOT be a known built-in marker (they keep their own parsers); the
    /// `(args)?` are OPTIONAL and captured as argument SPANS (source text is what the
    /// macro receives).
    fn parse_macro_attributed_item(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect_ctrl('[')?;
        let name_start = self.position;
        let name = match self.peek() {
            Some(Token::Ident(name)) if !is_known_attribute_marker(name) => *name,
            _ => return None,
        };
        self.bump();
        let name_span = self.span_from(name_start);
        let arguments = self
            .attempt(|parser| {
                parser.expect_ctrl('(')?;
                let arguments = parser
                    .comma_list(Self::parse_argument_span, |parser| parser.peek_is_ctrl(')'))?;
                parser.expect_ctrl(')')?;
                Some(arguments)
            })
            .unwrap_or_default();
        self.expect_ctrl(']')?;
        let item = match self.attempt(Self::parse_struct) {
            Some(item) => item,
            None => match self.attempt(Self::parse_enum) {
                Some(item) => item,
                None => self.attempt(Self::parse_function)?,
            },
        };
        Some((
            Node::MacroAttribute(name, name_span, arguments, Box::new(item)),
            self.span_from(start),
        ))
    }

    /// The SPAN of one macro-argument expression (its source text is what the macro
    /// receives — arguments are syntax). Parses a full expression, keeps its span.
    fn parse_argument_span(&mut self) -> Option<Span> {
        self.parse_expression().map(|(_, span)| span)
    }

    // --- Bracket attribute helpers -------------------------------------------

    /// `[ marker ]` — a bare marker attribute (`[must_use]`, `[rpc]`, `[trait_only]`,
    /// `[expose]`). Consumes it and returns `true` when the exact `[ marker ]` is
    /// next; leaves the cursor untouched and returns `false` otherwise.
    fn eat_marker_attribute(&mut self, marker: &str) -> bool {
        self.attempt(|parser| {
            parser.expect_ctrl('[')?;
            if parser.peek() != Some(&Token::Ident(marker)) {
                return None;
            }
            parser.bump();
            parser.expect_ctrl(']')?;
            Some(())
        })
        .is_some()
    }

    /// `[extern(args)]` — the host binding for the `external` function that follows,
    /// or `None` when no extern attribute leads. Args are bare words or quoted
    /// strings, interpreted by [`extern_binding_from_args`] (a malformed attribute
    /// lowers to an empty global symbol, exactly as the oracle does).
    fn parse_extern_attribute(&mut self) -> Option<ExternBinding<'src>> {
        self.attempt(|parser| {
            parser.expect_ctrl('[')?;
            if parser.peek() != Some(&Token::Ident("extern")) {
                return None;
            }
            parser.bump();
            parser.expect_ctrl('(')?;
            let args =
                parser.comma_list(Self::parse_extern_arg, |parser| parser.peek_is_ctrl(')'))?;
            parser.expect_ctrl(')')?;
            parser.expect_ctrl(']')?;
            Some(extern_binding_from_args(&args))
        })
    }

    /// One `[extern(..)]` argument: a bare word (`method`/`get`/`set`/`new`) or a
    /// quoted string (a module path or host symbol). Word is tried before Text,
    /// matching the chumsky choice.
    fn parse_extern_arg(&mut self) -> Option<ExternArg<'src>> {
        match self.peek() {
            Some(Token::Ident(word)) => {
                let word = *word;
                self.bump();
                Some(ExternArg::Word(word))
            }
            Some(Token::String(text)) => {
                let text = *text;
                self.bump();
                Some(ExternArg::Text(text))
            }
            _ => None,
        }
    }

    /// `[doc(hidden)]` — a tooling marker (omit from completion). Returns whether it
    /// is present.
    fn parse_doc_hidden_attribute(&mut self) -> bool {
        self.attempt(|parser| {
            parser.expect_ctrl('[')?;
            if parser.peek() != Some(&Token::Ident("doc")) {
                return None;
            }
            parser.bump();
            parser.expect_ctrl('(')?;
            if parser.peek() != Some(&Token::Ident("hidden")) {
                return None;
            }
            parser.bump();
            parser.expect_ctrl(')')?;
            parser.expect_ctrl(']')?;
            Some(())
        })
        .is_some()
    }

    /// `[platform("a", "b")]` — a platform fence (≥1 string patterns, allow-trailing),
    /// or `None` when no platform attribute leads.
    fn parse_platform_attribute(&mut self) -> Option<Vec<Spanned<&'src str>>> {
        self.attempt(|parser| {
            parser.expect_ctrl('[')?;
            if parser.peek() != Some(&Token::Ident("platform")) {
                return None;
            }
            parser.bump();
            parser.expect_ctrl('(')?;
            let patterns = parser.comma_list(Self::parse_platform_pattern, |parser| {
                parser.peek_is_ctrl(')')
            })?;
            if patterns.is_empty() {
                return None;
            }
            parser.expect_ctrl(')')?;
            parser.expect_ctrl(']')?;
            Some(patterns)
        })
    }

    /// One platform pattern: a quoted string with its span.
    fn parse_platform_pattern(&mut self) -> Option<Spanned<&'src str>> {
        let start = self.position;
        if let Some(Token::String(text)) = self.peek() {
            let text = *text;
            self.bump();
            Some((text, self.span_from(start)))
        } else {
            None
        }
    }

    // --- Macro forms ---------------------------------------------------------

    /// `macro fun name(..) { .. }` — a macro definition. The `function` production
    /// is reused and its `Node::Func` re-wrapped as `Node::MacroFun`.
    fn parse_macro_fun(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Macro)?;
        let (node, _) = self.parse_function()?;
        let node = match node {
            Node::Func(function) => Node::MacroFun(function),
            other => other,
        };
        Some((node, self.span_from(start)))
    }

    /// `macro { .. }` — an anonymous, immediately-expanded macro block. An atom
    /// (expression position) and a statement (via
    /// [`Parser::parse_macro_block_statement`]).
    fn parse_macro_block(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect(&Token::Macro)?;
            let body = parser.parse_block()?;
            Some((Node::MacroBlock(body), parser.span_from(start)))
        })
    }

    /// `macro { .. } ;?` — a macro block used as a statement (the `;` is OPTIONAL,
    /// unlike the mandatory `;` on an expression statement); the node's span
    /// excludes the `;`.
    fn parse_macro_block_statement(&mut self) -> Option<Spanned<Node<'src>>> {
        let macro_block = self.parse_macro_block()?;
        self.eat_ctrl(';');
        Some(macro_block)
    }

    /// `macro name(args)` — a macro invocation. An atom (expression position) and a
    /// statement (via [`Parser::parse_macro_invocation_statement`]). Arguments are
    /// captured as SPANS (their source text is what the macro receives).
    fn parse_macro_invocation(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect(&Token::Macro)?;
            let name_start = parser.position;
            let name = parser.eat_ident()?;
            let name_span = parser.span_from(name_start);
            parser.expect_ctrl('(')?;
            let arguments =
                parser.comma_list(Self::parse_argument_span, |parser| parser.peek_is_ctrl(')'))?;
            parser.expect_ctrl(')')?;
            Some((
                Node::MacroInvocation(name, name_span, arguments),
                parser.span_from(start),
            ))
        })
    }

    /// `macro name(args) ;?` — a macro invocation used as a statement (`;` OPTIONAL);
    /// the node's span excludes the `;`.
    fn parse_macro_invocation_statement(&mut self) -> Option<Spanned<Node<'src>>> {
        let invocation = self.parse_macro_invocation()?;
        self.eat_ctrl(';');
        Some(invocation)
    }

    /// `(binder in source => body)` — a tuple comprehension. The `in` distinguishes
    /// it from a tuple/group atom (and the `=>` from the mapped *type* `(U in T:
    /// F)`); `source` is a secondary expression, `body` a full expression.
    /// Backtracks when the `(binder in` shape is absent.
    fn parse_tuple_comprehension(&mut self) -> Option<Spanned<Node<'src>>> {
        self.attempt(|parser| {
            let start = parser.position;
            parser.expect_ctrl('(')?;
            let binder_start = parser.position;
            let binder = parser.eat_ident()?;
            let binder_span = parser.span_from(binder_start);
            parser.expect(&Token::In)?;
            let source = parser.parse_secondary(false)?;
            parser.expect_op("=>")?;
            let body = parser.parse_expression()?;
            parser.expect_ctrl(')')?;
            Some((
                Node::TupleComprehension {
                    binder,
                    binder_span,
                    source: Box::new(source),
                    body: Box::new(body),
                },
                parser.span_from(start),
            ))
        })
    }

    /// `resource` NOT followed by `external` / `struct` / `enum` — the misplaced-
    /// modifier steer (item 15 in the statement choice, after `struct`/`enum`, so a
    /// valid `resource struct` / `resource external struct` / `resource enum` is
    /// never shadowed). Emits a parse error and recovers to a `Node::Error` spanning
    /// the `resource` keyword, leaving the offending token unconsumed (so
    /// `fun`/`impl`/`let`/`trait` still parse on the next statement).
    fn parse_misplaced_resource(&mut self) -> Option<Spanned<Node<'src>>> {
        let start = self.position;
        self.expect(&Token::Resource)?;
        if matches!(
            self.peek(),
            Some(Token::External | Token::Struct | Token::Enum)
        ) {
            return None;
        }
        let span = self.span_from(start);
        self.errors.push(ParseError {
            span,
            expected: "`resource` may appear only before a `struct` or `enum` declaration",
        });
        Some((Node::Error, span))
    }

    /// The whole part of a `Number` token, consumed — the chumsky `integer`
    /// selector, used by tuple bounds, discriminants, and array lengths.
    fn eat_integer(&mut self) -> Option<&'src str> {
        if let Some(Token::Number(whole, _, _)) = self.peek() {
            let whole = *whole;
            self.bump();
            Some(whole)
        } else {
            None
        }
    }
}

/// Whether a node is a block-bearing form that may be a statement without a
/// trailing `;` (the chumsky `if_`/`for_`/`match_`/`block` statement alternatives).
fn is_block_like(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::If(_) | Node::For(..) | Node::ForIn(..) | Node::Match(..) | Node::Block(_)
    )
}

/// Apply one plain postfix to a subject, spanning from the chain's start. A
/// faithful copy of `chain_expr_parser::apply_postfix`.
fn apply_postfix<'src>(
    subject: Spanned<Node<'src>>,
    postfix: Postfix<'src>,
    end: usize,
) -> Spanned<Node<'src>> {
    let span: Span = (subject.1.start..end).into();
    match postfix {
        Postfix::Member(member) => (
            Node::MemberAccessor(Box::new(subject), Box::new(member)),
            span,
        ),
        Postfix::Index(index) => (Node::Index(Box::new(subject), Box::new(index)), span),
        Postfix::TryAssert => (Node::TryAssert(Box::new(subject)), span),
        Postfix::LiftBare => (Node::Lifted(Box::new(subject)), span),
        // Grouping is handled in the loop below; a `LiftMember` never reaches this
        // arm from `group_postfixes`, but the faithful construction is kept.
        Postfix::LiftMember(member) => (
            Node::Lift(
                Box::new(subject),
                Box::new((
                    Node::MemberAccessor(Box::new((Node::LiftBinder, member.1)), Box::new(member)),
                    span,
                )),
            ),
            span,
        ),
        Postfix::DirectCall(arguments) => (Node::Call(Box::new(subject), None, arguments), span),
    }
}

/// Group a collected postfix list onto its base, building each `?.` link's
/// continuation from the following plain postfixes (up to the next `?.`/`!`/chain
/// end). A faithful copy of `chain_expr_parser`'s member-accessor fold.
fn group_postfixes<'src>(
    base: Spanned<Node<'src>>,
    postfixes: Vec<(Postfix<'src>, Span)>,
) -> Spanned<Node<'src>> {
    let mut current = base;
    let mut items = postfixes.into_iter().peekable();
    while let Some((postfix, postfix_span)) = items.next() {
        match postfix {
            Postfix::LiftMember(member) => {
                let member_span = member.1;
                let mut continuation: Spanned<Node> = (
                    Node::MemberAccessor(
                        Box::new((Node::LiftBinder, member_span)),
                        Box::new(member),
                    ),
                    member_span,
                );
                let mut lift_end = postfix_span.end;
                while matches!(
                    items.peek(),
                    Some((
                        Postfix::Member(_) | Postfix::Index(_) | Postfix::DirectCall(_),
                        _
                    ))
                ) {
                    let (step, step_span) = items.next().unwrap();
                    lift_end = step_span.end;
                    continuation = apply_postfix(continuation, step, lift_end);
                }
                let span: Span = (current.1.start..lift_end).into();
                current = (Node::Lift(Box::new(current), Box::new(continuation)), span);
            }
            step => {
                current = apply_postfix(current, step, postfix_span.end);
            }
        }
    }
    current
}

#[cfg(test)]
mod tests {
    //! Durable pins for the S2 grammar. Unlike the differential (which retires at
    //! S5 with the chumsky oracle), these touch no chumsky and survive the cutover:
    //! they are the parser's own regression corpus for the shapes S2 introduced —
    //! precedence, associativity, the `?.` grouping, the split-shift reassembly, the
    //! §H.1 condition mode, and the type forms.

    use super::*;

    /// Parse `source` as a bare expression, asserting a clean full-consumption
    /// parse. Spans are at their natural source offsets (no wrapper).
    fn expr(source: &str) -> Spanned<Node<'_>> {
        let (tokens, errors) = lexing::tokenize(source);
        assert!(errors.is_empty(), "lex errors on {source:?}: {errors:?}");
        let mut parser = Parser::new(&tokens, source.len());
        let node = parser.parse_expression().expect("expression did not parse");
        assert_eq!(
            parser.position,
            tokens.len(),
            "unconsumed tokens parsing {source:?}: {node:?}"
        );
        node
    }

    /// Parse `source` as a condition-position expression (§H.1: struct-literal-free).
    fn condition(source: &str) -> Spanned<Node<'_>> {
        let (tokens, errors) = lexing::tokenize(source);
        assert!(errors.is_empty(), "lex errors on {source:?}: {errors:?}");
        let mut parser = Parser::new(&tokens, source.len());
        let node = parser.parse_condition().expect("condition did not parse");
        assert_eq!(
            parser.position,
            tokens.len(),
            "unconsumed parsing {source:?}"
        );
        node
    }

    /// Parse `source` as a type, asserting a clean full-consumption parse.
    fn type_(source: &str) -> Spanned<Node<'_>> {
        let (tokens, errors) = lexing::tokenize(source);
        assert!(errors.is_empty(), "lex errors on {source:?}: {errors:?}");
        let mut parser = Parser::new(&tokens, source.len());
        let node = parser.parse_type().expect("type did not parse");
        assert_eq!(
            parser.position,
            tokens.len(),
            "unconsumed parsing {source:?}"
        );
        node
    }

    /// Whether the whole-program entry declines `source` (a non-empty error list).
    fn declines(source: &str) -> bool {
        let (tree, errors) = parse(source);
        tree.is_none() || !errors.is_empty()
    }

    /// Parse a whole `source` file, asserting a clean parse (a tree and no errors),
    /// and return its statement list. The S3 whole-file entry — the one S2's seam
    /// could not reach.
    fn program(source: &str) -> Spanned<NodeList<'_>> {
        let (tree, errors) = parse(source);
        assert!(errors.is_empty(), "parse errors on {source:?}: {errors:?}");
        tree.expect("program did not parse")
    }

    /// The single top-level item's node, for the common one-item pins.
    fn only_item(source: &str) -> Node<'_> {
        let (mut statements, _span) = program(source);
        assert_eq!(statements.len(), 1, "expected one item in {source:?}");
        statements.remove(0).0
    }

    // --- Precedence and associativity (full span-inclusive Debug) ------------

    #[test]
    fn product_binds_tighter_than_sum() {
        assert_eq!(
            format!("{:?}", expr("a + b * c")),
            "(Binary(Add, (Accessor(\"a\"), 0..1), \
             (Binary(Mul, (Accessor(\"b\"), 4..5), (Accessor(\"c\"), 8..9)), 4..9)), 0..9)"
        );
    }

    #[test]
    fn subtraction_is_left_associative() {
        assert_eq!(
            format!("{:?}", expr("a - b - c")),
            "(Binary(Sub, (Binary(Sub, (Accessor(\"a\"), 0..1), (Accessor(\"b\"), 4..5)), 0..5), \
             (Accessor(\"c\"), 8..9)), 0..9)"
        );
    }

    #[test]
    fn bitand_binds_tighter_than_bitor_which_binds_tighter_than_compare() {
        // `a & b == c | d` — Rust's order: `&` over `|`, both over `==`.
        match &expr("a & b == c | d").0 {
            Node::Binary(BinaryOp::Eq, left, right) => {
                assert!(matches!(left.0, Node::Binary(BinaryOp::BitAnd, _, _)));
                assert!(matches!(right.0, Node::Binary(BinaryOp::BitOr, _, _)));
            }
            other => panic!("expected Eq at root, got {other:?}"),
        }
    }

    #[test]
    fn logical_and_binds_tighter_than_logical_or() {
        match &expr("a && b || c").0 {
            Node::Binary(BinaryOp::Or, left, _) => {
                assert!(matches!(left.0, Node::Binary(BinaryOp::And, _, _)));
            }
            other => panic!("expected Or at root, got {other:?}"),
        }
    }

    #[test]
    fn is_binds_tighter_than_logical_and() {
        // `x is None && ready` — the `is` groups with `x`, not with `ready`.
        match &expr("x is None && ready").0 {
            Node::Binary(BinaryOp::And, left, right) => {
                assert!(matches!(left.0, Node::Is(_, _)));
                assert!(matches!(right.0, Node::Accessor("ready")));
            }
            other => panic!("expected And at root, got {other:?}"),
        }
    }

    // --- The split-shift reassembly -----------------------------------------

    #[test]
    fn adjacent_angle_pair_is_a_shift() {
        assert_eq!(
            format!("{:?}", expr("a << b")),
            "(Binary(Shl, (Accessor(\"a\"), 0..1), (Accessor(\"b\"), 5..6)), 0..6)"
        );
        match &expr("a >> b").0 {
            Node::Binary(BinaryOp::Shr, _, _) => {}
            other => panic!("expected Shr, got {other:?}"),
        }
    }

    #[test]
    fn non_adjacent_angle_pair_is_not_a_shift() {
        // `a < < b` (a space between the angles) is neither a shift nor a valid
        // comparison — it does not fully parse.
        assert!(declines("let __probe = a < < b;"));
        // A lone `<` stays a comparison.
        match &expr("a < b").0 {
            Node::Binary(BinaryOp::Lt, _, _) => {}
            other => panic!("expected Lt, got {other:?}"),
        }
    }

    #[test]
    fn shift_binds_tighter_than_bitor_but_looser_than_sum() {
        // `a + b << c | d` — `+` over `<<` over `|`.
        match &expr("a + b << c | d").0 {
            Node::Binary(BinaryOp::BitOr, left, _) => match &left.0 {
                Node::Binary(BinaryOp::Shl, shift_left, _) => {
                    assert!(matches!(shift_left.0, Node::Binary(BinaryOp::Add, _, _)));
                }
                other => panic!("expected Shl under BitOr, got {other:?}"),
            },
            other => panic!("expected BitOr at root, got {other:?}"),
        }
    }

    // --- The postfix / `?.` grouping ----------------------------------------

    #[test]
    fn lift_link_absorbs_following_plain_postfixes() {
        // `a?.b.c` — the `.c` joins the `?.b` link's continuation.
        match &expr("a?.b.c").0 {
            Node::Lift(subject, continuation) => {
                assert!(matches!(subject.0, Node::Accessor("a")));
                // continuation = (LiftBinder.b).c
                match &continuation.0 {
                    Node::MemberAccessor(inner, member) => {
                        assert!(matches!(member.0, Node::Accessor("c")));
                        assert!(matches!(inner.0, Node::MemberAccessor(_, _)));
                    }
                    other => panic!("expected nested MemberAccessor continuation, got {other:?}"),
                }
            }
            other => panic!("expected Lift, got {other:?}"),
        }
    }

    #[test]
    fn try_assert_stops_the_lift_continuation() {
        // `a?.b!` — the `!` applies to the LIFTED result, not inside the link.
        match &expr("a?.b!").0 {
            Node::TryAssert(inner) => assert!(matches!(inner.0, Node::Lift(_, _))),
            other => panic!("expected TryAssert wrapping a Lift, got {other:?}"),
        }
    }

    #[test]
    fn chained_lifts_nest() {
        // `a?.b?.c` — Lift(Lift(a, .b), .c).
        match &expr("a?.b?.c").0 {
            Node::Lift(subject, _) => assert!(matches!(subject.0, Node::Lift(_, _))),
            other => panic!("expected nested Lift, got {other:?}"),
        }
    }

    #[test]
    fn bare_question_is_a_lift_mark_and_a_group_records_it() {
        assert!(matches!(expr("a?").0, Node::Lifted(_)));
        // Parens containing a mark become a LiftGroup; otherwise they dissolve.
        assert!(matches!(expr("(a?)").0, Node::LiftGroup(_)));
        assert!(matches!(expr("(a)").0, Node::Accessor("a")));
    }

    #[test]
    fn direct_call_folds_onto_a_method_result() {
        // `self.hook.read()(a)` — the trailing `(a)` is a DirectCall on the member
        // result (backlog §H.18).
        match &expr("self.hook.read()(a)").0 {
            Node::Call(callee, None, _) => assert!(matches!(callee.0, Node::MemberAccessor(_, _))),
            other => panic!("expected outer Call over a MemberAccessor, got {other:?}"),
        }
    }

    #[test]
    fn member_call_fuses_one_call_then_leaves_the_rest() {
        // `a.method<T>(x)` — the member is a single fused generic call.
        match &expr("a.method<T>(x)").0 {
            Node::MemberAccessor(_, member) => match &member.0 {
                Node::Call(_, Some(_), _) => {}
                other => panic!("expected a generic Call member, got {other:?}"),
            },
            other => panic!("expected MemberAccessor, got {other:?}"),
        }
    }

    #[test]
    fn tuple_index_reads_a_number_member() {
        match &expr("a.0").0 {
            Node::MemberAccessor(_, member) => {
                assert!(matches!(member.0, Node::Number("0", None, None)))
            }
            other => panic!("expected MemberAccessor with a Number member, got {other:?}"),
        }
    }

    // --- Static paths and generics ------------------------------------------

    #[test]
    fn generic_static_head_needs_a_trailing_colon_colon() {
        // `List<str>::new()` — the head keeps its generics because `::` follows.
        match &expr("List<str>::new()").0 {
            Node::Call(callee, None, _) => match &callee.0 {
                Node::StaticAccessor(head, "new") => {
                    assert!(matches!(head.0, Node::AccessorWithGenerics("List", _)))
                }
                other => panic!("expected StaticAccessor over generics, got {other:?}"),
            },
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn generic_call_without_colon_colon_attaches_to_the_call() {
        // `default<Id>()` — the bare name wins the atom, the generics go to the call.
        match &expr("default<Id>()").0 {
            Node::Call(callee, Some(_), _) => {
                assert!(matches!(callee.0, Node::Accessor("default")))
            }
            other => panic!("expected a generic Call over a bare name, got {other:?}"),
        }
    }

    #[test]
    fn bare_generic_lookalike_is_a_comparison() {
        // `foo<T>` with no `::` and no `(` does NOT form generics — the `<` is a
        // comparison. As a whole expression the `>` is left dangling, so only the
        // expression prefix is checked here; the generics-attach-to-a-call contrast
        // is `default<Id>()` above.
        let (tokens, errors) = lexing::tokenize("foo<T>");
        assert!(errors.is_empty());
        let mut parser = Parser::new(&tokens, "foo<T>".len());
        let node = parser.parse_expression().expect("prefix parses");
        assert!(matches!(node.0, Node::Binary(BinaryOp::Lt, _, _)));
    }

    // --- Unary --------------------------------------------------------------

    #[test]
    fn unary_stacks_and_binds_tighter_than_member() {
        assert!(matches!(expr("!!x").0, Node::Unary('!', _)));
        // `-a.b` is `-(a.b)` — unary binds looser than the member chain.
        match &expr("-a.b").0 {
            Node::Unary('-', inner) => assert!(matches!(inner.0, Node::MemberAccessor(_, _))),
            other => panic!("expected Unary('-') over a member, got {other:?}"),
        }
    }

    #[test]
    fn reference_and_dereference() {
        assert!(matches!(expr("&mut x").0, Node::Reference(true, _)));
        assert!(matches!(expr("&x").0, Node::Reference(false, _)));
        assert!(matches!(expr("*x").0, Node::Dereference(_)));
    }

    #[test]
    fn async_takes_a_block_or_a_unary() {
        match &expr("async { f() }").0 {
            Node::Async(inner) => assert!(matches!(inner.0, Node::Block(_))),
            other => panic!("expected Async(Block), got {other:?}"),
        }
        match &expr("async f()").0 {
            Node::Async(inner) => assert!(matches!(inner.0, Node::Call(_, _, _))),
            other => panic!("expected Async(Call), got {other:?}"),
        }
    }

    // --- §H.1 condition mode -------------------------------------------------

    #[test]
    fn condition_mode_excludes_struct_literals() {
        // In condition position a bare name is an accessor; the `{` after it is a
        // block, not a struct literal — so `Foo` alone parses to `Accessor`.
        assert!(matches!(condition("Foo").0, Node::Accessor("Foo")));
        // In expression position the same head with a brace IS a struct literal.
        assert!(matches!(
            expr("Foo { x = 1 }").0,
            Node::StructInitializer("Foo", _, _)
        ));
        // A parenthesised struct literal is admitted even in a condition.
        assert!(matches!(
            condition("(Foo { x = 1 })").0,
            Node::StructInitializer(..)
        ));
    }

    // --- Assignment ----------------------------------------------------------

    #[test]
    fn assignment_targets_and_operators() {
        assert!(matches!(expr("x = 5").0, Node::Assign(_, None, _)));
        assert!(matches!(
            expr("x += 1").0,
            Node::Assign(_, Some(BinaryOp::Add), _)
        ));
        match &expr("*p = 5").0 {
            Node::Assign(target, None, _) => assert!(matches!(target.0, Node::Dereference(_))),
            other => panic!("expected Assign over a Dereference target, got {other:?}"),
        }
    }

    // --- Closures ------------------------------------------------------------

    #[test]
    fn closures_parse_params_return_type_and_body() {
        assert!(matches!(expr("|| 0").0, Node::Closure(_)));
        match &expr("|x: i32|: i32 x + 1").0 {
            Node::Closure(closure) => {
                assert_eq!(closure.parameters.0.len(), 1);
                assert!(closure.return_type.is_some());
                assert!(matches!(
                    closure.return_value.0,
                    Node::Binary(BinaryOp::Add, _, _)
                ));
            }
            other => panic!("expected Closure, got {other:?}"),
        }
    }

    // --- match / if / blocks -------------------------------------------------

    #[test]
    fn match_legs_patterns_guards_and_or_patterns() {
        match &expr("match x { let a if a > 0 => a, Some(let b), None => b }").0 {
            Node::Match(_, legs) => {
                assert_eq!(legs.0.len(), 2);
                assert!(legs.0[0].1.is_some(), "first leg has a guard");
                assert_eq!(legs.0[1].0.len(), 2, "second leg is an or-pattern");
            }
            other => panic!("expected Match, got {other:?}"),
        }
    }

    #[test]
    fn block_takes_statements_then_a_trailing_value() {
        match &expr("{ let x = 1; x }").0 {
            Node::Block(body) => {
                assert_eq!(body.0.0.len(), 1, "one statement");
                assert!(matches!(body.0.1.0, Node::Accessor("x")), "trailing value");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn empty_block_value_is_void_at_the_closing_brace() {
        match &expr("{ }").0 {
            Node::Block(body) => assert!(matches!(body.0.1.0, Node::Void)),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn if_else_if_chains_nest_in_the_else_branch() {
        match &expr("if a { 1 } else if b { 2 } else { 3 }").0 {
            Node::If(NodeIfBranch::If(if_)) => match &if_.else_ {
                Some((NodeIfBranch::If(_), _)) => {}
                other => panic!("expected an else-if branch, got {other:?}"),
            },
            other => panic!("expected If, got {other:?}"),
        }
    }

    // --- Types ---------------------------------------------------------------

    #[test]
    fn reference_array_and_local_types() {
        assert_eq!(
            format!("{:?}", type_("&mut T")),
            "(Reference(true, (Accessor(\"T\"), 5..6)), 0..6)"
        );
        assert!(matches!(type_("[i32; 4]").0, Node::ArrayType(_, _)));
        assert!(matches!(
            type_("List<T>").0,
            Node::AccessorWithGenerics("List", _)
        ));
    }

    #[test]
    fn closure_types_have_no_arrow_and_take_markers() {
        // `|A| B` — the return type follows the params directly (no arrow).
        match &type_("|A| B").0 {
            Node::ClosureType(parameters, Some(_)) => assert_eq!(parameters.0.len(), 1),
            other => panic!("expected ClosureType with a return, got {other:?}"),
        }
        assert!(matches!(type_("async || T").0, Node::AsyncType(_)));
        assert!(matches!(type_("sync || T").0, Node::SyncType(_)));
        // A bare `sync` is still a type name (the marker only bites before `|`).
        assert!(matches!(type_("sync").0, Node::Accessor("sync")));
    }

    #[test]
    fn mapped_tuple_and_context_types() {
        assert!(matches!(type_("(U in T: F<U>)").0, Node::MappedType { .. }));
        assert!(matches!(type_("(A, B)").0, Node::Tuple(_)));
        assert!(
            matches!(type_("(A)").0, Node::Tuple(_)),
            "a one-tuple, not a group"
        );
        match &type_("Foo context bar").0 {
            Node::TypeWithContexts(_, contexts) => assert_eq!(contexts.len(), 1),
            other => panic!("expected TypeWithContexts, got {other:?}"),
        }
    }

    // --- Decline behaviour (never more permissive than the grammar) ----------

    #[test]
    fn trailing_comma_tuple_and_single_tuple_decline() {
        assert!(declines("let __probe = (a, b,);"));
        assert!(declines("let __probe = (a,);"));
    }

    // --- S3 items: functions -------------------------------------------------

    #[test]
    fn function_signature_only_has_no_body() {
        // A `;` body (a required trait method / external declaration) is `None`.
        match only_item("fun default(): Self;") {
            Node::Func(function) => {
                assert_eq!(function.name.0, "default");
                assert!(function.body.is_none(), "signature-only body is None");
                assert!(function.return_type.is_some());
            }
            other => panic!("expected Func, got {other:?}"),
        }
    }

    #[test]
    fn function_generics_conventions_and_borrows() {
        // Generics, an `own`/`&`/`&mut`/bare mix of conventions, a return type, and
        // a `borrows` clause — every function-signature axis in one pin.
        match only_item(
            "fun slot<T>(&mut self, own value: T, plain: i32): &mut T borrows self { self }",
        ) {
            Node::Func(function) => {
                let generics = function.generic_parameters.as_ref().expect("generics");
                assert_eq!(generics.0.len(), 1);
                let conventions: Vec<Convention> = function
                    .parameters
                    .0
                    .iter()
                    .map(|(_, _, convention, _)| *convention)
                    .collect();
                assert_eq!(
                    conventions,
                    vec![Convention::RefMut, Convention::Own, Convention::Bare]
                );
                assert_eq!(function.borrows, Some("self"));
                assert!(function.body.is_some());
            }
            other => panic!("expected Func, got {other:?}"),
        }
    }

    #[test]
    fn parameter_convention_is_inferred_from_a_reference_type() {
        // No prefix, but a `&mut T` type gives `RefMut`; a `&T` type gives `Ref`.
        match only_item("fun f(a: &mut i32, b: &i32, c: i32) { }") {
            Node::Func(function) => {
                let conventions: Vec<Convention> = function
                    .parameters
                    .0
                    .iter()
                    .map(|(_, _, convention, _)| *convention)
                    .collect();
                assert_eq!(
                    conventions,
                    vec![Convention::RefMut, Convention::Ref, Convention::Bare]
                );
            }
            other => panic!("expected Func, got {other:?}"),
        }
    }

    #[test]
    fn async_fun_is_a_function_not_an_expression() {
        // `async fun` is an item (the expression attempt fails on `fun`), unlike
        // `async { .. }` / `async expr`, which are expressions.
        match only_item("async fun tick() { }") {
            Node::Func(function) => {
                assert!(function.is_async);
                assert!(!function.external);
            }
            other => panic!("expected an async Func, got {other:?}"),
        }
        // `external fun` is a bodyless intrinsic.
        match only_item("external fun print(line: str);") {
            Node::Func(function) => {
                assert!(function.external);
                assert!(function.body.is_none());
            }
            other => panic!("expected an external Func, got {other:?}"),
        }
    }

    // --- S3 items: structs / enums -------------------------------------------

    #[test]
    fn struct_fields_generics_and_modifiers() {
        match only_item("struct Point<T> { x: T, y: T }") {
            Node::Struct(name, generics, external, resource, body) => {
                assert_eq!(name.0, "Point");
                assert!(generics.is_some());
                assert!(!external && !resource);
                assert_eq!(body.expect("fields").0.len(), 2);
            }
            other => panic!("expected Struct, got {other:?}"),
        }
        // `resource external struct null;` — every modifier, the `null` name, the
        // bodyless `;` form.
        match only_item("resource external struct null;") {
            Node::Struct(name, _, external, resource, body) => {
                assert_eq!(name.0, "null");
                assert!(external && resource);
                assert!(body.is_none());
            }
            other => panic!("expected Struct, got {other:?}"),
        }
    }

    #[test]
    fn exposed_struct_field_is_recorded() {
        match only_item("struct Room { [expose] count: Signal, name: str }") {
            Node::Struct(_, _, _, _, Some(fields)) => {
                let exposed: Vec<bool> = fields.0.iter().map(|field| field.0.2).collect();
                assert_eq!(exposed, vec![true, false]);
            }
            other => panic!("expected a struct with fields, got {other:?}"),
        }
    }

    #[test]
    fn enum_variants_payloads_and_discriminants() {
        match only_item("enum Sign { Less = -1, Zero = 0, More(i32, str) }") {
            Node::Enum(name, _, resource, variants) => {
                assert_eq!(name.0, "Sign");
                assert!(!resource);
                let (_, less_data, less_disc) = &variants.0[0].0;
                assert!(less_data.is_empty());
                assert_eq!(*less_disc, Some(-1));
                let (_, more_data, more_disc) = &variants.0[2].0;
                assert_eq!(more_data.len(), 2, "More carries two payload types");
                assert_eq!(*more_disc, None);
            }
            other => panic!("expected Enum, got {other:?}"),
        }
        // `resource enum` — the only leading modifier on an enum.
        match only_item("resource enum Handle { Open, Closed }") {
            Node::Enum(_, _, resource, _) => assert!(resource),
            other => panic!("expected a resource Enum, got {other:?}"),
        }
    }

    #[test]
    fn generic_parameter_bounds_and_tuple_bounds() {
        // A trait-bound list, a defaulted binder, and a tuple-arity bound.
        match only_item("fun f<A: Show + Eq, type B = Self, C: (2..: Display)>() { }") {
            Node::Func(function) => {
                let generics = &function.generic_parameters.as_ref().unwrap().0;
                assert_eq!(generics[0].bounds.len(), 2, "A: Show + Eq");
                assert!(
                    generics[1].is_type && generics[1].default.is_some(),
                    "type B = Self"
                );
                let tuple_bound = generics[2].tuple_bound.as_ref().expect("C tuple bound");
                assert_eq!(tuple_bound.lo, Some(2));
                assert_eq!(tuple_bound.hi, None);
                assert!(tuple_bound.element.is_some(), "(..: Display) element bound");
            }
            other => panic!("expected Func, got {other:?}"),
        }
    }

    // --- S3 items: impl / trait / mod ----------------------------------------

    #[test]
    fn impl_with_clause_and_body() {
        match only_item("impl Point<type T> with Show + Eq { fun show(&self): str { \"p\" } }") {
            Node::Impl(_subject, traits, body) => {
                assert_eq!(traits.len(), 2, "with Show + Eq");
                assert_eq!(body.0.len(), 1, "one method");
                assert!(matches!(body.0[0].0, Node::Func(_)));
            }
            other => panic!("expected Impl, got {other:?}"),
        }
    }

    #[test]
    fn trait_body_holds_declarations_and_default_members() {
        // A signature-only declaration and a defaulted method, plus a supertrait.
        match only_item(
            "trait Ord<T> with Eq { fun cmp(&self, other: &T): i32; fun max(&self): i32 { 0 } }",
        ) {
            Node::Trait(name, generics, supertraits, body) => {
                assert_eq!(name.0, "Ord");
                assert!(generics.is_some());
                assert_eq!(supertraits.len(), 1);
                assert_eq!(body.0.len(), 2);
                let bodies: Vec<bool> = body
                    .0
                    .iter()
                    .map(|member| match &member.0 {
                        Node::Func(function) => function.body.is_some(),
                        other => panic!("trait member is not a Func: {other:?}"),
                    })
                    .collect();
                assert_eq!(bodies, vec![false, true], "decl then default");
            }
            other => panic!("expected Trait, got {other:?}"),
        }
    }

    #[test]
    fn module_nests_items() {
        match only_item(
            "mod geometry { struct Point { x: i32 } fun origin(): Point { Point { x = 0 } } }",
        ) {
            Node::Module(name, body) => {
                assert_eq!(name, "geometry");
                assert_eq!(body.0.len(), 2);
            }
            other => panic!("expected Module, got {other:?}"),
        }
    }

    // --- S3 items: import / use / export -------------------------------------

    #[test]
    fn import_recursive_path_and_brace_set() {
        // `std::collections::{ Map, Set }` — a `::` path ending in a set.
        match only_item("import std::collections::{ Map, Set };") {
            Node::Import(ImportBranch::Path("std", _, Some(next))) => match &*next {
                ImportBranch::Path("collections", _, Some(set)) => match &**set {
                    ImportBranch::Set(members) => assert_eq!(members.len(), 2),
                    other => panic!("expected a Set continuation, got {other:?}"),
                },
                other => panic!("expected a nested path, got {other:?}"),
            },
            other => panic!("expected Import(Path), got {other:?}"),
        }
    }

    #[test]
    fn use_bare_path_and_export_reexport() {
        assert!(matches!(
            only_item("use option::Some;"),
            Node::Use(ImportBranch::Path("option", _, Some(_)))
        ));
        // `export import a::b;` — the inner import consumes its own `;`; the Export
        // wraps it (and its span, tested via the differential, includes the `;`).
        match only_item("export import shared::config;") {
            Node::Export(inner) => assert!(matches!(inner.0, Node::Import(_))),
            other => panic!("expected Export, got {other:?}"),
        }
    }

    #[test]
    fn top_level_let_and_mut_bindings() {
        assert!(matches!(only_item("let answer = 42;"), Node::Let(..)));
        assert!(matches!(only_item("mut total = 0;"), Node::Let(..)));
    }

    // --- S3 attribute / macro forms ------------------------------------------

    #[test]
    fn derive_and_service_attributes_wrap_their_item() {
        match only_item("[derive(Show, Eq)] struct P { x: i32 }") {
            Node::Derive(derives, item) => {
                let names: Vec<&str> = derives.iter().map(|(name, _)| *name).collect();
                assert_eq!(names, vec!["Show", "Eq"]);
                assert!(matches!(item.0, Node::Struct(..)));
            }
            other => panic!("expected Derive, got {other:?}"),
        }
        // `[service(Client)] struct` names its generated client type.
        match only_item("[service(RoomClient)] struct Room { }") {
            Node::Service(Some("RoomClient"), item) => assert!(matches!(item.0, Node::Struct(..))),
            other => panic!("expected Service(Some), got {other:?}"),
        }
        // Bare `[service]` defaults the client name to `None`.
        assert!(matches!(
            only_item("[service] struct Room { }"),
            Node::Service(None, _)
        ));
    }

    #[test]
    fn function_attributes_are_recognized_in_fixed_order() {
        // The full ordered attribute prefix (`extern`, `must_use`, `rpc`,
        // `trait_only`, `doc(hidden)`, `platform`) on one external function.
        match only_item(
            "[extern(\"node:http\", \"createServer\")] [must_use] [rpc] [trait_only] [doc(hidden)] [platform(\"@process\")] external fun serve();",
        ) {
            Node::Func(function) => {
                assert!(matches!(
                    function.extern_binding,
                    Some(ExternBinding::Function {
                        module: Some("node:http"),
                        symbol: "createServer"
                    })
                ));
                assert!(
                    function.must_use && function.rpc && function.trait_only && function.doc_hidden
                );
                assert_eq!(function.platform_fence.len(), 1);
                assert!(function.external);
            }
            other => panic!("expected a fully-attributed Func, got {other:?}"),
        }
    }

    #[test]
    fn function_attributes_out_of_order_decline() {
        // `[must_use]` must precede `[rpc]` (the chumsky attribute chain is ordered):
        // `[rpc] [must_use] fun` is NOT a function, and no other alternative claims
        // it, so the whole program declines.
        assert!(declines("[rpc] [must_use] fun f() { }"));
    }

    #[test]
    fn bracket_attribute_vs_list_literal_disambiguation() {
        // `[must_use] fun` is a function (the list-literal expression reading fails
        // for want of a `;`, then `function` claims it).
        assert!(matches!(only_item("[must_use] fun f() { }"), Node::Func(_)));
        // A genuine list-literal statement (`[a, b];`) is still an expression.
        assert!(matches!(only_item("[a, b];"), Node::List(_)));
        // A user macro attribute (name NOT a known marker) wraps its item.
        match only_item("[route(\"/\", get)] fun index() { }") {
            Node::MacroAttribute(name, _, arguments, item) => {
                assert_eq!(name, "route");
                assert_eq!(arguments.len(), 2, "argument SPANS");
                assert!(matches!(item.0, Node::Func(_)));
            }
            other => panic!("expected MacroAttribute, got {other:?}"),
        }
        // A bare user attribute with no args wraps too.
        assert!(matches!(
            only_item("[handler] struct H { }"),
            Node::MacroAttribute("handler", _, _, _)
        ));
    }

    #[test]
    fn macro_definition_block_and_invocation_forms() {
        // `macro fun` is a definition.
        assert!(matches!(
            only_item("macro fun expand(): Source { source(\"\") }"),
            Node::MacroFun(_)
        ));
        // `macro { .. }` is a block; the statement `;` is optional (both parse).
        assert!(matches!(
            only_item("macro { ret void }"),
            Node::MacroBlock(_)
        ));
        assert!(matches!(
            only_item("macro { ret void };"),
            Node::MacroBlock(_)
        ));
        // `macro name(args)` is an invocation, `;` optional; arguments are SPANS.
        match only_item("macro define(a, b + 1)") {
            Node::MacroInvocation(name, _, arguments) => {
                assert_eq!(name, "define");
                assert_eq!(arguments.len(), 2);
            }
            other => panic!("expected MacroInvocation, got {other:?}"),
        }
        assert!(matches!(
            only_item("macro define(a);"),
            Node::MacroInvocation(..)
        ));
    }

    #[test]
    fn macro_invocation_in_expression_position_is_an_atom() {
        // Anywhere but statement head, `macro name(args)` is an expression atom.
        match &expr("x + macro pick(a)").0 {
            Node::Binary(BinaryOp::Add, _, right) => {
                assert!(matches!(right.0, Node::MacroInvocation(..)));
            }
            other => panic!("expected a macro invocation operand, got {other:?}"),
        }
    }

    #[test]
    fn tuple_comprehension_atom_parses() {
        // `(x in xs => e)` — the deferred S2 atom, now live.
        match &expr("(x in items => x + 1)").0 {
            Node::TupleComprehension { binder, .. } => assert_eq!(*binder, "x"),
            other => panic!("expected TupleComprehension, got {other:?}"),
        }
        // The `in` is what forks it from a group / tuple — `(a + b)` still dissolves.
        assert!(matches!(
            expr("(a + b)").0,
            Node::Binary(BinaryOp::Add, _, _)
        ));
    }

    // --- S3 statement interleaving + the resource steer ----------------------

    #[test]
    fn misplaced_resource_declines_but_a_valid_resource_declaration_parses() {
        // `resource` before a non-declaration is the steer (an error) — declines.
        assert!(declines("resource fun f() { }"));
        assert!(declines("resource impl Foo { }"));
        // But `resource struct` / `resource external struct` / `resource enum` are
        // valid and parse cleanly (the steer never shadows them).
        assert!(matches!(
            only_item("resource struct File { }"),
            Node::Struct(_, _, false, true, _)
        ));
        assert!(matches!(
            only_item("resource enum State { A, B }"),
            Node::Enum(_, _, true, _)
        ));
    }

    #[test]
    fn a_block_bearing_form_is_a_statement_only_when_not_block_last() {
        // Inside a module body, `fun a` then `fun b` — two statements, no trailing
        // expression (an item body has none).
        match only_item("mod m { fun a() { } fun b() { } }") {
            Node::Module(_, body) => assert_eq!(body.0.len(), 2),
            other => panic!("expected Module, got {other:?}"),
        }
    }

    #[test]
    fn a_whole_file_is_a_sequence_of_items() {
        let (statements, _span) = program(
            "import std::io;\n\
             struct Point { x: i32, y: i32 }\n\
             [derive(Show)] enum Dir { N, S }\n\
             fun main() { print(\"hi\") }\n",
        );
        assert_eq!(statements.len(), 4);
        assert!(matches!(statements[0].0, Node::Import(_)));
        assert!(matches!(statements[1].0, Node::Struct(..)));
        assert!(matches!(statements[2].0, Node::Derive(..)));
        assert!(matches!(statements[3].0, Node::Func(_)));
    }
}
