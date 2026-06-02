use schemars::JsonSchema;
use sov_bank::{Amount, IntoPayable};
use sov_modules_api::macros::{config_value, serialize, UniversalWallet};
use sov_modules_api::registration_lib::RegistrationError;
use sov_modules_api::{
    Context, DaSpec, EventEmitter, ModuleInfo, Spec, StateReader, StateWriter, TxState,
};
use sov_state::{Kernel, User};

use crate::{
    gas_coins, BalanceState, CustomError, Event, KnownSequencer, PendingDaAddressUpdate,
    RetiredDaAddress, SequencerRegistry, SequencerRegistryError,
};

/// This enumeration represents the available call messages for interacting with
/// the `sov-sequencer-registry` module.
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
pub enum CallMessage<S: Spec> {
    /// Add a new sequencer to the sequencer registry.
    Register {
        /// The Da address of the sequencer you're registering.
        da_address: <S::Da as DaSpec>::Address,
        /// The initial balance of the sequencer.
        amount: Amount,
    },
    /// Increases the balance of the sequencer, transferring the funds from the sequencer account
    /// to the rollup.
    Deposit {
        /// The DA address of the sequencer.
        da_address: <S::Da as DaSpec>::Address,
        /// The amount to increase.
        amount: Amount,
    },
    /// Initiate a withdrawal of a sequencer's balance.
    InitiateWithdrawal {
        /// The DA address of the sequencer you're removing.
        da_address: <S::Da as DaSpec>::Address,
    },
    /// Withdraw a sequencer's balance after waiting for the withdrawal period.
    Withdraw {
        /// The DA address of the sequencer you're removing.
        da_address: <S::Da as DaSpec>::Address,
    },
    /// Rotates the sequencer's DA address without unstaking.
    ///
    /// Authorized by the rollup key (`context.sender()`) — the intended
    /// recovery path when a DA signing key is compromised but the rollup
    /// key is safe. Preserves `balance`, `balance_state`, and the
    /// `preferred_sequencer` pointer (atomically moved to `new_da_address`
    /// if the caller was the preferred sequencer).
    ///
    /// After this call lands on-chain, the sequencer node operator must
    /// restart their binary with the new DA signer keys. The registry
    /// cannot enforce this from state.
    ///
    /// At most one rotation may be pending per rollup block. A preferred
    /// sequencer rotating its own DA defers application to end-of-block;
    /// while that rotation is pending, any other `UpdateDaAddress` in the
    /// same block (by any sequencer) is rejected and must be retried in a
    /// later block.
    UpdateDaAddress {
        /// The sequencer's current DA address (the one being rotated away from).
        old_da_address: <S::Da as DaSpec>::Address,
        /// The new DA address. Must not already be registered.
        new_da_address: <S::Da as DaSpec>::Address,
    },
}

impl<S: Spec> SequencerRegistry<S> {
    /// Tries to register a sequencer by staking the provided amount of gas tokens.
    /// This method uses the context's sender as the sequencer's address.
    ///
    /// # Errors
    /// Will error
    ///
    /// - If the provided amount is below the minimum required to register a sequencer.
    /// - If the minimum bond is not set.
    /// - If the sender's account does not have enough funds to register itself as a sequencer.
    /// - If the sequencer is already registered.
    pub(crate) fn register<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        amount: Amount,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.register_staker(da_address, amount, *context.sender(), state)?;

        Ok(())
    }

    pub(crate) fn register_staker<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        amount: Amount,
        address: S::Address,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        if let Some(existing_sequencer) = self.known_sequencers.get(da_address, state)? {
            return Err(RegistrationError::AlreadyRegistered(
                existing_sequencer.address,
            ));
        }
        self.ensure_not_retired_da_address(da_address, state)?;
        self.ensure_not_pending_new_da_address(da_address, state)?;

        let Some(minimum_bond) = self.minimum_bond.get(state)? else {
            return Err(SequencerRegistryError::<S, ST>::NoMinimumBondSet);
        };

        if amount < minimum_bond {
            return Err(SequencerRegistryError::<S, ST>::InsufficientStakeAmount {
                address,
                bond_amount: amount,
                minimum_bond_amount: minimum_bond,
            });
        }

        self.bank
            .transfer_from(
                &address,
                self.id().clone().to_payable(),
                gas_coins(amount),
                state,
            )
            .map_err(
                |_| SequencerRegistryError::<S, ST>::InsufficientFundsToRegister {
                    address,
                    amount,
                },
            )?;
        let new_sequencer = KnownSequencer {
            address,
            balance: amount,
            balance_state: BalanceState::Active,
        };
        self.known_sequencers
            .set(da_address, &new_sequencer, state)?;

        self.emit_event(
            state,
            Event::<S>::Registered {
                sequencer: address,
                amount,
            },
        );
        Ok(())
    }

    pub(crate) fn deposit<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        amount: Amount,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.validate_sender(da_address, context.sender(), state)?;
        let Some(mut existing_sequencer) = self.known_sequencers.get(da_address, state)? else {
            return Err(RegistrationError::IsNotRegistered(*da_address));
        };
        let address = existing_sequencer.address;
        existing_sequencer.balance = existing_sequencer.balance.checked_add(amount).ok_or(
            SequencerRegistryError::<S, ST>::ToppingAccountMakesBalanceOverflow {
                address,
                existing_balance: existing_sequencer.balance,
                amount_to_add: amount,
            },
        )?;
        // Depositing re-activates the account if inactive.
        existing_sequencer.balance_state = BalanceState::Active;

        self.bank
            .transfer_from(
                &address,
                self.id().clone().to_payable(),
                gas_coins(amount),
                state,
            )
            .map_err(
                |_| SequencerRegistryError::<S, ST>::InsufficientFundsToTopUpAccount {
                    address,
                    amount_to_add: amount,
                },
            )?;

        self.known_sequencers
            .set(da_address, &existing_sequencer, state)?;

        self.emit_event(
            state,
            Event::<S>::Deposited {
                sequencer: address,
                amount: amount.0,
            },
        );

        Ok(())
    }

    /// Tries to remove a sequencer by unstaking the provided amount of gas tokens.
    /// This method uses the context's sender as the sequencer's address.
    ///
    /// # Errors
    /// Will error
    ///
    /// - If the sequencer is not registered.
    /// - If the sequencer tries to unregister itself during the execution of its own batch.
    /// - If the supplied `da_address` does not match the transaction sender.
    /// - If the module balance is not high enough to refund the sequencer's staked amount (this is a bug).
    pub(crate) fn initiate_withdrawal<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.validate_sender(da_address, context.sender(), state)?;
        let Some(mut existing_sequencer) = self.known_sequencers.get(da_address, state)? else {
            return Err(RegistrationError::IsNotRegistered(*da_address));
        };

        if &existing_sequencer.address == context.sequencer() {
            return Err(RegistrationError::Custom(
                CustomError::CannotUnregisterDuringOwnBatch(*da_address),
            ));
        }
        if existing_sequencer.balance_state != BalanceState::Active {
            return Err(RegistrationError::WithdrawalAlreadyPending(
                existing_sequencer.address,
            ));
        }

        // We force the sequencer to wait to withdraw until all of their pending blobs will have been selected for processing or dropped.
        // In the worst case, this could take up to `DEFERRED_SLOTS_COUNT` slots, so wait until the slot after that.
        existing_sequencer.balance_state = BalanceState::PendingWithdrawal {
            ready_at: state
                .current_visible_slot_number()
                .advance(config_value!("DEFERRED_SLOTS_COUNT") + 1),
        };
        self.known_sequencers
            .set(da_address, &existing_sequencer, state)?;

        self.emit_event(
            state,
            Event::<S>::InitiatedWithdrawal {
                sequencer: existing_sequencer.address,
            },
        );
        Ok(())
    }

    pub(crate) fn withdraw<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.validate_sender(da_address, context.sender(), state)?;
        let Some(existing_sequencer) = self.known_sequencers.get(da_address, state)? else {
            return Err(RegistrationError::IsNotRegistered(*da_address));
        };
        let BalanceState::PendingWithdrawal { ready_at } = existing_sequencer.balance_state else {
            return Err(RegistrationError::Custom(
                CustomError::WithdrawalNotInitiated(*da_address),
            ));
        };
        if ready_at > state.current_visible_slot_number() {
            return Err(RegistrationError::Custom(CustomError::WithdrawalNotReady {
                sequencer: *da_address,
                current_visible_height: state.current_visible_slot_number(),
                ready_at,
            }));
        }
        self.known_sequencers.delete(da_address, state)?;
        self.bank
            .transfer_from(
                self.id().clone().to_payable(),
                &existing_sequencer.address,
                gas_coins(existing_sequencer.balance),
                state,
            )
            .expect("Failed to withdraw a sequencer balance. This indicates a bug in accounting!");

        self.emit_event(
            state,
            Event::<S>::Withdrew {
                sequencer: existing_sequencer.address,
                amount_withdrawn: existing_sequencer.balance,
            },
        );

        Ok(())
    }

    /// Rotates a sequencer's DA address while preserving their stake and state.
    ///
    /// Authorized by the caller's rollup key: the entry at `old_da_address`
    /// must have been registered under `context.sender()`. This blocks
    /// impersonation of another sequencer's DA.
    ///
    /// # Errors
    /// - If `new_da_address` equals `old_da_address`.
    /// - If `old_da_address` is not registered.
    /// - If the caller's rollup key does not own the entry at `old_da_address`.
    /// - If another DA address update is already pending for this rollup block
    ///   (by any sequencer — at most one rotation may be pending per block).
    /// - If `old_da_address` is the DA address the caller is actively sequencing from
    ///   for this slot and the caller is not the preferred sequencer
    ///   (`CannotUnregisterDuringOwnBatch`). The preferred sequencer may rotate its own
    ///   producing DA; that rotation is deferred until the end of the rollup block.
    /// - If `new_da_address` is already registered.
    /// - If `new_da_address` was retired by a previous rotation.
    pub(crate) fn update_da_address<ST: TxState<S>>(
        &mut self,
        old_da_address: &<S::Da as DaSpec>::Address,
        new_da_address: &<S::Da as DaSpec>::Address,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        if old_da_address == new_da_address {
            return Err(RegistrationError::Custom(
                CustomError::NewDaAddressSameAsOld(*old_da_address),
            ));
        }

        let Some(existing_sequencer) = self.known_sequencers.get(old_da_address, state)? else {
            return Err(RegistrationError::IsNotRegistered(*old_da_address));
        };

        if context.sender() != &existing_sequencer.address {
            return Err(RegistrationError::Custom(
                CustomError::SuppliedAddressDoesNotMatchTxSender {
                    parameter: existing_sequencer.address,
                    sender: *context.sender(),
                },
            ));
        }

        if let Some(conflict) = self.known_sequencers.get(new_da_address, state)? {
            return Err(RegistrationError::AlreadyRegistered(conflict.address));
        }
        self.ensure_not_retired_da_address(new_da_address, state)?;

        // Only one rotation may be pending per rollup block. This unconditional check also
        // covers the case where `new_da_address` is itself the pending target, so a separate
        // `ensure_not_pending_new_da_address` call here would be redundant.
        if let Some(pending) = self.pending_da_address_update.get(state)? {
            return Err(RegistrationError::AlreadyRegistered(pending.sequencer));
        }

        // Dispatch on whether `old_da_address` is the DA address producing the current
        // batch. `context.sequencer_da_address()` is the authenticated DA address of the
        // batch currently executing; its bond is reserved for the whole batch, so its
        // registry entry must not be mutated mid-batch. The preferred sequencer is the
        // sole exception: it may rotate its own producing DA, but the move is deferred to
        // end-of-block (applied by `BlockHooks::end_rollup_block_hook`) to preserve batch
        // ordering. Rotating any other DA the caller owns is unrelated to the current
        // batch and applies immediately.
        if old_da_address == context.sequencer_da_address() {
            if !context.sequencer_is_preferred() {
                return Err(RegistrationError::Custom(
                    CustomError::CannotUnregisterDuringOwnBatch(*old_da_address),
                ));
            }

            self.pending_da_address_update.set(
                &PendingDaAddressUpdate {
                    sequencer: existing_sequencer.address,
                    old_da_address: *old_da_address,
                    new_da_address: *new_da_address,
                },
                state,
            )?;
        } else {
            self.apply_da_rotation(old_da_address, new_da_address, &existing_sequencer, state)?;
        }

        self.emit_event(
            state,
            Event::<S>::DaAddressUpdated {
                sequencer: existing_sequencer.address,
                old_da_address: *old_da_address,
                new_da_address: *new_da_address,
            },
        );
        Ok(())
    }

    /// Moves the `known_sequencers` entry from `old_da_address` to `new_da_address`,
    /// records `old_da_address` as retired (so escrow refunds addressed to it follow the
    /// sequencer to its current DA), and moves the `preferred_sequencer` pointer if it
    /// referenced the old address.
    ///
    /// Shared by the immediate path ([`Self::update_da_address`]) and the deferred
    /// end-of-block path (`BlockHooks::end_rollup_block_hook`) so the two cannot drift.
    /// The caller is responsible for all validation and for emitting
    /// [`Event::DaAddressUpdated`].
    pub(crate) fn apply_da_rotation<E, Accessor>(
        &mut self,
        old_da_address: &<S::Da as DaSpec>::Address,
        new_da_address: &<S::Da as DaSpec>::Address,
        existing_sequencer: &KnownSequencer<S>,
        state: &mut Accessor,
    ) -> Result<(), E>
    where
        Accessor: StateReader<Kernel, Error = E>
            + StateWriter<Kernel, Error = E>
            + StateReader<User, Error = E>
            + StateWriter<User, Error = E>,
    {
        self.known_sequencers.delete(old_da_address, state)?;
        self.known_sequencers
            .set(new_da_address, existing_sequencer, state)?;
        self.retired_da_addresses.set(
            old_da_address,
            &RetiredDaAddress {
                sequencer: existing_sequencer.address,
                current_da_address: *new_da_address,
            },
            state,
        )?;

        if self.preferred_sequencer.get(state)?.as_ref() == Some(old_da_address) {
            self.preferred_sequencer.set(new_da_address, state)?;
        }

        Ok(())
    }

    fn ensure_not_retired_da_address<ST: TxState<S>>(
        &self,
        da_address: &<S::Da as DaSpec>::Address,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        if let Some(retired) = self.retired_da_addresses.get(da_address, state)? {
            return Err(RegistrationError::AlreadyRegistered(retired.sequencer));
        }

        Ok(())
    }

    fn ensure_not_pending_new_da_address<ST: TxState<S>>(
        &self,
        da_address: &<S::Da as DaSpec>::Address,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        if let Some(pending) = self.pending_da_address_update.get(state)? {
            if &pending.new_da_address == da_address {
                return Err(RegistrationError::AlreadyRegistered(pending.sequencer));
            }
        }

        Ok(())
    }

    fn validate_sender<ST: TxState<S>>(
        &self,
        da_address: &<S::Da as DaSpec>::Address,
        sender: &S::Address,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        let belongs_to = self
            .known_sequencers
            .get_or_err(da_address, state)?
            .map_err(|_| RegistrationError::IsNotRegistered(*da_address))?
            .address;

        if sender != &belongs_to {
            return Err(RegistrationError::Custom(
                CustomError::SuppliedAddressDoesNotMatchTxSender {
                    parameter: belongs_to,
                    sender: *sender,
                },
            ));
        }

        Ok(())
    }
}
