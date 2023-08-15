mod call;
mod context;
mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

pub use call::CallMessage;
use ibc::applications::transfer::context::{
    on_acknowledgement_packet_validate, on_chan_open_ack_validate, on_chan_open_confirm_validate,
    on_chan_open_init_execute, on_chan_open_init_validate, on_chan_open_try_execute,
    on_chan_open_try_validate, on_recv_packet_execute, on_timeout_packet_execute,
    on_timeout_packet_validate,
};
use ibc::applications::transfer::error::TokenTransferError;
use ibc::applications::transfer::VERSION;
use ibc::core::ics04_channel::acknowledgement::Acknowledgement;
use ibc::core::ics04_channel::channel::{Counterparty, Order};
use ibc::core::ics04_channel::error::{ChannelError, PacketError};
use ibc::core::ics04_channel::packet::Packet;
use ibc::core::ics04_channel::Version as ChannelVersion;
use ibc::core::ics24_host::identifier::{ChannelId, ConnectionId, PortId};
use ibc::core::router::ModuleExtras;
use ibc::Signer;
#[cfg(feature = "native")]
pub use query::Response;
use sov_modules_api::{Error, ModuleInfo};
use sov_state::WorkingSet;

pub struct ExampleModuleConfig {}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Should derive `ModuleCallJsonSchema` if the "native" feature is enabled.
///   This is optional, and is only used to generate a JSON Schema for your
///   module's call messages (which is useful to develop clients, CLI tooling
///   etc.).
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo)]
pub struct Transfer<C: sov_modules_api::Context> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// Reference to the Bank module.
    #[module]
    pub(crate) _bank: sov_bank::Bank<C>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Transfer<C> {
    type Context = C;

    type Config = ExampleModuleConfig;

    type CallMessage = call::CallMessage;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        todo!()
    }
}

impl<C> core::fmt::Debug for Transfer<C>
where
    C: sov_modules_api::Context,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // FIXME: put real values here, or remove `Debug` requirement from router::Module
        f.debug_struct("Transfer")
            .field("address", &self.address)
            .finish()
    }
}

impl<C> ibc::core::router::Module for Transfer<C>
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
    ) -> Result<ibc::core::ics04_channel::Version, ChannelError> {
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
