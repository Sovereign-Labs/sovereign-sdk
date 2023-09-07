use std::cell::RefCell;
use std::rc::Rc;

use ibc::applications::transfer::context::{
    on_acknowledgement_packet_validate, on_chan_open_ack_validate, on_chan_open_confirm_validate,
    on_chan_open_init_execute, on_chan_open_init_validate, on_chan_open_try_execute,
    on_chan_open_try_validate, on_recv_packet_execute, on_timeout_packet_execute,
    on_timeout_packet_validate, TokenTransferExecutionContext, TokenTransferValidationContext,
};
use ibc::applications::transfer::error::TokenTransferError;
use ibc::applications::transfer::{self, PrefixedCoin, PORT_ID_STR, VERSION};
use ibc::core::ics04_channel::acknowledgement::Acknowledgement;
use ibc::core::ics04_channel::channel::{Counterparty, Order};
use ibc::core::ics04_channel::error::{ChannelError, PacketError};
use ibc::core::ics04_channel::packet::Packet;
use ibc::core::ics04_channel::Version as ChannelVersion;
use ibc::core::ics24_host::identifier::{ChannelId, ConnectionId, PortId};
use ibc::core::router::ModuleExtras;
use ibc::Signer;
use sov_bank::Coins;
use sov_rollup_interface::digest::Digest;
use sov_state::WorkingSet;
use uint::FromDecStrErr;

use crate::Transfer;

/// We need to create a wrapper around the `Transfer` module and `WorkingSet`,
/// because we only get the `WorkingSet` at call-time from the Sovereign SDK,
/// which must be passed to `TokenTransferValidationContext` methods through
/// the `self` argument.
pub struct TransferContext<'ws, 'c, C: sov_modules_api::Context> {
    pub transfer_mod: Transfer<C>,
    pub sdk_context: &'c C,
    pub working_set: Rc<RefCell<&'ws mut WorkingSet<C::Storage>>>,
}

impl<'ws, 'c, C> TransferContext<'ws, 'c, C>
where
    C: sov_modules_api::Context,
{
    pub fn new(
        transfer_mod: Transfer<C>,
        sdk_context: &'c C,
        working_set: Rc<RefCell<&'ws mut WorkingSet<C::Storage>>>,
    ) -> Self {
        Self {
            transfer_mod,
            sdk_context,
            working_set,
        }
    }

    // The escrow address follows the format as outlined in ADR 028:
    // https://github.com/cosmos/cosmos-sdk/blob/master/docs/architecture/adr-028-public-key-addresses.md
    // except that we don't use a different hash function.
    fn get_escrow_account(&self, port_id: &PortId, channel_id: &ChannelId) -> C::Address {
        // TODO: Probably cache so we don't need to hash every time
        let escrow_account_bytes: [u8; 32] = {
            let mut hasher = <C::Hasher as Digest>::new();
            hasher.update(VERSION);
            hasher.update([0]);
            hasher.update(format!("{port_id}/{channel_id}"));

            let hash = hasher.finalize();
            *hash.as_ref()
        };

        escrow_account_bytes.into()
    }

    /// Transfers `amount` tokens from `from_account` to `to_account`
    fn transfer(
        &self,
        token_address: C::Address,
        from_account: &C::Address,
        to_account: &C::Address,
        amount: &transfer::Amount,
    ) -> Result<(), TokenTransferError> {
        let amount: sov_bank::Amount = (*amount.as_ref())
            .try_into()
            .map_err(|_| TokenTransferError::InvalidAmount(FromDecStrErr::InvalidLength))?;
        let coin = Coins {
            amount,
            token_address,
        };

        self.transfer_mod
            .bank
            .transfer_from(
                from_account,
                to_account,
                coin,
                &mut self.working_set.borrow_mut(),
            )
            .map_err(|err| TokenTransferError::InternalTransferFailed(err.to_string()))?;

        Ok(())
    }
}

impl<'ws, 'c, C> core::fmt::Debug for TransferContext<'ws, 'c, C>
where
    C: sov_modules_api::Context,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferContext")
            .field("transfer_mod", &self.transfer_mod)
            .finish()
    }
}

/// Extra data to be passed to `TokenTransfer` contexts' escrow methods
pub struct EscrowExtraData<C: sov_modules_api::Context> {
    /// The address of the token being escrowed
    pub token_address: C::Address,
}

impl<'ws, 'c, C> TokenTransferValidationContext<EscrowExtraData<C>> for TransferContext<'ws, 'c, C>
where
    C: sov_modules_api::Context,
{
    type AccountId = Address<C>;

    fn get_port(&self) -> Result<PortId, TokenTransferError> {
        PortId::new(PORT_ID_STR.to_string()).map_err(TokenTransferError::InvalidIdentifier)
    }

    fn can_send_coins(&self) -> Result<(), TokenTransferError> {
        Ok(())
    }

    fn can_receive_coins(&self) -> Result<(), TokenTransferError> {
        Ok(())
    }

    fn mint_coins_validate(
        &self,
        _account: &Self::AccountId,
        _coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        // We can always mint
        Ok(())
    }

    /// Any token that is to be burned will have been previously minted, so we
    /// can expect to find the token address in our `minted_tokens` map.
    ///
    /// This is called in a `send_transfer()` in the case where we are NOT the
    /// token source
    fn burn_coins_validate(
        &self,
        account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        let token_address = {
            self.transfer_mod
                .minted_tokens
                .get(&coin.denom.to_string(), &mut self.working_set.borrow_mut())
                .ok_or(TokenTransferError::InvalidCoin {
                    coin: coin.to_string(),
                })?
        };

        let sender_balance: u64 = self
            .transfer_mod
            .bank
            .get_balance_of(
                account.address.clone(),
                token_address,
                &mut self.working_set.borrow_mut(),
            )
            .ok_or(TokenTransferError::InvalidCoin {
                coin: coin.denom.to_string(),
            })?;

        let sender_balance: transfer::Amount = sender_balance.into();

        if coin.amount > sender_balance {
            return Err(TokenTransferError::InsufficientFunds {
                send_attempt: sender_balance,
                available_funds: coin.amount,
            });
        }

        Ok(())
    }

    /// This is called in a `send_transfer()` in the case where we are the token source
    fn escrow_coins_validate(
        &self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
        from_account: &Self::AccountId,
        coin: &PrefixedCoin,
        extra: &EscrowExtraData<C>,
    ) -> Result<(), TokenTransferError> {
        let sender_balance: u64 = self
            .transfer_mod
            .bank
            .get_balance_of(
                from_account.address.clone(),
                extra.token_address.clone(),
                &mut self.working_set.borrow_mut(),
            )
            .ok_or(TokenTransferError::InvalidCoin {
                coin: coin.denom.to_string(),
            })?;

        let sender_balance: transfer::Amount = sender_balance.into();

        if coin.amount > sender_balance {
            return Err(TokenTransferError::InsufficientFunds {
                send_attempt: sender_balance,
                available_funds: coin.amount,
            });
        }

        Ok(())
    }

    /// This is called in a `recv_packet()` in the case where we are the token
    /// source.
    ///
    /// Note: ibc-rs strips the first prefix upon receival. That is, if token
    /// with denom `my_token` was previously sent on channel `channel-1` and
    /// port `transfer` (on the counterparty), it will be received in
    /// `recv_packet` as `transfer/channel-1/my_token`. However, ibc-rs strips
    /// `transfer/channel-1/` off the denom before passing it here, such that
    /// `coin.denom` would be `my_token`.
    ///
    /// This is especially important for us, as we use the denom to lookup the
    /// token address. Hence, we need to be careful not to use `my_token` in
    /// some instances and `transfer/channel-1/my_token` in others. Fortunately,
    /// ibc-rs solves that problem for us.
    fn unescrow_coins_validate(
        &self,
        port_id: &PortId,
        channel_id: &ChannelId,
        _to_account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        // ensure that escrow account has enough balance
        let escrow_balance: transfer::Amount = {
            let token_address = {
                self.transfer_mod
                    .escrowed_tokens
                    .get(&coin.denom.to_string(), &mut self.working_set.borrow_mut())
                    .ok_or(TokenTransferError::InvalidCoin {
                        coin: coin.to_string(),
                    })?
            };
            let escrow_address = self.get_escrow_account(port_id, channel_id);

            let escrow_balance = self
                .transfer_mod
                .bank
                .get_balance_of(
                    escrow_address,
                    token_address,
                    &mut self.working_set.borrow_mut(),
                )
                .ok_or(TokenTransferError::Other(format!(
                    "No escrow account for token {}",
                    coin.to_string()
                )))?;

            escrow_balance.into()
        };

        if coin.amount > escrow_balance {
            return Err(TokenTransferError::InsufficientFunds {
                send_attempt: coin.amount,
                available_funds: escrow_balance,
            });
        }

        Ok(())
    }
}

impl<'ws, 'c, C> TokenTransferExecutionContext<EscrowExtraData<C>> for TransferContext<'ws, 'c, C>
where
    C: sov_modules_api::Context,
{
    fn mint_coins_execute(
        &mut self,
        account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        let denom = coin.denom.to_string();

        // 1. if token address doesn't exist in `minted_tokens`, then create a new token and store in `minted_tokens`
        let token_address: C::Address = {
            let maybe_token_address = self
                .transfer_mod
                .minted_tokens
                .get(&denom, &mut self.working_set.borrow_mut());

            match maybe_token_address {
                Some(token_address) => token_address,
                // Create a new token
                None => {
                    let token_name = coin.denom.to_string();
                    let salt = 0u64; // FIXME: proper salt to use?
                    let initial_balance = 0;
                    // Note: unused since initial_balance = 0
                    let minter_address = account.address.clone();
                    // Only the transfer module is allowed to mint
                    let authorized_minters = vec![self.transfer_mod.address.clone()];
                    let new_token_addr = self
                        .transfer_mod
                        .bank
                        .create_token(
                            token_name,
                            salt,
                            initial_balance,
                            minter_address,
                            authorized_minters,
                            self.sdk_context,
                            &mut self.working_set.borrow_mut(),
                        )
                        .map_err(|err| TokenTransferError::Other(err.to_string()))?;

                    // Store the new address in `minted_tokens`
                    self.transfer_mod.minted_tokens.set(
                        &denom,
                        &new_token_addr,
                        &mut self.working_set.borrow_mut(),
                    );

                    new_token_addr
                }
            }
        };

        // 2. mint tokens
        {
            let amount: sov_bank::Amount = (*coin.amount.as_ref())
                .try_into()
                .map_err(|_| TokenTransferError::InvalidAmount(FromDecStrErr::InvalidLength))?;
            let sdk_coins = Coins {
                amount,
                token_address,
            };

            self.transfer_mod
                .bank
                .mint(
                    &sdk_coins,
                    &account.address,
                    &self.transfer_mod.address,
                    &mut self.working_set.borrow_mut(),
                )
                .map_err(|err| TokenTransferError::Other(err.to_string()))?;
        }

        Ok(())
    }

    fn burn_coins_execute(
        &mut self,
        account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        let token_address = {
            self.transfer_mod
                .minted_tokens
                .get(&coin.denom.to_string(), &mut self.working_set.borrow_mut())
                .ok_or(TokenTransferError::InvalidCoin {
                    coin: coin.to_string(),
                })?
        };

        let amount: sov_bank::Amount = (*coin.amount.as_ref())
            .try_into()
            .map_err(|_| TokenTransferError::InvalidAmount(FromDecStrErr::InvalidLength))?;
        let sdk_coins = Coins {
            amount,
            token_address,
        };

        self.transfer_mod
            .bank
            .burn(
                sdk_coins,
                &account.address,
                &mut self.working_set.borrow_mut(),
            )
            .map_err(|err| TokenTransferError::Other(err.to_string()))?;

        Ok(())
    }

    /// This is called in a `send_transfer()` in the case where we are the token source
    fn escrow_coins_execute(
        &self,
        port_id: &PortId,
        channel_id: &ChannelId,
        from_account: &Self::AccountId,
        coin: &PrefixedCoin,
        extra: &EscrowExtraData<C>,
    ) -> Result<(), TokenTransferError> {
        // 1. ensure that token exists in `self.escrowed_tokens` map, which is
        // necessary information when unescrowing tokens
        self.transfer_mod.escrowed_tokens.set(
            &coin.denom.to_string(),
            &extra.token_address,
            &mut self.working_set.borrow_mut(),
        );

        // 2. transfer coins to escrow account
        {
            let escrow_account = self.get_escrow_account(port_id, channel_id);

            self.transfer(
                extra.token_address.clone(),
                &from_account.address,
                &escrow_account,
                &coin.amount,
            )?;
        }

        Ok(())
    }

    /// This is called in a `recv_packet()` in the case where we are the token source.
    ///
    /// For more details, see note in `unescrow_coins_validate()`.
    fn unescrow_coins_execute(
        &self,
        port_id: &PortId,
        channel_id: &ChannelId,
        to_account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        let token_address = self
            .transfer_mod
            .escrowed_tokens
            .get(&coin.denom.to_string(), &mut self.working_set.borrow_mut())
            .ok_or(TokenTransferError::InvalidCoin {
                coin: coin.to_string(),
            })?;

        // transfer coins out of escrow account to `to_account`
        {
            let escrow_account = self.get_escrow_account(port_id, channel_id);

            self.transfer(
                token_address,
                &escrow_account,
                &to_account.address,
                &coin.amount,
            )?;
        }

        Ok(())
    }
}

/// Address type, which wraps C::Address. This is needed to implement
/// `TryFrom<Signer>` (circumventing the orphan rule).
pub struct Address<C: sov_modules_api::Context> {
    pub address: C::Address,
}

impl<C> TryFrom<Signer> for Address<C>
where
    C: sov_modules_api::Context,
{
    type Error = anyhow::Error;

    fn try_from(signer: Signer) -> Result<Self, Self::Error> {
        Ok(Address {
            address: signer.as_ref().parse()?,
        })
    }
}

impl<'ws, 'c, C> ibc::core::router::Module for TransferContext<'ws, 'c, C>
where
    C: sov_modules_api::Context,
{
    fn on_chan_open_init_validate(
        &self,
        order: Order,
        connection_hops: &[ConnectionId],
        port_id: &PortId,
        channel_id: &ChannelId,
        counterparty: &Counterparty,
        version: &ChannelVersion,
    ) -> Result<ChannelVersion, ChannelError> {
        on_chan_open_init_validate(
            self,
            order,
            connection_hops,
            port_id,
            channel_id,
            counterparty,
            version,
        )
        .map_err(|e: TokenTransferError| ChannelError::AppModule {
            description: e.to_string(),
        })?;

        Ok(ChannelVersion::new(VERSION.to_string()))
    }

    fn on_chan_open_init_execute(
        &mut self,
        order: Order,
        connection_hops: &[ConnectionId],
        port_id: &PortId,
        channel_id: &ChannelId,
        counterparty: &Counterparty,
        version: &ChannelVersion,
    ) -> Result<(ModuleExtras, ChannelVersion), ChannelError> {
        on_chan_open_init_execute(
            self,
            order,
            connection_hops,
            port_id,
            channel_id,
            counterparty,
            version,
        )
        .map_err(|e: TokenTransferError| ChannelError::AppModule {
            description: e.to_string(),
        })
    }

    fn on_chan_open_try_validate(
        &self,
        order: Order,
        connection_hops: &[ConnectionId],
        port_id: &PortId,
        channel_id: &ChannelId,
        counterparty: &Counterparty,
        counterparty_version: &ChannelVersion,
    ) -> Result<ChannelVersion, ChannelError> {
        on_chan_open_try_validate(
            self,
            order,
            connection_hops,
            port_id,
            channel_id,
            counterparty,
            counterparty_version,
        )
        .map_err(|e: TokenTransferError| ChannelError::AppModule {
            description: e.to_string(),
        })?;
        Ok(ChannelVersion::new(VERSION.to_string()))
    }

    fn on_chan_open_try_execute(
        &mut self,
        order: Order,
        connection_hops: &[ConnectionId],
        port_id: &PortId,
        channel_id: &ChannelId,
        counterparty: &Counterparty,
        counterparty_version: &ChannelVersion,
    ) -> Result<(ModuleExtras, ChannelVersion), ChannelError> {
        on_chan_open_try_execute(
            self,
            order,
            connection_hops,
            port_id,
            channel_id,
            counterparty,
            counterparty_version,
        )
        .map_err(|e: TokenTransferError| ChannelError::AppModule {
            description: e.to_string(),
        })
    }

    fn on_chan_open_ack_validate(
        &self,
        port_id: &PortId,
        channel_id: &ChannelId,
        counterparty_version: &ChannelVersion,
    ) -> Result<(), ChannelError> {
        on_chan_open_ack_validate(self, port_id, channel_id, counterparty_version).map_err(
            |e: TokenTransferError| ChannelError::AppModule {
                description: e.to_string(),
            },
        )
    }

    fn on_chan_open_ack_execute(
        &mut self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
        _counterparty_version: &ChannelVersion,
    ) -> Result<ModuleExtras, ChannelError> {
        Ok(ModuleExtras::empty())
    }

    fn on_chan_open_confirm_validate(
        &self,
        port_id: &PortId,
        channel_id: &ChannelId,
    ) -> Result<(), ChannelError> {
        on_chan_open_confirm_validate(self, port_id, channel_id).map_err(|e: TokenTransferError| {
            ChannelError::AppModule {
                description: e.to_string(),
            }
        })
    }

    fn on_chan_open_confirm_execute(
        &mut self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
    ) -> Result<ModuleExtras, ChannelError> {
        Ok(ModuleExtras::empty())
    }

    fn on_chan_close_init_validate(
        &self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    fn on_chan_close_init_execute(
        &mut self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
    ) -> Result<ModuleExtras, ChannelError> {
        Ok(ModuleExtras::empty())
    }

    fn on_chan_close_confirm_validate(
        &self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    fn on_chan_close_confirm_execute(
        &mut self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
    ) -> Result<ModuleExtras, ChannelError> {
        Ok(ModuleExtras::empty())
    }

    fn on_recv_packet_execute(
        &mut self,
        packet: &Packet,
        _relayer: &Signer,
    ) -> (ModuleExtras, Acknowledgement) {
        on_recv_packet_execute(self, packet)
    }

    fn on_acknowledgement_packet_validate(
        &self,
        packet: &Packet,
        acknowledgement: &Acknowledgement,
        relayer: &Signer,
    ) -> Result<(), PacketError> {
        on_acknowledgement_packet_validate(self, packet, acknowledgement, relayer).map_err(
            |e: TokenTransferError| PacketError::AppModule {
                description: e.to_string(),
            },
        )
    }

    fn on_acknowledgement_packet_execute(
        &mut self,
        _packet: &Packet,
        _acknowledgement: &Acknowledgement,
        _relayer: &Signer,
    ) -> (ModuleExtras, Result<(), PacketError>) {
        (ModuleExtras::empty(), Ok(()))
    }

    /// Note: `MsgTimeout` and `MsgTimeoutOnClose` use the same callback
    fn on_timeout_packet_validate(
        &self,
        packet: &Packet,
        relayer: &Signer,
    ) -> Result<(), PacketError> {
        on_timeout_packet_validate(self, packet, relayer).map_err(|e: TokenTransferError| {
            PacketError::AppModule {
                description: e.to_string(),
            }
        })
    }

    /// Note: `MsgTimeout` and `MsgTimeoutOnClose` use the same callback
    fn on_timeout_packet_execute(
        &mut self,
        packet: &Packet,
        relayer: &Signer,
    ) -> (ModuleExtras, Result<(), PacketError>) {
        let res = on_timeout_packet_execute(self, packet, relayer);
        (
            res.0,
            res.1
                .map_err(|e: TokenTransferError| PacketError::AppModule {
                    description: e.to_string(),
                }),
        )
    }
}
