use crate::span::Spanned;

#[derive(Debug)]
pub struct Func<'src> {
    pub name: Spanned<&'src str>,
    pub parameters: Spanned<Vec<(&'src str, Option<Box<Spanned<Node<'src>>>>)>>,
    pub return_type: Option<Box<Spanned<Node<'src>>>>,
    pub body: Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>,
}

#[derive(Debug)]
pub struct Closure<'src> {
    pub parameters: Spanned<Vec<(&'src str, Option<Box<Spanned<Node<'src>>>>)>>,
    pub return_type: Option<Box<Spanned<Node<'src>>>>,
    pub return_value: Box<Spanned<Node<'src>>>,
}

#[derive(Debug)]
pub struct If<'src> {
    pub condition: Box<Spanned<Node<'src>>>,
    pub then: Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>,
    pub else_: Option<Spanned<NodeIfBranch<'src>>>,
}

#[derive(Debug)]
pub enum NodeIfBranch<'src> {
    If(Box<If<'src>>),
    Else(Spanned<(NodeList<'src>, Box<Spanned<Node<'src>>>)>),
}

#[derive(Debug)]
pub enum ImportBranch<'src> {
    Path(&'src str, Option<Box<Self>>),
    Set(Vec<Self>),
}

pub type NodeList<'src> = Vec<Spanned<Node<'src>>>;

#[derive(Debug)]
pub enum Node<'src> {
    Binary(BinaryOp, Box<Spanned<Self>>, Box<Spanned<Self>>),
    Block(Spanned<(NodeList<'src>, Box<Spanned<Self>>)>),
    Bool(bool),
    Call(Box<Spanned<Self>>, Spanned<NodeList<'src>>),
    Closure(Closure<'src>),
    ClosureType(
        Spanned<Vec<(Option<&'src str>, Box<Spanned<Node<'src>>>)>>,
        Option<Box<Spanned<Node<'src>>>>,
    ),
    Error,
    Func(Func<'src>),
    FuncReturn(Box<Spanned<Self>>),
    If(NodeIfBranch<'src>),
    Impl(Box<Spanned<Self>>, Spanned<NodeList<'src>>),
    Import(ImportBranch<'src>),
    Let(
        &'src str,
        Option<Box<Spanned<Self>>>,
        Option<Box<Spanned<Self>>>,
    ),
    List(NodeList<'src>),
    Accessor(&'src str),
    MemberAccessor(Box<Spanned<Self>>, Box<Spanned<Self>>),
    StaticAccessor(Box<Spanned<Self>>, &'src str),
    Null,
    Number(&'src str, Option<&'src str>),
    String(&'src str),
    Struct(
        &'src str,
        Spanned<Vec<Spanned<(&'src str, Option<Spanned<Self>>)>>>,
    ),
    StructInitializer(
        &'src str,
        Spanned<Vec<Spanned<(&'src str, Option<Spanned<Self>>)>>>,
    ),
    Tuple(NodeList<'src>),
    Void,
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
