
use chumsky::{input::ValueInput, prelude::*};
use std::{collections::HashMap};
use crate::{lexer::Token, shared::{Error, Span, Spanned}};

#[derive(Clone, Debug, PartialEq)]
pub enum Value<'src> {
	Null,
	Bool(bool),
	Num(f64),
	Str(&'src str),
	List(Vec<Self>),
	Func(&'src str),
}

impl Value<'_> {
	pub fn num(self, span: Span) -> Result<f64, Error> {
		if let Value::Num(x) = self {
			Ok(x)
		} else {
			Err(Error {
				span,
				msg: format!("'{self}' is not a number"),
			})
		}
	}
}

impl std::fmt::Display for Value<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			Self::Null => write!(f, "null"),
			Self::Bool(x) => write!(f, "{x}"),
			Self::Num(x) => write!(f, "{x}"),
			Self::Str(x) => write!(f, "{x}"),
			Self::List(xs) => write!(
				f,
				"[{}]",
				xs.iter()
					.map(|x| x.to_string())
					.collect::<Vec<_>>()
					.join(", ")
			),
			Self::Func(name) => write!(f, "<function: {name}>"),
		}
	}
}

#[derive(Clone, Debug)]
pub enum BinaryOp {
	Add,
	Sub,
	Mul,
	Div,
	Eq,
	NotEq,
}

// pub struct ImportPart<'src> (&'src str, Option<Box<ImportPart<'src>>>);

// An expression node in the AST. Children are spanned so we can generate useful runtime errors.
#[derive(Debug)]
pub enum Node<'src> {
	Error,
	Value(Value<'src>),
	List(Vec<Spanned<Self>>),
	Local(&'src str),
	Import(Vec<Spanned<Vec<&'src str>>>),
	Let(&'src str, Box<Spanned<Self>>),
	Then(Box<Spanned<Self>>, Box<Spanned<Self>>),
	Binary(Box<Spanned<Self>>, BinaryOp, Box<Spanned<Self>>),
	Call(Box<Spanned<Self>>, Spanned<Vec<Spanned<Self>>>),
	If(Box<Spanned<Self>>, Box<Spanned<Self>>, Box<Spanned<Self>>),
	Print(Box<Spanned<Self>>),
}

// A function node in the AST.
#[derive(Debug)]
pub struct Func<'src> {
	pub args: Vec<&'src str>,
	pub span: Span,
	pub body: Spanned<Node<'src>>,
}

// // An import node in the AST.
// #[derive(Debug)]
// pub struct Import<'src> {
// 	pub subject: &'src str,
// 	pub span: Span,
// }

pub fn expr_parser<'tokens, 'src: 'tokens, I>() -> impl Parser<'tokens, I, Spanned<Node<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
	I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
	recursive(|expr| {
		let inline_expr = recursive(|inline_expr| {
			let val =
				select! {
					Token::Null => Node::Value(Value::Null),
					Token::Bool(x) => Node::Value(Value::Bool(x)),
					Token::Num(n) => Node::Value(Value::Num(n)),
					Token::Str(s) => Node::Value(Value::Str(s)),
				}
				.labelled("value");
			
			let ident = select! { Token::Ident(ident) => ident }.labelled("identifier");
			
			// A list of expressions
			let items =
				expr
				.clone()
				.separated_by(just(Token::Ctrl(',')))
				.allow_trailing()
				.collect::<Vec<_>>();
			
			// A let expression
			let let_ =
				just(Token::Let)
				.ignore_then(ident)
				.then_ignore(just(Token::Op("=")))
				.then(inline_expr)
				.map(|(name, val)| Node::Let(name, Box::new(val)));
			
			let import_part = recursive(|import_part|
				ident
				.foldl_with()
			);
			
			let import =
				just(Token::Import)
				.ignore_then(ident)
				.then_ignore(just(Token::Ctrl(';')))
				.map_with(|subject, e| Node::Import(vec! [ (vec! [ subject ], e.span()) ]));
			
			let list =
				items
				.clone()
				.map(Node::List)
				.delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')));
			
			// 'Atoms' are expressions that contain no ambiguity
			let atom =
				choice((
					val,
					ident.map(Node::Local),
					let_,
					import,
					list,
					just(Token::Print)
						.ignore_then(
							expr
							.clone()
							.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))),
						)
						.map(|expr| Node::Print(Box::new(expr))),
				))
				.map_with(|expr, e| (expr, e.span()))
				// Atoms can also just be normal expressions, but surrounded with parentheses
				.or(
					expr
					.clone()
					.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
				)
				// Attempt to recover anything that looks like a parenthesised expression but contains errors
				.recover_with(via_parser(nested_delimiters(
					Token::Ctrl('('),
					Token::Ctrl(')'),
					[
						(Token::Ctrl('['), Token::Ctrl(']')),
						(Token::Ctrl('{'), Token::Ctrl('}')),
					],
					|span| (Node::Error, span),
				)))
				// Attempt to recover anything that looks like a list but contains errors
				.recover_with(via_parser(nested_delimiters(
					Token::Ctrl('['),
					Token::Ctrl(']'),
					[
						(Token::Ctrl('('), Token::Ctrl(')')),
						(Token::Ctrl('{'), Token::Ctrl('}')),
					],
					|span| (Node::Error, span),
				)))
				.boxed();
			
			// Function calls have very high precedence so we prioritize them
			let call = atom.foldl_with(
				items
					.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
					.map_with(|args, e| (args, e.span()))
					.repeated(),
				|f, args, e| (Node::Call(Box::new(f), args), e.span()),
			);
			
			// Product ops (multiply and divide) have equal precedence
			let op =
				just(Token::Op("*"))
				.to(BinaryOp::Mul)
				.or(just(Token::Op("/")).to(BinaryOp::Div));
			let product =
				call
				.clone()
				.foldl_with(op.then(call).repeated(), |a, (op, b), e| {
					(Node::Binary(Box::new(a), op, Box::new(b)), e.span())
				});
			
			// Sum ops (add and subtract) have equal precedence
			let op =
				just(Token::Op("+"))
				.to(BinaryOp::Add)
				.or(just(Token::Op("-")).to(BinaryOp::Sub));
			let sum =
				product
				.clone()
				.foldl_with(op.then(product).repeated(), |a, (op, b), e| {
					(Node::Binary(Box::new(a), op, Box::new(b)), e.span())
				});
			
			// Comparison ops (equal, not-equal) have equal precedence
			let op =
				just(Token::Op("=="))
				.to(BinaryOp::Eq)
				.or(just(Token::Op("!=")).to(BinaryOp::NotEq));
			let compare =
				sum
				.clone()
				.foldl_with(op.then(sum).repeated(), |a, (op, b), e| {
					(Node::Binary(Box::new(a), op, Box::new(b)), e.span())
				});
			
			compare.labelled("expression").as_context()
		});
		
		// Blocks are expressions but delimited with braces
		let block =
			expr
			.clone()
			.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
			// Attempt to recover anything that looks like a block but contains errors
			.recover_with(via_parser(nested_delimiters(
				Token::Ctrl('{'),
				Token::Ctrl('}'),
				[
					(Token::Ctrl('('), Token::Ctrl(')')),
					(Token::Ctrl('['), Token::Ctrl(']')),
				],
				|span| (Node::Error, span),
			)));
		
		let if_ = recursive(|if_| {
			just(Token::If)
			.ignore_then(expr.clone())
			.then(block.clone())
			.then(
				just(Token::Else)
				.ignore_then(block.clone().or(if_))
				.or_not(),
			)
			.map_with(|((cond, a), b), e| {
				(
					Node::If(
						Box::new(cond),
						Box::new(a),
						// If an `if` expression has no trailing `else` block, we magic up one that just produces null
						Box::new(b.unwrap_or_else(|| (Node::Value(Value::Null), e.span()))),
					),
					e.span(),
				)
			})
		});
		
		// Both blocks and `if` are 'block expressions' and can appear in the place of statements
		let block_expr = block.or(if_);
		
		let block_chain = block_expr
			.clone()
			.foldl_with(block_expr.clone().repeated(), |a, b, e| {
				(Node::Then(Box::new(a), Box::new(b)), e.span())
			});
		
		let block_recovery = nested_delimiters(
			Token::Ctrl('{'),
			Token::Ctrl('}'),
			[
				(Token::Ctrl('('), Token::Ctrl(')')),
				(Token::Ctrl('['), Token::Ctrl(']')),
			],
			|span| (Node::Error, span),
		);
		
		block_chain
		.labelled("block")
		// Expressions, chained by semicolons, are statements
		.or(inline_expr.clone())
		.recover_with(skip_then_retry_until(
			block_recovery.ignored().or(any().ignored()),
			one_of([
				Token::Ctrl(';'),
				Token::Ctrl('}'),
				Token::Ctrl(')'),
				Token::Ctrl(']'),
			]).ignored(),
		))
		.foldl_with(
			just(Token::Ctrl(';')).ignore_then(expr.or_not()).repeated(),
			|a, b, e| {
				let span: Span = e.span();
				(
					Node::Then(
						Box::new(a),
						// If there is no b expression then its span is the end of the statement/block.
						Box::new(
							b.unwrap_or_else(|| (Node::Value(Value::Null), span.to_end())),
						),
					),
					span,
				)
			},
		)
	})
}

pub fn functions_parser<'tokens, 'src: 'tokens, I>() -> impl Parser<
	'tokens,
	I,
	HashMap<&'src str, Func<'src>>,
	extra::Err<Rich<'tokens, Token<'src>, Span>>,
> + Clone
where
	I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
	let ident = select! { Token::Ident(ident) => ident };
	
	// Argument lists are just identifiers separated by commas, surrounded by parentheses
	let parameters =
		ident
		.separated_by(just(Token::Ctrl(',')))
		.allow_trailing()
		.collect()
		.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
		.labelled("function parameters");
	
	let function =
		just(Token::Fun)
		.ignore_then(
			ident
			.map_with(|name, e| (name, e.span()))
			.labelled("function name"),
		)
		.then(parameters)
		.map_with(|start, e| (start, e.span()))
		.then(
			expr_parser()
			.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
			// Attempt to recover anything that looks like a function body but contains errors
			.recover_with(via_parser(nested_delimiters(
				Token::Ctrl('{'),
				Token::Ctrl('}'),
				[
					(Token::Ctrl('('), Token::Ctrl(')')),
					(Token::Ctrl('['), Token::Ctrl(']')),
				],
				|span| (Node::Error, span),
			))),
		)
		.map(|(((name, args), span), body)| (name, Func { args, span, body }))
		.labelled("function");
	
	function.repeated()
	.collect::<Vec<_>>()
	.validate(|fs, _, emitter| {
		let mut functions = HashMap::new();
		for ((name, name_span), f) in fs {
			if functions.insert(name, f).is_some() {
				emitter.emit(Rich::custom(
					name_span,
					format!("Function '{name}' already exists"),
				));
			}
		}
		functions
	})
}
