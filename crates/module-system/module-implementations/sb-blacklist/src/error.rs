use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlacklistError {

    #[error("Owner not initialized")]
    OwnerNotInitialized,

    #[error("Manager not initialized")]
    ManagerNotInitialized,

    #[error("Caller is not the owner")]
    UnauthorizedOwner,

    #[error("Caller is not the manager")]
    UnauthorizedManager,

    #[error("Caller is not an authorized blacklist signer")]
    UnauthorizedBlacklistSigner,

    #[error("Wallet is blacklisted")]
    WalletBlacklisted,

    #[error("Discrepancy in wallets/blacklisted lengths")]
    InvalidBatchLengths,
}
