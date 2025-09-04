use std::fmt;

use sov_encryption::EncryptionError;

#[derive(thiserror::Error, Debug)]
pub enum EncryptedDaError {
    #[error("Encryption error: {0}")]
    Encryption(#[from] EncryptionError),

    #[error("Inner DA service error: {0}")]
    InnerService(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Blob processing error: {0}")]
    BlobProcessing(String),
}

impl EncryptedDaError {
    pub fn inner_service<E: fmt::Display>(error: E) -> Self {
        Self::InnerService(error.to_string())
    }

    pub fn blob_processing<S: Into<String>>(msg: S) -> Self {
        Self::BlobProcessing(msg.into())
    }
}

impl From<anyhow::Error> for EncryptedDaError {
    fn from(error: anyhow::Error) -> Self {
        Self::InnerService(error.to_string())
    }
}