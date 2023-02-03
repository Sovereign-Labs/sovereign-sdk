#[cfg(feature = "sync")]
use std::sync::Arc;
use std::{ops::Deref, rc::Rc};

// use borsh::maybestd::io::Error as StdError;
use borsh::{
    maybestd::{self, io::Read},
    BorshDeserialize,
};

use serde::{
    de::{DeserializeOwned, Visitor},
    Deserialize,
};
use thiserror::Error;

#[derive(Debug, PartialEq, Error)]
pub enum DeserializationError {
    #[error("Data was too short to deserialize. Expected {expected:}, got {got:}")]
    DataTooShort { expected: usize, got: usize },
    #[error("Invalid enum tag. Only tags 0-{max_allowed:} are valid, got {got:}")]
    InvalidTag { max_allowed: u8, got: u8 },
}

/// Trait used to express encoding relationships.
pub trait Encode {
    fn encode(&self, target: &mut impl std::io::Write);

    fn encode_to_vec(&self) -> Vec<u8> {
        let mut target = Vec::new();
        self.encode(&mut target);
        target
    }
}

/// Decode a type from an arbitrary reader.
///
/// Decoding cannot be zero-copy, since zero-copy deserialization depends on the liftime of the input.
/// Types that support zero-copy deserialization implement `DecodeBorrowed` only.
///
///
/// For example, one could implement Decode using serde_json:
///```no_run
/// impl<T> Decode for T where T: DeserializeOwned + for<'de> DecodeBorrowed<'de>  {
///     type Error = serde_json::Error;
///
///     fn decode<R: Read>(target: R) -> Result<Self, Self::Error> {
///         serde_json::from_reader(target)
///     }
/// }
/// ```
pub trait Decode: Sized + for<'de> DecodeBorrowed<'de> {
    type Error;
    fn decode<R: Read>(target: &mut R) -> Result<Self, <Self as Decode>::Error>;
}

/// Decode a type from a slice of bytes with a known lifetime, tying
/// the lifetime of the deserialized value to the lifetime of the input.
/// Supports zero-copy deserialization.
///
/// For example, one could implement DecodeBorrowed using serde_json:
/// ```no_run
/// impl<'de, T> DecodeBorrowed<'de> for T where T: Deserialize<'de> {
///     type Error = serde_json::Error;
///
///     fn decode_from_slice(target: &'de [u8]) -> Result<Self, Self::Error> {
///         serde_json::from_slice(target)
///     }
/// }
/// ```
pub trait DecodeBorrowed<'de>: Sized {
    type Error;
    fn decode_from_slice(target: &'de [u8]) -> Result<Self, Self::Error>;
}

impl<T: BorshDeserialize> Decode for T {
    type Error = maybestd::io::Error;

    fn decode<R: Read>(target: &mut R) -> Result<Self, <Self as Decode>::Error> {
        T::deserialize_reader(&mut target)
    }
}

impl<'de, T: BorshDeserialize> DecodeBorrowed<'de> for T {
    type Error = maybestd::io::Error;

    fn decode_from_slice(target: &'de [u8]) -> Result<Self, Self::Error> {
        T::deserialize(&mut &target)
    }
}
