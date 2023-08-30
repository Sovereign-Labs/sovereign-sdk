use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use anyhow::{bail, Result};
use ibc::applications::transfer::msgs::transfer::MsgTransfer;
use ibc::applications::transfer::send_transfer;
use ibc::clients::ics07_tendermint::client_state::TENDERMINT_CLIENT_STATE_TYPE_URL;
use ibc::clients::ics07_tendermint::consensus_state::TENDERMINT_CONSENSUS_STATE_TYPE_URL;
use ibc::core::ics02_client::msgs::create_client::MsgCreateClient;
use ibc::core::ics02_client::msgs::ClientMsg;
use ibc::core::{dispatch, MsgEnvelope};
use ibc::Any;
use sov_ibc_transfer::context::{EscrowExtraData, TransferContext};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;
use thiserror::Error;

use crate::context::IbcExecutionContext;
use crate::router::IbcRouter;
use crate::IbcModule;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct RawMsgCreateClient {
    client_state: Vec<u8>,
    consensus_state: Vec<u8>,
    signer: String,
}

// TODO: Put back when we change `MsgCreateClient` with `Core(MsgEnvelope)`
// #[cfg_attr(
//     feature = "native",
//     derive(schemars::JsonSchema),
//     schemars(bound = "C::Address: ::schemars::JsonSchema", rename = "CallMessage")
// )]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    // Currently a hack since our types are not borsh de/serializable
    MsgCreateClient(RawMsgCreateClient),

    // TODO: add Transfer message, and remove from transfer module
    Transfer {
        msg_transfer: MsgTransfer,
        token_address: C::Address,
    },
}

/// Example of a custom error.
#[derive(Debug, Error)]
enum SetValueError {}

impl<C: sov_modules_api::Context> IbcModule<C> {
    pub(crate) fn create_client(
        &self,
        raw_msg: RawMsgCreateClient,
        context: &C,
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

        let shared_working_set = Rc::new(RefCell::new(working_set));

        let mut execution_context = IbcExecutionContext {
            ibc: self,
            working_set: shared_working_set.clone(),
        };

        let mut router = IbcRouter::new(self, context, shared_working_set);

        match dispatch(&mut execution_context, &mut router, domain_msg) {
            Ok(_) => Ok(CallResponse::default()),
            Err(e) => bail!(e.to_string()),
        }
    }

    pub(crate) fn transfer(
        &self,
        msg_transfer: MsgTransfer,
        token_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        let shared_working_set = Rc::new(RefCell::new(working_set));
        let mut execution_context = IbcExecutionContext {
            ibc: self,
            working_set: shared_working_set.clone(),
        };

        let mut token_ctx =
            TransferContext::new(self.transfer.clone(), context, shared_working_set);

        send_transfer(
            &mut execution_context,
            &mut token_ctx,
            msg_transfer,
            &EscrowExtraData { token_address },
        )?;
        todo!()
    }
}