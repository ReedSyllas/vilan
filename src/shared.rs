
use chumsky::span::SimpleSpan;

pub type Span = SimpleSpan;
pub type Spanned<T> = (T, Span);

pub struct Error {
	pub span: Span,
	pub msg: String,
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

#[derive(Clone, Debug, PartialEq)]
pub enum Value<'src> {
	Null,
	Bool(bool),
	Num(f64),
	Str(&'src str),
	List(Vec<Self>),
	Func(&'src str),
	RetFlag(Box<Self>),
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
			Self::RetFlag(x) => write!(f, "{x}"),
		}
	}
}
