use core::result::Result::Ok;
use std::cmp::{max, min};
use std::fmt::{self, Debug};

use anyhow::{ensure, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_bank::Coins;
use sov_modules_api::{CallResponse, Context, Spec};
use sov_rollup_interface::optimistic::Attestation;
use sov_rollup_interface::zk::{
    StateTransition, ValidityCondition, ValidityConditionChecker, Zkvm,
};
use sov_state::storage::{StorageKey, StorageProof};
use sov_state::{Storage, WorkingSet};
use thiserror::Error;

use crate::AttesterIncentives;

/// This enumeration represents the available call messages for interacting with the `ExampleModule` module.
#[derive(BorshDeserialize, BorshSerialize, Debug)]
// TODO: allow call messages to borrow data
//     https://github.com/Sovereign-Labs/sovereign-sdk/issues/274

pub enum CallMessage<C>
where
    C: Context,
{
    BondAttester(u64),
    BeginUnbondingAttester,
    EndUnbondingAttester,
    BondChallenger(u64),
    UnbondChallenger,
    ProcessAttestation(Attestation<StorageProof<<<C as Spec>::Storage as Storage>::Proof>>),
    ProcessChallenge(Vec<u8>, u64),
}

/// Error raised while processessing the attester incentives
#[derive(Debug, Clone, Error)]
enum AttesterIncentiveErrors {
    AttesterSlashed,
}

impl fmt::Display for AttesterIncentiveErrors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let error_msg = match self {
            Self::AttesterSlashed => "Attester slashed",
        };

        write!(f, "{error_msg}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A role in the attestation process
pub enum Role {
    /// A user who attests to new state transitions
    Attester,
    /// A user who challenges attestations
    Challenger,
}

impl<
        C: sov_modules_api::Context,
        Vm: Zkvm,
        Cond: ValidityCondition,
        Checker: ValidityConditionChecker<Cond> + BorshDeserialize + BorshSerialize,
    > AttesterIncentives<C, Vm, Cond, Checker>
{
    /// A helper function that simply slashes an attester and returns a reward value
    fn slash_attester_helper(
        &self,
        attester: &C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> u64 {
        // We have to deplete the attester's bonded account, it amounts to removing the attester from the bonded set
        let reward = self
            .bonded_attesters
            .get(attester, working_set)
            .unwrap_or_default();
        self.bonded_attesters.remove(attester, working_set);

        // We raise an event
        working_set.add_event("slashed_attester", &format!("address {attester:?}"));

        reward
    }

    fn slash_burn_reward(
        &self,
        attester: &C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        self.slash_attester_helper(attester, working_set);
        Err(AttesterIncentiveErrors::AttesterSlashed.into())
    }

    fn reward_sender(
        &self,
        amount: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let coins = Coins {
            token_address: self
                .bonding_token_address
                .get(working_set)
                .expect("Bonding token address must be set"),
            amount,
        };
        // Try to unbond the entire balance
        // If the unbonding fails, no state is changed
        self.bank
            .transfer_from(&self.address, context.sender(), coins, working_set)?;

        Ok(CallResponse::default())
    }

    /// A helper function that is used to slash an attester, and put the associated attestation in the slashed pool
    fn slash_and_invalidate_attestation(
        &self,
        attester: &C::Address,
        transition_nb: u64,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        let reward = self.slash_attester_helper(attester, working_set);

        let curr_reward_value = self
            .bad_transition_pool
            .get(&transition_nb, working_set)
            .unwrap_or_default();

        self.bad_transition_pool
            .set(&transition_nb, &(curr_reward_value + reward), working_set);

        Err(AttesterIncentiveErrors::AttesterSlashed.into())
    }

    /// A helper function for the `bond_challenger/attester` call. Also used to bond challengers/attesters
    /// during genesis when no context is available.
    pub(super) fn bond_user_helper(
        &self,
        bond_amount: u64,
        user_address: &C::Address,
        role: Role,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        // Transfer the bond amount from the sender to the module's address.
        // On failure, no state is changed
        let coins = Coins {
            token_address: self
                .bonding_token_address
                .get(working_set)
                .expect("Bonding token address must be set"),
            amount: bond_amount,
        };
        self.bank
            .transfer_from(user_address, &self.address, coins, working_set)?;

        let (balances, event_key) = match role {
            Role::Attester => (&self.bonded_attesters, "bonded_attester"),
            Role::Challenger => (&self.bonded_challengers, "bonded_challenger"),
        };

        // Update our record of the total bonded amount for the sender.
        // This update is infallible, so no value can be destroyed.
        let old_balance = balances.get(user_address, working_set).unwrap_or_default();
        let total_balance = old_balance + bond_amount;
        balances.set(user_address, &total_balance, working_set);

        // Emit the bonding event
        working_set.add_event(
            event_key,
            &format!("new_deposit: {bond_amount:?}. total_bond: {total_balance:?}"),
        );

        Ok(CallResponse::default())
    }

    /// Try to unbond the requested amount of coins with context.sender() as the beneficiary.
    pub(crate) fn unbond_challenger(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // Get the user's old balance.
        if let Some(old_balance) = self.bonded_challengers.get(context.sender(), working_set) {
            // Transfer the bond amount from the sender to the module's address.
            // On failure, no state is changed
            self.reward_sender(old_balance, context, working_set)?;

            // Update our internal tracking of the total bonded amount for the sender.
            self.bonded_challengers
                .remove(context.sender(), working_set);

            // Emit the unbonding event
            working_set.add_event(
                "unbonded_challenger",
                &format!("amount_withdrawn: {old_balance:?}"),
            );
        }

        Ok(CallResponse::default())
    }

    /// The attester starts the first phase of the two phase unbonding. We put the current max
    /// finalized height with the attester address in the set of unbonding attesters iff the attester
    /// is already present in the unbonding set
    pub(crate) fn begin_unbond_attester(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // First get the bonded attester
        ensure!(
            self.bonded_attesters
                .get_or_err(context.sender(), working_set)
                .is_ok(),
            "Error, the sender is not an existing attester"
        );

        // Then add the bonded attester to the unbonding set, with the current finalized height
        let finalized_height = self.light_client_finalized_height.get_or_err(working_set)?;
        self.unbonding_attesters
            .set(context.sender(), &finalized_height, working_set);

        Ok(CallResponse::default())
    }

    pub(crate) fn end_unbond_attester(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // We have to ensure that the attester is unbonding, and that the unbonding transaction
        // occurred at least `finality_period` blocks ago to let the attester unbond
        if let Ok(begin_unbond_height) = self
            .unbonding_attesters
            .get_or_err(context.sender(), working_set)
        {
            // These two constants should always be set beforehand, hence we can panic if they're not set
            let curr_height = self.light_client_finalized_height.get(working_set).unwrap();
            let finality_period = self.rollup_finality_period.get(working_set).unwrap();

            ensure!(
                begin_unbond_height + finality_period < curr_height,
                "Cannot unbond the attester: finality has not completed yet"
            );

            // Get the user's old balance.
            if let Some(old_balance) = self.bonded_challengers.get(context.sender(), working_set) {
                // Transfer the bond amount from the sender to the module's address.
                // On failure, no state is changed
                self.reward_sender(old_balance, context, working_set)?;

                // Update our internal tracking of the total bonded amount for the sender.
                self.bonded_challengers
                    .remove(context.sender(), working_set);
                self.unbonding_attesters
                    .remove(context.sender(), working_set);

                // Emit the unbonding event
                working_set.add_event(
                    "unbonded_challenger",
                    &format!("amount_withdrawn: {old_balance:?}"),
                );
            }
        }

        Ok(CallResponse::default())
    }

    /// The bonding proof is now a proof that an attester was bonded during the last `finality_period` range.
    /// The proof must refer to a valid state of the rollup. The initial root hash must represent a state between
    /// the bonding proof one and the current state.
    fn check_bonding_proof(
        &self,
        attestation: &Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let bonding_root = {
            if attestation.proof_of_bond.transition_num == 0 {
                self.chain_state.genesis_hash.get_or_err(working_set)?
            } else {
                self.chain_state
                    .historical_transitions
                    .get_or_err(&(attestation.proof_of_bond.transition_num - 1), working_set)?
                    .post_state_root()
            }
        };

        // This proof checks that the attester was bonded at the given transition num
        let (attester_key, bond_opt) = working_set
            .backing()
            .open_proof(bonding_root, attestation.proof_of_bond.proof.clone())?;

        // We have to check that the storage key is the same as the sender's
        ensure!(
            attester_key == StorageKey::new(self.bonded_attesters.prefix(), context.sender()),
            "The sender key doesn't match the attester key provided in the proof"
        );

        let bond = bond_opt.unwrap_or_default();
        let bond = bond.value();

        ensure!(bond.len() < 8, "The bond is not a 64 bits number");

        let bond_u64 = {
            let mut bond_slice = [0_u8; 8];
            bond_slice[..min(bond.len(), 8)].copy_from_slice(bond);
            u64::from_le_bytes(bond_slice)
        };

        let minimum_bond = self.minimum_attester_bond.get_or_err(working_set)?;

        // We then have to check that the bond was greater than the minimum bond
        ensure!(
            bond_u64 >= minimum_bond,
            "Attester is not bonded at the time of the attestation"
        );

        Ok(())
    }

    fn check_transition(
        &self,
        max_attested_height: u64,
        attester: &C::Address,
        attestation: &Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        let curr_tx = self
            .chain_state
            .historical_transitions
            .get_or_err(&max_attested_height, working_set)?;
        // We first need to compare the initial block hash to the previous post state root
        if !curr_tx.compare_hashes(&attestation.da_block_hash, &attestation.post_state_root) {
            // It is the right attestation, we have to compare the initial block hash to the
            // previous post state root
            // Slash the attester
            self.slash_and_invalidate_attestation(attester, max_attested_height, working_set)?;
        }

        Ok(CallResponse::default())
    }

    fn check_initial_hash(
        &self,
        max_attested_height: u64,
        attester: &C::Address,
        attestation: &Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // Genesis state
        if max_attested_height == 0 {
            // We can assume that the genesis hash is always set, otherwise we need to panic.
            // We don't need to prove that the attester was bonded, simply need to check that the current bond is higher than the
            // minimal bond and that the attester is not unbonding
            if !(self.chain_state.genesis_hash.get_or_err(working_set)?
                == attestation.initial_state_root)
            {
                // Slash the attester, and burn the fees
                return self.slash_burn_reward(attester, working_set);
            }
        // Normal state
        } else {
            // We need to check that the transition is legit, if it is,
            // then we can perform the height checks
            if !(self.chain_state.historical_transitions.get(&(max_attested_height-1), working_set).unwrap().post_state_root()
                 /* Always defined, due to loop invariant */
                            == attestation.initial_state_root)
            {
                // The initial root hashes don't match, just slash the attester
                return self.slash_burn_reward(attester, working_set);
            }
        }
        Ok(CallResponse::default())
    }

    /// Try to process an attestation, if the attester is bonded
    pub(crate) fn process_attestation(
        &self,
        attestation: Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // We first need to check the bonding proof (because we can't slash the attester if he's not bonded)
        self.check_bonding_proof(&attestation, context, working_set)?;

        // We suppose that these values are always defined, otherwise we panic
        let old_max_attested_height = self.maximum_attested_height.get_or_err(working_set)?;
        let current_finalized_height =
            self.light_client_finalized_height.get_or_err(working_set)?;
        let finality = self.rollup_finality_period.get_or_err(working_set)?;

        let min_height = if current_finalized_height > finality {
            current_finalized_height - finality
        } else {
            0
        };

        // Update the max_attested_height in case the blocks have already been finalized
        let new_max_attested_height = max(&old_max_attested_height, &min_height);
        self.maximum_attested_height
            .set(new_max_attested_height, working_set);

        let max_attested_height = self.maximum_attested_height.get(working_set).unwrap();

        // We have to check the following order invariant is respected:
        // min_height <= bonding_proof.transition_num <= max_attested_height
        // If this invariant is respected, we can be sure that the attester was bonded at max_attested_height.
        ensure!(
            min_height <= attestation.proof_of_bond.transition_num
                && attestation.proof_of_bond.transition_num <= *new_max_attested_height,
            "Transition invariant not respected"
        );

        // First compare the initial hashes
        self.check_initial_hash(
            max_attested_height,
            context.sender(),
            &attestation,
            working_set,
        )?;

        // Then compare the transition
        self.check_transition(
            max_attested_height,
            context.sender(),
            &attestation,
            working_set,
        )?;

        // Then we can check that the attester is not unbonding, otherwise we slash the attester
        if self
            .unbonding_attesters
            .get(context.sender(), working_set)
            .is_some()
        {
            return self.slash_and_invalidate_attestation(
                context.sender(),
                max_attested_height,
                working_set,
            );
        }

        working_set.add_event(
            "processed_valid_attestation",
            &format!("attester: {:?}", context.sender()),
        );

        // Reward the sender
        self.reward_sender(
            self.minimum_attester_bond.get(working_set).unwrap(),
            context,
            working_set,
        )?;

        // Then we can optimistically process the transaction
        Ok(CallResponse::default())
    }

    fn check_challenge_outputs_against_transition(
        &self,
        public_outputs: StateTransition<Cond, C::Address>,
        transition_num: u64,
        condition_checker: &mut impl ValidityConditionChecker<Cond>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let transition = self
            .chain_state
            .historical_transitions
            .get_or_err(&transition_num, working_set)?;
        let initial_hash = {
            if transition_num == 0 {
                self.chain_state.genesis_hash.get_or_err(working_set)?
            } else {
                self.chain_state
                    .historical_transitions
                    .get_or_err(&(transition_num - 1), working_set)?
                    .post_state_root()
            }
        };

        ensure!(
            public_outputs.initial_state_root == initial_hash,
            "Not the same initial root"
        );
        ensure!(public_outputs.slot_hash == transition.da_block_hash());
        // TODO: Should we compare the validity conditions of the public outputs with the ones of the recorded transition?
        ensure!(
            condition_checker
                .check(&public_outputs.validity_condition)
                .is_ok(),
            "Unable to verify the validity conditions"
        );
        Ok(())
    }

    /// Try to process a zk proof, if the challenger is bonded.
    pub(crate) fn process_challenge(
        &self,
        proof: &[u8],
        transition_num: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // Get the challenger's old balance.
        // Revert if they aren't bonded
        let old_balance = self
            .bonded_challengers
            .get_or_err(context.sender(), working_set)?;

        // Check that the challenger has enough balance to process the proof.
        let minimum_bond = self.minimum_challenger_bond.get_or_err(working_set)?;

        anyhow::ensure!(old_balance >= minimum_bond, "Prover is not bonded");
        let code_commitment = self
            .commitment_to_allowed_challenge_method
            .get_or_err(working_set)?
            .commitment;

        // Find the faulty attestation pool and get the associated reward
        let attestation_reward: u64 = self
            .bad_transition_pool
            .get_or_err(&transition_num, working_set)?;

        let public_outputs_opt: Result<StateTransition<Cond, C::Address>> =
            Vm::verify_and_extract_output::<Cond, C::Address>(proof, &code_commitment)
                .map_err(|e| anyhow::format_err!("{:?}", e));

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        match public_outputs_opt {
            Ok(public_output) => {
                // We get the validity condition checker from the state
                let mut validity_checker = self.validity_cond_checker.get_or_err(working_set)?;

                // We have to perform the checks to ensure that the challenge is valid while the attestation isn't.
                self.check_challenge_outputs_against_transition(
                    public_output,
                    transition_num,
                    &mut validity_checker,
                    working_set,
                )?;

                // Reward the challenger with half of the attestation reward (avoid DOS)
                self.reward_sender(attestation_reward / 2, context, working_set)?;

                working_set.add_event(
                    "processed_valid_proof",
                    &format!("challenger: {:?}", context.sender()),
                );
            }
            Err(_err) => {
                // Slash the challenger
                self.bonded_challengers
                    .remove(context.sender(), working_set);

                working_set.add_event(
                    "processed_invalid_proof",
                    &format!("challenger: {:?}", context.sender()),
                );
            }
        }

        Ok(CallResponse::default())
    }
}
