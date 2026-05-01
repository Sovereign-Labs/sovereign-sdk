//! Timelock capability interfaces.

use std::num::NonZeroU64;

use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{calculate_hash, calculate_hash_metered};
use crate::{
    err_detail, Error, ErrorContext, ErrorDetail, GasMeter, GasMeteringError, HexHash, HexString,
    ModuleId, Spec, TxState,
};

/// Identifier for a timelock proposal.
///
/// This is the hash of an encoded runtime call message, not the hash of a raw transaction.
pub type ProposalId = HexHash;

/// Default number of seconds after unlock during which a proposal may be executed.
pub const DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK: u64 = 86_400 * 2; // 2 days

/// Domain-separated proposal data used to compute timelock proposal ids.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TimelockProposalData {
    /// Encoded runtime call message bytes.
    CallMessage {
        /// Encoded runtime call message bytes.
        data: HexString,
    },
    /// Module-owned custom proposal bytes.
    CustomData {
        /// Module owning the custom proposal.
        module_id: ModuleId,
        /// Encoded module-specific proposal payload.
        data: HexString,
    },
}

impl TimelockProposalData {
    /// Creates proposal data for an encoded runtime call message.
    pub fn call_message(data: impl Into<Vec<u8>>) -> Self {
        Self::CallMessage {
            data: HexString::new(data.into()),
        }
    }

    /// Creates module-owned custom proposal data.
    pub fn custom_data(module_id: ModuleId, data: impl Into<Vec<u8>>) -> Self {
        Self::CustomData {
            module_id,
            data: HexString::new(data.into()),
        }
    }
}

/// Calculates a timelock proposal id and charges gas for hashing.
///
/// The input is domain-separated so runtime call messages cannot overlap with
/// module-owned custom proposal payloads.
pub fn calculate_timelock_proposal_id_metered<G: GasMeter<Spec = S>, S: Spec>(
    data: &TimelockProposalData,
    gas_meter: &mut G,
) -> Result<ProposalId, GasMeteringError<S::Gas>> {
    let encoded_data = borsh::to_vec(data).expect("Serialization to vec is infallible");
    calculate_hash_metered::<G, S>(&encoded_data, gas_meter)
}

/// Calculates a module-owned custom timelock proposal id and charges gas for hashing.
pub fn calculate_custom_timelock_proposal_id_metered<G: GasMeter<Spec = S>, S: Spec>(
    module_id: &ModuleId,
    data: &[u8],
    gas_meter: &mut G,
) -> Result<ProposalId, GasMeteringError<S::Gas>> {
    let proposal_data = TimelockProposalData::custom_data(*module_id, data);
    calculate_timelock_proposal_id_metered::<G, S>(&proposal_data, gas_meter)
}

/// Calculates a timelock proposal id without charging gas.
///
/// This helper is intended for tests and clients that need to derive the same proposal id
/// outside transaction execution.
pub fn calculate_timelock_proposal_id<S: Spec>(data: &TimelockProposalData) -> ProposalId {
    let encoded_data = borsh::to_vec(data).expect("Serialization to vec is infallible");
    calculate_hash::<S>(&encoded_data)
}

/// Calculates a module-owned custom timelock proposal id without charging gas.
///
/// This helper is intended for tests and clients that need to derive the same proposal id
/// outside transaction execution.
pub fn calculate_custom_timelock_proposal_id<S: Spec>(
    module_id: &ModuleId,
    data: &[u8],
) -> ProposalId {
    calculate_timelock_proposal_id::<S>(&TimelockProposalData::custom_data(*module_id, data))
}

/// Timelock policy returned by a runtime for call messages that must be delayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelockPolicy {
    /// Number of seconds after proposal registration before the proposal may be unlocked.
    pub unlock_seconds_from_proposal: NonZeroU64,
    /// Optional override for the post-unlock execution window, in seconds. If unset,
    /// [`DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK`] is used.
    ///
    /// Caution: an override of `0` has no special meaning, and the proposal will be executable
    /// only at its exact unlock timestamp. Take care to set reasonable expiry windows.
    pub expire_seconds_after_unlock_override: Option<u64>,
}

impl TimelockPolicy {
    /// Returns the post-unlock execution window in seconds, applying the default if needed.
    pub fn expire_seconds_after_unlock(&self) -> u64 {
        self.expire_seconds_after_unlock_override
            .unwrap_or(DEFAULT_EXPIRE_SECONDS_AFTER_UNLOCK)
    }
}

/// Outcome of registering or unlocking a timelock proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelockProposalOutcome {
    /// The proposal did not exist and has been registered.
    Registered,
    /// The proposal existed, was unlocked, and has been consumed.
    Unlocked,
}

/// Errors returned when trying to unlock a timelock proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum TimelockError {
    /// The runtime does not provide a timelock capability.
    #[error("Timelocks are not available in this runtime")]
    TimelocksNotAvailable,
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
    /// Registers a proposal if it does not exist, or tries to unlock and consume an existing proposal.
    fn register_or_try_unlock_proposal(
        &mut self,
        address: &S::Address,
        proposal_data: TimelockProposalData,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> Result<TimelockProposalOutcome, Error>;
}

impl<S: Spec> TimelockCapability<S> for () {
    fn register_or_try_unlock_proposal(
        &mut self,
        _address: &S::Address,
        _proposal_data: TimelockProposalData,
        _policy: TimelockPolicy,
        _state: &mut impl TxState<S>,
    ) -> Result<TimelockProposalOutcome, Error> {
        Err(TimelockError::TimelocksNotAvailable.into())
    }
}

impl<S: Spec, T: TimelockCapability<S> + ?Sized> TimelockCapability<S> for &mut T {
    fn register_or_try_unlock_proposal(
        &mut self,
        address: &S::Address,
        proposal_data: TimelockProposalData,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> Result<TimelockProposalOutcome, Error> {
        (**self).register_or_try_unlock_proposal(address, proposal_data, policy, state)
    }
}
