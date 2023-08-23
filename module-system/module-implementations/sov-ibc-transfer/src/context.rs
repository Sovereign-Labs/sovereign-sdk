use std::cell::RefCell;

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
pub struct TransferContext<'ws, C: sov_modules_api::Context> {
    pub transfer_mod: Transfer<C>,
    pub working_set: RefCell<&'ws mut WorkingSet<C::Storage>>,
}

impl<'ws, C> TransferContext<'ws, C>
where
    C: sov_modules_api::Context,
{
    pub fn new(transfer_mod: Transfer<C>, working_set: &'ws mut WorkingSet<C::Storage>) -> Self {
        Self {
            transfer_mod,
            working_set: RefCell::new(working_set),
        }
    }

    fn get_escrow_account(&self, _port_id: &PortId, _channel_id: &ChannelId) -> C::Address {
        // Q: What is the escrow account?
        todo!()
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
                &from_account,
                &to_account,
                coin,
                &mut self.working_set.borrow_mut(),
            )
            .map_err(|err| TokenTransferError::InternalTransferFailed(err.to_string()))?;

        Ok(())
    }
}

impl<'ws, C> core::fmt::Debug for TransferContext<'ws, C>
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

impl<'ws, C> TokenTransferValidationContext<EscrowExtraData<C>> for TransferContext<'ws, C>
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
        todo!()
    }

    fn burn_coins_validate(
        &self,
        _account: &Self::AccountId,
        _coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

    /// Check if the sender has enough balance
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
                let mut hasher = <C::Hasher as Digest>::new();
                hasher.update(coin.denom.to_string());
                let denom_hash = hasher.finalize().to_vec();

                self.transfer_mod
                    .escrowed_tokens
                    .get(&denom_hash, &mut self.working_set.borrow_mut())
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

impl<'ws, C> TokenTransferExecutionContext<EscrowExtraData<C>> for TransferContext<'ws, C>
where
    C: sov_modules_api::Context,
{
    fn mint_coins_execute(
        &mut self,
        _account: &Self::AccountId,
        _coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

    fn burn_coins_execute(
        &mut self,
        _account: &Self::AccountId,
        _coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

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
        {
            let mut hasher = <C::Hasher as Digest>::new();
            hasher.update(coin.denom.to_string());
            let denom_hash = hasher.finalize().to_vec();

            self.transfer_mod.escrowed_tokens.set(
                &denom_hash,
                &extra.token_address,
                &mut self.working_set.borrow_mut(),
            );
        }

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

    fn unescrow_coins_execute(
        &self,
        port_id: &PortId,
        channel_id: &ChannelId,
        to_account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        let token_address = {
            let mut hasher = <C::Hasher as Digest>::new();
            hasher.update(coin.denom.to_string());
            let denom_hash = hasher.finalize().to_vec();

            self.transfer_mod
                .escrowed_tokens
                .get(&denom_hash, &mut self.working_set.borrow_mut())
                .ok_or(TokenTransferError::InvalidCoin {
                    coin: coin.to_string(),
                })?
        };

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

impl<'ws, C> ibc::core::router::Module for TransferContext<'ws, C>
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
