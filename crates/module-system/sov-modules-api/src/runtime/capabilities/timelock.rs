//! Timelock capability interfaces.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::{err_detail, Error, ErrorContext, ErrorDetail, HexHash, Spec, TxState};

/// Identifier for a timelock proposal.
///
/// This is the hash of an encoded runtime call message, not the hash of a raw transaction.
pub type ProposalId = HexHash;

/// Default number of seconds after unlock during which a proposal may be executed.
pub const DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK: u64 = 86_400 * 2; // 2 days

/// Timelock policy returned by a runtime for call messages that must be delayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelockPolicy {
    /// Number of seconds after proposal registration before the proposal may be unlocked.
    pub unlock_seconds_from_proposal: NonZeroU64,
    /// Optional override for the post-unlock execution window, in seconds. If unset,
    /// [`DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK`] is used.
    ///
    /// Caution: an override of `0` has no special meaning, and the proposal will be executable
    /// only at its exact unlock timestamp. Take care to set reasonable expiraty delays.
    pub expire_seconds_after_unlock_override: Option<u64>,
}

impl TimelockPolicy {
    /// Returns the post-unlock execution window in seconds, applying the default if needed.
    pub fn expire_seconds_after_unlock(&self) -> u64 {
        self.expire_seconds_after_unlock_override
            .unwrap_or(DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK)
    }
}

/// Errors returned when trying to unlock a timelock proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum TimelockError {
    /// The proposal does not exist.
    #[error("Timelock proposal does not exist")]
    ProposalNotFound,
    /// The proposal exists but is still locked.
    #[error("Timelock proposal is still locked")]
    ProposalLocked,
    /// The proposal existed but is expired.
    #[error("Timelock proposal has expired")]
    ProposalExpired,
}

impl ErrorDetail for TimelockError {
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!(self))
    }
}

/// Capability for managing timelock proposals.
///
/// Methods return [`crate::Error`] so semantic timelock failures and state/gas access
/// failures can use the same error path as [`crate::DispatchCall::dispatch_call`].
pub trait TimelockCapability<S: Spec> {
    /// Returns true if the proposal exists for the provided address.
    fn has_proposal(
        &self,
        address: &S::Address,
        proposal_id: &ProposalId,
        state: &mut impl TxState<S>,
    ) -> Result<bool, Error>;

    /// Registers a new proposal for the provided address.
    fn register_proposal(
        &mut self,
        address: &S::Address,
        proposal_id: ProposalId,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error>;

    /// Tries to unlock a proposal, consuming it if unlocking succeeds.
    fn try_unlock_proposal(
        &mut self,
        address: &S::Address,
        proposal_id: &ProposalId,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error>;
}

impl<S: Spec> TimelockCapability<S> for () {
    fn has_proposal(
        &self,
        _address: &S::Address,
        _proposal_id: &ProposalId,
        _state: &mut impl TxState<S>,
    ) -> Result<bool, Error> {
        Ok(false)
    }

    fn register_proposal(
        &mut self,
        _address: &S::Address,
        _proposal_id: ProposalId,
        _policy: TimelockPolicy,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn try_unlock_proposal(
        &mut self,
        _address: &S::Address,
        _proposal_id: &ProposalId,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        Err(TimelockError::ProposalNotFound.into())
    }
}

impl<S: Spec, T: TimelockCapability<S> + ?Sized> TimelockCapability<S> for &mut T {
    fn has_proposal(
        &self,
        address: &S::Address,
        proposal_id: &ProposalId,
        state: &mut impl TxState<S>,
    ) -> Result<bool, Error> {
        (**self).has_proposal(address, proposal_id, state)
    }

    fn register_proposal(
        &mut self,
        address: &S::Address,
        proposal_id: ProposalId,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        (**self).register_proposal(address, proposal_id, policy, state)
    }

    fn try_unlock_proposal(
        &mut self,
        address: &S::Address,
        proposal_id: &ProposalId,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        (**self).try_unlock_proposal(address, proposal_id, state)
    }
}
