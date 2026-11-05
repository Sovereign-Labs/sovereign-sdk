use std::convert::Infallible;

use sov_bank::error::TransferTokenError;
use sov_bank::{config_gas_token_id, Coins, IntoPayable, Payable};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, DaSpec, InfallibleStateAccessor, Spec, StateReader, StateWriter};
use sov_state::{Kernel, User};

use crate::{BalanceState, KnownSequencer, RetiredDaAddress, SequencerRegistry};

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
    ) -> Result<(), AddToStakeError<S>> {
        let Some(sequencer) = self.refund_da_address(sequencer, state) else {
            return Err(AddToStakeError::SequencerNotRegistered {
                sequencer: *sequencer,
            });
        };
        if let Some(KnownSequencer {
            address,
            balance,
            balance_state,
        }) = self
            .known_sequencers
            .get(&sequencer, state)
            .unwrap_infallible()
        {
            // Note that we don't check if the sequencer is active here, because the sequencer can get
            // refunded from escrow while their withdrawal is pending. In fact, that's the whole point of the
            // withdrawal period - to wait until all possible escrows involving the sequencer are resolved.
            let new_balance =
                balance
                    .checked_add(amount)
                    .ok_or_else(|| AddToStakeError::BalanceOverflow {
                        sequencer,
                        original_balance: balance,
                        amount_to_add: amount,
                    })?;

            let coins = Coins {
                amount,
                token_id: config_gas_token_id(),
            };
            self.bank
                .transfer_from(sender, self.id.to_payable(), coins, state)?;

            self.known_sequencers
                .set(
                    &sequencer,
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
            Err(AddToStakeError::SequencerNotRegistered { sequencer })
        }
    }

    /// Returns the DA address whose stake receives funds addressed to `da_address`: the address
    /// itself, or, if it was retired by a rotation, the sequencer's current DA address.
    fn refund_da_address<
        Accessor: StateReader<Kernel, Error = Infallible> + StateReader<User, Error = Infallible>,
    >(
        &self,
        da_address: &<S::Da as DaSpec>::Address,
        state: &mut Accessor,
    ) -> Option<<S::Da as DaSpec>::Address> {
        // A retired DA address is never registered: registering it deletes the retired entry.
        match self
            .retired_da_addresses
            .get(da_address, state)
            .unwrap_infallible()
        {
            Some(retired) => self.active_successor(&retired, state).unwrap_infallible(),
            None => Some(*da_address),
        }
    }

    /// Returns the registered DA address that `retired` was rotated into, or [`None`] if that
    /// registration has since been withdrawn or slashed.
    pub(crate) fn active_successor<E>(
        &self,
        retired: &RetiredDaAddress<S>,
        state: &mut impl StateReader<Kernel, Error = E>,
    ) -> Result<Option<<S::Da as DaSpec>::Address>, E> {
        let Some(current) = self
            .current_da_address_by_sequencer
            .get(&retired.sequencer, state)?
        else {
            return Ok(None);
        };
        if current.registration_da_address != retired.registration_da_address {
            return Ok(None);
        }

        Ok(self
            .known_sequencers
            .get(&current.current_da_address, state)?
            .is_some_and(|known| known.address == retired.sequencer)
            .then_some(current.current_da_address))
    }
}

/// Errors that can arise when incrementing a sequencer's stake
#[derive(thiserror::Error, Debug)]
pub enum AddToStakeError<S: Spec> {
    /// The bank transfer failed
    #[error(transparent)]
    Bank(#[from] TransferTokenError),
    /// The sequencer is not in the registry, e.g. because it was slashed after one of its blobs was queued.
    #[error("Sequencer {sequencer} is not registered")]
    SequencerNotRegistered {
        /// DA address of the sequencer.
        sequencer: <S::Da as DaSpec>::Address,
    },
    /// The sequencer balance would overflow if rewarded in this way
    #[error("Balance overflow for {sequencer} while trying to add to their stake. original_balance: {original_balance}. amount_to_add: {amount_to_add}")]
    BalanceOverflow {
        /// DA address of the sequencer.
        sequencer: <S::Da as DaSpec>::Address,
        /// The sequencer's previous balance
        original_balance: Amount,
        /// the amount to add
        amount_to_add: Amount,
    },
}
