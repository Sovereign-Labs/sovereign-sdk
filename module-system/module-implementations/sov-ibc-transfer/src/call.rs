use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use ibc::applications::transfer::context::TokenTransferExecutionContext;
use ibc::applications::transfer::Memo;
use ibc::applications::transfer::msgs::transfer::MsgTransfer;
use ibc::core::ics04_channel::timeout::TimeoutHeight;
use ibc::core::ics24_host::identifier::{ChannelId, PortId};
use ibc::core::timestamp::Timestamp;
use ibc::core::ExecutionContext;
use ibc::Signer;
use sov_state::WorkingSet;

use crate::context::EscrowExtraData;
use crate::Transfer;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct SDKTokenTransfer<C: sov_modules_api::Context> {
    /// the port on which the packet will be sent
    pub port_id_on_a: PortId,
    /// the channel by which the packet will be sent
    pub chan_id_on_a: ChannelId,
    /// Timeout height relative to the current block height.
    /// The timeout is disabled when set to None.
    pub timeout_height_on_b: TimeoutHeight,
    /// Timeout timestamp relative to the current block timestamp.
    /// The timeout is disabled when set to 0.
    pub timeout_timestamp_on_b: Timestamp,

    /// The address of the token to be sent
    pub token_address: C::Address,
    /// The address of the token sender
    pub sender: Signer,
    /// The address of the token receiver on the counterparty chain
    pub receiver: Signer,
    /// Additional note associated with the message
    pub memo: Memo,
}

impl<C> Transfer<C>
where
    C: sov_modules_api::Context,
{
    pub fn transfer(
        &self,
        sdk_token_transfer: SDKTokenTransfer<C>,
        execution_context: &mut impl ExecutionContext,
        token_ctx: &mut impl TokenTransferExecutionContext<EscrowExtraData<C>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse> {
        let shared_working_set = Rc::new(RefCell::new(working_set));

        let msg_transfer: MsgTransfer = {
            todo!()
        };

        send_transfer(
            execution_context,
            token_ctx,
            msg_transfer,
            &EscrowExtraData {
                token_address: sdk_token_transfer.token_address,
            },
        )?;

        todo!()
    }
}
