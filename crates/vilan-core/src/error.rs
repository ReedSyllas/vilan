use crate::span::Span;

#[derive(Debug, Clone)]
pub struct Error {
    pub span: Span,
    pub msg: String,
}
