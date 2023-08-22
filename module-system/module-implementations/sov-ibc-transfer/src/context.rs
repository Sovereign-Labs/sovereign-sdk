use std::cell::RefCell;

use ibc::applications::transfer::context::{
    on_acknowledgement_packet_validate, on_chan_open_ack_validate, on_chan_open_confirm_validate,
    on_chan_open_init_execute, on_chan_open_init_validate, on_chan_open_try_execute,
    on_chan_open_try_validate, on_recv_packet_execute, on_timeout_packet_execute,
    on_timeout_packet_validate, TokenTransferExecutionContext, TokenTransferValidationContext,
};
use ibc::applications::transfer::error::TokenTransferError;
use ibc::applications::transfer::{Amount, PrefixedCoin, PORT_ID_STR, VERSION};
use ibc::core::ics04_channel::acknowledgement::Acknowledgement;
use ibc::core::ics04_channel::channel::{Counterparty, Order};
use ibc::core::ics04_channel::error::{ChannelError, PacketError};
use ibc::core::ics04_channel::packet::Packet;
use ibc::core::ics04_channel::Version as ChannelVersion;
use ibc::core::ics24_host::identifier::{ChannelId, ConnectionId, PortId};
use ibc::core::router::ModuleExtras;
use ibc::Signer;
use sov_state::WorkingSet;

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
            transfer_mod: transfer_mod,
            working_set: RefCell::new(working_set),
        }
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
    pub token_addr: C::Address,
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
                extra.token_addr.clone(),
                &mut self.working_set.borrow_mut(),
            )
            .ok_or(TokenTransferError::InvalidCoin {
                coin: coin.denom.to_string(),
            })?;

        let sender_balance: Amount = sender_balance.into();

        if coin.amount > sender_balance {
            return Err(TokenTransferError::InsufficientFunds {
                send_attempt: sender_balance,
                available_funds: coin.amount.clone(),
            });
        }

        Ok(())
    }

    fn unescrow_coins_validate(
        &self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
        _to_account: &Self::AccountId,
        _coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
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
        _port_id: &PortId,
        _channel_id: &ChannelId,
        _from_account: &Self::AccountId,
        _coin: &PrefixedCoin,
        _extra: &EscrowExtraData<C>,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

    fn unescrow_coins_execute(
        &self,
        _port_id: &PortId,
        _channel_id: &ChannelId,
        _to_account: &Self::AccountId,
        _coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
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
