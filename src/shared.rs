
use chumsky::span::SimpleSpan;

pub type Span = SimpleSpan;
pub type Spanned<T> = (T, Span);

pub struct Error {
	pub span: Span,
	pub msg: String,
}

#[derive(Clone, Copy, Debug)]
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
	Interrupt(Box<Self>),
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
			Self::Interrupt(x) => write!(f, "{x}"),
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
	Void,
	Unknown,
	Interrupt,
	Primitive(PrimitiveType),
}

impl Type {
	pub fn reconcile(self, peer: Type) -> Type {
		match (self, peer) {
			(Type::Unknown, x) => x,
			(x, Type::Unknown) => x,
			(a, b) if a == b => a,
			(a, _) => a,
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrimitiveType {
	I32,
	U32,
	F64,
	Bool,
	Null,
	String,
	List(Box<Type>),
}
