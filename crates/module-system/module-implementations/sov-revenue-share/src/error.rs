use thiserror::Error;

/// Errors that can occur in the revenue share module
#[derive(Debug, Error)]
pub enum RevenueShareError {
    /// The caller is not authorized to perform this action
    #[error("Not authorized")]
    NotAuthorized,

    /// The sovereign admin is not set
    #[error("Sovereign admin not set")]
    AdminNotSet,

    /// Cannot increase the revenue share percentage
    #[error("Cannot increase revenue share percentage: current is {current_bps} bps, attempted to set {new_bps} bps")]
    CannotIncreasePercentage {
        /// The current revenue share percentage in basis points
        current_bps: u16,
        /// The new revenue share percentage that was attempted
        new_bps: u16,
    },

    /// Invalid percentage value
    #[error("Invalid percentage value: {value} bps (must be 0-10000 basis points)")]
    InvalidPercentage {
        /// The invalid percentage value that was provided
        value: u16,
    },

    /// No revenue available to withdraw
    #[error("No revenue available to withdraw")]
    NoRevenueToWithdraw,
}
