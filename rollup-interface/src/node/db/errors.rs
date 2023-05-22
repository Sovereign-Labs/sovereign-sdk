use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodecError {
    #[error("Invalid key length. Expected {expected:}, got {got:}")]
    InvalidKeyLength { expected: usize, got: usize },
    #[error(transparent)]
    Wrapped(#[from] anyhow::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn convert_to_codec_error(e: impl Into<anyhow::Error>) -> CodecError {
    CodecError::Wrapped(e.into())
}
