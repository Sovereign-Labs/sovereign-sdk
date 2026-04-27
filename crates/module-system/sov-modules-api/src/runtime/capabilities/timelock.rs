//! Timelock capability interfaces.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::{HexHash, Spec, TxState};

/// Identifier for a timelock proposal.
///
/// This is the hash of an encoded runtime call message, not the hash of a raw transaction.
pub type ProposalId = HexHash;

/// Timelock policy returned by a runtime for call messages that must be delayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelockPolicy {
    /// Number of seconds that must pass before the proposal may be unlocked.
    pub unlock_seconds_from_now: NonZeroU64,
    /// Number of seconds after unlock during which the proposal may be executed.
    pub expire_seconds_after_unlock: NonZeroU64,
}

/// Errors returned when trying to unlock a timelock proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
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

/// Capability for managing timelock proposals.
pub trait TimelockCapability<S: Spec> {
    /// Returns true if the proposal exists for the provided address.
    fn has_proposal(
        &self,
        address: &S::Address,
        proposal_id: &ProposalId,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool>;

    /// Registers a new proposal for the provided address.
    fn register_proposal(
        &mut self,
        address: &S::Address,
        proposal_id: ProposalId,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()>;

    /// Tries to unlock a proposal, consuming it if unlocking succeeds.
    fn try_unlock_proposal(
        &mut self,
        address: &S::Address,
        proposal_id: &ProposalId,
        state: &mut impl TxState<S>,
    ) -> Result<(), TimelockError>;
}

impl<S: Spec> TimelockCapability<S> for () {
    fn has_proposal(
        &self,
        _address: &S::Address,
        _proposal_id: &ProposalId,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn register_proposal(
        &mut self,
        _address: &S::Address,
        _proposal_id: ProposalId,
        _policy: TimelockPolicy,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn try_unlock_proposal(
        &mut self,
        _address: &S::Address,
        _proposal_id: &ProposalId,
        _state: &mut impl TxState<S>,
    ) -> Result<(), TimelockError> {
        Err(TimelockError::ProposalNotFound)
    }
}
