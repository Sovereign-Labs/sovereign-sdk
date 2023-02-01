#[cfg(feature = "sync")]
use std::sync::Arc;
use std::{ops::Deref, rc::Rc};

use thiserror::Error;

#[derive(Debug, PartialEq, Error)]
pub enum DeserializationError {
    #[error("Data was too short to deserialize. Expected {expected:}, got {got:}")]
    DataTooShort { expected: usize, got: usize },
    #[error("Invalid enum tag. Only tags 0-{max_allowed:} are valid, got {got:}")]
    InvalidTag { max_allowed: u8, got: u8 },
}

// TODO: do this in a sensible/generic way
// The objective is to not introduce a forcible serde dependency and potentially
// allow implementers to use rykv or another zero-copy framework. But we
// need to design that. This will work for now

/// Trait used to express encoding relationships.
pub trait Encode {
    fn encode(&self, target: &mut impl std::io::Write);

    fn encode_to_vec(&self) -> Vec<u8> {
        let mut target = Vec::new();
        self.encode(&mut target);
        target
    }
}

/// Trait used to express decoding relationships.
pub trait Decode: Sized {
    type Error;

    fn decode(target: &mut &[u8]) -> Result<Self, Self::Error>;
}

#[cfg(feature = "sync")]
impl<T> Encode for Arc<T>
where
    T: Encode,
{
    fn encode(&self, target: &mut impl std::io::Write) {
        self.deref().encode(target)
    }
}

#[cfg(feature = "sync")]
impl<T, E> Decode for Arc<T>
where
    T: Decode<Error = E>,
{
    type Error = E;

    fn decode(target: &mut &[u8]) -> Result<Self, Self::Error> {
        Ok(Arc::new(T::decode(target)?))
    }
}

impl<T> Encode for Rc<T>
where
    T: Encode,
{
    fn encode(&self, target: &mut impl std::io::Write) {
        self.deref().encode(target)
    }
}

impl<T, E> Decode for Rc<T>
where
    T: Decode<Error = E>,
{
    type Error = E;

    fn decode(target: &mut &[u8]) -> Result<Self, Self::Error> {
        Ok(Rc::new(T::decode(target)?))
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, target: &mut impl std::io::Write) {
        target
            .write_all(&(self.len() as u64).to_le_bytes())
            .expect("Serialization should not fail");
        for item in self.iter() {
            item.encode(target)
        }
    }
}

impl<T: Decode<Error = E>, E> Decode for Vec<T>
where
    DeserializationError: From<E>,
{
    type Error = DeserializationError;

    fn decode(target: &mut &[u8]) -> Result<Self, Self::Error> {
        if target.len() < 8 {
            return Err(DeserializationError::DataTooShort {
                expected: 8,
                got: target.len(),
            });
        }
        let mut serialized_len = [0u8; 8];
        serialized_len.copy_from_slice(&target[..8]);
        let len: u64 = u64::from_le_bytes(serialized_len);
        *target = &mut &target[8..];

        let mut out = Vec::new();
        for _ in 0..len {
            out.push(T::decode(target)?)
        }
        Ok(out)
    }
}

impl<T, U> Encode for (T, U)
where
    T: Encode,
    U: Encode,
{
    fn encode(&self, target: &mut impl std::io::Write) {
        self.0.encode(target);
        self.1.encode(target)
    }
}

impl<T, U, E1, E2> Decode for (T, U)
where
    T: Decode<Error = E1>,
    U: Decode<Error = E2>,
    DeserializationError: From<E1>,
    DeserializationError: From<E2>,
{
    type Error = DeserializationError;

    fn decode(target: &mut &[u8]) -> Result<Self, Self::Error> {
        Ok((T::decode(target)?, U::decode(target)?))
    }
}
