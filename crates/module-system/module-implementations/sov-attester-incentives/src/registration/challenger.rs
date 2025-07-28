use core::result::Result::Ok;

use sov_bank::Amount;
use sov_modules_api::registration_lib::StakeRegistration;
use sov_modules_api::{Context, EventEmitter, Spec, TxState};

use super::{AttesterRegistryError, Staker};
use crate::{AttesterIncentives, Event};

impl<S> AttesterIncentives<S>
where
    S: Spec,
{
    pub(crate) fn register_challenger<ST: TxState<S>>(
        &mut self,
        bond_amount: Amount,
        user_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), AttesterRegistryError<S, ST>> {
        let mut challenger = Staker::new_challenger(self);
        challenger.register_staker(user_address, user_address, bond_amount, state)?;

        self.emit_event(
            state,
            Event::<S>::RegisteredChallenger {
                amount: bond_amount,
            },
        );

        Ok(())
    }

    /// Try to unbond the requested amount of coins with context.sender() as the beneficiary.
    pub(crate) fn exit_challenger(
        &mut self,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let mut challenger = Staker::new_challenger(self);
        let amount_withdrawn = challenger.exit_staker(context.sender(), state)?;

        self.emit_event(state, Event::<S>::ExitedChallenger { amount_withdrawn });

        Ok(())
    }
}
