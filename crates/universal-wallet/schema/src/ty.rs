pub mod byte_display;
pub mod visitor;
use core::panic;
use std::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
pub use byte_display::ByteDisplay;
#[cfg(feature = "serde")]
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::schema::{IndexLinking, Link, OverrideSchema, Primitive};

macro_rules! macro_for_ints {
    ($macro:ident) => {
        $macro!(u8);
        $macro!(u16);
        $macro!(u32);
        $macro!(u64);
        $macro!(u128);
        $macro!(i8);
        $macro!(i16);
        $macro!(i32);
        $macro!(i64);
        $macro!(i128);
    };
}
pub(crate) use macro_for_ints;

pub trait LinkingScheme: Clone + Debug {
    /// The type used to link to other types in the schema representation. Usually, this is an enum
    /// which represents primitives with an immediate value and complex types with some kind of pointer.
    #[cfg(not(feature = "serde"))]
    type TypeLink: Clone + Debug + PartialEq + Eq + BorshSerialize + BorshDeserialize;
    #[cfg(feature = "serde")]
    type TypeLink: Clone
        + Debug
        + PartialEq
        + Eq
        + BorshSerialize
        + BorshDeserialize
        + Serialize
        + DeserializeOwned;
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Ty<L: LinkingScheme> {
    Enum(Enum<L>),
    Struct(Struct<L>),
    Tuple(Tuple<L>),
    Option {
        value: L::TypeLink,
    },
    Integer(IntegerType, IntegerDisplay),
    ByteArray {
        len: usize,
        display: ByteDisplay,
    },
    Float32,
    Float64,
    String,
    Boolean,
    Skip {
        len: usize,
    },
    ByteVec {
        display: ByteDisplay,
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

impl<L: LinkingScheme> Ty<L> {
    /// Returns true if the type is a skip type
    pub fn is_skip(&self) -> bool {
        matches!(self, Ty::Skip { .. })
    }
}

impl Ty<IndexLinking> {
    /// Fills the next available placeholder in the type with the given link, panicking if no placeholder is available.
    pub fn fill_next_placholder(&mut self, child: Link) {
        let err_msg =
            format!("Called `fill_next_placholder` on a type with no placeholders: {self:?}");
        match self {
            Ty::Enum(e) => {
                e.variants
                    .iter_mut()
                    .find(|v| v.value == Some(Link::Placeholder))
                    .expect(&err_msg).value = Some(child);
            }
            Ty::Struct(s) => {
                s.fields
                    .iter_mut()
                    .find(|field| field.value == Link::Placeholder)
                    .expect(&err_msg)
                    .value = child;
            }
            Ty::Tuple(t) => {
                t.fields
                    .iter_mut()
                    .find(|field| field.value == Link::Placeholder)
                    .expect(&err_msg)
                    .value = child;
            }
            Ty::Option { value } => if *value == Link::Placeholder {
                *value = child;
            } else {
                panic!("{}", err_msg);
            }
            Ty::Array { value, .. } => {
				if *value == Link::Placeholder {
					*value = child;
				} else {
					panic!("{}", err_msg);
				}
			}
            Ty::Vec { value } => if *value == Link::Placeholder {
				*value = child;
			} else {
				panic!("{}", err_msg);
			}
            Ty::Map { key, value } => if *key == Link::Placeholder {
				*key = child;
			} else if *value == Link::Placeholder {
				*value = child;
			} else {
				panic!("{}", err_msg);
			}
            _ => panic!(
                "Tried to fill a placholder on a type with no children. Only Vec, Tuple, Option, Array, Struct and Map types have children. Self: {:?}",
                self
            ),
        }
    }
}

impl<L: LinkingScheme> Ty<L> {
    pub fn is_primitive(&self) -> bool {
        // Match exhaustively in case additional primitives are added later
        match self {
            Ty::Enum(_)
            | Ty::Struct(_)
            | Ty::Tuple(_)
            | Ty::Option { .. }
            | Ty::Array { .. }
            | Ty::Vec { .. }
            | Ty::Map { .. } => false,
            Ty::Integer(_, _)
            | Ty::ByteArray { .. }
            | Ty::ByteVec { .. }
            | Ty::String
            | Ty::Float32
            | Ty::Float64
            | Ty::Boolean
            | Ty::Skip { .. } => true,
        }
    }

    pub fn parent_byte_references(&self) -> Vec<(usize, usize)> {
        match self {
            Ty::Integer(
                _,
                IntegerDisplay::FixedPoint(FixedPointDisplay::FromSiblingField {
                    field_index,
                    byte_offset,
                }),
            ) => vec![(*field_index, *byte_offset)],
            _ => Vec::new(),
        }
    }
}

/// An enum variant can contain...
/// - A (possibly anonymous) struct
/// - Another Enum
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EnumVariant<L: LinkingScheme> {
    pub name: String,
    pub discriminant: u8,
    pub template: Option<String>,
    pub value: Option<L::TypeLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Enum<L: LinkingScheme> {
    pub type_name: String,
    pub variants: Vec<EnumVariant<L>>,
    /// Whether this enum is "hide_tag"ged, meaning that the variant tags shouldn't be displayed.
    pub hide_tag: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Struct<L: LinkingScheme> {
    pub type_name: String,
    pub template: Option<String>,
    pub peekable: bool,
    pub fields: Vec<NamedField<L>>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Tuple<L: LinkingScheme> {
    pub template: Option<String>,
    pub peekable: bool,
    pub fields: Vec<UnnamedField<L>>,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NamedField<L: LinkingScheme> {
    pub display_name: String,
    pub silent: bool,
    pub value: L::TypeLink,
    pub doc: String,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct UnnamedField<L: LinkingScheme> {
    pub value: L::TypeLink,
    pub silent: bool,
    pub doc: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[allow(non_camel_case_types)]
pub enum IntegerType {
    i8,
    i16,
    i32,
    i64,
    i128,
    u8,
    u16,
    u32,
    u64,
    u128,
}

impl IntegerType {
    pub fn size(&self) -> usize {
        match self {
            IntegerType::i8 => core::mem::size_of::<i8>(),
            IntegerType::i16 => core::mem::size_of::<i16>(),
            IntegerType::i32 => core::mem::size_of::<i32>(),
            IntegerType::i64 => core::mem::size_of::<i64>(),
            IntegerType::i128 => core::mem::size_of::<i128>(),
            IntegerType::u8 => core::mem::size_of::<u8>(),
            IntegerType::u16 => core::mem::size_of::<u16>(),
            IntegerType::u32 => core::mem::size_of::<u32>(),
            IntegerType::u64 => core::mem::size_of::<u64>(),
            IntegerType::u128 => core::mem::size_of::<u128>(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum FixedPointDisplay {
    Decimals(u8),
    FromSiblingField {
        field_index: usize,
        byte_offset: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum IntegerDisplay {
    Hex,
    #[default]
    Decimal,
    FixedPoint(FixedPointDisplay),
}

pub trait IntegerDisplayable {
    fn integer_type() -> IntegerType;

    fn with_display(display: IntegerDisplay) -> Link {
        Link::Immediate(Primitive::Integer(Self::integer_type(), display))
    }
}

macro_rules! integer_displayable {
    ($t:ident) => {
        impl IntegerDisplayable for $t {
            fn integer_type() -> IntegerType {
                IntegerType::$t
            }
        }
    };
}
macro_for_ints!(integer_displayable);

pub trait ByteDisplayable {
    fn with_display(display: ByteDisplay) -> Link {
        Link::Immediate(Primitive::ByteVec { display })
    }
}

impl<T: OverrideSchema<Output = Vec<u8>>> ByteDisplayable for T {
    fn with_display(display: ByteDisplay) -> Link {
        Link::Immediate(Primitive::ByteVec { display })
    }
}
impl ByteDisplayable for Vec<u8> {
    fn with_display(display: ByteDisplay) -> Link {
        Link::Immediate(Primitive::ByteVec { display })
    }
}

impl<const N: usize> ByteDisplayable for [u8; N] {
    fn with_display(display: ByteDisplay) -> Link {
        Link::Immediate(Primitive::ByteArray { len: N, display })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ContainerSerdeMetadata {
    pub name: String,
    pub fields_or_variants: Vec<FieldOrVariantSerdeMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FieldOrVariantSerdeMetadata {
    pub name: String,
}
