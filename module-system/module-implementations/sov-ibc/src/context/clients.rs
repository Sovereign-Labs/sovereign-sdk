use ibc::clients::ics07_tendermint::client_state::ClientState as TmClientState;
use ibc::clients::ics07_tendermint::consensus_state::ConsensusState as TmConsensusState;
use ibc::core::ics02_client::client_state::{ClientStateValidation, ClientStateExecution, ClientStateCommon};
use ibc::core::ics02_client::consensus_state::ConsensusState;
use ibc::core::ics02_client::ClientExecutionContext;
use ibc::core::ValidationContext;

use super::IbcExecutionContext;

// Q: How do we enable users to set the light clients they want?
pub enum AnyConsensusState {
    Tendermint(TmConsensusState)
}

impl ConsensusState for AnyConsensusState {
    fn root(&self) -> &ibc::core::ics23_commitment::commitment::CommitmentRoot {
        todo!()
    }

    fn timestamp(&self) -> ibc::core::timestamp::Timestamp {
        todo!()
    }

    fn encode_vec(&self) -> Result<Vec<u8>, tendermint_proto::Error> {
        todo!()
    }
}

// Q: How do we enable users to set the light clients they want?
pub enum AnyClientState {
    Tendermint(TmClientState)
}

impl ClientStateCommon for AnyClientState {
    fn verify_consensus_state(&self, consensus_state: ibc::Any) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn client_type(&self) -> ibc::core::ics02_client::client_type::ClientType {
        todo!()
    }

    fn latest_height(&self) -> ibc::Height {
        todo!()
    }

    fn validate_proof_height(&self, proof_height: ibc::Height) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn confirm_not_frozen(&self) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn expired(&self, elapsed: std::time::Duration) -> bool {
        todo!()
    }

    fn verify_upgrade_client(
        &self,
        upgraded_client_state: ibc::Any,
        upgraded_consensus_state: ibc::Any,
        proof_upgrade_client: ibc::core::ics23_commitment::commitment::CommitmentProofBytes,
        proof_upgrade_consensus_state: ibc::core::ics23_commitment::commitment::CommitmentProofBytes,
        root: &ibc::core::ics23_commitment::commitment::CommitmentRoot,
    ) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn verify_membership(
        &self,
        prefix: &ibc::core::ics23_commitment::commitment::CommitmentPrefix,
        proof: &ibc::core::ics23_commitment::commitment::CommitmentProofBytes,
        root: &ibc::core::ics23_commitment::commitment::CommitmentRoot,
        path: ibc::core::ics24_host::path::Path,
        value: Vec<u8>,
    ) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn verify_non_membership(
        &self,
        prefix: &ibc::core::ics23_commitment::commitment::CommitmentPrefix,
        proof: &ibc::core::ics23_commitment::commitment::CommitmentProofBytes,
        root: &ibc::core::ics23_commitment::commitment::CommitmentRoot,
        path: ibc::core::ics24_host::path::Path,
    ) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }
}

impl<'a, C> ClientStateExecution<IbcExecutionContext<'a, C>> for AnyClientState where
    C: sov_modules_api::Context
{
    fn initialise(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ibc::core::ics24_host::identifier::ClientId,
        consensus_state: ibc::Any,
    ) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn update_state(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ibc::core::ics24_host::identifier::ClientId,
        header: ibc::Any,
    ) -> Result<Vec<ibc::Height>, ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn update_state_on_misbehaviour(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ibc::core::ics24_host::identifier::ClientId,
        client_message: ibc::Any,
        update_kind: &ibc::core::ics02_client::client_state::UpdateKind,
    ) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn update_state_on_upgrade(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ibc::core::ics24_host::identifier::ClientId,
        upgraded_client_state: ibc::Any,
        upgraded_consensus_state: ibc::Any,
    ) -> Result<ibc::Height, ibc::core::ics02_client::error::ClientError> {
        todo!()
    }
}
impl<'a, C> ClientStateValidation<IbcExecutionContext<'a, C>> for AnyClientState where
    C: sov_modules_api::Context
{
    fn verify_client_message(
        &self,
        ctx: &IbcExecutionContext<'a, C>,
        client_id: &ibc::core::ics24_host::identifier::ClientId,
        client_message: ibc::Any,
        update_kind: &ibc::core::ics02_client::client_state::UpdateKind,
    ) -> Result<(), ibc::core::ics02_client::error::ClientError> {
        todo!()
    }

    fn check_for_misbehaviour(
        &self,
        ctx: &IbcExecutionContext<'a, C>,
        client_id: &ibc::core::ics24_host::identifier::ClientId,
        client_message: ibc::Any,
        update_kind: &ibc::core::ics02_client::client_state::UpdateKind,
    ) -> Result<bool, ibc::core::ics02_client::error::ClientError> {
        todo!()
    }
}

impl<'a, C> ClientExecutionContext for IbcExecutionContext<'a, C>
where
    C: sov_modules_api::Context,
{
    type ClientValidationContext = <Self as ValidationContext>::ClientValidationContext;
    type AnyClientState = <Self as ValidationContext>::AnyClientState;
    type AnyConsensusState = <Self as ValidationContext>::AnyConsensusState;

    fn store_client_state(
        &mut self,
        client_state_path: ibc::core::ics24_host::path::ClientStatePath,
        client_state: Self::AnyClientState,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }

    fn store_consensus_state(
        &mut self,
        consensus_state_path: ibc::core::ics24_host::path::ClientConsensusStatePath,
        consensus_state: Self::AnyConsensusState,
    ) -> Result<(), ibc::core::ContextError> {
        todo!()
    }
}
