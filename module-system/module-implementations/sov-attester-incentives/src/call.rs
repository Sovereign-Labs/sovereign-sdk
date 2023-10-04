use core::result::Result::Ok;
use std::fmt::Debug;

use anyhow::ensure;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_bank::{Amount, Coins};
use sov_chain_state::TransitionHeight;
use sov_modules_api::optimistic::Attestation;
use sov_modules_api::{
    CallResponse, DaSpec, Spec, StateTransition, ValidityConditionChecker, WorkingSet,
};
use sov_state::storage::{Storage, StorageKey, StorageProof, StorageValue};
use thiserror::Error;

use crate::{AttesterIncentives, UnbondingInfo};

#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
/// A wrapper for attestations which implements `borsh` serialization. This is necessary since
/// Attestations are treated as `CallMessage`s, and we only support borsh encoding for transactions.
pub struct WrappedAttestation<Da: DaSpec, StorageProof, Root> {
    #[serde(
        bound = "Da::SlotHash: Serialize + DeserializeOwned, StorageProof: Serialize + DeserializeOwned, Root: Serialize + DeserializeOwned"
    )]
    /// The inner attestation
    pub inner: Attestation<Da, StorageProof, Root>,
}

impl<Da: DaSpec, StorageProof: Debug, Root: Debug> Debug
    for WrappedAttestation<Da, StorageProof, Root>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WrappedAttestation")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<Da: DaSpec, StorageProof, Root> From<Attestation<Da, StorageProof, Root>>
    for WrappedAttestation<Da, StorageProof, Root>
{
    fn from(value: Attestation<Da, StorageProof, Root>) -> Self {
        Self { inner: value }
    }
}

impl<Da: DaSpec, StorageProof: Serialize, Root: Serialize> BorshSerialize
    for WrappedAttestation<Da, StorageProof, Root>
{
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // TODO: Implement bcs `to_writer`
        let value = bcs::to_bytes(&self.inner).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "Failed to serialize attestation")
        })?;
        writer.write_all(&value)?;
        Ok(())
    }
}

impl<
        Da: DaSpec,
        StorageProof: Serialize + DeserializeOwned,
        Root: Serialize + DeserializeOwned,
    > BorshDeserialize for WrappedAttestation<Da, StorageProof, Root>
{
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        bcs::from_reader(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    }

    fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> {
        bcs::from_bytes(buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    }
}

/// This enumeration represents the available call messages for interacting with the `AttesterIncentives` module.
#[derive(BorshDeserialize, BorshSerialize)]
pub enum CallMessage<C: sov_modules_api::Context, Da: DaSpec> {
    /// Bonds an attester, the parameter is the bond amount
    BondAttester(Amount),
    /// Start the first phase of the two-phase unbonding process
    BeginUnbondingAttester,
    /// Finish the two phase unbonding
    EndUnbondingAttester,
    /// Bonds a challenger, the parameter is the bond amount
    BondChallenger(Amount),
    /// Unbonds a challenger
    UnbondChallenger,
    /// Processes an attestation.
    ProcessAttestation(
        #[allow(clippy::type_complexity)]
        WrappedAttestation<
            Da,
            StorageProof<<<C as Spec>::Storage as Storage>::Proof>,
            <C::Storage as Storage>::Root,
        >,
    ),
    /// Processes a challenge. The challenge is encoded as a [`Vec<u8>`]. The second parameter is the transition number
    ProcessChallenge(Vec<u8>, TransitionHeight),
}

// Manually implement Debug to remove spurious Debug bound on C::Storage
impl<C: sov_modules_api::Context, Da: DaSpec> Debug for CallMessage<C, Da> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BondAttester(arg0) => f.debug_tuple("BondAttester").field(arg0).finish(),
            Self::BeginUnbondingAttester => write!(f, "BeginUnbondingAttester"),
            Self::EndUnbondingAttester => write!(f, "EndUnbondingAttester"),
            Self::BondChallenger(arg0) => f.debug_tuple("BondChallenger").field(arg0).finish(),
            Self::UnbondChallenger => write!(f, "UnbondChallenger"),
            Self::ProcessAttestation(arg0) => {
                f.debug_tuple("ProcessAttestation").field(arg0).finish()
            }
            Self::ProcessChallenge(arg0, arg1) => f
                .debug_tuple("ProcessChallenge")
                .field(arg0)
                .field(arg1)
                .finish(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
/// Error type that explains why a user is slashed
pub enum SlashingReason {
    #[error("Transition isn't found")]
    /// The specified transition does not exist
    TransitionNotFound,

    #[error("The attestation does not contain the right block hash and post-state transition")]
    /// The specified transition is invalid (block hash, post-root hash or validity condition)
    TransitionInvalid,

    #[error("The initial hash of the transition is invalid")]
    /// The initial hash of the transition is invalid.
    InvalidInitialHash,

    #[error("The proof opening raised an error")]
    /// The proof verification raised an error
    InvalidProofOutputs,

    #[error("No invalid transition to challenge")]
    /// No invalid transition to challenge.
    NoInvalidTransition,
}

/// Error raised while processing the attester incentives
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

    #[error("Attester is unbonding")]
    /// The attester is in the first unbonding phase
    AttesterIsUnbonding,

    #[error("User is not trying to unbond at the time of the transaction")]
    /// User is not trying to unbond at the time of the transaction
    AttesterIsNotUnbonding,

    #[error("The first phase of unbonding has not been finalized")]
    /// The attester is trying to finish the two-phase unbonding too soon
    UnbondingNotFinalized,

    #[error("The bond is not a 64-bit number")]
    /// The bond is not a 64-bit number
    InvalidBondFormat,

    #[error("User is not bonded at the time of the transaction")]
    /// User is not bonded at the time of the transaction
    UserNotBonded,

    #[error("Transition invariant isn't respected")]
    /// Transition invariant isn't respected
    InvalidTransitionInvariant,

    #[error("Error occurred when transferred funds")]
    /// An error occurred when transferred funds
    TransferFailure,

    #[error("Error when trying to mint the reward token")]
    /// An error occurred when trying to mint the reward token
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

impl<C, Vm, Da, Checker> AttesterIncentives<C, Vm, Da, Checker>
where
    C: sov_modules_api::Context,
    Vm: sov_modules_api::Zkvm,
    Da: sov_modules_api::DaSpec,
    Checker: ValidityConditionChecker<Da::ValidityCondition>,
{
    /// This returns the address of the reward token supply
    pub fn get_reward_token_supply_address(&self, working_set: &mut WorkingSet<C>) -> C::Address {
        self.reward_token_supply_address
            .get(working_set)
            .expect("The reward token supply address should be set at genesis")
    }

    /// Verifies the provided proof, returning its underlying storage value, if present.
    pub fn verify_proof(
        &self,
        state_root: <C::Storage as Storage>::Root,
        proof: StorageProof<<C::Storage as Storage>::Proof>,
        expected_key: &C::Address,
        working_set: &mut WorkingSet<C>,
    ) -> Result<Option<StorageValue>, anyhow::Error> {
        let storage = working_set.backing();
        let (storage_key, storage_value) = storage.open_proof(state_root, proof)?;
        let prefix = self.bonded_attesters.prefix();
        let codec = self.bonded_attesters.codec();

        // We have to check that the storage key is the same as the external key
        ensure!(
            storage_key == StorageKey::new(prefix, expected_key, codec),
            "The storage key from the proof doesn't match the expected storage key."
        );

        Ok(storage_value)
    }

    /// A helper function that simply slashes an attester and returns a reward value
    fn slash_user(&self, user: &C::Address, role: Role, working_set: &mut WorkingSet<C>) -> u64 {
        let bonded_set = match role {
            Role::Attester => {
                // We have to remove the attester from the unbonding set
                // to prevent him from skipping the first phase
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
        working_set: &mut WorkingSet<C>,
    ) -> AttesterIncentiveErrors {
        self.slash_user(user, role, working_set);
        AttesterIncentiveErrors::UserSlashed(reason)
    }

    /// A helper function that is used to slash an attester, and put the associated attestation in the slashed pool
    fn slash_and_invalidate_attestation(
        &self,
        attester: &C::Address,
        height: TransitionHeight,
        reason: SlashingReason,
        working_set: &mut WorkingSet<C>,
    ) -> AttesterIncentiveErrors {
        let reward = self.slash_user(attester, Role::Attester, working_set);

        let curr_reward_value = self
            .bad_transition_pool
            .get(&height, working_set)
            .unwrap_or_default();

        self.bad_transition_pool
            .set(&height, &(curr_reward_value + reward), working_set);

        AttesterIncentiveErrors::UserSlashed(reason)
    }

    fn reward_sender(
        &self,
        context: &C,
        amount: u64,
        working_set: &mut WorkingSet<C>,
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
            .mint_from_eoa(
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
        working_set: &mut WorkingSet<C>,
    ) -> Result<CallResponse, AttesterIncentiveErrors> {
        // If the user is an attester, we have to check that he's not trying to unbond
        if role == Role::Attester
            && self
                .unbonding_attesters
                .get(user_address, working_set)
                .is_some()
        {
            return Err(AttesterIncentiveErrors::AttesterIsUnbonding);
        }

        // Transfer the bond amount from the module's token minting address to the sender.
        // On failure, no state is changed
        let coins = Coins {
            token_address: self
                .bonding_token_address
                .get(working_set)
                .expect("Bonding token address must be set"),
            amount: bond_amount,
        };

        self.bank
            .transfer_from(user_address, &self.address, coins, working_set)
            .map_err(|_err| AttesterIncentiveErrors::TransferFailure)?;

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
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse> {
        // Get the user's old balance.
        if let Some(old_balance) = self.bonded_challengers.get(context.sender(), working_set) {
            // Transfer the bond amount from the sender to the module's address.
            // On failure, no state is changed
            self.reward_sender(context, old_balance, working_set)?;

            // Emit the unbonding event
            working_set.add_event(
                "unbonded_challenger",
                &format!("amount_withdrawn: {old_balance:?}"),
            );
        }

        Ok(CallResponse::default())
    }

    /// The attester starts the first phase of the two-phase unbonding.
    /// We put the current max finalized height with the attester address
    /// in the set of unbonding attesters if the attester
    /// is already present in the unbonding set
    pub(crate) fn begin_unbond_attester(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse, AttesterIncentiveErrors> {
        // First get the bonded attester
        if let Some(bond) = self.bonded_attesters.get(context.sender(), working_set) {
            let finalized_height = self
                .light_client_finalized_height
                .get(working_set)
                .expect("Must be set at genesis");

            // Remove the attester from the bonding set
            self.bonded_attesters.remove(context.sender(), working_set);

            // Then add the bonded attester to the unbonding set, with the current finalized height
            self.unbonding_attesters.set(
                context.sender(),
                &UnbondingInfo {
                    unbonding_initiated_height: finalized_height,
                    amount: bond,
                },
                working_set,
            );
        }

        Ok(CallResponse::default())
    }

    pub(crate) fn end_unbond_attester(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse, AttesterIncentiveErrors> {
        // We have to ensure that the attester is unbonding, and that the unbonding transaction
        // occurred at least `finality_period` blocks ago to let the attester unbond
        if let Some(unbonding_info) = self.unbonding_attesters.get(context.sender(), working_set) {
            // These two constants should always be set beforehand, hence we can panic if they're not set
            let curr_height = self
                .light_client_finalized_height
                .get(working_set)
                .expect("Should be defined at genesis");
            let finality_period = self
                .rollup_finality_period
                .get(working_set)
                .expect("Should be defined at genesis");

            if unbonding_info
                .unbonding_initiated_height
                .saturating_add(finality_period)
                > curr_height
            {
                return Err(AttesterIncentiveErrors::UnbondingNotFinalized);
            }

            // Get the user's old balance.
            // Transfer the bond amount from the sender to the module's address.
            // On failure, no state is changed
            self.reward_sender(context, unbonding_info.amount, working_set)?;

            // Update our internal tracking of the total bonded amount for the sender.
            self.bonded_attesters.remove(context.sender(), working_set);
            self.unbonding_attesters
                .remove(context.sender(), working_set);

            // Emit the unbonding event
            working_set.add_event("unbonded_challenger", {
                let amount = unbonding_info.amount;
                &format!("amount_withdrawn: {:?}", amount)
            });
        } else {
            return Err(AttesterIncentiveErrors::AttesterIsNotUnbonding);
        }
        Ok(CallResponse::default())
    }

    /// The bonding proof is now a proof that an attester was bonded during the last `finality_period` range.
    /// The proof must refer to a valid state of the rollup. The initial root hash must represent a state between
    /// the bonding proof one and the current state.
    #[allow(clippy::type_complexity)]
    fn check_bonding_proof(
        &self,
        context: &C,
        attestation: &Attestation<
            Da,
            StorageProof<<C::Storage as Storage>::Proof>,
            <C::Storage as Storage>::Root,
        >,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<(), AttesterIncentiveErrors> {
        let bonding_root = {
            // If we cannot get the transition before the current one, it means that we are trying
            // to get the genesis state root
            let transition_height = TransitionHeight::from(
                attestation
                    .proof_of_bond
                    .claimed_transition_num
                    .checked_sub(1)
                    .expect("The transition height should be greater than 1"),
            );
            if let Some(transition) = self
                .chain_state
                .get_historical_transitions(transition_height, working_set)
            {
                transition.post_state_root().clone()
            } else {
                self.chain_state
                    .get_genesis_hash(working_set)
                    .expect("The genesis hash should be set at genesis")
            }
        };

        // This proof checks that the attester was bonded at the given transition num
        let bond_opt = self
            .verify_proof(
                bonding_root,
                attestation.proof_of_bond.proof.clone(),
                context.sender(),
                working_set,
            )
            .map_err(|_err| AttesterIncentiveErrors::InvalidBondingProof)?;

        let bond = bond_opt.ok_or(AttesterIncentiveErrors::UserNotBonded)?;
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

    #[allow(clippy::type_complexity)]
    fn check_transition(
        &self,
        claimed_transition_height: TransitionHeight,
        attester: &C::Address,
        attestation: &Attestation<
            Da,
            StorageProof<<C::Storage as Storage>::Proof>,
            <C::Storage as Storage>::Root,
        >,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse, AttesterIncentiveErrors> {
        if let Some(curr_tx) = self
            .chain_state
            .get_historical_transitions(claimed_transition_height, working_set)
        {
            // We first need to compare the initial block hash to the previous post state root
            if !curr_tx.compare_hashes(&attestation.da_block_hash, &attestation.post_state_root) {
                // Check if the attestation has the same da_block_hash and post_state_root as the actual transition
                // that we found in state. If not, slash the attester.
                // If so, the attestation is valid, so return Ok
                return Err(self.slash_and_invalidate_attestation(
                    attester,
                    claimed_transition_height,
                    SlashingReason::TransitionInvalid,
                    working_set,
                ));
            }
            Ok(CallResponse::default())
        } else {
            // Case where we cannot get the transition from the chain state historical transitions.
            Err(self.slash_burn_reward(
                attester,
                Role::Attester,
                SlashingReason::TransitionNotFound,
                working_set,
            ))
        }
    }

    #[allow(clippy::type_complexity)]
    fn check_initial_hash(
        &self,
        claimed_transition_height: TransitionHeight,
        attester: &C::Address,
        attestation: &Attestation<
            Da,
            StorageProof<<C::Storage as Storage>::Proof>,
            <C::Storage as Storage>::Root,
        >,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse, AttesterIncentiveErrors> {
        // Normal state
        if let Some(transition) = self
            .chain_state
            .get_historical_transitions(claimed_transition_height.saturating_sub(1), working_set)
        {
            if transition.post_state_root() != &attestation.initial_state_root {
                // The initial root hashes don't match, just slash the attester
                return Err(self.slash_burn_reward(
                    attester,
                    Role::Attester,
                    SlashingReason::InvalidInitialHash,
                    working_set,
                ));
            }
        } else {
            // Genesis state
            // We can assume that the genesis hash is always set, otherwise we need to panic.
            // We don't need to prove that the attester was bonded, simply need to check that the current bond is higher than the
            // minimal bond and that the attester is not unbonding

            // We add a check here that the claimed transition height is the same as the genesis height.
            let genesis_height = self
                .chain_state
                .get_genesis_height(working_set)
                .expect("Must be set at genesis");
            let previous = claimed_transition_height
                .checked_sub(1)
                .expect("Transition height must be > 0");
            if genesis_height != previous {
                return Err(self.slash_burn_reward(
                    attester,
                    Role::Attester,
                    SlashingReason::TransitionNotFound,
                    working_set,
                ));
            }

            if self
                .chain_state
                .get_genesis_hash(working_set)
                .expect("The initial hash should be set")
                != attestation.initial_state_root
            {
                // Slash the attester, and burn the fees
                return Err(self.slash_burn_reward(
                    attester,
                    Role::Attester,
                    SlashingReason::InvalidInitialHash,
                    working_set,
                ));
            }

            // Normal state
        }

        Ok(CallResponse::default())
    }

    /// Try to process an attestation if the attester is bonded
    #[allow(clippy::type_complexity)]
    pub(crate) fn process_attestation(
        &self,
        context: &C,
        attestation: WrappedAttestation<
            Da,
            StorageProof<<C::Storage as Storage>::Proof>,
            <C::Storage as Storage>::Root,
        >,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse, AttesterIncentiveErrors> {
        let attestation = attestation.inner;
        // We first need to check that the attester is still in the bonding set
        if self
            .bonded_attesters
            .get(context.sender(), working_set)
            .is_none()
        {
            return Err(AttesterIncentiveErrors::UserNotBonded);
        }

        // If the bonding proof in the attestation is invalid, light clients will ignore the attestation. In that case, we should too.
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

        assert!(
            current_finalized_height <= last_attested_height,
            "The last attested height should always be below the current finalized height."
        );

        // Update the max_attested_height in case the blocks have already been finalized
        let new_height_to_attest = last_attested_height + 1;

        // Minimum height at which the proof of bond can be valid
        let min_height = new_height_to_attest.saturating_sub(finality);

        // We have to check the following order invariant is respected:
        // (height to attest - finality) <= bonding_proof.transition_num <= height to attest
        //
        // Which with our variable gives:
        // min_height <= bonding_proof.transition_num <= new_height_to_attest
        // If this invariant is respected, we can be sure that the attester was bonded at new_height_to_attest.
        if !(min_height <= attestation.proof_of_bond.claimed_transition_num
            && attestation.proof_of_bond.claimed_transition_num <= new_height_to_attest)
        {
            return Err(AttesterIncentiveErrors::InvalidTransitionInvariant);
        }

        // First compare the initial hashes
        self.check_initial_hash(
            attestation.proof_of_bond.claimed_transition_num,
            context.sender(),
            &attestation,
            working_set,
        )?;

        // Then compare the transition
        self.check_transition(
            attestation.proof_of_bond.claimed_transition_num,
            context.sender(),
            &attestation,
            working_set,
        )?;

        working_set.add_event(
            "processed_valid_attestation",
            &format!("attester: {:?}", context.sender()),
        );

        // Now we have to check whether the claimed_transition_num is the max_attested_height.
        // If so, update the maximum attested height and reward the sender
        if attestation.proof_of_bond.claimed_transition_num == new_height_to_attest {
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
        }

        // Then we can optimistically process the transaction
        Ok(CallResponse::default())
    }

    fn check_challenge_outputs_against_transition(
        &self,
        public_outputs: StateTransition<Da, C::Address, <C::Storage as Storage>::Root>,
        height: &TransitionHeight,
        condition_checker: &mut impl ValidityConditionChecker<Da::ValidityCondition>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<(), SlashingReason> {
        let transition = self
            .chain_state
            .get_historical_transitions(*height, working_set)
            .ok_or(SlashingReason::TransitionInvalid)?;

        let initial_hash = {
            if let Some(prev_transition) = self
                .chain_state
                .get_historical_transitions(height.saturating_sub(1), working_set)
            {
                prev_transition.post_state_root().clone()
            } else {
                self.chain_state
                    .get_genesis_hash(working_set)
                    .expect("The genesis hash should be set")
            }
        };

        if public_outputs.initial_state_root != initial_hash {
            return Err(SlashingReason::InvalidInitialHash);
        }

        if &public_outputs.slot_hash != transition.da_block_hash() {
            return Err(SlashingReason::TransitionInvalid);
        }

        if public_outputs.validity_condition != *transition.validity_condition() {
            return Err(SlashingReason::TransitionInvalid);
        }

        // TODO: Should we compare the validity conditions of the public outputs with the ones of the recorded transition?
        condition_checker
            .check(&public_outputs.validity_condition)
            .map_err(|_err| SlashingReason::TransitionInvalid)?;

        Ok(())
    }

    /// Try to process a zk proof if the challenger is bonded.
    pub(crate) fn process_challenge(
        &self,
        context: &C,
        proof: &[u8],
        transition_num: &TransitionHeight,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse, AttesterIncentiveErrors> {
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
            .expect("Should be set at genesis");

        // Find the faulty attestation pool and get the associated reward
        let attestation_reward: u64 = self
            .bad_transition_pool
            .get_or_err(transition_num, working_set)
            .map_err(|_| {
                self.slash_burn_reward(
                    context.sender(),
                    Role::Challenger,
                    SlashingReason::NoInvalidTransition,
                    working_set,
                )
            })?;

        let public_outputs_opt: anyhow::Result<
            StateTransition<Da, C::Address, <C::Storage as Storage>::Root>,
        > = Vm::verify_and_extract_output::<C::Address, Da, <C::Storage as Storage>::Root>(
            proof,
            &code_commitment,
        )
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
                })?;

                // Reward the challenger with half of the attestation reward (avoid DOS)
                self.reward_sender(context, attestation_reward / 2, working_set)?;

                // Now remove the bad transition from the pool
                self.bad_transition_pool.remove(transition_num, working_set);

                working_set.add_event(
                    "processed_valid_proof",
                    &format!("challenger: {:?}", context.sender()),
                );
            }
            Err(_err) => {
                // Slash the challenger
                return Err(self.slash_burn_reward(
                    context.sender(),
                    Role::Challenger,
                    SlashingReason::InvalidProofOutputs,
                    working_set,
                ));
            }
        }

        Ok(CallResponse::default())
    }
}
