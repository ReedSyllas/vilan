
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

#[derive(Debug)]
pub struct ImportPath<'src> (Vec<&'src str>, Option<Box<Self>>);

// An expression node in the AST. Children are spanned so we can generate useful runtime errors.
#[derive(Debug)]
pub enum Node<'src> {
	Error,
	Value(Value<'src>),
	List(Vec<Spanned<Self>>),
	Local(&'src str),
	Import(ImportPath<'src>),
	Let(&'src str, Box<Spanned<Self>>),
	Then(Box<Spanned<Self>>, Box<Spanned<Self>>),
	Binary(BinaryOp, Box<Spanned<Self>>, Box<Spanned<Self>>),
	Call(Box<Spanned<Self>>, Spanned<Vec<Spanned<Self>>>),
	If(Box<Spanned<Self>>, Box<Spanned<Self>>, Box<Spanned<Self>>),
	Func {
		name: Spanned<&'src str>,
		parameters: Spanned<Vec<(&'src str, Option<&'src str>)>>,
		body: Box<Spanned<Self>>,
	},
}

// #[derive(Debug)]
// pub struct Scope<'src> {
	
// }

// A function node in the AST.
#[derive(Debug)]
pub struct Func<'src> {
	pub args: Vec<&'src str>,
	pub span: Span,
	pub body: Spanned<Node<'src>>,
}

pub fn node_parser<'tokens, 'src: 'tokens, I>() -> impl Parser<'tokens, I, Spanned<Node<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
	I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
	recursive(|scope| {
		let val =
			select! {
				Token::Null => Node::Value(Value::Null),
				Token::Bool(x) => Node::Value(Value::Bool(x)),
				Token::Num(n) => Node::Value(Value::Num(n)),
				Token::Str(s) => Node::Value(Value::Str(s)),
			}
			.labelled("value");
		
		let ident = select! { Token::Ident(ident) => ident }.labelled("identifier");
		
		let mut expression = Recursive::declare();
		
		let import =
			just(Token::Import)
			.ignore_then(
				ident
				.map(|a| ImportPath(vec![ a ], None))
				.foldl_with(
					just(Token::Op("::"))
						.ignore_then(choice((
							ident.map(|x| vec![ x ]),
							ident
								.separated_by(just(Token::Ctrl(',')))
								.allow_trailing()
								.collect::<Vec<_>>()
								.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}'))),
						)))
						.repeated(),
					|a, b, _| ImportPath(b, Some(Box::new(a)))
				)
			)
			.map_with(|import_path, e| (Node::Import(import_path), e.span()));
		
		// Blocks are expressions but delimited with braces
		let block =
			scope.clone()
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
			.ignore_then(expression.clone())
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
		
		let parameters =
			ident
			.then(
				just(Token::Op(":"))
				.ignore_then(ident)
				.or_not()
			)
			.separated_by(just(Token::Ctrl(',')))
			.allow_trailing()
			.collect::<Vec<_>>()
			.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
			.map_with(|parameters, e| (parameters, e.span()))
			.labelled("function parameters");
		
		let function =
			just(Token::Fun)
			.ignore_then(
				ident
				.map_with(|name, e| (name, e.span()))
				.labelled("function name"),
			)
			.then(parameters)
			.then(block.clone())
			.map_with(|((name, parameters), body), e| (Node::Func { name, parameters, body: Box::new(body) }, e.span()))
			.labelled("function")
			.boxed();
		
		// A comma-delimited list of expressions.
		let items =
			expression.clone()
			.separated_by(just(Token::Ctrl(',')))
			.allow_trailing()
			.collect::<Vec<_>>();
		
		let local = ident.map(Node::Local);
		
		let let_ =
			just(Token::Let)
			.ignore_then(ident)
			.then_ignore(just(Token::Op("=")))
			.then(expression.clone())
			.map(|(name, val)| Node::Let(name, Box::new(val)));
		
		let list =
			items.clone()
			.map(Node::List)
			.delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')));
		
		// 'Atoms' are expressions that contain no ambiguity
		let atom =
			choice((
				val,
				local,
				let_,
				list,
			))
			.map_with(|expr, e| (expr, e.span()))
			// Atoms can also just be normal expressions, but surrounded with parentheses
			.or(
				expression.clone()
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
			call.clone()
			.foldl_with(op.then(call).repeated(), |a, (op, b), e| {
				(Node::Binary(op, Box::new(a), Box::new(b)), e.span())
			});
		
		// Sum ops (add and subtract) have equal precedence
		let op =
			just(Token::Op("+"))
			.to(BinaryOp::Add)
			.or(just(Token::Op("-")).to(BinaryOp::Sub));
		let sum =
			product.clone()
			.foldl_with(op.then(product).repeated(), |a, (op, b), e| {
				(Node::Binary(op, Box::new(a), Box::new(b)), e.span())
			});
		
		// Comparison ops (equal, not-equal) have equal precedence
		let op =
			just(Token::Op("=="))
			.to(BinaryOp::Eq)
			.or(just(Token::Op("!=")).to(BinaryOp::NotEq));
		let compare =
			sum.clone()
			.foldl_with(op.then(sum).repeated(), |a, (op, b), e| {
				(Node::Binary(op, Box::new(a), Box::new(b)), e.span())
			});
		
		expression.define(choice((
			compare.labelled("expression").as_context(),
			if_.clone(),
			block.clone(),
		)));
		
		let statement = choice((
			if_,
			function,
			import,
			block,
			expression.clone()
				.foldl_with(
					just(Token::Ctrl(';')).ignore_then(expression.or_not()).repeated(),
					|a, b, e| {
						let span: Span = e.span();
						(
							Node::Then(
								Box::new(a),
								Box::new(b.unwrap_or_else(|| (Node::Value(Value::Null), span.to_end()))),
							),
							span,
						)
					}
				),
		));
		
		statement.clone()
		.foldl_with(
			statement.repeated(),
			|a, b, e| (Node::Then(Box::new(a), Box::new(b)), e.span()),
		)
		.recover_with(skip_then_retry_until(
			nested_delimiters(
				Token::Ctrl('{'),
				Token::Ctrl('}'),
				[
					(Token::Ctrl('('), Token::Ctrl(')')),
					(Token::Ctrl('['), Token::Ctrl(']')),
				],
				|span| (Node::Error, span),
			).ignored().or(any().ignored()),
			one_of([
				Token::Ctrl(';'),
				Token::Ctrl('}'),
				Token::Ctrl(')'),
				Token::Ctrl(']'),
			]).ignored(),
		))
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
			node_parser()
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
		let empty_span = Span::new((), 0..0);
		functions.insert("print", Func { args: vec![ "value" ], body: (Node::Value(Value::Null), empty_span), span: empty_span });
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
