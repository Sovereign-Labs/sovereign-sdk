use core::result::Result::Ok;
use std::cmp::max;
use std::fmt::Debug;

use anyhow::Result;
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
    /// Bonds an attester, the parameter is the bond amount
    BondAttester(u64),
    /// Start the first phase of the two phase unbonding process
    BeginUnbondingAttester,
    /// Finish the two phase unbonding
    EndUnbondingAttester,
    /// Bonds a challenger, the parameter is the bond amount
    BondChallenger(u64),
    /// Unbonds a challenger
    UnbondChallenger,
    /// Proccesses an attestation.
    ProcessAttestation(Attestation<StorageProof<<<C as Spec>::Storage as Storage>::Proof>>),
    /// Processes a challenge. The challenge is encoded as a [`Vec<u8>`]. The second parameter is the transition number
    ProcessChallenge(Vec<u8>, u64),
}

#[derive(Debug, Error, PartialEq, Eq)]
/// Error type that explains why an user is slashed
pub enum SlashingReason {
    #[error("Transition not found")]
    /// The specified transition does not exist
    TransitionNotFound,

    #[error("The attestation does not contain the right block hash and post state transition")]
    /// The specified transition is invalid (block hash, post root hash or validity condition)
    TransitionInvalid,

    #[error("The initial hash of the transition is invalid")]
    /// The initial hash of the transition is invalid.
    InvalidInitialHash,

    #[error("Attester is unbonding")]
    /// The attester is in the first unbonding phase
    AttesterIsUnbonding,

    #[error("The proof opening raised an error")]
    /// The proof verification raised an error
    InvalidProofOutputs,

    #[error("No invalid transition to challenge")]
    /// No invalid transition to challenge.
    NoInvalidTransition,

    #[error("User is not trying to unbond at the time of the transaction")]
    /// User is not trying to unbond at the time of the transaction
    AttesterIsNotUnbonding,

    #[error("First phase unbonding has not been finalized")]
    /// The attester is trying to finish the two phase unbonding too soon
    UnbondingNotFinalized,
}

/// Error raised while processessing the attester incentives
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AttesterIncentiveErrors {
    #[error("Attester slashed")]
    /// The user was slashed. Reason specified by [`SlashingReason`]
    UserSlashed(#[source] SlashingReason),

    #[error("Invalid bonding proof")]
    /// The bonding proof was invalid
    InvalidBondingProof,

    #[error("The sender key doesn't match the attester key provided in the proof")]
    /// The sender key doesn't match the attester key provided in the proof
    InvalidSender,

    #[error("The bond is not a 64 bits number")]
    /// The bond is not a 64 bits number
    InvalidBondFormat,

    #[error("User is not bonded at the time of the transaction")]
    /// User is not bonded at the time of the transaction
    UserNotBonded,

    #[error("Transition invariant not respected")]
    /// Transition invariant not respected
    InvalidTransitionInvariant,

    #[error("Error occured when transfered funds")]
    /// An error occured when transfered funds
    TransferFailure,

    #[error("Error when trying to mint the reward token")]
    /// An error occured when trying to mint the reward token
    MintFailure,
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
    /// This returns the address of the reward token supply
    pub fn get_reward_token_supply_address(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> C::Address {
        self.reward_token_supply_address
            .get(working_set)
            .expect("The reward token supply address should be set at genesis")
    }

    /// A helper function that simply slashes an attester and returns a reward value
    fn slash_user(
        &self,
        user: &C::Address,
        role: Role,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> u64 {
        let bonded_set = match role {
            Role::Attester => {
                // We have to remove the attester from the unbonding set to prevent him from skiping the first phase
                // unbonding if he bonds himself again.
                self.unbonding_attesters.remove(user, working_set);

                &self.bonded_attesters
            }
            Role::Challenger => &self.bonded_challengers,
        };

        // We have to deplete the attester's bonded account, it amounts to removing the attester from the bonded set
        let reward = bonded_set.get(user, working_set).unwrap_or_default();
        bonded_set.remove(user, working_set);

        // We raise an event
        working_set.add_event("user_slashed", &format!("address {user:?}"));

        reward
    }

    fn slash_burn_reward(
        &self,
        user: &C::Address,
        role: Role,
        reason: SlashingReason,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        self.slash_user(user, role, working_set);
        Err(AttesterIncentiveErrors::UserSlashed(reason))
    }

    /// A helper function that is used to slash an attester, and put the associated attestation in the slashed pool
    fn slash_and_invalidate_attestation(
        &self,
        attester: &C::Address,
        transition_nb: u64,
        reason: SlashingReason,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        let reward = self.slash_user(attester, Role::Attester, working_set);

        let curr_reward_value = self
            .bad_transition_pool
            .get(&transition_nb, working_set)
            .unwrap_or_default();

        self.bad_transition_pool
            .set(&transition_nb, &(curr_reward_value + reward), working_set);

        Err(AttesterIncentiveErrors::UserSlashed(reason))
    }

    fn reward_sender(
        &self,
        context: &C,
        amount: u64,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse, AttesterIncentiveErrors> {
        let reward_address = self
            .reward_token_supply_address
            .get(working_set)
            .expect("The reward supply address must be set at genesis");

        let coins = Coins {
            token_address: self
                .bonding_token_address
                .get(working_set)
                .expect("Bonding token address must be set"),
            amount,
        };

        // Mint tokens and send them
        self.bank
            .mint(
                &coins,
                context.sender(),
                &C::new(reward_address),
                working_set,
            )
            .map_err(|_err| AttesterIncentiveErrors::MintFailure)?;

        Ok(CallResponse::default())
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
            self.reward_sender(context, old_balance, working_set)?;

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
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        // First get the bonded attester
        if self.bonded_attesters.get(context.sender(), working_set).is_some() {
            // Then add the bonded attester to the unbonding set, with the current finalized height
            let finalized_height = self
                .light_client_finalized_height
                .get(working_set)
                .expect("Must be set at genesis");
            self.unbonding_attesters
                .set(context.sender(), &finalized_height, working_set);
        }

        Ok(CallResponse::default())
    }

    pub(crate) fn end_unbond_attester(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        // We first have to ensure that the attester is bonded
        if self.bonded_attesters.get(context.sender(), working_set).is_some() {
            // We have to ensure that the attester is unbonding, and that the unbonding transaction
            // occurred at least `finality_period` blocks ago to let the attester unbond
            if let Some(begin_unbond_height) =
                self.unbonding_attesters.get(context.sender(), working_set)
            {
                // These two constants should always be set beforehand, hence we can panic if they're not set
                let curr_height = self
                    .light_client_finalized_height
                    .get(working_set)
                    .expect("Should be defined at genesis");
                let finality_period = self
                    .rollup_finality_period
                    .get(working_set)
                    .expect("Should be defined at genesis");

                if begin_unbond_height + finality_period > curr_height {
                    return self.slash_burn_reward(
                        context.sender(),
                        Role::Attester,
                        SlashingReason::UnbondingNotFinalized,
                        working_set,
                    );
                }

                // Get the user's old balance.
                if let Some(old_balance) = self.bonded_attesters.get(context.sender(), working_set)
                {
                    // Transfer the bond amount from the sender to the module's address.
                    // On failure, no state is changed
                    self.reward_sender(context, old_balance, working_set)?;

                    // Update our internal tracking of the total bonded amount for the sender.
                    self.bonded_attesters.remove(context.sender(), working_set);
                    self.unbonding_attesters
                        .remove(context.sender(), working_set);

                    // Emit the unbonding event
                    working_set.add_event(
                        "unbonded_challenger",
                        &format!("amount_withdrawn: {old_balance:?}"),
                    );
                }
            } else {
                return self.slash_burn_reward(
                    context.sender(),
                    Role::Attester,
                    SlashingReason::AttesterIsNotUnbonding,
                    working_set,
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
        context: &C,
        attestation: &Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), AttesterIncentiveErrors> {
        let bonding_root = {
            // If we cannot get the transition before the current one, it means that we are trying
            // to get the genesis state root
            if let Some(transition) = self
                .chain_state
                .historical_transitions
                .get(&(attestation.proof_of_bond.transition_num - 1), working_set)
            {
                transition.post_state_root()
            } else {
                self.chain_state
                    .genesis_hash
                    .get(working_set)
                    .expect("The genesis hash should be set at genesis")
            }
        };

        // This proof checks that the attester was bonded at the given transition num
        let (attester_key, bond_opt) = working_set
            .backing()
            .open_proof(bonding_root, attestation.proof_of_bond.proof.clone())
            .map_err(|_err| AttesterIncentiveErrors::InvalidBondingProof)?;

        // We have to check that the storage key is the same as the sender's
        if attester_key
            != StorageKey::new(
                self.bonded_attesters.prefix(),
                context.sender(),
                &self.bonded_attesters.codec,
            )
        {
            return Err(AttesterIncentiveErrors::InvalidSender);
        }

        let bond = bond_opt.unwrap_or_default();
        let bond: u64 = BorshDeserialize::deserialize(&mut bond.value())
            .map_err(|_err| AttesterIncentiveErrors::InvalidBondFormat)?;

        let minimum_bond = self
            .minimum_attester_bond
            .get_or_err(working_set)
            .expect("The minimum bond should be set at genesis");

        // We then have to check that the bond was greater than the minimum bond
        if bond < minimum_bond {
            return Err(AttesterIncentiveErrors::UserNotBonded);
        }

        Ok(())
    }

    fn check_transition(
        &self,
        max_attested_height: u64,
        attester: &C::Address,
        attestation: &Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        if let Some(curr_tx) = self
            .chain_state
            .historical_transitions
            .get(&max_attested_height, working_set)
        {
            // We first need to compare the initial block hash to the previous post state root
            if !curr_tx.compare_hashes(&attestation.da_block_hash, &attestation.post_state_root) {
                // It is the right attestation, we have to compare the initial block hash to the
                // previous post state root
                // Slash the attester
                self.slash_and_invalidate_attestation(
                    attester,
                    max_attested_height,
                    SlashingReason::TransitionInvalid,
                    working_set,
                )?;
            }
            Ok(CallResponse::default())
        } else {
            self.slash_burn_reward(
                attester,
                Role::Attester,
                SlashingReason::TransitionNotFound,
                working_set,
            )
        }
    }

    fn check_initial_hash(
        &self,
        max_attested_height: u64,
        attester: &C::Address,
        attestation: &Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        // Normal state
        if let Some(transition) = self
            .chain_state
            .historical_transitions
            .get(&(max_attested_height - 1), working_set)
        {
            if transition.post_state_root() != attestation.initial_state_root {
                // The initial root hashes don't match, just slash the attester
                return self.slash_burn_reward(
                    attester,
                    Role::Attester,
                    SlashingReason::InvalidInitialHash,
                    working_set,
                );
            }
        } else {
            // Genesis state
            // We can assume that the genesis hash is always set, otherwise we need to panic.
            // We don't need to prove that the attester was bonded, simply need to check that the current bond is higher than the
            // minimal bond and that the attester is not unbonding
            if self
                .chain_state
                .get_genesis_hash(working_set)
                .expect("The initial hash should be set")
                != attestation.initial_state_root
            {
                // Slash the attester, and burn the fees
                return self.slash_burn_reward(
                    attester,
                    Role::Attester,
                    SlashingReason::InvalidInitialHash,
                    working_set,
                );
            }
            // Normal state
        }

        Ok(CallResponse::default())
    }

    /// Try to process an attestation, if the attester is bonded
    pub(crate) fn process_attestation(
        &self,
        context: &C,
        attestation: Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        // We first need to check the bonding proof (because we can't slash the attester if he's not bonded)
        self.check_bonding_proof(context, &attestation, working_set)?;

        // We suppose that these values are always defined, otherwise we panic
        let last_attested_height = self
            .maximum_attested_height
            .get(working_set)
            .expect("The maximum attested height should be set at genesis");
        let current_finalized_height = self
            .light_client_finalized_height
            .get(working_set)
            .expect("The light client finalized height should be set at genesis");
        let finality = self
            .rollup_finality_period
            .get(working_set)
            .expect("The rollup finality period should be set at genesis");

        // Update the max_attested_height in case the blocks have already been finalized
        let new_height_to_attest = max(last_attested_height, current_finalized_height) + 1;

        // Minimum height at which the proof of bond can be valid
        let min_height = if new_height_to_attest > finality {
            new_height_to_attest - finality
        } else {
            0
        };

        // We have to check the following order invariant is respected:
        // (height to attest - finality) <= bonding_proof.transition_num <= height to attest
        //
        // Which with our variable gives:
        // min_height <= bonding_proof.transition_num <= new_height_to_attest
        // If this invariant is respected, we can be sure that the attester was bonded at new_height_to_attest.
        if !(min_height <= attestation.proof_of_bond.transition_num
            && attestation.proof_of_bond.transition_num <= new_height_to_attest)
        {
            return Err(AttesterIncentiveErrors::InvalidTransitionInvariant);
        }

        // First compare the initial hashes
        self.check_initial_hash(
            new_height_to_attest,
            context.sender(),
            &attestation,
            working_set,
        )?;

        // Then compare the transition
        self.check_transition(
            new_height_to_attest,
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
                new_height_to_attest,
                SlashingReason::AttesterIsUnbonding,
                working_set,
            );
        }

        working_set.add_event(
            "processed_valid_attestation",
            &format!("attester: {:?}", context.sender()),
        );

        // Update the maximum attested height
        self.maximum_attested_height
            .set(&(new_height_to_attest), working_set);

        // Reward the sender
        self.reward_sender(
            context,
            self.minimum_attester_bond
                .get(working_set)
                .expect("Should be defined at genesis"),
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
    ) -> Result<(), SlashingReason> {
        let transition = self
            .chain_state
            .historical_transitions
            .get_or_err(&transition_num, working_set)
            .map_err(|_| SlashingReason::TransitionInvalid)?;

        let initial_hash = {
            if let Some(prev_transition) = self
                .chain_state
                .historical_transitions
                .get(&(transition_num - 1), working_set)
            {
                prev_transition.post_state_root()
            } else {
                self.chain_state
                    .genesis_hash
                    .get(working_set)
                    .expect("The genesis hash should be set")
            }
        };

        if public_outputs.initial_state_root != initial_hash {
            return Err(SlashingReason::InvalidInitialHash);
        }

        if public_outputs.slot_hash != transition.da_block_hash() {
            return Err(SlashingReason::TransitionInvalid);
        }

        // TODO: Should we compare the validity conditions of the public outputs with the ones of the recorded transition?
        if condition_checker
            .check(&public_outputs.validity_condition)
            .is_err()
        {
            return Err(SlashingReason::TransitionInvalid);
        }

        Ok(())
    }

    /// Try to process a zk proof, if the challenger is bonded.
    pub(crate) fn process_challenge(
        &self,
        context: &C,
        proof: &[u8],
        transition_num: u64,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, AttesterIncentiveErrors> {
        // Get the challenger's old balance.
        // Revert if they aren't bonded
        let old_balance = self
            .bonded_challengers
            .get_or_err(context.sender(), working_set)
            .map_err(|_| AttesterIncentiveErrors::UserNotBonded)?;

        // Check that the challenger has enough balance to process the proof.
        let minimum_bond = self
            .minimum_challenger_bond
            .get(working_set)
            .expect("Should be set at genesis");

        if old_balance < minimum_bond {
            return Err(AttesterIncentiveErrors::UserNotBonded);
        }

        let code_commitment = self
            .commitment_to_allowed_challenge_method
            .get(working_set)
            .expect("Should be set at genesis")
            .commitment;

        // Find the faulty attestation pool and get the associated reward
        let attestation_reward: u64 = self
            .bad_transition_pool
            .get_or_err(&transition_num, working_set)
            .map_err(|_| {
                self.slash_burn_reward(
                    context.sender(),
                    Role::Challenger,
                    SlashingReason::NoInvalidTransition,
                    working_set,
                )
                .unwrap_err()
            })?;

        let public_outputs_opt: Result<StateTransition<Cond, C::Address>> =
            Vm::verify_and_extract_output::<Cond, C::Address>(proof, &code_commitment)
                .map_err(|e| anyhow::format_err!("{:?}", e));

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        match public_outputs_opt {
            Ok(public_output) => {
                // We get the validity condition checker from the state
                let mut validity_checker = self
                    .validity_cond_checker
                    .get(working_set)
                    .expect("Should be defined at genesis");

                // We have to perform the checks to ensure that the challenge is valid while the attestation isn't.
                self.check_challenge_outputs_against_transition(
                    public_output,
                    transition_num,
                    &mut validity_checker,
                    working_set,
                )
                .map_err(|err| {
                    self.slash_burn_reward(context.sender(), Role::Challenger, err, working_set)
                        .unwrap_err()
                })?;

                // Reward the challenger with half of the attestation reward (avoid DOS)
                self.reward_sender(context, attestation_reward / 2, working_set)?;

                // Now remove the bad transition from the pool
                self.bad_transition_pool
                    .remove(&transition_num, working_set);

                working_set.add_event(
                    "processed_valid_proof",
                    &format!("challenger: {:?}", context.sender()),
                );
            }
            Err(_err) => {
                // Slash the challenger
                return self.slash_burn_reward(
                    context.sender(),
                    Role::Challenger,
                    SlashingReason::InvalidProofOutputs,
                    working_set,
                );
            }
        }

        Ok(CallResponse::default())
    }
}
