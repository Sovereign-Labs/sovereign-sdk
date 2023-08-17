use derive_more::{From, TryInto};
use ibc::clients::ics07_tendermint::client_state::ClientState as TmClientState;
use ibc::clients::ics07_tendermint::consensus_state::ConsensusState as TmConsensusState;
use ibc::clients::ics07_tendermint::{
    CommonContext as TmCommonContext, ValidationContext as TmValidationContext,
};
use ibc::core::ics02_client::client_state::{
    ClientStateCommon, ClientStateExecution, ClientStateValidation, Status, UpdateKind,
};
use ibc::core::ics02_client::client_type::ClientType;
use ibc::core::ics02_client::consensus_state::ConsensusState;
use ibc::core::ics02_client::error::ClientError;
use ibc::core::ics02_client::ClientExecutionContext;
use ibc::core::ics23_commitment::commitment::{
    CommitmentPrefix, CommitmentProofBytes, CommitmentRoot,
};
use ibc::core::ics24_host::identifier::ClientId;
use ibc::core::ics24_host::path::{ClientConsensusStatePath, ClientStatePath, Path};
use ibc::core::timestamp::Timestamp;
use ibc::core::{ContextError, ValidationContext};
use ibc::Any;
use ibc_proto::protobuf::Protobuf;

use super::IbcExecutionContext;
use crate::ConsensusStateKey;

// Q: How do we enable users to set the light clients they want?
#[derive(From, TryInto, ConsensusState)]
pub enum AnyConsensusState {
    Tendermint(TmConsensusState),
}

#[derive(From, TryInto)]
pub enum AnyClientState {
    Tendermint(TmClientState),
}

// Next 3 trait impls are boilerplate
// We have a `ClientState` macro, but unfortunately it doesn't currently support
// the context (`IbcExecutionContext` in this case) to be generic
impl ClientStateCommon for AnyClientState {
    fn verify_consensus_state(&self, consensus_state: ibc::Any) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.verify_consensus_state(consensus_state),
        }
    }

    fn client_type(&self) -> ClientType {
        match self {
            AnyClientState::Tendermint(cs) => cs.client_type(),
        }
    }

    fn latest_height(&self) -> ibc::Height {
        match self {
            AnyClientState::Tendermint(cs) => cs.latest_height(),
        }
    }

    fn validate_proof_height(&self, proof_height: ibc::Height) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.validate_proof_height(proof_height),
        }
    }

    fn verify_upgrade_client(
        &self,
        upgraded_client_state: ibc::Any,
        upgraded_consensus_state: ibc::Any,
        proof_upgrade_client: CommitmentProofBytes,
        proof_upgrade_consensus_state: CommitmentProofBytes,
        root: &CommitmentRoot,
    ) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.verify_upgrade_client(
                upgraded_client_state,
                upgraded_consensus_state,
                proof_upgrade_client,
                proof_upgrade_consensus_state,
                root,
            ),
        }
    }

    fn verify_membership(
        &self,
        prefix: &CommitmentPrefix,
        proof: &CommitmentProofBytes,
        root: &CommitmentRoot,
        path: Path,
        value: Vec<u8>,
    ) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => {
                cs.verify_membership(prefix, proof, root, path, value)
            }
        }
    }

    fn verify_non_membership(
        &self,
        prefix: &CommitmentPrefix,
        proof: &CommitmentProofBytes,
        root: &CommitmentRoot,
        path: Path,
    ) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.verify_non_membership(prefix, proof, root, path),
        }
    }
}

impl<'a, C> ClientStateExecution<IbcExecutionContext<'a, C>> for AnyClientState
where
    C: sov_modules_api::Context,
{
    fn initialise(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ClientId,
        consensus_state: ibc::Any,
    ) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.initialise(ctx, client_id, consensus_state),
        }
    }

    fn update_state(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ClientId,
        header: ibc::Any,
    ) -> Result<Vec<ibc::Height>, ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.update_state(ctx, client_id, header),
        }
    }

    fn update_state_on_misbehaviour(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ClientId,
        client_message: ibc::Any,
        update_kind: &UpdateKind,
    ) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => {
                cs.update_state_on_misbehaviour(ctx, client_id, client_message, update_kind)
            }
        }
    }

    fn update_state_on_upgrade(
        &self,
        ctx: &mut IbcExecutionContext<'a, C>,
        client_id: &ClientId,
        upgraded_client_state: ibc::Any,
        upgraded_consensus_state: ibc::Any,
    ) -> Result<ibc::Height, ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.update_state_on_upgrade(
                ctx,
                client_id,
                upgraded_client_state,
                upgraded_consensus_state,
            ),
        }
    }
}

impl<'a, C> ClientStateValidation<IbcExecutionContext<'a, C>> for AnyClientState
where
    C: sov_modules_api::Context,
{
    fn verify_client_message(
        &self,
        ctx: &IbcExecutionContext<'a, C>,
        client_id: &ClientId,
        client_message: ibc::Any,
        update_kind: &UpdateKind,
    ) -> Result<(), ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => {
                cs.verify_client_message(ctx, client_id, client_message, update_kind)
            }
        }
    }

    fn check_for_misbehaviour(
        &self,
        ctx: &IbcExecutionContext<'a, C>,
        client_id: &ClientId,
        client_message: ibc::Any,
        update_kind: &UpdateKind,
    ) -> Result<bool, ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => {
                cs.check_for_misbehaviour(ctx, client_id, client_message, update_kind)
            }
        }
    }

    fn status(
        &self,
        ctx: &IbcExecutionContext<'a, C>,
        client_id: &ClientId,
    ) -> Result<Status, ClientError> {
        match self {
            AnyClientState::Tendermint(cs) => cs.status(ctx, client_id),
        }
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
        client_state_path: ClientStatePath,
        client_state: Self::AnyClientState,
    ) -> Result<(), ContextError> {
        let client_state_bytes = {
            let tm_client_state: TmClientState = client_state.try_into().map_err(|e: &str| {
                ContextError::ClientError(ClientError::Other {
                    description: e.to_string(),
                })
            })?;

            <TmClientState as Protobuf<Any>>::encode_vec(&tm_client_state)
        };

        // FIXME: storing like this does NOT result in a proper Merkle proof The
        // SDK will BorshSerialize our bytes before storing. We need to wait for
        // the SDK to allow us to store raw bytes (i.e. to choose our encoding
        // format)
        self.ibc.client_state_store.set(
            &client_state_path.to_string(),
            &client_state_bytes,
            self.working_set.get_mut(),
        );

        Ok(())
    }

    fn store_consensus_state(
        &mut self,
        consensus_state_path: ClientConsensusStatePath,
        consensus_state: Self::AnyConsensusState,
    ) -> Result<(), ContextError> {
        let key: ConsensusStateKey = consensus_state_path.clone().into();

        let consensus_state_bytes = {
            let tm_consensus_state: TmConsensusState =
                consensus_state.try_into().map_err(|e: &str| {
                    ContextError::ClientError(ClientError::Other {
                        description: e.to_string(),
                    })
                })?;

            <TmConsensusState as Protobuf<Any>>::encode_vec(&tm_consensus_state)
        };

        // FIXME: storing like this does NOT result in a proper Merkle proof The
        // SDK will BorshSerialize our bytes before storing. We need to wait for
        // the SDK to allow us to store raw bytes (i.e. to choose our encoding
        // format)
        self.ibc.consensus_state_store.set(
            &key,
            &consensus_state_bytes,
            self.working_set.get_mut(),
        );

        Ok(())
    }
}

impl<'a, C> TmCommonContext for IbcExecutionContext<'a, C>
where
    C: sov_modules_api::Context,
{
    type ConversionError = &'static str;
    type AnyConsensusState = AnyConsensusState;

    fn consensus_state(
        &self,
        client_cons_state_path: &ClientConsensusStatePath,
    ) -> Result<Self::AnyConsensusState, ContextError> {
        <Self as ValidationContext>::consensus_state(&self, client_cons_state_path)
    }
}

impl<'a, C> TmValidationContext for IbcExecutionContext<'a, C>
where
    C: sov_modules_api::Context,
{
    fn host_timestamp(&self) -> Result<Timestamp, ContextError> {
        <Self as ValidationContext>::host_timestamp(&self)
    }

    fn next_consensus_state(
        &self,
        client_id: &ClientId,
        height: &ibc::Height,
    ) -> Result<Option<Self::AnyConsensusState>, ContextError> {
        todo!()
    }

    fn prev_consensus_state(
        &self,
        client_id: &ClientId,
        height: &ibc::Height,
    ) -> Result<Option<Self::AnyConsensusState>, ContextError> {
        todo!()
    }
}
