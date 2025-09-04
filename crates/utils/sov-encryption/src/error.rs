
#[derive(thiserror::Error, Debug)]
pub enum EncryptionError {
    #[error("Failed to connect to key server: {0}")]
    KeyServerConnection(#[from] std::io::Error),

    #[error("Key server response error: {0}")]
    KeyServerResponse(String),

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(String),

    #[error("Invalid ciphertext format: {0}")]
    InvalidCiphertextFormat(String),

    #[error("Key rotation error: {0}")]
    KeyRotation(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Hex decoding error: {0}")]
    HexError(#[from] hex::FromHexError),
}

impl EncryptionError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self, 
            EncryptionError::KeyServerConnection(_) | 
            EncryptionError::KeyServerResponse(_) | 
            EncryptionError::KeyRotation(_)
        )
    }
}