use std::cmp::min;
use std::fmt::{self, Debug};
use std::option;

use anyhow::{ensure, Ok, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_bank::Coins;
use sov_chain_state::StateTransitionId;
use sov_modules_api::{CallResponse, Context, Spec};
use sov_rollup_interface::optimistic::Attestation;
use sov_rollup_interface::zk::{StateTransition, ValidityCondition, Zkvm};
use sov_state::storage::StorageProof;
use sov_state::{Storage, WorkingSet};
use thiserror::Error;

use crate::{AttesterIncentives, UnbondingInfo};

/// This enumeration represents the available call messages for interacting with the `ExampleModule` module.
#[derive(BorshDeserialize, BorshSerialize, Debug)]
// TODO: allow call messages to borrow data
//     https://github.com/Sovereign-Labs/sovereign-sdk/issues/274
pub enum CallMessage<C: Context> {
    BondAttester(u64),
    UnbondAttester,
    BondChallenger(u64),
    UnbondChallenger,
    ProcessAttestation(Attestation<StorageProof<<<C as Spec>::Storage as Storage>::Proof>>),
    ProcessChallenge(Vec<u8>, [u8; 32]),
}

/// Error raised while processessing the attester incentives
#[derive(Debug, Clone, Error)]
enum AttesterIncentiveErrors {
    AttesterSlashed,
    AttesterNotUnbonding,
}

impl fmt::Display for AttesterIncentiveErrors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let error_msg = match self {
            Self::AttesterSlashed => "Attester slashed",
            Self::AttesterNotUnbonding => "Attester not unbonding",
        };

        write!(f, "{error_msg}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A role in the attestation process
pub(crate) enum Role {
    /// A user who attests to new state transitions
    Attester,
    /// A user who challenges attestations
    Challenger,
}

impl<C: sov_modules_api::Context, Vm: Zkvm, P: BorshSerialize> AttesterIncentives<C, Vm, P> {
    /// A helper function that simply slashes an attester and returns a reward value
    fn slash_attester_helper(
        &mut self,
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
        &mut self,
        attester: &C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        self.slash_attester_helper(attester, working_set);
        Err(AttesterIncentiveErrors::AttesterSlashed)
    }

    fn reward_sender(&mut self, amount: u64, context: &C) -> Result<CallResponse> {
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
        &mut self,
        attester: &C::Address,
        attestation: Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        let reward = self.slash_attester_helper(attester, working_set);

        let curr_reward_value = self
            .bad_transition_pool
            .get(&attestation.initial_state_root, working_set)
            .unwrap_or_default();

        self.bad_transition_pool.set(
            &attestation.initial_state_root,
            &(curr_reward_value + reward),
            working_set,
        );

        Err(AttesterIncentiveErrors::AttesterSlashed)
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
    pub(crate) fn unbond_user_helper(
        &self,
        context: &C,
        role: Role,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        let bonded_set = match role {
            Role::Challenger => self.bonded_challengers,
            Role::Attester => {
                // We have to ensure that the last block attested occurred 24 hours ago
                // to let the attester unbond
                if let Some(last_attested_block) =
                    self.last_attested_block.get(context.sender(), working_set)
                {
                    // These two constants should always be set beforehand, hence we can panic if they're not set
                    let curr_height = self.light_client_finalized_height.get(working_set).unwrap();
                    let finality_period = self.rollup_finality_period.get(working_set).unwrap();

                    ensure!(
                        last_attested_block + finality_period < curr_height,
                        "Cannot unbond the attester: finality has not completed yet"
                    );

                    self.bonded_attesters
                } else {
                    return Ok(CallResponse::default());
                }
            }
        };

        // Get the user's old balance.
        if let Some(old_balance) = bonded_set.get(context.sender(), working_set) {
            // Transfer the bond amount from the sender to the module's address.
            // On failure, no state is changed
            self.reward_sender(old_balance, context)?;

            // Update our internal tracking of the total bonded amount for the sender.
            bonded_set.remove(context.sender(), working_set);

            // Emit the unbonding event
            working_set.add_event(
                "unbonded_user",
                &format!("role: {role:?}, amount_withdrawn: {old_balance:?}"),
            );
        }

        Ok(CallResponse::default())
    }

    fn check_bonding_proof(
        &self,
        attestation: Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        // This proof checks that the attester was bonded at the initial root hash
        let (attester_key, bond_opt) = working_set
            .backing()
            .open_proof(attestation.initial_state_root, attestation.proof_of_bond)?;

        let sender_u8: Vec<u8> = context.sender().as_ref().into();

        // We have to check that the storage key is the same as the sender's
        ensure!(
            *attester_key.as_ref() == sender_u8,
            "The sender key doesn't match the attester key provided in the proof"
        );

        // Maybe we can check directly against the bonded attester set?
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

    fn check_transition(self, max_attested_height: u64) -> Result<sov_modules_api::CallResponse> {
        let transitions = self.chain_state.historical_transitions;
        let curr_tx = transitions.get(&max_attested_height, working_set);
        // We first need to compare the initial block hash to the previous post state root
        if !curr_tx.compare_tx_hashes(attestation.da_block_hash, attestation.post_state_root) {
            // It is the right attestation, we have to compare the initial block hash to the
            // previous post state root
            // Slash the attester
            self.slash_and_invalidate_attestation(context.sender(), attestation, working_set)
        }

        Ok(CallResponse::default())
    }

    fn check_initial_hash(
        &mut self,
        max_attested_height: u64,
    ) -> Result<sov_modules_api::CallResponse> {
        // Genesis state
        if max_attested_height == 0 {
            // We can assume that the genesis hash is always set, otherwise we need to panic.
            // We don't need to prove that the attester was bonded, simply need to check that the current bond is higher than the
            // minimal bond and that the attester is not unbonding
            if (!self.chain_state.genesis_hash.get(working_set).unwrap()
                == attestation.initial_state_root)
            {
                // Slash the attester, and burn the fees
                self.slash_attester(attester, working_set)
            }
        // Normal state
        } else {
            if let Some(curr_tx) = transitions.get(&max_attested_height, working_set) {
                // We need to check that the transition is legit, if it is,
                // then we can perform the height checks
                if (!transitions.get(&(max_attested_height-1), working_set).unwrap() /* Always defined, due to loop invariant */
                            == attestation.initial_state_root)
                {
                    // The initial root hashes don't match, just slash the attester
                    self.slash_attester(attester, working_set)
                }
            } else {
                // We only need to slash the attester. We don't put the transaction in the pool
                self.slash_attester(attester, working_set)
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
        self.check_bonding_proof(attestation, context, working_set)?;

        // We suppose that these values are always defined, otherwise we panic
        let old_max_attested_height = self.maximum_attested_height.get_or_err(working_set)?;
        let current_finalized_height =
            self.light_client_finalized_height.get_or_err(working_set)?;
        let finality = self.rollup_finality_period.get_or_err(working_set)?;
        let minimum_bond = self.minimum_attester_bond.get_or_err(working_set)?;

        // Update the max_attested_height in case the blocks have already been finalized
        self.maximum_attested_height.set(
            max(old_max_attested_height, {
                if current_finalized_height > finality {
                    current_finalized_height - finality
                } else {
                    0
                }
            }),
            working_set,
        );

        let max_attested_height = self.maximum_attested_height.get(working_set).unwrap();

        // First compare the initial hashes
        self.check_initial_hash(max_attested_height)?;

        // Then compare the transition
        self.check_transition(max_attested_height)?;

        working_set.add_event(
            "processed_valid_attestation",
            &format!("attester: {:?}", context.sender()),
        );

        // Reward the sender
        self.reward_sender(
            self.minimum_attester_bond.get(working_set).unwrap(),
            context,
        )?;

        // Then we can optimistically process the transaction
        Ok(CallResponse::default())
    }

    fn check_challenge_outputs_against_attestation<Cond>(
        self,
        public_outputs: StateTransition<Cond>,
        attestation: Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
    ) -> Result<()> {
        ensure!(
            public_outputs.initial_state_root == attestation.initial_state_root,
            "Not the same initial root"
        );
        ensure!(
            public_outputs.final_state_root != attestation.post_state_root,
            "Same final root"
        );
        ensure!(public_outputs.block_hash == attestation.da_block_hash);
        ensure!(public_outputs.validity_condition.check().is_ok());
        Ok(())
    }

    /// Try to process a zk proof, if the challenger is bonded.
    pub(crate) fn process_challenge<Cond: ValidityCondition>(
        &self,
        proof: &[u8],
        initial_hash: [u8; 32],
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
            .get_or_err(&initial_hash, working_set)?;

        let public_outputs_opt: Result<StateTransition<Cond>> =
            Vm::verify_and_extract_output::<Cond>(proof, &code_commitment)
                .map_err(|e| anyhow::format_err!("{:?}", e));

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        match public_outputs_opt {
            Ok(public_output) => {
                // We have to perform the checks to ensure that the challenge is valid while the attestation isn't.
                self.check_challenge_outputs_against_attestation::<Cond>(
                    public_output,
                    attestation,
                );

                // Reward the challenger with half of the attestation reward (avoid DOS)
                self.reward_sender(attestation_reward / 2, context);

                working_set.add_event(
                    "processed_valid_proof",
                    &format!("challenger: {:?}", context.sender()),
                );
            }
            Err(err) => {
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
