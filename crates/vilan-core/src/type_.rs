use std::collections::HashMap;

use crate::id::Id;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Any,
    Closure(Vec<TypeId>, TypeId),
    // A nominal enum/struct and its type arguments (`Option<i32>` ->
    // `Enum(option_id, [i32])`, `List<str>` -> `Struct(list_id, [str])`). The
    // arguments are empty for a non-generic type, or where they are not (yet)
    // known; member/variant resolution substitutes the type's declared
    // parameters with them.
    Enum(Id, Vec<TypeId>),
    Function(Id),
    Generic(TypeId),
    Module(Id),
    Struct(Id, Vec<TypeId>),
    Trait(Id),
    Tuple(Vec<TypeId>),
    Unknown,
    Unresolved,
    Void,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

impl std::fmt::Debug for TypeId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TypeId({})", self.0)
    }
}

pub type SubstitutionContext = HashMap<TypeId, TypeId>;
