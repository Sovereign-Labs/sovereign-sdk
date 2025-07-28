use core::result::Result::Ok;

use sov_bank::Amount;
use sov_modules_api::registration_lib::{RegistrationError, StakeRegistration};
use sov_modules_api::{Context, EventEmitter, Spec, TxState};

use super::{AttesterRegistryError, CustomError, Staker};
use crate::{AttesterIncentives, Event, UnbondingInfo};

impl<S> AttesterIncentives<S>
where
    S: Spec,
{
    pub(crate) fn register_attester<ST: TxState<S>>(
        &mut self,
        bond_amount: Amount,
        user_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), AttesterRegistryError<S, ST>> {
        if self.unbonding_attesters.get(user_address, state)?.is_some() {
            return Err(RegistrationError::Custom(CustomError::AttesterIsUnbonding(
                user_address.clone(),
            )));
        }

        let mut attester = Staker::new_attester(self);
        attester.register_staker(user_address, user_address, bond_amount, state)?;
        let event = Event::<S>::RegisteredAttester {
            amount: bond_amount,
        };

        self.emit_event(state, event);
        Ok(())
    }

    pub(crate) fn deposit_attester<ST: TxState<S>>(
        &mut self,
        amount: Amount,
        attester_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), AttesterRegistryError<S, ST>> {
        if self
            .unbonding_attesters
            .get(attester_address, state)?
            .is_some()
        {
            return Err(RegistrationError::Custom(CustomError::AttesterIsUnbonding(
                attester_address.clone(),
            )));
        }

        let mut attester = Staker::new_attester(self);
        attester.deposit_funds(attester_address, amount, state)?;

        Ok(())
    }

    /// The attester starts the first phase of the two-phase unbonding.
    /// We put the current max finalized height with the attester address
    /// in the set of unbonding attesters if the attester
    /// is already present in the unbonding set
    pub(crate) fn begin_exit_attester<ST: TxState<S>>(
        &mut self,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), AttesterRegistryError<S, ST>> {
        // First get the bonded attester
        if let Some(bond) = self.bonded_attesters.get(context.sender(), state)? {
            let finalized_height = self
                .light_client_finalized_height
                .get(state)?
                .expect("Must be set at genesis");

            // Remove the attester from the bonding set
            self.bonded_attesters.remove(context.sender(), state)?;

            // Then add the bonded attester to the unbonding set, with the current finalized height
            self.unbonding_attesters.set(
                context.sender(),
                &UnbondingInfo {
                    unbonding_initiated_height: finalized_height,
                    amount: bond,
                },
                state,
            )?;
        }

        Ok(())
    }

    pub(crate) fn exit_attester<ST: TxState<S>>(
        &mut self,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), AttesterRegistryError<S, ST>> {
        // We have to ensure that the attester is unbonding, and that the unbonding transaction
        // occurred at least `finality_period` blocks ago to let the attester unbond
        if let Some(unbonding_info) = self.unbonding_attesters.get(context.sender(), state)? {
            // These two constants should always be set beforehand, hence we can panic if they're not set
            let curr_height = self
                .light_client_finalized_height
                .get(state)?
                .expect("Should be defined at genesis");

            let finality_period = self
                .rollup_finality_period
                .get(state)?
                .expect("Should be defined at genesis");

            if unbonding_info
                .unbonding_initiated_height
                .saturating_add(finality_period.get())
                > curr_height
            {
                return Err(RegistrationError::Custom(
                    CustomError::UnbondingNotFinalized(context.sender().clone()),
                ));
            }

            // Get the user's old balance.
            // Transfer the bond amount from the sender to the module's id.
            // On failure, no state is changed
            self.transfer_tokens_to_sender(context.sender(), unbonding_info.amount, state)
                .map_err(|_err| {
                    AttesterRegistryError::<S, ST>::InsufficientFundsToRefundStakedAmount {
                        address: context.sender().clone(),
                        amount: unbonding_info.amount,
                    }
                })?;

            // Update our internal tracking of the total bonded amount for the sender.
            self.bonded_attesters.remove(context.sender(), state)?;
            self.unbonding_attesters.remove(context.sender(), state)?;

            self.emit_event(
                state,
                Event::<S>::ExitedAttester {
                    amount_withdrawn: unbonding_info.amount,
                },
            );
        } else {
            return Err(RegistrationError::Custom(
                CustomError::AttesterIsNotUnbonding(context.sender().clone()),
            ));
        }
        Ok(())
    }
}
