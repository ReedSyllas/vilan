#[derive(Clone, Debug, PartialEq)]
pub enum Token<'src> {
    Bool(bool),
    Ctrl(char),
    Else,
    Enum,
    Export,
    External,
    For,
    Fun,
    Ident(&'src str),
    If,
    Impl,
    Import,
    In,
    Is,
    Jump,
    Let,
    Match,
    Mod,
    Mut,
    Null,
    // The whole part, an optional fractional part, and an optional type suffix
    // (`u32`, `f`, `n`, ...).
    Number(&'src str, Option<&'src str>, Option<&'src str>),
    Op(&'src str),
    Ret,
    String(&'src str),
    Struct,
    Trait,
    Type,
    Use,
    With,
}

impl std::fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Token::Bool(x) => write!(f, "{x}"),
            Token::Ctrl(c) => write!(f, "{c}"),
            Token::Else => write!(f, "else"),
            Token::Enum => write!(f, "enum"),
            Token::Export => write!(f, "export"),
            Token::External => write!(f, "external"),
            Token::For => write!(f, "for"),
            Token::Fun => write!(f, "fun"),
            Token::Ident(s) => write!(f, "{s}"),
            Token::If => write!(f, "if"),
            Token::Impl => write!(f, "impl"),
            Token::Import => write!(f, "import"),
            Token::In => write!(f, "in"),
            Token::Is => write!(f, "is"),
            Token::Jump => write!(f, "jump"),
            Token::Let => write!(f, "let"),
            Token::Match => write!(f, "match"),
            Token::Mod => write!(f, "mod"),
            Token::Mut => write!(f, "mut"),
            Token::Null => write!(f, "null"),
            Token::Number(whole, fraction, suffix) => write!(
                f,
                "{}{}{}",
                whole,
                fraction
                    .map(|x| format!(".{}", x))
                    .unwrap_or("".to_string()),
                suffix.unwrap_or("")
            ),
            Token::Op(s) => write!(f, "{s}"),
            Token::Ret => write!(f, "ret"),
            Token::String(s) => write!(f, "{s}"),
            Token::Struct => write!(f, "struct"),
            Token::Trait => write!(f, "trait"),
            Token::Type => write!(f, "type"),
            Token::Use => write!(f, "use"),
            Token::With => write!(f, "with"),
        }
    }
}
