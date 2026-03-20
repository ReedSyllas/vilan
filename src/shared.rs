
use chumsky::span::SimpleSpan;

pub type Span = SimpleSpan;
pub type Spanned<T> = (T, Span);

pub struct Error {
	pub span: Span,
	pub msg: String,
}
