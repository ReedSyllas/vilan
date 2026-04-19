use crate::id::Id;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Any,
    Function(Vec<Self>, Box<Self>),
    Primitive(PrimitiveType),
    Struct(Id),
    Tuple(Vec<Self>),
    Unknown,
    Void,
}

impl Type {
    pub fn reconcile(self, peer: Type) -> Type {
        match (self, peer) {
            (a, Type::Unknown) => a,
            (Type::Unknown, b) => b,
            (Type::Primitive(a), Type::Primitive(b)) => match (a, b) {
                (PrimitiveType::List(a), PrimitiveType::List(b)) => {
                    Type::Primitive(PrimitiveType::List(Box::new(a.reconcile(*b))))
                }
                (a, b) if a == b => Type::Primitive(a),
                (a, b) => panic!("types {:#?} and {:#?} are mismatched", a, b),
            },
            (Type::Tuple(aa), Type::Tuple(bb)) => Type::Tuple(
                aa.iter()
                    .zip(bb.iter())
                    .map(|(a, b)| a.clone().reconcile(b.clone()))
                    .collect(),
            ),
            (a, b) if a == b => a,
            (a, b) => panic!("types {:#?} and {:#?} are mismatched", a, b),
        }
    }
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
