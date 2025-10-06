//! Methods used to process attestations and challenges.
use core::result::Result::Ok;

use sov_modules_api::{
    Gas, GetGasPrice, InvalidProofError, SerializedAttestation, SerializedChallenge,
    SovAttestation, SovStateTransitionPublicData, Spec, StateTransitionPublicData, TxState,
    ZkVerifier, Zkvm,
};
use sov_rollup_interface::common::SlotNumber;
use sov_state::storage::Storage;
use thiserror::Error;
use tracing::error;

use super::call::SlashingReason;
use crate::helpers::{CheckInitialHashStatus, CheckTransitionStatus};
use crate::AttesterIncentives;

/// Error raised while processing the attester incentives.
#[derive(Debug, Error)]
pub enum ProcessAttestationErrors {
    #[error("Unable to deserialize the attestation.")]
    /// Unable to deserialize the attestation.
    InvalidAttestationFormat,

    #[error("Attester slashed: {0}")]
    /// Attester slashed
    AttesterSlashedNoRevert(SlashingReason),

    #[error("Attester is not bonded at the time of the transaction")]
    /// Attester is not bonded at the time of the transaction
    AttesterNotBonded,

    #[error("Invalid bonding proof")]
    /// The bonding proof was invalid
    InvalidBondingProof,

    #[error("The bond is not a 64-bit number")]
    /// The bond is not a 64-bit number
    InvalidBondFormat,

    #[error("Transition invariant isn't respected")]
    /// Transition invariant isn't respected
    InvalidTransitionInvariant,

    #[error("Error occurred when trying to reward a user. {0}. This is a bug.")]
    /// An error occurred when transferred funds
    RewardTransferFailure(String),

    #[error("Error occurred when accessing the state, error: {0}")]
    /// An error occurred when accessing the state
    StateAccessError(#[from] anyhow::Error),

    #[error("Attester incentives called with invalid operating mode")]
    /// An error occurred due to incorrect operating mode of the rollup.
    InvalidOperatingMode,
}

impl From<ProcessAttestationErrors> for InvalidProofError {
    fn from(error: ProcessAttestationErrors) -> Self {
        match error {
            ProcessAttestationErrors::AttesterSlashedNoRevert(reason) => {
                InvalidProofError::ProverSlashed(format!("{reason}"))
            }
            ProcessAttestationErrors::InvalidAttestationFormat
            | ProcessAttestationErrors::AttesterNotBonded
            | ProcessAttestationErrors::InvalidBondingProof
            | ProcessAttestationErrors::InvalidTransitionInvariant
            | ProcessAttestationErrors::InvalidOperatingMode
            | ProcessAttestationErrors::InvalidBondFormat => {
                InvalidProofError::PreconditionNotMet(format!("{error}"))
            }
            ProcessAttestationErrors::RewardTransferFailure(e) => {
                InvalidProofError::RewardFailure(e)
            }
            ProcessAttestationErrors::StateAccessError(e) => {
                InvalidProofError::StateAccess(e.to_string())
            }
        }
    }
}

/// Error raised while processing the attester incentives.
#[derive(Debug, Error)]
pub enum ProcessChallengeErrors {
    #[error("Challenger slashed")]
    /// The user was slashed. Reason specified by [`SlashingReason`]
    ChallengerSlashedNoRevert(#[source] SlashingReason),

    #[error("Challenger is not bonded at the time of the transaction")]
    /// User is not bonded at the time of the transaction
    ChallengerNotBonded,

    #[error("Error occurred when trying to reward a user: {0}. This is a bug.")]
    /// An error occurred when transferred funds
    RewardTransferFailure(String),

    #[error("Error occurred when accessing the state, error: {0}")]
    /// An error occurred when accessing the state
    StateAccessError(#[from] anyhow::Error),

    #[error("Attester incentives called with invalid operating mode")]
    /// An error occurred due to incorrect operating mode of the rollup.
    InvalidOperatingMode,
}

impl From<ProcessChallengeErrors> for InvalidProofError {
    fn from(error: ProcessChallengeErrors) -> Self {
        match error {
            ProcessChallengeErrors::ChallengerSlashedNoRevert(reason) => {
                InvalidProofError::ProverSlashed(reason.to_string())
            }
            ProcessChallengeErrors::ChallengerNotBonded
            | ProcessChallengeErrors::InvalidOperatingMode => {
                InvalidProofError::PreconditionNotMet(format!("{error}"))
            }
            ProcessChallengeErrors::RewardTransferFailure(e) => InvalidProofError::RewardFailure(e),
            ProcessChallengeErrors::StateAccessError(e) => {
                InvalidProofError::StateAccess(e.to_string())
            }
        }
    }
}

impl<S> AttesterIncentives<S>
where
    S: Spec,
{
    /// Try to process an attestation if the attester is bonded.
    /// This function returns an error (hence ignores the transaction) when the attester is not bonded
    /// or when the module is unable to verify the bonding proof.
    #[allow(clippy::type_complexity)]
    pub fn process_attestation<State: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        sender: &S::Address,
        serialized_attestation: SerializedAttestation,
        state: &mut State,
    ) -> anyhow::Result<SovAttestation<S>, ProcessAttestationErrors> {
        if !self.should_reward_fees(state) {
            return Err(ProcessAttestationErrors::InvalidOperatingMode);
        }

        let attestation = serialized_attestation.to_attestation().map_err(|e| {
            error!(error = ?e, "Unable to deserialize the attestation.");
            ProcessAttestationErrors::InvalidAttestationFormat
        })?;
        if attestation.proof_of_bond.claimed_slot_number == SlotNumber::GENESIS {
            tracing::debug!("Cannot claim attestation for genesis");
            return Err(ProcessAttestationErrors::InvalidTransitionInvariant);
        }
        // We first need to check that the attester is still in the bonding set
        if self
            .bonded_attesters
            .get(sender, state)
            .map_err(Into::<anyhow::Error>::into)?
            .is_none()
        {
            return Err(ProcessAttestationErrors::AttesterNotBonded);
        }

        // If the bonding proof in the attestation is invalid, light clients will ignore the attestation. In that case, we should too.
        self.check_bonding_proof(sender, &attestation, state)?;

        // We suppose that these values are always defined, otherwise we panic
        let last_attested_height = self
            .maximum_attested_height
            .get(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("The maximum attested height should be set at genesis");
        let current_finalized_height = self
            .light_client_finalized_height
            .get(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("The light client finalized height should be set at genesis");
        let finality = self
            .rollup_finality_period
            .get(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("The rollup finality period should be set at genesis");

        assert!(
            current_finalized_height <= last_attested_height,
            "The last attested height should always be below the current finalized height."
        );

        // Update the max_attested_height in case the blocks have already been finalized
        let new_height_to_attest = last_attested_height
            .checked_add(1)
            .expect("reached end of the chain");

        // Minimum height at which the proof of bond can be valid
        let min_height = new_height_to_attest.saturating_sub(finality.get());

        // We have to check the following order invariant is respected:
        // (height to attest - finality) <= bonding_proof.transition_height <= height to attest
        //
        // Which with our variable gives:
        // min_height <= bonding_proof.rollup_height <= new_height_to_attest
        // If this invariant is respected, we can be sure that the attester was bonded at new_height_to_attest.
        if min_height > attestation.proof_of_bond.claimed_slot_number {
            tracing::debug!(%min_height, claimed_slot_number = %attestation.proof_of_bond.claimed_slot_number, "Invalid transition invariant: min_height > bonding_proof.rollup_height");
            return Err(ProcessAttestationErrors::InvalidTransitionInvariant);
        }
        if attestation.proof_of_bond.claimed_slot_number > new_height_to_attest {
            tracing::debug!(%attestation.proof_of_bond.claimed_slot_number, %new_height_to_attest, "Invalid transition invariant: bonding_proof.rollup_height > new_height_to_attest");
            return Err(ProcessAttestationErrors::InvalidTransitionInvariant);
        }

        // From this point below, the attester has been correctly authenticated -
        // any error constitutes a slashable offense which *needs to be reflected in the state*.
        // Hence we don't want to return an error after this point, but rather slash the attester and exit gracefully.

        // First compare the initial hashes

        let check_initial_hash_status = self
            .check_initial_hash(
                attestation.proof_of_bond.claimed_slot_number,
                &attestation,
                state,
            )
            .map_err(Into::<anyhow::Error>::into)?;

        if matches!(check_initial_hash_status, CheckInitialHashStatus::Slash) {
            let reason = SlashingReason::InvalidInitialHash;
            self.slash_attester(sender, state)?;
            error!(reason = ?reason, "Attester was slashed");
            // The state won't be reverted.
            return Err(ProcessAttestationErrors::AttesterSlashedNoRevert(reason));
        }

        let check_transition = self
            .check_transition(
                attestation.proof_of_bond.claimed_slot_number,
                &attestation,
                state,
            )
            .map_err(Into::<anyhow::Error>::into)?;

        match check_transition {
            CheckTransitionStatus::SlashedNoHistoricalTransition => {
                let reason = SlashingReason::TransitionNotFound;
                self.slash_attester(sender, state)?;
                error!(reason = ?reason, "Attester was slashed");

                // The state won't be reverted.
                return Err(ProcessAttestationErrors::AttesterSlashedNoRevert(reason));
            }
            CheckTransitionStatus::SlashInvalidateWrongHash => {
                let reason = SlashingReason::TransitionInvalid;
                error!(reason = ?reason, "Attester was slashed");

                self.slash_and_invalidate_attestation(
                    sender,
                    attestation.proof_of_bond.claimed_slot_number,
                    state,
                )?;

                // The state won't be reverted.
                return Err(ProcessAttestationErrors::AttesterSlashedNoRevert(reason));
            }
            CheckTransitionStatus::Valid => {}
        }

        // Now we have to check whether the claimed_slot_number is the max_attested_height.
        // If so, update the maximum attested height and reward the sender
        if attestation.proof_of_bond.claimed_slot_number == new_height_to_attest {
            // We reward the attester with the amount of gas used for the transition.
            let transition = self
                .chain_state
                .slot_at_height(new_height_to_attest, state)
                .map_err(Into::<anyhow::Error>::into)?
                .expect("The transition should exist. The check has been done above");

            let reward = transition.gas_used().value(transition.gas_price());

            // Update the maximum attested height
            self.maximum_attested_height
                .set(&(new_height_to_attest), state)
                .map_err(Into::<anyhow::Error>::into)?;

            self.transfer_tokens_to_sender(sender, self.burn_rate().apply(reward), state)
                .map_err(|err| {
                    error!( error = ?err, "Error raised transferring reward to the attester");
                    ProcessAttestationErrors::RewardTransferFailure(err.to_string())
                })?;
        }

        // Then we can optimistically process the transaction
        Ok(attestation)
    }

    /// Try to process a zk proof if the challenger is bonded.
    /// Same comment as above for the [`AttesterIncentives::process_attestation`] method: if we have a slashable
    /// offense, we want to be able to exit gracefully.
    #[allow(clippy::type_complexity)]
    pub fn process_challenge<State: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        sender: &S::Address,
        serialized_challenge: &SerializedChallenge,
        rollup_height: SlotNumber,
        state: &mut State,
    ) -> anyhow::Result<SovStateTransitionPublicData<S>, ProcessChallengeErrors> {
        if !self.should_reward_fees(state) {
            return Err(ProcessChallengeErrors::InvalidOperatingMode);
        }

        let proof = &serialized_challenge.raw_challenge;
        // Get the challenger's old balance.
        // Revert if they aren't bonded
        let old_balance = self
            .bonded_challengers
            .get_or_err(sender, state)
            .map_err(Into::<anyhow::Error>::into)?
            .map_err(|_| ProcessChallengeErrors::ChallengerNotBonded)?;

        // Check that the challenger has enough balance to process the proof.
        let minimum_bond = self
            .minimum_challenger_bond
            .get(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("Should be set at genesis");

        if old_balance < minimum_bond.value(state.gas_price()) {
            return Err(ProcessChallengeErrors::ChallengerNotBonded);
        }

        let code_commitment = self
            .chain_state
            .inner_code_commitment(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("Should be set at genesis");

        // Find the faulty attestation pool and get the associated reward
        let attestation_reward = match self
            .bad_transition_pool
            .get_or_err(&rollup_height, state)
            .map_err(Into::<anyhow::Error>::into)?
        {
            Ok(reward) => reward,
            Err(_err) => {
                let reason = SlashingReason::NoInvalidTransition;
                error!(reason = ?reason, "Challenger slashed");
                self.slash_challenger(sender, state)?;

                // The state won't be reverted.
                return Err(ProcessChallengeErrors::ChallengerSlashedNoRevert(reason));
            }
        };

        let public_outputs_opt = <<S::InnerZkvm as Zkvm>::Verifier as ZkVerifier>::verify::<
            StateTransitionPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        >(proof, &code_commitment)
        .map_err(|e| anyhow::format_err!("{:?}", e));

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        match public_outputs_opt {
            Ok(public_output) => {
                // We have to perform the checks to ensure that the challenge is valid while the attestation isn't.

                let check = self
                    .check_challenge_outputs_against_transition(
                        &public_output,
                        rollup_height,
                        state,
                    )
                    .map_err(Into::<anyhow::Error>::into)?;

                if let Some(slashing_reason) = check {
                    error!(reason = ?slashing_reason, "Challenger slashed: Invalid outputs");
                    self.slash_challenger(sender, state)?;

                    // The state won't be reverted.
                    return Err(ProcessChallengeErrors::ChallengerSlashedNoRevert(
                        slashing_reason,
                    ));
                }

                // Reward the sender
                self.transfer_tokens_to_sender(
                    sender,
                    self.burn_rate().apply(attestation_reward),
                    state,
                )
                .map_err(|err| {
                    error!(error = ?err,"Error raised transferring reward to the challenger");
                    ProcessChallengeErrors::RewardTransferFailure(err.to_string())
                })?;

                // Now remove the bad transition from the pool
                self.bad_transition_pool
                    .remove(&rollup_height, state)
                    .map_err(Into::<anyhow::Error>::into)?;
                Ok(public_output)
            }
            Err(err) => {
                // Slash the challenger
                let reason = SlashingReason::InvalidZkProof;
                error!(reason = ?reason, error = ?err, "Challenger slashed: Invalid zk proof");
                self.slash_challenger(sender, state)?;

                // The state won't be reverted.
                Err(ProcessChallengeErrors::ChallengerSlashedNoRevert(reason))
            }
        }
    }
}
