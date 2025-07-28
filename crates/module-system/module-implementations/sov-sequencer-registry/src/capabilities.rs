use std::convert::Infallible;

use sov_bank::{config_gas_token_id, Coins, IntoPayable, Payable};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, DaSpec, InfallibleStateAccessor, Spec, StateReader, StateWriter};
use sov_state::{Kernel, User};

use crate::{BalanceState, KnownSequencer, SequencerRegistry};

impl<S: Spec> SequencerRegistry<S> {
    /// Returns the preferred sequencer, or [`None`] it wasn't set.
    pub fn preferred_sequencer(
        &self,
        scratchpad: &mut impl InfallibleStateAccessor,
    ) -> Option<<S::Da as DaSpec>::Address> {
        self.preferred_sequencer.get(scratchpad).unwrap_infallible()
    }

    /// Transfers a portion of the sequencer's stake to the beneficiary and decreases the staked balance.
    pub fn remove_part_of_the_stake<
        Accessor: StateWriter<Kernel, Error = Infallible>
            + StateWriter<User, Error = Infallible>
            + StateReader<Kernel, Error = Infallible>
            + StateReader<User, Error = Infallible>,
    >(
        &mut self,
        sequencer: &<S::Da as DaSpec>::Address,
        beneficiary: impl Payable<S>,
        amount: Amount,
        state: &mut Accessor,
    ) -> Result<(), anyhow::Error> {
        if let Some(KnownSequencer {
            address,
            balance,
            balance_state,
        }) = self
            .known_sequencers
            .get(sequencer, state)
            .unwrap_infallible()
        {
            // The sequencer has to be active in order to use their stake for blob submission.
            if balance_state != BalanceState::Active {
                anyhow::bail!("Sequencer {} is not active", sequencer);
            }
            let new_balance = balance.checked_sub(amount).ok_or_else(|| {
                anyhow::anyhow!(
                    "Sequencer's: {} stake is too low. Current stake: {}, amount to deduct: {}",
                    sequencer,
                    balance,
                    amount
                )
            })?;

            let coins = Coins {
                amount,
                token_id: config_gas_token_id(),
            };

            self.bank
                .transfer_from(self.id.clone().to_payable(), beneficiary, coins, state)?;

            self.known_sequencers
                .set(
                    sequencer,
                    &KnownSequencer {
                        address,
                        balance: new_balance,
                        balance_state,
                    },
                    state,
                )
                .unwrap_infallible();

            Ok(())
        } else {
            anyhow::bail!("Sequencer {} is not registered", sequencer)
        }
    }

    /// Increases the staked balance of the sequencer by transferring the given amount from the sender to the SequencerRegistry module.
    pub fn add_to_stake<
        Accessor: StateWriter<Kernel, Error = Infallible>
            + StateWriter<User, Error = Infallible>
            + StateReader<Kernel, Error = Infallible>
            + StateReader<User, Error = Infallible>,
    >(
        &mut self,
        sender: impl Payable<S>,
        sequencer: &<S::Da as DaSpec>::Address,
        amount: Amount,
        state: &mut Accessor,
    ) -> anyhow::Result<()> {
        if let Some(KnownSequencer {
            address,
            balance,
            balance_state,
        }) = self
            .known_sequencers
            .get(sequencer, state)
            .unwrap_infallible()
        {
            // Note that we don't check if the sequencer is active here, because the sequencer can get
            // refunded from escrow while their withdrawal is pending. In fact, that's the whole point of the
            // withdrawal period - to wait until all possible escrows involving the sequencer are resolved.
            let new_balance = balance.checked_add(amount).ok_or_else(|| {
                anyhow::anyhow!(
                    "Sequencer {}: overflow error unable to increase sequencer's stake",
                    sequencer
                )
            })?;

            let coins = Coins {
                amount,
                token_id: config_gas_token_id(),
            };
            self.bank
                .transfer_from(sender, self.id.to_payable(), coins, state)?;

            self.known_sequencers
                .set(
                    sequencer,
                    &KnownSequencer {
                        address,
                        balance: new_balance,
                        balance_state,
                    },
                    state,
                )
                .unwrap_infallible();

            Ok(())
        } else {
            anyhow::bail!("Sequencer {} is not registered", sequencer)
        }
    }
}
