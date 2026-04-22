use crate::id::Id;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Any,
    Function(Vec<TypeId>, Box<TypeId>),
    Primitive(PrimitiveType),
    Struct(Id),
    Tuple(Vec<TypeId>),
    Unknown,
    Void,
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

impl std::fmt::Debug for TypeId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TypeId({})", self.0)
    }
}
