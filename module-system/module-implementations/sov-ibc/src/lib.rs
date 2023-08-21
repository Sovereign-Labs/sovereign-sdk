#![allow(unused_variables)]
#![allow(dead_code)]

pub mod call;
pub mod codec;
pub mod genesis;

pub(crate) mod context;
mod router;

use codec::BorshKeyProtobufValueCodec;
use context::clients::AnyClientState;
use ibc::core::ics24_host::path::ClientConsensusStatePath;
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

pub struct ExampleModuleConfig {}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct ConsensusStateKey {
    pub client_id: String,
    pub epoch: u64,
    pub height: u64,
}

impl From<ClientConsensusStatePath> for ConsensusStateKey {
    fn from(path: ClientConsensusStatePath) -> Self {
        Self {
            client_id: path.client_id.to_string(),
            epoch: path.epoch,
            height: path.height,
        }
    }
}

/// FIXME: As of today, the SDK borsh serializes all our data before it makes it
/// to the store. They will add a feature to allow us to store "raw" bytes (e.g.
/// Vec<u8>). Hence, all our data types are in their serialized form (i.e.
/// you'll see `Vec<u8>` instead of `AnyClientState`)
#[derive(ModuleInfo)]
pub struct IbcModule<C: sov_modules_api::Context> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// The `ClientState` store indexed by `ClientId`. Note: we cannot index by
    /// `ClientId` StateMap requires `ClientId` to implement `BorshSerialize`,
    /// which isn't the case even with ibc-rs's borsh feature since ibc-rs uses
    /// borsh v0.9 and the Sovereign SDK uses v0.10.
    #[state]
    pub client_state_store:
        sov_state::StateMap<String, AnyClientState, BorshKeyProtobufValueCodec>,

    // Key -> AnyConsensusState (encoded with Protobuf<Any>::encode_vec)
    #[state]
    pub consensus_state_store: sov_state::StateMap<ConsensusStateKey, Vec<u8>>,
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
