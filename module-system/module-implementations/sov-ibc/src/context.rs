
mod clients;

use ibc::core::ics24_host::identifier::ClientId;
use ibc::core::router::{Router, ModuleId};
use ibc::core::{ValidationContext, ExecutionContext};
use sov_state::WorkingSet;

use crate::IbcModule;

pub struct IbcExecutionContext<'a, C: sov_modules_api::Context> {
    pub ibc: &'a IbcModule<C>,
    pub working_set: &'a WorkingSet<C::Storage>,
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
    ) -> Result<Self::AnyClientState, ibc::core::ContextError> {
        todo!()
    }

    fn decode_client_state(
        &self,
        client_state: ibc::Any,
    ) -> Result<Self::AnyClientState, ibc::core::ContextError> {
        todo!()
    }

    fn consensus_state(
        &self,
        client_cons_state_path: &ibc::core::ics24_host::path::ClientConsensusStatePath,
    ) -> Result<Self::AnyConsensusState, ibc::core::ContextError> {
        todo!()
    }

    fn client_update_time(
        &self,
        client_id: &ClientId,
        height: &ibc::Height,
    ) -> Result<ibc::core::timestamp::Timestamp, ibc::core::ContextError> {
        todo!()
    }

    fn client_update_height(
        &self,
        client_id: &ClientId,
        height: &ibc::Height,
    ) -> Result<ibc::Height, ibc::core::ContextError> {
        todo!()
    }

    fn host_height(&self) -> Result<ibc::Height, ibc::core::ContextError> {
        todo!()
    }

    fn host_timestamp(&self) -> Result<ibc::core::timestamp::Timestamp, ibc::core::ContextError> {
        todo!()
    }

    fn host_consensus_state(
        &self,
        height: &ibc::Height,
    ) -> Result<Self::AnyConsensusState, ibc::core::ContextError> {
        todo!()
    }

    fn client_counter(&self) -> Result<u64, ibc::core::ContextError> {
        todo!()
    }

    fn connection_end(
        &self,
        conn_id: &ibc::core::ics24_host::identifier::ConnectionId,
    ) -> Result<ibc::core::ics03_connection::connection::ConnectionEnd, ibc::core::ContextError>
    {
        todo!()
    }

    fn validate_self_client(
        &self,
        client_state_of_host_on_counterparty: ibc::Any,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn commitment_prefix(&self) -> ibc::core::ics23_commitment::commitment::CommitmentPrefix {
        todo!()
    }

    fn connection_counter(&self) -> Result<u64, ibc::core::ContextError> {
        todo!()
    }

    fn channel_end(
        &self,
        channel_end_path: &ibc::core::ics24_host::path::ChannelEndPath,
    ) -> Result<ibc::core::ics04_channel::channel::ChannelEnd, ibc::core::ContextError> {
        todo!()
    }

    fn get_next_sequence_send(
        &self,
        seq_send_path: &ibc::core::ics24_host::path::SeqSendPath,
    ) -> Result<ibc::core::ics04_channel::packet::Sequence, ibc::core::ContextError> {
        todo!()
    }

    fn get_next_sequence_recv(
        &self,
        seq_recv_path: &ibc::core::ics24_host::path::SeqRecvPath,
    ) -> Result<ibc::core::ics04_channel::packet::Sequence, ibc::core::ContextError> {
        todo!()
    }

    fn get_next_sequence_ack(
        &self,
        seq_ack_path: &ibc::core::ics24_host::path::SeqAckPath,
    ) -> Result<ibc::core::ics04_channel::packet::Sequence, ibc::core::ContextError> {
        todo!()
    }

    fn get_packet_commitment(
        &self,
        commitment_path: &ibc::core::ics24_host::path::CommitmentPath,
    ) -> Result<ibc::core::ics04_channel::commitment::PacketCommitment, ibc::core::ContextError>
    {
        todo!()
    }

    fn get_packet_receipt(
        &self,
        receipt_path: &ibc::core::ics24_host::path::ReceiptPath,
    ) -> Result<ibc::core::ics04_channel::packet::Receipt, ibc::core::ContextError> {
        todo!()
    }

    fn get_packet_acknowledgement(
        &self,
        ack_path: &ibc::core::ics24_host::path::AckPath,
    ) -> Result<
        ibc::core::ics04_channel::commitment::AcknowledgementCommitment,
        ibc::core::ContextError,
    > {
        todo!()
    }

    fn channel_counter(&self) -> Result<u64, ibc::core::ContextError> {
        todo!()
    }

    fn max_expected_time_per_block(&self) -> std::time::Duration {
        todo!()
    }

    fn validate_message_signer(&self, signer: &ibc::Signer) -> Result<(), ibc::core::ContextError> {
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
        height: ibc::Height,
        timestamp: ibc::core::timestamp::Timestamp,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_update_height(
        &mut self,
        client_id: ClientId,
        height: ibc::Height,
        host_height: ibc::Height,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_connection(
        &mut self,
        connection_path: &ibc::core::ics24_host::path::ConnectionPath,
        connection_end: ibc::core::ics03_connection::connection::ConnectionEnd,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_connection_to_client(
        &mut self,
        client_connection_path: &ibc::core::ics24_host::path::ClientConnectionPath,
        conn_id: ibc::core::ics24_host::identifier::ConnectionId,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn increase_connection_counter(&mut self) {
        todo!()
    }

    fn store_packet_commitment(
        &mut self,
        commitment_path: &ibc::core::ics24_host::path::CommitmentPath,
        commitment: ibc::core::ics04_channel::commitment::PacketCommitment,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn delete_packet_commitment(
        &mut self,
        commitment_path: &ibc::core::ics24_host::path::CommitmentPath,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_packet_receipt(
        &mut self,
        receipt_path: &ibc::core::ics24_host::path::ReceiptPath,
        receipt: ibc::core::ics04_channel::packet::Receipt,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_packet_acknowledgement(
        &mut self,
        ack_path: &ibc::core::ics24_host::path::AckPath,
        ack_commitment: ibc::core::ics04_channel::commitment::AcknowledgementCommitment,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn delete_packet_acknowledgement(&mut self, ack_path: &ibc::core::ics24_host::path::AckPath) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_channel(
        &mut self,
        channel_end_path: &ibc::core::ics24_host::path::ChannelEndPath,
        channel_end: ibc::core::ics04_channel::channel::ChannelEnd,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_next_sequence_send(
        &mut self,
        seq_send_path: &ibc::core::ics24_host::path::SeqSendPath,
        seq: ibc::core::ics04_channel::packet::Sequence,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_next_sequence_recv(
        &mut self,
        seq_recv_path: &ibc::core::ics24_host::path::SeqRecvPath,
        seq: ibc::core::ics04_channel::packet::Sequence,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_next_sequence_ack(
        &mut self,
        seq_ack_path: &ibc::core::ics24_host::path::SeqAckPath,
        seq: ibc::core::ics04_channel::packet::Sequence,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn increase_channel_counter(&mut self) {
        todo!()
    }

    fn emit_ibc_event(&mut self, event: ibc::core::events::IbcEvent) {
        todo!()
    }

    fn log_message(&mut self, message: String) {
        todo!()
    }
}
