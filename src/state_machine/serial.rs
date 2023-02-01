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

// Automatically implement encode for smart pointers
impl<T, U> Encode for T
where
    T: Deref<Target = U>,
    U: Encode,
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

impl<T, E> Decode for Rc<T>
where
    T: Decode<Error = E>,
{
    type Error = E;

    fn decode(target: &mut &[u8]) -> Result<Self, Self::Error> {
        Ok(Rc::new(T::decode(target)?))
    }
}
