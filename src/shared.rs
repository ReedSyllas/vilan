
use chumsky::span::SimpleSpan;

pub type Span = SimpleSpan;
pub type Spanned<T> = (T, Span);

pub struct Error {
	pub span: Span,
	pub msg: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(pub u32);

impl std::fmt::Debug for Id {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "Id({})", self.0)
	}
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

// TODO: Move to interpreter
// #[derive(Clone, Debug, PartialEq)]
// pub enum Value<'src> {
// 	Null,
// 	Bool(bool),
// 	Num(f64),
// 	Str(&'src str),
// 	List(Vec<Self>),
// 	Func(&'src str),
// 	Interrupt(Box<Self>),
// }

// impl std::fmt::Display for Value<'_> {
// 	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
// 		match self {
// 			Self::Null => write!(f, "null"),
// 			Self::Bool(x) => write!(f, "{x}"),
// 			Self::Num(x) => write!(f, "{x}"),
// 			Self::Str(x) => write!(f, "{x}"),
// 			Self::List(xs) => write!(
// 				f,
// 				"[{}]",
// 				xs.iter()
// 					.map(|x| x.to_string())
// 					.collect::<Vec<_>>()
// 					.join(", ")
// 			),
// 			Self::Func(name) => write!(f, "<function: {name}>"),
// 			Self::Interrupt(x) => write!(f, "{x}"),
// 		}
// 	}
// }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
	Any,
	Function(Vec<Self>, Box<Self>),
	Primitive(PrimitiveType),
	Struct(Id),
	Tuple(Vec<Self>),
	Unknown,
	Void,
}

impl Type {
	pub fn reconcile(self, peer: Type) -> Type {
		match (self, peer) {
			(a, Type::Unknown) => a,
			(Type::Unknown, b) => b,
			(Type::Primitive(a), Type::Primitive(b)) => match (a, b) {
				(PrimitiveType::List(a), PrimitiveType::List(b)) => Type::Primitive(PrimitiveType::List(Box::new(a.reconcile(*b)))),
				(a, b) if a == b => Type::Primitive(a),
				(a, b) => panic!("types {:#?} and {:#?} are mismatched", a, b),
			},
			(Type::Tuple(aa), Type::Tuple(bb)) => Type::Tuple(aa.iter().zip(bb.iter()).map(|(a, b)| a.clone().reconcile(b.clone())).collect()),
			(a, b) if a == b => a,
			(a, b) => panic!("types {:#?} and {:#?} are mismatched", a, b),
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrimitiveType {
	I32,
	U32,
	F64,
	String,
	Bool,
	Null,
	List(Box<Type>),
}
