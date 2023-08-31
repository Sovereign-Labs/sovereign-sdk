pub(crate) mod clients;

use std::cell::RefCell;
use std::rc::Rc;

use ibc::clients::ics07_tendermint::client_state::ClientState as TmClientState;
use ibc::core::events::IbcEvent;
use ibc::core::ics02_client::error::ClientError;
use ibc::core::ics03_connection::connection::ConnectionEnd;
use ibc::core::ics04_channel::channel::ChannelEnd;
use ibc::core::ics04_channel::commitment::{AcknowledgementCommitment, PacketCommitment};
use ibc::core::ics04_channel::packet::{Receipt, Sequence};
use ibc::core::ics23_commitment::commitment::CommitmentPrefix;
use ibc::core::ics24_host::identifier::{ClientId, ConnectionId};
use ibc::core::ics24_host::path::{
    AckPath, ChannelEndPath, ClientConnectionPath, ClientConsensusStatePath, CommitmentPath,
    ConnectionPath, ReceiptPath, SeqAckPath, SeqRecvPath, SeqSendPath,
};
use ibc::core::timestamp::Timestamp;
use ibc::core::{ContextError, ExecutionContext, ValidationContext};
use ibc::Height;
use sov_state::WorkingSet;

use crate::IbcModule;

pub struct IbcExecutionContext<'a, C: sov_modules_api::Context> {
    pub ibc: &'a IbcModule<C>,
    pub working_set: Rc<RefCell<&'a mut WorkingSet<C::Storage>>>,
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
        self
    }

    fn client_state(&self, client_id: &ClientId) -> Result<Self::AnyClientState, ContextError> {
        self.ibc
            .client_state_store
            .get(client_id, *self.working_set.borrow_mut())
            .ok_or(
                ClientError::ClientStateNotFound {
                    client_id: client_id.clone(),
                }
                .into(),
            )
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
        self.ibc
            .consensus_state_store
            .get(client_cons_state_path, *self.working_set.borrow_mut())
            .ok_or(
                ClientError::ConsensusStateNotFound {
                    client_id: client_cons_state_path.client_id.clone(),
                    height: Height::new(
                        client_cons_state_path.epoch,
                        client_cons_state_path.height,
                    )
                    .map_err(|_| ClientError::InvalidHeight)?,
                }
                .into(),
            )
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
        // TODO: In order to implement this, we need to first define the
        // `ConsensusState` protobuf definition that SDK chains will use
        todo!()
    }

    fn client_counter(&self) -> Result<u64, ContextError> {
        todo!()
    }

    fn connection_end(&self, conn_id: &ConnectionId) -> Result<ConnectionEnd, ContextError> {
        todo!()
    }

    fn validate_self_client(
        &self,
        client_state_of_host_on_counterparty: ibc::Any,
    ) -> Result<(), ContextError> {
        // Note: We can optionally implement this.
        // It would require having a Protobuf definition of the chain's `ClientState` that other chains would use.
        // The relayer sends us this `ClientState` as stored on other chains, and we validate it here.
        Ok(())
    }

    fn commitment_prefix(&self) -> CommitmentPrefix {
        todo!()
    }

    fn connection_counter(&self) -> Result<u64, ContextError> {
        todo!()
    }

    fn channel_end(&self, channel_end_path: &ChannelEndPath) -> Result<ChannelEnd, ContextError> {
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

    fn get_next_sequence_ack(&self, seq_ack_path: &SeqAckPath) -> Result<Sequence, ContextError> {
        todo!()
    }

    fn get_packet_commitment(
        &self,
        commitment_path: &CommitmentPath,
    ) -> Result<PacketCommitment, ContextError> {
        todo!()
    }

    fn get_packet_receipt(&self, receipt_path: &ReceiptPath) -> Result<Receipt, ContextError> {
        todo!()
    }

    fn get_packet_acknowledgement(
        &self,
        ack_path: &AckPath,
    ) -> Result<AcknowledgementCommitment, ContextError> {
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
        self
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
