use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ty::{ContainerSerdeMetadata, Enum, LinkingScheme, Struct, Tuple, Ty};

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct StructWithSerde<L: LinkingScheme> {
    pub ty: Struct<L>,
    pub serde: ContainerSerdeMetadata,
}

impl<L: LinkingScheme> From<Struct<L>> for StructWithSerde<L> {
    fn from(value: Struct<L>) -> Self {
        Self {
            ty: value,
            serde: ContainerSerdeMetadata::default(),
        }
    }
}

impl<L: LinkingScheme> From<StructWithSerde<L>> for Struct<L> {
    fn from(value: StructWithSerde<L>) -> Self {
        value.ty
    }
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EnumWithSerde<L: LinkingScheme> {
    pub ty: Enum<L>,
    pub serde: ContainerSerdeMetadata,
}

impl<L: LinkingScheme> From<Enum<L>> for EnumWithSerde<L> {
    fn from(value: Enum<L>) -> Self {
        Self {
            ty: value,
            serde: ContainerSerdeMetadata::default(),
        }
    }
}

impl<L: LinkingScheme> From<EnumWithSerde<L>> for Enum<L> {
    fn from(value: EnumWithSerde<L>) -> Self {
        value.ty
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Container<L: LinkingScheme> {
    Struct(StructWithSerde<L>),
    Enum(EnumWithSerde<L>),
    Tuple(Tuple<L>),
    Option {
        value: L::TypeLink,
    },
    Array {
        len: usize,
        value: L::TypeLink,
    },
    Vec {
        value: L::TypeLink,
    },
    Map {
        key: L::TypeLink,
        value: L::TypeLink,
    },
}

impl<L: LinkingScheme> Container<L> {
    /// Returns the number of child types required by this container.
    pub fn num_children(&self) -> usize {
        match self {
            Container::Struct(s) => s.ty.fields.len(),
            Container::Enum(e) => {
                e.ty.variants
                    .iter()
                    .map(|variant| match &variant.value {
                        Some(_) => 1,
                        _ => 0,
                    })
                    .sum()
            }
            Container::Tuple(t) => t.fields.len(),
            Container::Option { .. } => 1,
            Container::Array { .. } => 1,
            Container::Vec { .. } => 1,
            Container::Map { .. } => 2,
        }
    }

    pub fn serde(&self) -> ContainerSerdeMetadata {
        match self {
            Container::Struct(s) => s.serde.clone(),
            Container::Enum(e) => e.serde.clone(),
            _ => ContainerSerdeMetadata::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrNotAContainer;

impl<L: LinkingScheme> TryFrom<Ty<L>> for Container<L> {
    type Error = ErrNotAContainer;

    fn try_from(value: Ty<L>) -> Result<Self, Self::Error> {
        match value {
            Ty::Enum(e) => Ok(Container::Enum(e.into())),
            Ty::Struct(s) => Ok(Container::Struct(s.into())),
            Ty::Tuple(t) => Ok(Container::Tuple(t)),
            Ty::Option { value } => Ok(Container::Option { value }),
            Ty::Array { len, value } => Ok(Container::Array { len, value }),
            Ty::Map { key, value } => Ok(Container::Map { key, value }),
            Ty::Vec { value } => Ok(Container::Vec { value }),
            _ => Err(ErrNotAContainer),
        }
    }
}

impl<L: LinkingScheme> From<Container<L>> for Ty<L> {
    fn from(value: Container<L>) -> Self {
        match value {
            Container::Struct(s) => Ty::Struct(s.into()),
            Container::Enum(e) => Ty::Enum(e.into()),
            Container::Tuple(t) => Ty::Tuple(t),
            Container::Option { value } => Ty::Option { value },
            Container::Array { len, value } => Ty::Array { len, value },
            Container::Vec { value } => Ty::Vec { value },
            Container::Map { key, value } => Ty::Map { key, value },
        }
    }
}
