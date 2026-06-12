#[derive(Clone, Debug, PartialEq)]
pub enum Token<'src> {
    Bool(bool),
    Ctrl(char),
    Else,
    Enum,
    For,
    Fun,
    Ident(&'src str),
    If,
    Impl,
    Import,
    Jump,
    Let,
    Match,
    Mod,
    Mut,
    Null,
    Number(&'src str, Option<&'src str>),
    Op(&'src str),
    Ret,
    String(&'src str),
    Struct,
    Trait,
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
            Token::For => write!(f, "for"),
            Token::Fun => write!(f, "fun"),
            Token::Ident(s) => write!(f, "{s}"),
            Token::If => write!(f, "if"),
            Token::Impl => write!(f, "impl"),
            Token::Import => write!(f, "import"),
            Token::Jump => write!(f, "jump"),
            Token::Let => write!(f, "let"),
            Token::Match => write!(f, "match"),
            Token::Mod => write!(f, "mod"),
            Token::Mut => write!(f, "mut"),
            Token::Null => write!(f, "null"),
            Token::Number(whole, fraction) => write!(
                f,
                "{}{}",
                whole,
                fraction
                    .map(|x| format!(".{}", x))
                    .unwrap_or("".to_string())
            ),
            Token::Op(s) => write!(f, "{s}"),
            Token::Ret => write!(f, "ret"),
            Token::String(s) => write!(f, "{s}"),
            Token::Struct => write!(f, "struct"),
            Token::Trait => write!(f, "trait"),
            Token::Use => write!(f, "use"),
            Token::With => write!(f, "with"),
        }
    }
}
