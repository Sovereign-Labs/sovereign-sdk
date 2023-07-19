use std::fmt::Debug;

use anyhow::{Result, bail};
use ibc::clients::ics07_tendermint::client_state::TENDERMINT_CLIENT_STATE_TYPE_URL;
use ibc::clients::ics07_tendermint::consensus_state::TENDERMINT_CONSENSUS_STATE_TYPE_URL;
use ibc::core::ics02_client::msgs::create_client::MsgCreateClient;
use ibc::core::ics02_client::msgs::ClientMsg;
use ibc::core::{MsgEnvelope, dispatch};
use ibc::Any;
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;
use thiserror::Error;

use crate::context::IbcExecutionContext;
use crate::IbcModule;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct RawMsgCreateClient {
    client_state: Vec<u8>,
    consensus_state: Vec<u8>,
    signer: String,
}

/// This enumeration represents the available call messages for interacting with the `ExampleModule` module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage {
    // Currently a hack since our types are not borsh de/serializable
    MsgCreateClient(RawMsgCreateClient),
}

/// Example of a custom error.
#[derive(Debug, Error)]
enum SetValueError {}

impl<C: sov_modules_api::Context> IbcModule<C> {
    pub(crate) fn create_client(
        &self,
        raw_msg: RawMsgCreateClient,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        // Hack: Normally this value would be converted from the incoming message
        let domain_msg = MsgEnvelope::Client(ClientMsg::CreateClient(MsgCreateClient::new(
            Any {
                type_url: TENDERMINT_CLIENT_STATE_TYPE_URL.to_string(),
                value: raw_msg.client_state,
            },
            Any {
                type_url: TENDERMINT_CONSENSUS_STATE_TYPE_URL.to_string(),
                value: raw_msg.consensus_state,
            },
            raw_msg.signer.into(),
        )));

        let mut execution_context = IbcExecutionContext {
            ibc: self,
            working_set: &working_set,
        };

        match dispatch(&mut execution_context, domain_msg) {
            Ok(_) => Ok(CallResponse::default()),
            Err(e) => bail!(e.to_string()),
        }
    }
}
