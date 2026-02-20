use sov_modules_api::{err_detail, CoreModuleError, ErrorContext, ErrorDetail};

/// Errors that can occur when pruning generation data (admin operation).
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum PruneGenerationsError {
    /// A core module error occurred (e.g. state read/write failure).
    #[error(transparent)]
    CoreModuleError(#[from] CoreModuleError),
    /// The sender is not the chain state admin.
    #[error("Only the chain state admin can prune generations. Admin: {admin}, sender: {sender}")]
    NotAdmin {
        /// The expected admin address.
        admin: String,
        /// The actual sender address.
        sender: String,
    },
    /// No admin address has been configured in chain state.
    #[error("No admin address configured in chain state")]
    NoAdmin,
}

/// Errors that can occur when pruning own generation data (self-service operation).
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum PruneSelfGenerationsError {
    /// A core module error occurred (e.g. state read/write failure).
    #[error(transparent)]
    CoreModuleError(#[from] CoreModuleError),
    /// The provided credential ID does not derive to the sender's address.
    #[error("Credential {credential_id} does not belong to sender. Expected sender: {expected_sender}, actual: {actual_sender}")]
    NotOwner {
        /// The credential ID that was provided.
        credential_id: String,
        /// The address derived from the credential ID.
        expected_sender: String,
        /// The actual sender address.
        actual_sender: String,
    },
}

/// The top-level error type for all uniqueness module operations.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Error {
    /// An error occurred during admin generation pruning.
    #[error("Prune generations error: {0}")]
    PruneGenerations(#[from] PruneGenerationsError),
    /// An error occurred during self generation pruning.
    #[error("Prune self generations error: {0}")]
    PruneSelfGenerations(#[from] PruneSelfGenerationsError),
}

impl ErrorDetail for Error {
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!(self))
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        PruneGenerationsError::from(CoreModuleError::Generic(err)).into()
    }
}
