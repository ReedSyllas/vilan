use std::collections::HashMap;

use crate::id::Id;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Any,
    // The type of expressions that never produce a value: `panic(..)`,
    // `ret ..`, `jump break`/`continue`. Never unifies by YIELDING to the
    // other side (a diverging match leg doesn't constrain the match's
    // type), unlike `Any`, which absorbs. Internal — not written in source.
    Never,
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
    // A trait and its generic arguments (`Display` -> `Trait(display_id, [])`,
    // `Into<bool>` -> `Trait(into_id, [bool])`, `Readable<U>` ->
    // `Trait(readable_id, [U])`). The arguments drive parameterized-trait impl
    // selection and a mapped trait template's inversion.
    Trait(Id, Vec<TypeId>),
    Tuple(Vec<TypeId>),
    // A mapped tuple type `(U in T: F<U>)`, symbolic while the source tuple `T` is
    // still abstract: the binder `U`'s generic id, the source tuple type, and the
    // template `F<U>`. Expands to a concrete `Tuple` once `T` resolves to one
    // (each element `X` maps to `F[U := X]`).
    Mapped(TypeId, TypeId, TypeId),
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
