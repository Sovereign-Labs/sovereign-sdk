
mod clients;

use ibc::Height;
use ibc::clients::ics07_tendermint::client_state::ClientState as TmClientState;
use ibc::core::events::IbcEvent;
use ibc::core::ics03_connection::connection::ConnectionEnd;
use ibc::core::ics04_channel::channel::ChannelEnd;
use ibc::core::ics04_channel::commitment::{PacketCommitment, AcknowledgementCommitment};
use ibc::core::ics04_channel::packet::{Sequence, Receipt};
use ibc::core::ics23_commitment::commitment::CommitmentPrefix;
use ibc::core::ics24_host::identifier::{ClientId, ConnectionId};
use ibc::core::ics24_host::path::{ClientConsensusStatePath, ChannelEndPath, SeqSendPath, SeqAckPath, SeqRecvPath, CommitmentPath, ReceiptPath, AckPath, ConnectionPath, ClientConnectionPath};
use ibc::core::router::{Router, ModuleId};
use ibc::core::timestamp::Timestamp;
use ibc::core::{ValidationContext, ExecutionContext, ContextError};
use sov_state::WorkingSet;

use crate::IbcModule;

pub struct IbcExecutionContext<'a, C: sov_modules_api::Context> {
    pub ibc: &'a IbcModule<C>,
    pub working_set: &'a mut WorkingSet<C::Storage>,
}

impl<'a, C> Router for IbcExecutionContext<'a, C>
where
    C: sov_modules_api::Context,
{
    fn get_route(
        &self,
        module_id: &ModuleId,
    ) -> Option<&dyn ibc::core::router::Module> {
        todo!()
    }

    fn get_route_mut(
        &mut self,
        module_id: &ModuleId,
    ) -> Option<&mut dyn ibc::core::router::Module> {
        todo!()
    }

    fn lookup_module_by_port(
        &self,
        port_id: &ibc::core::ics24_host::identifier::PortId,
    ) -> Option<ModuleId> {
        todo!()
    }
}

impl<'a, C> ValidationContext for IbcExecutionContext<'a, C>
where
    C: sov_modules_api::Context,
{
    type ClientValidationContext = Self;
    type E = Self;
    type AnyConsensusState = clients::AnyConsensusState;
    type AnyClientState = clients::AnyClientState;

    fn get_client_validation_context(&self) -> &Self::ClientValidationContext {
        todo!()
    }

    fn client_state(
        &self,
        client_id: &ClientId,
    ) -> Result<Self::AnyClientState, ContextError> {
        todo!()
    }

    fn decode_client_state(
        &self,
        client_state: ibc::Any,
    ) -> Result<Self::AnyClientState, ContextError> {
        let tm_client_state: TmClientState = client_state.try_into()?;

        Ok(tm_client_state.into())
    }

    fn consensus_state(
        &self,
        client_cons_state_path: &ClientConsensusStatePath,
    ) -> Result<Self::AnyConsensusState, ContextError> {
        todo!()
    }

    fn client_update_time(
        &self,
        client_id: &ClientId,
        height: &Height,
    ) -> Result<Timestamp, ContextError> {
        todo!()
    }

    fn client_update_height(
        &self,
        client_id: &ClientId,
        height: &Height,
    ) -> Result<Height, ContextError> {
        todo!()
    }

    fn host_height(&self) -> Result<Height, ContextError> {
        todo!()
    }

    fn host_timestamp(&self) -> Result<Timestamp, ContextError> {
        todo!()
    }

    fn host_consensus_state(
        &self,
        height: &Height,
    ) -> Result<Self::AnyConsensusState, ContextError> {
        todo!()
    }

    fn client_counter(&self) -> Result<u64, ContextError> {
        todo!()
    }

    fn connection_end(
        &self,
        conn_id: &ConnectionId,
    ) -> Result<ConnectionEnd, ContextError>
    {
        todo!()
    }

    fn validate_self_client(
        &self,
        client_state_of_host_on_counterparty: ibc::Any,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn commitment_prefix(&self) -> CommitmentPrefix {
        todo!()
    }

    fn connection_counter(&self) -> Result<u64, ContextError> {
        todo!()
    }

    fn channel_end(
        &self,
        channel_end_path: &ChannelEndPath,
    ) -> Result<ChannelEnd, ContextError> {
        todo!()
    }

    fn get_next_sequence_send(
        &self,
        seq_send_path: &SeqSendPath,
    ) -> Result<Sequence, ContextError> {
        todo!()
    }

    fn get_next_sequence_recv(
        &self,
        seq_recv_path: &SeqRecvPath,
    ) -> Result<Sequence, ContextError> {
        todo!()
    }

    fn get_next_sequence_ack(
        &self,
        seq_ack_path: &SeqAckPath,
    ) -> Result<Sequence, ContextError> {
        todo!()
    }

    fn get_packet_commitment(
        &self,
        commitment_path: &CommitmentPath,
    ) -> Result<PacketCommitment, ContextError>
    {
        todo!()
    }

    fn get_packet_receipt(
        &self,
        receipt_path: &ReceiptPath,
    ) -> Result<Receipt, ContextError> {
        todo!()
    }

    fn get_packet_acknowledgement(
        &self,
        ack_path: &AckPath,
    ) -> Result<
        AcknowledgementCommitment,
        ContextError,
    > {
        todo!()
    }

    fn channel_counter(&self) -> Result<u64, ContextError> {
        todo!()
    }

    fn max_expected_time_per_block(&self) -> core::time::Duration {
        todo!()
    }

    fn validate_message_signer(&self, signer: &ibc::Signer) -> Result<(), ContextError> {
        todo!()
    }
}

impl<'a, C> ExecutionContext for IbcExecutionContext<'a, C>
where
    C: sov_modules_api::Context,
{
    fn get_client_execution_context(&mut self) -> &mut Self::E {
        todo!()
    }

    fn increase_client_counter(&mut self) {
        todo!()
    }

    fn store_update_time(
        &mut self,
        client_id: ClientId,
        height: Height,
        timestamp: Timestamp,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_update_height(
        &mut self,
        client_id: ClientId,
        height: Height,
        host_height: Height,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_connection(
        &mut self,
        connection_path: &ConnectionPath,
        connection_end: ConnectionEnd,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_connection_to_client(
        &mut self,
        client_connection_path: &ClientConnectionPath,
        conn_id: ConnectionId,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn increase_connection_counter(&mut self) {
        todo!()
    }

    fn store_packet_commitment(
        &mut self,
        commitment_path: &CommitmentPath,
        commitment: PacketCommitment,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn delete_packet_commitment(
        &mut self,
        commitment_path: &CommitmentPath,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_packet_receipt(
        &mut self,
        receipt_path: &ReceiptPath,
        receipt: Receipt,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_packet_acknowledgement(
        &mut self,
        ack_path: &AckPath,
        ack_commitment: AcknowledgementCommitment,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn delete_packet_acknowledgement(&mut self, ack_path: &AckPath) -> Result<(), ContextError> {
        todo!()
    }

    fn store_channel(
        &mut self,
        channel_end_path: &ChannelEndPath,
        channel_end: ChannelEnd,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_next_sequence_send(
        &mut self,
        seq_send_path: &SeqSendPath,
        seq: Sequence,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_next_sequence_recv(
        &mut self,
        seq_recv_path: &SeqRecvPath,
        seq: Sequence,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn store_next_sequence_ack(
        &mut self,
        seq_ack_path: &SeqAckPath,
        seq: Sequence,
    ) -> Result<(), ContextError> {
        todo!()
    }

    fn increase_channel_counter(&mut self) {
        todo!()
    }

    fn emit_ibc_event(&mut self, event: IbcEvent) {
        todo!()
    }

    fn log_message(&mut self, message: String) {
        todo!()
    }
}
