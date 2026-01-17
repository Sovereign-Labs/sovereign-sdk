use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionRegistryError {
    #[error("Owner not initialized")]
    OwnerNotInitialized,

    #[error("Manager not initialized")]
    ManagerNotInitialized,

    #[error("Caller is not the owner")]
    UnauthorizedOwner,

    #[error("Caller is not the manager")]
    UnauthorizedManager,

    #[error("Caller is not an authorized session signer")]
    UnauthorizedSessionSigner,

    #[error("Session not active")]
    SessionNotActive,

    #[error("Session not present")]
    SessionNotPresent,

    #[error("Discrepancy in wallets/expiries lengths")]
    InvalidBatchLengths,
}
