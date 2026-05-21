use crate::span::Span;

#[derive(Debug)]
pub struct Error {
    pub span: Span,
    pub msg: String,
}
