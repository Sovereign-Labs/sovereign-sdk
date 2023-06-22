use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_bank::Coins;
use sov_modules_api::{CallResponse, Context, Spec};
use sov_rollup_interface::{optimistic::Attestation, zk::traits::Zkvm};
use sov_state::{Storage, WorkingSet};
use std::fmt::Debug;

use crate::{AttesterIncentives, UnbondingInfo};

/// This enumeration represents the available call messages for interacting with the `ExampleModule` module.
#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
// TODO: allow call messages to borrow data
//     https://github.com/Sovereign-Labs/sovereign-sdk/issues/274
pub enum CallMessage<C: Context> {
    BondAttester(u64),
    BeginAttesterUnbonding,
    FinishAttesterUnbonding,
    BondChallenger(u64),
    UnbondChallenger,
    ProcessAttestation(Attestation<<<C as Spec>::Storage as Storage>::Proof>),
    ProcessChallenge(Vec<u8>),
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
    /// A helper function for the `bond_prover` call. Also used to bond provers
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
            Role::Attester => (&self.bonded_attesters, "bonded_prover"),
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
                unbonding_initiated_height: context.block_height(),
            };

            // Update our internal tracking of the total bonded amount for the sender.
            self.bonded_attesters.set(context.sender(), &0, working_set);
            // Update our internal tracking of the total bonded amount for the sender.
            self.unbonding_attesters
                .set(context.sender(), &0, working_set);

            // Emit the unbonding event
            working_set.add_event(
                "unbonded_challenger",
                &format!("amount_withdrawn: {old_balance:?}"),
            );
        }

        Ok(CallResponse::default())
    }

    /// Try to process a zk proof, if the prover is bonded.
    pub(crate) fn process_proof(
        &self,
        proof: &[u8],
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // Get the prover's old balance.
        // Revert if they aren't bonded
        let old_balance = self
            .bonded_provers
            .get_or_err(context.sender(), working_set)?;

        // Check that the prover has enough balance to process the proof.
        let minimum_bond = self.minimum_bond.get_or_err(working_set)?;

        anyhow::ensure!(old_balance >= minimum_bond, "Prover is not bonded");
        let code_commitment = self
            .commitment_of_allowed_verifier_method
            .get_or_err(working_set)?
            .commitment;

        // Lock the prover's bond amount.
        self.bonded_provers
            .set(context.sender(), &(old_balance - minimum_bond), working_set);

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        if let Ok(_public_outputs) =
            Vm::verify(proof, &code_commitment).map_err(|e| anyhow::format_err!("{:?}", e))
        {
            // TODO: decide what the proof output is and do something with it
            //     https://github.com/Sovereign-Labs/sovereign-sdk/issues/272

            // Unlock the prover's bond
            // TODO: reward the prover with newly minted tokens as appropriate based on gas fees.
            //     https://github.com/Sovereign-Labs/sovereign-sdk/issues/271
            self.bonded_provers
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
