use crate::span::Span;

pub struct Error {
    pub span: Span,
    pub msg: String,
}
