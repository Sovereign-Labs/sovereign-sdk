use std::cmp::max;

use sov_bank::{config_gas_token_id, Amount, Coins, IntoPayable};
use sov_modules_api::registration_lib::StakeRegistration;
use sov_modules_api::{
    AggregatedProofPublicData, Gas, GasSpec, GetGasPrice, InvalidProofError,
    SerializedAggregatedProof, Spec, StateReader, Storage, TxState, VersionReader, ZkVerifier,
    Zkvm,
};
use sov_rollup_interface::common::SlotNumber;
use sov_state::Kernel;
use thiserror::Error;

use crate::event::SlashingReason;
use crate::ProverIncentives;

/// Error raised while processing the aggregated proof.
#[derive(Debug, Error)]
pub enum ProcessProofError {
    #[error(
        "Error occurred when rewarding the prover. This module's account may not have enough funds. This is a bug. Error: {0}"
    )]
    TransferFailure(String),

    #[error("Prover slashed: {0}")]
    ProverSlashedNoRevert(String),

    #[error("Prover penalized: {0}")]
    ProverPenalizedNoRevert(String),

    #[error("Prover is not bonded at the time of the transaction")]
    ProverNotBonded,

    #[error("The bond is not high enough")]
    BondNotHighEnough,

    #[error("An error occurred when trying to access the state, error: {0}")]
    StateAccessorError(#[from] anyhow::Error),

    #[error("Prover incentives called with invalid operating mode")]
    InvalidOperatingMode,
}

impl From<ProcessProofError> for InvalidProofError {
    fn from(error: ProcessProofError) -> Self {
        match error {
            ProcessProofError::ProverSlashedNoRevert(e) => {
                InvalidProofError::ProverSlashed(e.to_string())
            }
            ProcessProofError::ProverPenalizedNoRevert(e) => {
                InvalidProofError::ProverPenalized(e.to_string())
            }
            ProcessProofError::ProverNotBonded
            | ProcessProofError::BondNotHighEnough
            | ProcessProofError::InvalidOperatingMode => {
                InvalidProofError::PreconditionNotMet(error.to_string())
            }
            ProcessProofError::StateAccessorError(e) => {
                InvalidProofError::StateAccess(e.to_string())
            }
            ProcessProofError::TransferFailure(e) => InvalidProofError::RewardFailure(e),
        }
    }
}

enum Paycheck {
    Penalized,
    Rewarded(Amount),
}

impl<S: Spec> ProverIncentives<S> {
    /// Try to process a zk proof, if the prover is bonded.
    #[allow(clippy::type_complexity)]
    pub fn process_proof<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: &SerializedAggregatedProof,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<
        AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        ProcessProofError,
    > {
        if !self.should_reward_fees(state) {
            return Err(ProcessProofError::InvalidOperatingMode);
        }

        // Get the prover's old balance.
        // Revert if they aren't bonded
        let old_balance = match self
            .bonded_provers
            .get(prover_address, state)
            .map_err(Into::<anyhow::Error>::into)?
        {
            Some(balance) => balance,
            None => return Err(ProcessProofError::ProverNotBonded),
        };

        // Check that the prover has enough balance to process the proof.
        let minimum_bond = self
            .get_minimum_bond(state)
            .map_err(Into::<anyhow::Error>::into)?;
        let minimum_bond = minimum_bond.expect("The minimum bond should be set at genesis");

        if old_balance < minimum_bond {
            return Err(ProcessProofError::BondNotHighEnough);
        };

        let code_commitment = self
            .chain_state
            .outer_code_commitment(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("The code commitment should be set at genesis");

        state
            .charge_gas(&<S as GasSpec>::fixed_gas_to_charge_per_proof())
            .map_err(Into::<anyhow::Error>::into)?;

        state
            .charge_linear_gas(
                &<S as GasSpec>::gas_to_charge_per_proof_byte(),
                proof
                    .raw_aggregated_proof
                    .len()
                    .try_into()
                    .map_err(Into::<anyhow::Error>::into)?,
            )
            .map_err(Into::<anyhow::Error>::into)?;

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        let verification_result = <<S as Spec>::OuterZkvm as Zkvm>::Verifier::verify::<
            AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        >(&proof.raw_aggregated_proof, &code_commitment);

        let public_outputs = match verification_result {
            Ok(public_outputs) => public_outputs,
            Err(e) => {
                tracing::debug!(verification_error = ?e, "Slashing prover for invalid proof");

                self.slash_prover(prover_address, state)?;
                // The state won't be reverted.
                return Err(ProcessProofError::ProverSlashedNoRevert(
                    "Verification failed".to_string(),
                ));
            }
        };

        if let Some(slashing_reason) = self
            .check_proof_outputs(&public_outputs, state)
            .map_err(Into::<anyhow::Error>::into)?
        {
            tracing::debug!(?slashing_reason, "Slashing prover");

            self.slash_prover(prover_address, state)?;
            // The state won't be reverted.
            return Err(ProcessProofError::ProverSlashedNoRevert(format!(
                "Invalid output {slashing_reason}"
            )));
        }

        match self.calculate_reward_and_remove(
            public_outputs.initial_slot_number,
            public_outputs.final_slot_number,
            state,
        )? {
            Paycheck::Penalized => {
                self.penalize_prover(old_balance, prover_address, state)?;
                // The state won't be reverted.
                Err(ProcessProofError::ProverPenalizedNoRevert(
                    "Prover penalized".to_string(),
                ))
            }
            Paycheck::Rewarded(total_reward) => {
                self.reward_prover(total_reward, prover_address, state)?;
                Ok(public_outputs)
            }
        }
    }

    fn slash_prover(
        &mut self,
        prover_address: &S::Address,
        state: &mut impl TxState<S>,
    ) -> Result<(), anyhow::Error> {
        Ok(self.bonded_provers.delete(prover_address, state)?)
    }

    /// Computes the total reward from the aggregated state transition and rewards the prover with the unclaimed
    /// transition rewards. If all the rewards were already claimed, the prover is fined by a constant amount.
    fn calculate_reward_and_remove(
        &mut self,
        init_slot_num: SlotNumber,
        final_slot_num: SlotNumber,
        state: &mut impl TxState<S>,
    ) -> Result<Paycheck, ProcessProofError> {
        // Let's compute the total reward
        let mut total_reward = Amount::ZERO;

        let first_available_reward = self
            .last_claimed_reward
            .get(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("The last claimed reward should be set at genesis")
            .next();

        // The first reward we can claim is the maximum between the initial rollup height and the first available reward
        let first_claimed_reward = max(init_slot_num, first_available_reward);

        // Here the final rollup height is inclusive
        for slot_num in first_claimed_reward.range_inclusive(final_slot_num) {
            // Check if the reward was already claimed

            // If not, reward the prover with the block reward
            // `get_historical_transitions` should always return `Some` because we are iterating over the range of `init_slot_num..=final_slot_num`
            // whose integrity was checked beforehand.
            if let Some(transition) = self
                .chain_state
                .slot_at_height(slot_num, state)
                .map_err(Into::<anyhow::Error>::into)?
            {
                // SAFETY: this cannot overflow, because that would require more than the entire token supply to be spent on gas
                // *before* the prover claimed their reward, but gas fees are locked until the prover claims them.
                let curr_reward = transition.gas_used().value(transition.gas_price());
                total_reward = total_reward
                    .checked_add(curr_reward)
                    .expect("Gas token Overflow");
            }
        }

        // We need to remove the reward once it is claimed
        self.last_claimed_reward
            .set(&max(first_available_reward, final_slot_num), state)
            .map_err(Into::<anyhow::Error>::into)?;

        if first_claimed_reward > final_slot_num {
            Ok(Paycheck::Penalized)
        } else {
            Ok(Paycheck::Rewarded(total_reward))
        }
    }

    fn penalize_prover<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        old_balance: Amount,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), ProcessProofError> {
        // Penalize the prover
        let fine = self
            .proving_penalty_value(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("Should be set at genesis");

        let new_balance = old_balance
            .checked_sub(fine)
            .expect("We already checked that the balance is greater than the fine");

        self.bonded_provers
            .set(prover_address, &new_balance, state)
            .map_err(Into::<anyhow::Error>::into)?;

        Ok(())
    }

    fn reward_prover(
        &mut self,
        total_reward: Amount,
        prover_address: &S::Address,
        state: &mut impl TxState<S>,
    ) -> Result<(), ProcessProofError> {
        // We only reward a portion of the total reward - we burn some of it
        // to avoid the provers to collude to prove empty blocks.
        let reward_amount = self.burn_rate().apply(total_reward);

        let coins = Coins {
            token_id: config_gas_token_id(),
            amount: reward_amount,
        };

        self.bank
            .transfer_from(self.id.to_payable(), prover_address, coins, state)
            .map_err(|err| ProcessProofError::TransferFailure(err.to_string()))?;

        Ok(())
    }

    /// Check that the initial and final state values of the proof output are valid against the chain state module
    fn check_proof_outputs<ST: VersionReader + StateReader<Kernel>>(
        &self,
        public_outputs: &AggregatedProofPublicData<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
        >,
        state: &mut ST,
    ) -> Result<Option<SlashingReason>, ST::Error> {
        let expected_genesis_hash = self
            .chain_state
            .get_genesis_hash(state)?
            .expect("The genesis hash should be set at genesis");

        // We have to check that the genesis hash is valid
        if expected_genesis_hash != public_outputs.genesis_state_root {
            return Ok(Some(SlashingReason::IncorrectGenesisHash));
        }

        // We start with the initial state values
        let initial_slot_num = public_outputs.initial_slot_number;
        let Some(initial_slot) = self.chain_state.slot_at_height(initial_slot_num, state)? else {
            return Ok(Some(SlashingReason::InitialTransitionDoesNotExist));
        };

        if initial_slot.prev_state_root() != &public_outputs.initial_state_root {
            return Ok(Some(SlashingReason::IncorrectInitialStateRoot));
        }

        let initial_transition_hash = initial_slot.slot_hash();
        if initial_transition_hash != &public_outputs.initial_slot_hash {
            return Ok(Some(SlashingReason::IncorrectInitialSlotHash));
        }

        // Let's move on to the final state values
        let final_slot_num = public_outputs.final_slot_number;
        // Check that the final da block hash is correct
        let expected_final_transition = match self
            .chain_state
            .get_historical_transition_dangerous(final_slot_num, state)?
        {
            Some(expected_final_transition) => expected_final_transition,
            None => {
                tracing::debug!(%final_slot_num, "No historical state transition found for final slot number. Recall that state transitions are not visible until the slot *after* the transition is visible.");
                return Ok(Some(SlashingReason::FinalTransitionDoesNotExist));
            }
        };

        if expected_final_transition.slot().slot_hash() != &public_outputs.final_slot_hash {
            return Ok(Some(SlashingReason::IncorrectFinalSlotHash));
        }

        if expected_final_transition.post_state_root() != &public_outputs.final_state_root {
            return Ok(Some(SlashingReason::IncorrectFinalStateRoot));
        }

        Ok(None)
    }
}
