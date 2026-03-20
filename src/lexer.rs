
use chumsky::prelude::*;
use crate::shared::{Span, Spanned};

#[derive(Clone, Debug, PartialEq)]
pub enum Token<'src> {
	Bool(bool),
	Ctrl(char),
	Else,
	Export,
	Fun,
	Ident(&'src str),
	If,
	Import,
	Let,
	Null,
	Num(f64),
	Op(&'src str),
	Print,
	Str(&'src str),
}

impl std::fmt::Display for Token<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			Token::Bool(x) => write!(f, "{x}"),
			Token::Ctrl(c) => write!(f, "{c}"),
			Token::Else => write!(f, "else"),
			Token::Export => write!(f, "export"),
			Token::Fun => write!(f, "fun"),
			Token::Ident(s) => write!(f, "{s}"),
			Token::If => write!(f, "if"),
			Token::Import => write!(f, "import"),
			Token::Let => write!(f, "let"),
			Token::Null => write!(f, "null"),
			Token::Num(n) => write!(f, "{n}"),
			Token::Op(s) => write!(f, "{s}"),
			Token::Print => write!(f, "print"),
			Token::Str(s) => write!(f, "{s}"),
		}
	}
}

pub fn lexer<'src>() -> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
	// A parser for numbers
	let num =
		text::int(10)
		.then(just('.').then(text::digits(10)).or_not())
		.to_slice()
		.from_str()
		.unwrapped()
		.map(Token::Num);
	
	// A parser for strings
	let str_ =
		just('"')
		.ignore_then(none_of('"').repeated().to_slice())
		.then_ignore(just('"'))
		.map(Token::Str);
	
	// A parser for operators
	let op =
		one_of("-:!*/+=")
		.repeated()
		.at_least(1)
		.to_slice()
		.map(Token::Op);
	
	// A parser for control characters (delimiters, semicolons, etc.)
	let ctrl = one_of("()[]{};,").map(Token::Ctrl);
	
	// A parser for identifiers and keywords
	let ident = text::ascii::ident().map(|ident: &str| match ident {
		"let" => Token::Let,
		"true" => Token::Bool(true),
		"false" => Token::Bool(false),
		"null" => Token::Null,
		"fun" => Token::Fun,
		"if" => Token::If,
		"else" => Token::Else,
		"import" => Token::Import,
		"export" => Token::Export,
		"print" => Token::Print,
		_ => Token::Ident(ident),
	});
	
	// A single token can be one of the above
	let token = choice((
		num,
		str_,
		op,
		ctrl,
		ident,
	));
	
	let comment =
		just("//")
		.then(any().and_is(just('\n').not()).repeated())
		.padded();
	
	token
	.map_with(|tok, e| (tok, e.span()))
	.padded_by(comment.repeated())
	.padded()
	// If we encounter an error, skip and attempt to lex the next character as a token instead
	.recover_with(skip_then_retry_until(any().ignored(), end()))
	.repeated()
	.collect()
}
