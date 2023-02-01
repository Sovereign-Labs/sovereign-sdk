use thiserror::Error;

use crate::serial::DeserializationError;

#[derive(Error, Debug)]
pub enum CodecError {
    #[error("Invalid key length. Expected {expected:}, got {got:}")]
    InvalidKeyLength { expected: usize, got: usize },
    #[error(transparent)]
    DeserializationError(#[from] DeserializationError),
    #[error(transparent)]
    Wrapped(#[from] anyhow::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn convert_to_codec_error(e: impl Into<anyhow::Error>) -> CodecError {
    CodecError::Wrapped(e.into())
}

// fn test() {
//     let t = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "errr".to_string());
//     let x: Result<(), std::io::Error> = Err(t);
//     let y: CodecError = x.map_err(sovereign_sdk::db::errors::convert_to_codec_error)
//     // println!("{}", t.into())
// }

// impl<T> From<T> for CodecError
// where
//     T: Into<anyhow::Error>,
// {
//     fn from(value: T) -> Self {
//         Self::Unknown(value.into())
//     }
// }
