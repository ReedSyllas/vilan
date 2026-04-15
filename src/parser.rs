
use chumsky::{input::ValueInput, prelude::*};

use crate::{lexer::Token, shared::{BinaryOp, Span, Spanned}};

#[derive(Debug)]
pub struct Func<'src> {
	pub name: Spanned<&'src str>,
	pub return_type: Option<Box<Spanned<Node<'src>>>>,
	pub parameters: Spanned<Vec<(&'src str, Option<Box<Spanned<Node<'src>>>>)>>,
	pub body: Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>,
}

#[derive(Debug)]
pub struct If<'src> {
	pub condition: Box<Spanned<Node<'src>>>,
	pub then: Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>,
	pub else_: Option<Spanned<IfElseBranch<'src>>>,
}

#[derive(Debug)]
pub enum IfElseBranch<'src> {
	If(Box<If<'src>>),
	Else(Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>),
}

#[derive(Debug)]
pub enum ImportBranch<'src> {
	Path(&'src str, Option<Box<Self>>),
	Set(Vec<Self>),
}

pub type NodeList<'src> = Vec<Spanned<Node<'src>>>;

#[derive(Debug)]
pub enum Node<'src> {
	Binary(BinaryOp, Box<Spanned<Self>>, Box<Spanned<Self>>),
	Block(Spanned<(NodeList<'src>, Box<Spanned<Self>>)>),
	Bool(bool),
	Call(Box<Spanned<Self>>, Spanned<NodeList<'src>>),
	Error,
	Func(Func<'src>),
	FuncReturn(Box<Spanned<Self>>),
	If(If<'src>),
	Import(ImportBranch<'src>),
	Let(&'src str, Option<Box<Spanned<Self>>>, Option<Box<Spanned<Self>>>),
	List(NodeList<'src>),
	Local(&'src str),
	Null,
	Number(&'src str),
	String(&'src str),
	Struct(&'src str, Spanned<NodeList<'src>>),
	Tuple(NodeList<'src>),
	Void,
}

pub fn parser<'tokens, 'src: 'tokens, I>() -> impl Parser<'tokens, I, Spanned<NodeList<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
	I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
	let val =
		select! {
			Token::Null => Node::Null,
			Token::Bool(x) => Node::Bool(x),
			Token::Num(n) => Node::Number(n),
			Token::String(s) => Node::String(s),
		}
		.labelled("value")
		.map_with(|x, e| (x, e.span()));
	
	let ident = select! { Token::Ident(ident) => ident }.labelled("identifier");
	
	let mut statement = Recursive::declare();
	
	let mut expression = Recursive::declare();
	
	let mut type_ = Recursive::declare();
	
	// Blocks are a sequence of statements delimited with braces.
	let block =
		statement.clone()
		.repeated()
		.collect::<Vec<_>>()
		.then(
			expression.clone()
			.map(|x| Box::new(x))
			.or_not()
		)
		.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
		.map_with(|(statements, expression), e| {
			let span: Span = e.span();
			(Some((statements, expression.unwrap_or_else(|| Box::new((Node::Void, span.to_end()))))), span)
		})
		.recover_with(via_parser(nested_delimiters(
			Token::Ctrl('{'),
			Token::Ctrl('}'),
			[
				(Token::Ctrl('('), Token::Ctrl(')')),
				(Token::Ctrl('['), Token::Ctrl(']')),
			],
			|span| (None, span),
		)))
		.map_with(|x, e| {
			let span: Span = e.span();
			(x.0.unwrap_or_else(|| (Vec::new(), Box::new((Node::Void, span.to_end())))), x.1)
		});
	
	let import =
		just(Token::Import)
		.ignore_then(
			recursive(|branch| {
				let path =
					ident
					.then(
						just(Token::Op("::"))
						.ignore_then(branch)
						.or_not()
					)
					.map(|(a, b)| ImportBranch::Path(a, b.map(|b| Box::new(b))));
				
				path.clone()
				.or(
					path
					.separated_by(just(Token::Ctrl(',')))
					.allow_trailing()
					.collect::<Vec<_>>()
					.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
					.map(|x| ImportBranch::Set(x))
				)
			})
		)
		.map_with(|import_path, e| (Node::Import(import_path), e.span()))
		.boxed();
	
	let if_ =
		recursive(|if_| {
			just(Token::If)
			.ignore_then(expression.clone())
			.then(block.clone())
			.then(
				just(Token::Else)
				.ignore_then(block.clone().map_with(|x, e| (IfElseBranch::Else(x), e.span())).or(if_))
				.or_not(),
			)
			.map_with(|((cond, a), b), e| {
				(
					IfElseBranch::If(Box::new(If {
						condition: Box::new(cond),
						then: a,
						else_: b,
					})),
					e.span(),
				)
			})
		})
		.map(|x| match x {
			(IfElseBranch::If(x), span) => (Node::If(*x), span),
			_ => unreachable!(),
		});
	
	let let_ =
		just(Token::Let)
		.ignore_then(ident)
		.then(
			just(Token::Op(":"))
			.ignore_then(type_.clone())
			.labelled("type")
			.or_not()
		)
		.then(
			just(Token::Op("="))
			.ignore_then(expression.clone())
			.labelled("value")
			.or_not()
		)
		.map_with(|((name, type_), val), e| (Node::Let(name, type_.map(|x| Box::new(x)), val.map(|x| Box::new(x))), e.span()));
	
	let function =
		just(Token::Fun)
		.ignore_then(
			ident
			.map_with(|name, e| (name, e.span()))
			.labelled("function name"),
		)
		.then(
			ident
			.labelled("parameter name")
			.then(
				just(Token::Op(":"))
				.ignore_then(type_.clone().map(|x| Box::new(x)))
				.labelled("parameter type")
				.or_not()
			)
			.labelled("parameter")
			.separated_by(just(Token::Ctrl(',')))
			.allow_trailing()
			.collect::<Vec<_>>()
			.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
			.map_with(|parameters, e| (parameters, e.span()))
			.labelled("function parameters")
		)
		.then(
			just(Token::Op(":"))
			.ignore_then(type_.clone().map(|x| Box::new(x)))
			.labelled("return type")
			.or_not()
		)
		.then(block.clone())
		.map_with(|(((name, parameters), return_type), body), e| {
			(Node::Func(Func { name, return_type, parameters, body }), e.span())
		})
		.labelled("function")
		.boxed();
	
	let return_ =
		just(Token::Ret)
		.ignore_then(expression.clone())
		.map_with(|x, e| (Node::FuncReturn(Box::new(x)), e.span()));
	
	let struct_ =
		just(Token::Struct)
		.ignore_then(
			ident
			.labelled("struct name"),
		)
		.then(
			statement.clone()
			.repeated()
			.collect::<Vec<_>>()
			.delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
			.map_with(|statements, e| (Some(statements), e.span()))
			.recover_with(via_parser(nested_delimiters(
				Token::Ctrl('{'),
				Token::Ctrl('}'),
				[
					(Token::Ctrl('('), Token::Ctrl(')')),
					(Token::Ctrl('['), Token::Ctrl(']')),
				],
				|span| (None, span),
			)))
			.map_with(|x, e| (x.0.unwrap_or_else(|| Vec::new()), x.1))
		)
		.map_with(|(name, body), e| (Node::Struct(name, body), e.span()))
		.labelled("struct")
		.boxed();
	
	// A comma-delimited list of expressions.
	let items =
		expression.clone()
		.separated_by(just(Token::Ctrl(',')))
		.allow_trailing()
		.collect::<Vec<_>>();
	
	let local = ident.map_with(|x, e| (Node::Local(x), e.span()));
	
	let list =
		items.clone()
		.delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')))
		.map_with(|x, e| (Node::List(x), e.span()));
	
	let tuple =
		expression.clone()
		.separated_by(just(Token::Ctrl(',')))
		.allow_trailing()
		.collect::<Vec<_>>()
		.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
		.map_with(|x, e| (Node::Tuple(x), e.span()));
	
	// 'Atoms' are expressions that contain no ambiguity
	let atom =
		choice((
			val,
			local,
			list,
			tuple,
			if_.clone(),
			block.clone().map(|(x, span)| (Node::Block((x, span)), span)),
			expression.clone()
				.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))),
		))
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
		let_,
		return_,
	)));
	
	statement.define(choice((
		expression.clone().then_ignore(just(Token::Ctrl(';'))),
		if_,
		function,
		struct_,
		import.then_ignore(just(Token::Ctrl(';'))),
		block.map(|(x, span)| (Node::Block((x, span)), span)),
	)));
	
	let tuple_type =
		type_.clone()
		.separated_by(just(Token::Ctrl(',')))
		.allow_trailing()
		.collect::<Vec<_>>()
		.delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
		.map_with(|x, e| (Node::Tuple(x), e.span()));
	
	type_.define(choice((
		local,
		tuple_type,
	)));
	
	statement.clone()
		.repeated()
		.collect::<Vec<_>>()
		.map_with(|children, e| (children, e.span()))
}
