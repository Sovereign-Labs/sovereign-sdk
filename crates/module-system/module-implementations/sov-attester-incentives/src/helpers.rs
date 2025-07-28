use anyhow::ensure;
use borsh::BorshDeserialize;
use sov_bank::{config_gas_token_id, Amount, BurnRate, Coins, IntoPayable};
use sov_modules_api::macros::config_value;
use sov_modules_api::optimistic::Attestation;
use sov_modules_api::{
    DaSpec, Gas, GetGasPrice, Spec, StateAccessor, StateReader, StateTransitionPublicData, TxState,
    VersionReader,
};
use sov_rollup_interface::common::SlotNumber;
use sov_state::storage::{SlotKey, SlotValue, Storage, StorageProof};
use sov_state::Kernel;
use tracing::debug;

use crate::{AttesterIncentives, ProcessAttestationErrors, SlashingReason};

impl<S> AttesterIncentives<S>
where
    S: Spec,
{
    /// Returns the burn rate for the reward
    pub fn burn_rate(&self) -> BurnRate {
        BurnRate::new_unchecked(config_value!("PERCENT_BASE_FEE_TO_BURN"))
    }

    /// Verifies the provided proof, returning its underlying storage value, if present.
    pub fn verify_proof(
        &self,
        state_root: <S::Storage as Storage>::Root,
        proof: StorageProof<<S::Storage as Storage>::Proof>,
        expected_key: &S::Address,
    ) -> anyhow::Result<Option<SlotValue>> {
        let (storage_key, storage_value) = S::Storage::open_proof(state_root, proof)?;
        let prefix = self.bonded_attesters.prefix();
        let codec = self.bonded_attesters.codec();

        // We have to check that the storage key is the same as the external key
        ensure!(
            storage_key == SlotKey::new(prefix, expected_key, codec),
            "The storage key from the proof doesn't match the expected storage key."
        );

        Ok(storage_value)
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn check_initial_hash<ST: VersionReader + StateReader<Kernel>>(
        &self,
        claimed_slot_number: SlotNumber,
        attestation: &Attestation<
            <S::Da as DaSpec>::SlotHash,
            <S::Storage as Storage>::Root,
            StorageProof<<S::Storage as Storage>::Proof>,
        >,
        state: &mut ST,
    ) -> anyhow::Result<CheckInitialHashStatus, ST::Error> {
        let previous_height = claimed_slot_number
            .checked_sub(1)
            .expect("Claimed transition height must be > 0");

        // Normal state
        let Some(claimed_root) = self.chain_state.root_at_height(previous_height, state)? else {
            return Ok(CheckInitialHashStatus::Slash);
        };

        if claimed_root != attestation.initial_state_root {
            // The initial root hashes don't match, just slash the attester
            return Ok(CheckInitialHashStatus::Slash);
        }

        Ok(CheckInitialHashStatus::Valid)
    }

    /// A helper function that simply slashes an attester and returns a reward value
    pub(crate) fn slash_attester<TxStateAccessor: TxState<S>>(
        &self,
        user: &S::Address,
        state: &mut TxStateAccessor,
    ) -> Result<Amount, anyhow::Error> {
        // We have to remove the attester from the unbonding set
        // to prevent him from skipping the first phase
        // unbonding if he bonds himself again.
        self.unbonding_attesters.remove(user, state)?;
        let bonded_set = &self.bonded_attesters;

        // We have to deplete the attester's bonded account, it amounts to removing the attester from the bonded set
        let reward = bonded_set.get(user, state)?.unwrap_or_default();
        bonded_set.remove(user, state)?;

        Ok(reward)
    }

    /// A helper function that is used to slash an attester, and put the associated attestation in the slashed pool
    pub(crate) fn slash_and_invalidate_attestation<TxStateAccessor: TxState<S>>(
        &mut self,
        attester: &S::Address,
        height: SlotNumber,
        state: &mut TxStateAccessor,
    ) -> Result<(), anyhow::Error> {
        let reward = self.slash_attester(attester, state)?;

        let curr_reward_value = self
            .bad_transition_pool
            .get(&height, state)?
            .unwrap_or_default();

        let new_value = curr_reward_value.saturating_add(reward);
        self.bad_transition_pool.set(&height, &new_value, state)?;

        Ok(())
    }

    pub(crate) fn transfer_tokens_to_sender(
        &mut self,
        sender: &S::Address,
        amount: Amount,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        let coins = Coins {
            token_id: config_gas_token_id(),
            amount,
        };

        // The reward tokens are unlocked from the module's id.
        self.bank
            .transfer_from(self.id.to_payable(), sender, coins, state)?;

        Ok(())
    }

    /// The bonding proof is now a proof that an attester was bonded during the last `finality_period` range.
    /// The proof must refer to a valid state of the rollup. The initial root hash must represent a state between
    /// the bonding proof one and the current state.
    #[allow(clippy::type_complexity)]
    pub(crate) fn check_bonding_proof<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &self,
        sender: &S::Address,
        attestation: &Attestation<
            <S::Da as DaSpec>::SlotHash,
            <S::Storage as Storage>::Root,
            StorageProof<<S::Storage as Storage>::Proof>,
        >,
        state: &mut ST,
    ) -> Result<(), ProcessAttestationErrors> {
        if attestation.proof_of_bond.claimed_slot_number == SlotNumber::GENESIS {
            debug!("Cannot claim attestation for genesis");
            return Err(ProcessAttestationErrors::InvalidTransitionInvariant);
        }

        // If we cannot get the transition before the current one, it means that we are trying
        // to get the genesis state root
        let Some(bonding_root) = self
            .chain_state
            .root_at_height(attestation.proof_of_bond.claimed_slot_number, state)
            .map_err(Into::<anyhow::Error>::into)?
        else {
            return Err(ProcessAttestationErrors::InvalidBondingProof);
        };

        // This proof checks that the attester was bonded at the given transition num
        let bond_opt = self
            .verify_proof(
                bonding_root,
                attestation.proof_of_bond.proof.clone(),
                sender,
            )
            .map_err(|err| {
                debug!(error = ?err, "Error during verifying bonding proof");
                ProcessAttestationErrors::InvalidBondingProof
            })?;

        let bond = bond_opt.ok_or(ProcessAttestationErrors::AttesterNotBonded)?;
        let bond: Amount = BorshDeserialize::deserialize(&mut bond.value())
            .map_err(|_err| ProcessAttestationErrors::InvalidBondFormat)?;

        let minimum_bond = self
            .minimum_attester_bond
            .get_or_err(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("The minimum bond should be set at genesis");

        // We then have to check that the bond was greater than the minimum bond
        if bond < minimum_bond.value(state.gas_price()) {
            return Err(ProcessAttestationErrors::AttesterNotBonded);
        }

        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn check_transition<ST: VersionReader + StateReader<Kernel>>(
        &self,
        claimed_slot_number: SlotNumber,
        attestation: &Attestation<
            <S::Da as DaSpec>::SlotHash,
            <S::Storage as Storage>::Root,
            StorageProof<<S::Storage as Storage>::Proof>,
        >,
        state: &mut ST,
    ) -> Result<CheckTransitionStatus, ST::Error> {
        if let Some(saved_transition) = self
            .chain_state
            .get_historical_transition_dangerous(claimed_slot_number, state)?
        {
            // We first need to compare the initial block hash to the previous post state root
            if saved_transition.slot().slot_hash() != &attestation.slot_hash {
                debug!(
                    %claimed_slot_number,
                    attestation_slot_hash = ?attestation.slot_hash,
                    actual_transition_slot_hash = ?saved_transition.slot().slot_hash(),
                    "The attestation has an invalid DA block hash");
                return Ok(CheckTransitionStatus::SlashInvalidateWrongHash);
            }

            if saved_transition.post_state_root() != &attestation.post_state_root {
                debug!(
                    %claimed_slot_number,
                    attestation_post_state_root = ?attestation.post_state_root,
                    actual_post_state_root = ?saved_transition.post_state_root(),
                    "The attestation has an invalid post state root");
                return Ok(CheckTransitionStatus::SlashInvalidateWrongHash);
            }
        } else {
            debug!(%claimed_slot_number, "No historical state transition found for claimed slot number. Recall that state transitions are not visible until the slot *after* the transition is visible.");
            // Case where we cannot get the transition from the chain state historical transitions.
            return Ok(CheckTransitionStatus::SlashedNoHistoricalTransition);
        }

        Ok(CheckTransitionStatus::Valid)
    }

    pub(crate) fn check_challenge_outputs_against_transition<
        ST: VersionReader + StateReader<Kernel>,
    >(
        &self,
        public_outputs: &StateTransitionPublicData<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
        >,
        height: SlotNumber,
        state: &mut ST,
    ) -> anyhow::Result<Option<SlashingReason>, ST::Error> {
        let transition = match self.chain_state.slot_at_height(height, state)? {
            Some(transition) => transition,
            None => return Ok(Some(SlashingReason::TransitionInvalid)),
        };

        if &public_outputs.initial_state_root != transition.prev_state_root() {
            return Ok(Some(SlashingReason::InvalidInitialHash));
        }

        if &public_outputs.slot_hash != transition.slot_hash() {
            return Ok(Some(SlashingReason::TransitionInvalid));
        }

        Ok(None)
    }

    pub(crate) fn slash_challenger(
        &self,
        sender: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<(), anyhow::Error> {
        self.bonded_challengers.remove(sender, state)?;
        Ok(())
    }
}

pub(crate) enum CheckInitialHashStatus {
    Valid,
    Slash,
}

pub(crate) enum CheckTransitionStatus {
    Valid,
    SlashedNoHistoricalTransition,
    SlashInvalidateWrongHash,
}
