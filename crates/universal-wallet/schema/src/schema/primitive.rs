use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ty::{ByteDisplay, IntegerDisplay, IntegerType, LinkingScheme, Ty};

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Primitive {
    Integer(IntegerType, IntegerDisplay),
    ByteArray { len: usize, display: ByteDisplay },
    ByteVec { display: ByteDisplay },
    Float32,
    Float64,
    String,
    Boolean,
    Skip { len: usize },
}

impl<L: LinkingScheme> From<Primitive> for Ty<L> {
    fn from(val: Primitive) -> Self {
        match val {
            Primitive::Integer(t, d) => Ty::Integer(t, d),
            Primitive::ByteArray { len, display } => Ty::ByteArray { len, display },
            Primitive::ByteVec { display } => Ty::ByteVec { display },
            Primitive::Float32 => Ty::Float32,
            Primitive::Float64 => Ty::Float64,
            Primitive::String => Ty::String,
            Primitive::Boolean => Ty::Boolean,
            Primitive::Skip { len } => Ty::Skip { len },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrNotAPrimitive;

impl<L: LinkingScheme> TryFrom<Ty<L>> for Primitive {
    type Error = ErrNotAPrimitive;

    fn try_from(value: Ty<L>) -> Result<Self, Self::Error> {
        match value {
            Ty::Integer(t, d) => Ok(Primitive::Integer(t, d)),
            Ty::ByteArray { len, display } => Ok(Primitive::ByteArray { len, display }),
            Ty::Float32 => Ok(Primitive::Float32),
            Ty::Float64 => Ok(Primitive::Float64),
            Ty::String => Ok(Primitive::String),
            Ty::Boolean => Ok(Primitive::Boolean),
            Ty::Skip { len } => Ok(Primitive::Skip { len }),
            _ => Err(ErrNotAPrimitive),
        }
    }
}
