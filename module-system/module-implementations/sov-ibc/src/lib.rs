#![allow(unused_variables)]
#![allow(dead_code)]

pub mod call;
pub mod codec;
pub mod genesis;

pub(crate) mod context;
mod router;

use codec::ProtobufCodec;
use context::clients::{AnyClientState, AnyConsensusState};
use ibc::core::ics24_host::{path::ClientConsensusStatePath, identifier::ClientId};
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

pub struct ExampleModuleConfig {}

#[derive(Hash, PartialEq, Eq)]
pub struct ConsensusStateKey {
    pub client_id: ClientId,
    pub epoch: u64,
    pub height: u64,
}

impl From<ClientConsensusStatePath> for ConsensusStateKey {
    fn from(path: ClientConsensusStatePath) -> Self {
        Self {
            client_id: path.client_id,
            epoch: path.epoch,
            height: path.height,
        }
    }
}

#[derive(ModuleInfo)]
pub struct IbcModule<C: sov_modules_api::Context> {
    #[address]
    pub address: C::Address,

    #[module]
    pub(crate) transfer: sov_ibc_transfer::Transfer<C>,

    #[state]
    pub client_state_store: sov_state::StateMap<ClientId, AnyClientState, ProtobufCodec>,

    #[state]
    pub consensus_state_store:
        sov_state::StateMap<ConsensusStateKey, AnyConsensusState, ProtobufCodec>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for IbcModule<C> {
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
        // Note: Here, we would convert into a `MsgEnvelope`, and send to `dispatch()` (i.e. no match statement)
        match msg {
            call::CallMessage::MsgCreateClient(msg) => {
                Ok(self.create_client(msg, context, working_set)?)
            }
        }

        // Q: Do we have to checkpoint the working set here, given that there were no errors?
        // Or is this done by the caller?
        // Similarly for reverting.
    }
}
