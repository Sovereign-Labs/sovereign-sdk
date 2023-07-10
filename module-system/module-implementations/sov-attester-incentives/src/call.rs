use crate::{AttesterIncentives, UnbondingInfo};
use anyhow::{ensure, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_bank::Coins;
use sov_modules_api::{CallResponse, Context, Spec};
use sov_rollup_interface::{optimistic::Attestation, zk::traits::Zkvm};
use sov_state::{storage::StorageProof, Storage, WorkingSet};
use std::{
    cmp::min,
    fmt::{self, Debug},
};
use thiserror::Error;

/// This enumeration represents the available call messages for interacting with the `ExampleModule` module.
#[derive(BorshDeserialize, BorshSerialize, Debug)]
// TODO: allow call messages to borrow data
//     https://github.com/Sovereign-Labs/sovereign-sdk/issues/274
pub enum CallMessage<C: Context> {
    BondAttester(u64),
    BeginAttesterUnbonding,
    FinishAttesterUnbonding,
    BondChallenger(u64),
    UnbondChallenger,
    ProcessAttestation(Attestation<StorageProof<<<C as Spec>::Storage as Storage>::Proof>>),
    ProcessChallenge(Vec<u8>),
}

// Raised when an attester is slashed
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

impl<C: sov_modules_api::Context, Vm: Zkvm> AttesterIncentives<C, Vm> {
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
        // Get the challenger's old balance.
        if let Some(old_balance) = self.bonded_challengers.get(context.sender(), working_set) {
            // Transfer the bond amount from the sender to the module's address.
            // On failure, no state is changed
            let coins = Coins {
                token_address: self
                    .bonding_token_address
                    .get(working_set)
                    .expect("Bonding token address must be set"),
                amount: old_balance,
            };
            // Try to unbond the entire balance
            // If the unbonding fails, no state is changed
            self.bank
                .transfer_from(&self.address, context.sender(), coins, working_set)?;

            // Update our internal tracking of the total bonded amount for the sender.
            self.bonded_challengers
                .set(context.sender(), &0, working_set);

            // Emit the unbonding event
            working_set.add_event(
                "unbonded_challenger",
                &format!("amount_withdrawn: {old_balance:?}"),
            );
        }

        Ok(CallResponse::default())
    }

    /// Try to unbond the requested amount of coins with context.sender() as the beneficiary.
    pub(crate) fn begin_unbonding_attester(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // Get the attester's old balance.
        if let Some(old_balance) = self.bonded_attesters.get(context.sender(), working_set) {
            let unbonding_info = UnbondingInfo {
                amount: old_balance,
                unbonding_initiated_height: self
                    .light_client_finalized_height
                    .get(working_set)
                    .unwrap(),
            };

            // Update our internal tracking of the total bonded amount for the sender.
            self.bonded_attesters.set(context.sender(), &0, working_set);
            // Update our internal tracking of the total bonded amount for the sender.
            self.unbonding_attesters
                .set(context.sender(), &unbonding_info, working_set);

            // Emit the unbonding event
            working_set.add_event(
                "unbonded_challenger",
                &format!("amount_withdrawn: {old_balance:?}"),
            );
        }

        Ok(CallResponse::default())
    }

    /// Second phase of the two phase unbonding, we finish unbonding the attester
    /// by checking that at the rollup finality period has expired.
    pub(crate) fn finish_unbonding_attester(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // Check that the finality period has expired
        let unbonding_info = self
            .unbonding_attesters
            .get(context.sender(), working_set)
            .ok_or(AttesterIncentiveErrors::AttesterNotUnbonding)?;
        let finality_period = self
            .rollup_finality_period
            .get(working_set)
            .unwrap_or_default();

        // Panic if we cannot get the finalized height
        let finalized_height = self.light_client_finalized_height.get(working_set).unwrap();

        ensure!(
            unbonding_info.unbonding_initiated_height + finality_period < finalized_height,
            "The finality period has not expired yet"
        );

        // We can start the transfer
        let coins = Coins {
            token_address: self
                .bonding_token_address
                .get(working_set)
                .expect("Bonding token address must be set"),
            amount: unbonding_info.amount,
        };

        // Try to unbond the entire balance
        // If the unbonding fails, no state is changed
        self.bank
            .transfer_from(&self.address, context.sender(), coins, working_set)?;

        // We can safely remove the attester from the unbonding set once we've transfered the bond
        self.unbonding_attesters
            .remove(context.sender(), working_set);

        Ok(CallResponse::default())
    }

    /// Try to process an attestation, if the attester is bonded
    pub(crate) fn process_attestation(
        &self,
        attestation: Attestation<StorageProof<<C::Storage as Storage>::Proof>>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // We have to check that the current bond is still greater than the minimum bond
        let current_bond = self.bonded_attesters.get(context.sender(), working_set);

        ensure!(current_bond.is_some(), "Attester is not currently bonded");

        let current_bond = current_bond.unwrap();

        let min_bond = self.minimum_attester_bond.get(working_set).unwrap_or(0);

        ensure!(
            current_bond >= min_bond,
            "The current bond is not high enough"
        );

        // We need to check the bonding proof
        // This proof checks that the attester was bonded at the initial root hash
        let (attester_key, bond_opt) = working_set
            .backing()
            .open_proof(attestation.initial_state_root, attestation.proof_of_bond)?;

        // TODO: Do we have to check here the initial state root against the storage? I feel that it is already done at the
        // previous step right above.
        // We also need to check the block hash somewhere (but where?)
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

        // We have to check that this attester is currently not unbonding:
        // if so we have to slash it
        if !self
            .unbonding_attesters
            .get(context.sender(), working_set)
            .is_none()
        {
            // The attester is in the unbonding phase, we have to slash him
            working_set.add_event(
                "an attester tried to execute an attestation during the unbonding phase",
                &format!("slashed_attester: {:?}", context.sender()),
            );
            // TODO: Should we set it to 0? or should we only remove the minimal bond?
            self.bonded_attesters
                .set(context.sender(), &(current_bond - min_bond), working_set);

            Err(AttesterIncentiveErrors::AttesterSlashed.into())
        } else {
            working_set.add_event(
                "processed_valid_attestation",
                &format!("attester: {:?}", context.sender()),
            );

            // Then we can optimistically process the transaction
            // TODO: How do we put the attestation on the DA layer?
            Ok(CallResponse::default())
        }
    }

    /// Try to process a zk proof, if the challenger is bonded.
    pub(crate) fn process_challenge(
        &self,
        proof: &[u8],
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

        // Lock the challenger's bond amount.
        self.bonded_challengers
            .set(context.sender(), &(old_balance - minimum_bond), working_set);

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        if let Ok(_public_outputs) =
            Vm::verify(proof, &code_commitment).map_err(|e| anyhow::format_err!("{:?}", e))
        {
            // TODO: decide what the proof output is and do something with it
            //     https://github.com/Sovereign-Labs/sovereign-sdk/issues/272

            // Unlock the challenger's bond
            // TODO: reward the challenger with newly minted tokens as appropriate based on gas fees.
            //     https://github.com/Sovereign-Labs/sovereign-sdk/issues/271
            self.bonded_challengers
                .set(context.sender(), &old_balance, working_set);

            working_set.add_event(
                "processed_valid_proof",
                &format!("prover: {:?}", context.sender()),
            );
        } else {
            working_set.add_event(
                "processed_invalid_proof",
                &format!("slashed_prover: {:?}", context.sender()),
            );
        }

        Ok(CallResponse::default())
    }
}
