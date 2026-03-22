
use chumsky::{input::ValueInput, prelude::*};
use crate::{lexer::Token, shared::{BinaryOp, Span, Spanned, Value}};

#[derive(Debug)]
pub struct ImportPath<'src> (Vec<&'src str>, Option<Box<Self>>);

#[derive(Debug)]
pub struct Func<'src> {
	pub name: Spanned<&'src str>,
	pub parameters: Spanned<Vec<(&'src str, Option<&'src str>)>>,
	pub body: Box<Spanned<Node<'src>>>,
}

#[derive(Debug)]
pub enum Node<'src> {
	Binary(BinaryOp, Box<Spanned<Self>>, Box<Spanned<Self>>),
	Call(Box<Spanned<Self>>, Spanned<Vec<Spanned<Self>>>),
	Error,
	Func(Func<'src>),
	If(Box<Spanned<Self>>, Box<Spanned<Self>>, Box<Spanned<Self>>),
	Import(ImportPath<'src>),
	Let(&'src str, Box<Spanned<Self>>),
	List(Vec<Spanned<Self>>),
	Local(&'src str),
	Ret(Box<Spanned<Self>>),
	Seq(Vec<Spanned<Node<'src>>>),
	Value(Value<'src>),
}

pub fn parser<'tokens, 'src: 'tokens, I>() -> impl Parser<'tokens, I, Spanned<Node<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
	I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
	recursive(|sequence| {
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
			.map_with(|import_path, e| (Node::Import(import_path), e.span()))
			.boxed();
		
		// Attempt to recover anything that looks like a block but contains errors.
		let block_recovery = via_parser(nested_delimiters(
			Token::Ctrl('{'),
			Token::Ctrl('}'),
			[
				(Token::Ctrl('('), Token::Ctrl(')')),
				(Token::Ctrl('['), Token::Ctrl(']')),
			],
			|span| (Node::Error, span),
		));
		
		// Blocks are a sequence delimited with braces.
		let block =
			sequence.clone()
			.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
			.recover_with(block_recovery.clone());
		
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
		
		let function =
			just(Token::Fun)
			.ignore_then(
				ident
				.map_with(|name, e| (name, e.span()))
				.labelled("function name"),
			)
			.then(
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
				.labelled("function parameters")
			)
			.then(block.clone())
			.map_with(|((name, parameters), body), e| {
				(Node::Func(Func { name, parameters, body: Box::new(body) }), e.span())
			})
			.labelled("function")
			.boxed();
		
		let return_ =
			just(Token::Ret)
			.ignore_then(expression.clone())
			.map(|x| Node::Ret(Box::new(x)));
		
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
				return_,
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
			expression.clone().then_ignore(just(Token::Ctrl(';'))),
			if_,
			function,
			import,
			block,
		)).boxed();
		
		statement.clone()
		.repeated()
		.collect::<Vec<_>>()
		.then(expression.clone().or_not())
		.map_with(|(mut nodes, expression), e| {
			if let Some(expression) = expression {
				nodes.push(expression);
			}
			
			match nodes.len() {
				0 => (Node::Value(Value::Null), e.span()),
				1 => nodes.into_iter().next().unwrap(),
				_ => (Node::Seq(nodes), e.span()),
			}
		})
	})
}
