use sov_modules_api::capabilities::{ProposalId, TimelockError};
use sov_modules_api::{err_detail, CoreModuleError, ErrorContext, ErrorDetail, Spec};

use crate::ProposalKey;

/// Errors returned by the timelock module.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(
    tag = "error_code",
    rename_all = "snake_case",
    bound = "S::Address: serde::Serialize"
)]
pub enum Error<S: Spec> {
    /// A core module error occurred.
    #[error(transparent)]
    Core(#[from] CoreModuleError),
    /// A timelock semantic error occurred.
    #[error(transparent)]
    Timelock(#[from] TimelockError),
    /// The proposal already exists.
    #[error("Proposal {proposal_id} already exists for address {address}")]
    ProposalAlreadyExists {
        /// Proposal owner.
        address: S::Address,
        /// Existing proposal id.
        proposal_id: ProposalId,
    },
    /// The proposal owner already has the maximum number of pending proposals.
    #[error(
        "Address {address} already has the maximum of {max_pending_proposals} pending proposals"
    )]
    MaxPendingProposalsReached {
        /// Proposal owner.
        address: S::Address,
        /// Maximum pending proposals per address.
        max_pending_proposals: u32,
    },
    /// The proposal counter is inconsistent with the proposal map.
    #[error(
        "Proposal count for address {address} is inconsistent while deleting proposal {proposal_id}"
    )]
    ProposalCountUnderflow {
        /// Proposal owner.
        address: S::Address,
        /// Proposal id being deleted.
        proposal_id: ProposalId,
    },
    /// The proposal exists but has not expired yet.
    #[error(
        "Proposal {proposal_key} is not expired at current time {current_time}; executable until {executable_until}"
    )]
    ProposalNotExpired {
        /// Proposal key.
        proposal_key: ProposalKey<S>,
        /// Current chain time in seconds.
        current_time: u64,
        /// Unix timestamp after which the proposal is expired.
        executable_until: u64,
    },
    /// The sender is not authorized to cancel the proposal.
    #[error(
        "Address {sender} is not authorized to cancel proposal {proposal_id} for {address}; authorized canceller is {authorized_canceller}"
    )]
    UnauthorizedCanceller {
        /// Transaction sender.
        sender: S::Address,
        /// Proposal owner.
        address: S::Address,
        /// Address authorized by the proposal's cancellation policy.
        authorized_canceller: S::Address,
        /// Proposal id.
        proposal_id: ProposalId,
    },
    /// The current chain time is negative.
    #[error("Current chain time {current_time} is before the Unix epoch")]
    InvalidCurrentTime {
        /// Current chain time in seconds.
        current_time: i64,
    },
    /// Computing an unlock timestamp overflowed.
    #[error(
        "Timestamp overflow while computing unlock condition from current time {current_time}"
    )]
    TimestampOverflow {
        /// Current chain time in seconds.
        current_time: u64,
    },
}

impl<S> ErrorDetail for Error<S>
where
    S: Spec,
    S::Address: serde::Serialize,
{
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!(self))
    }
}
