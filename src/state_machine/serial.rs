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
pub trait Encode {
    fn encode(&self, target: &mut impl std::io::Write);

    fn encode_to_vec(&self) -> Vec<u8> {
        let mut target = Vec::new();
        self.encode(&mut target);
        target
    }
}

pub trait Decode: Sized {
    fn decode(target: &mut &[u8]) -> Result<Self, DeserializationError>;
}
