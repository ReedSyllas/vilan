use std::collections::HashMap;

use crate::id::Id;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Any,
    Closure(Vec<TypeId>, TypeId),
    Enum(Id),
    Function(Id),
    Generic(TypeId),
    Module(Id),
    Primitive(PrimitiveType),
    Struct(Id),
    Trait(Id),
    Tuple(Vec<TypeId>),
    Unknown,
    Unresolved,
    Void,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrimitiveType {
    // The scalar primitives (`i32`, `str`, ...) are now built-in structs of
    // the `std` package; `List` is the remaining parameterized container.
    List(TypeId),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

impl std::fmt::Debug for TypeId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TypeId({})", self.0)
    }
}

pub type SubstitutionContext = HashMap<TypeId, TypeId>;
