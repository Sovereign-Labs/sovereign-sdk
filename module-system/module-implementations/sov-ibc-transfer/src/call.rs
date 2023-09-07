use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use ibc::applications::transfer::context::TokenTransferExecutionContext;
use ibc::applications::transfer::msgs::transfer::MsgTransfer;
use ibc::applications::transfer::packet::PacketData;
use ibc::applications::transfer::{send_transfer, Memo};
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
        working_set: Rc<RefCell<&mut WorkingSet<C::Storage>>>,
    ) -> Result<sov_modules_api::CallResponse> {
        let msg_transfer: MsgTransfer = {
            let denom = {
                // FIXME: Call the `Bank` method to get token name by address (currently doesn't exist)
                let token_name = String::from("hi");
                if self.is_unique_name_token(&token_name, &mut working_set.borrow_mut()) {
                    // Token name is unique, so it is safe to use it as denom
                    token_name
                } else {
                    // Token name is not guaranteed to be unique, so we need to
                    // make up a unique denom for this token. We use the
                    // stringified token address, as it is guaranteed to be
                    // unique.
                    sdk_token_transfer.token_address.to_string()
                }
            };

            MsgTransfer {
                port_id_on_a: sdk_token_transfer.port_id_on_a,
                chan_id_on_a: sdk_token_transfer.chan_id_on_a,
                packet_data: PacketData {
                    token: denom
                        .parse()
                        .map_err(|_err| anyhow::anyhow!("Failed to parse denom {denom}"))?,
                    sender: sdk_token_transfer.sender,
                    receiver: sdk_token_transfer.receiver,
                    memo: sdk_token_transfer.memo,
                },
                timeout_height_on_b: sdk_token_transfer.timeout_height_on_b,
                timeout_timestamp_on_b: sdk_token_transfer.timeout_timestamp_on_b,
            }
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

    /// This function returns true if the token's name is unique. This is not
    /// true for all tokens native to the Sovereign SDK, as the SDK only uses a
    /// token's address as a unique identifier. The only tokens that are
    /// guaranteed to have a unique name are the ones that were minted by the
    /// IBC module, as these take their name from the ICS-20 token denom, which
    /// is guaranteed to be unique.
    fn is_unique_name_token(
        &self,
        token_name: &str,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> bool {
        self.minted_tokens.get(token_name, working_set).is_some()
    }
}
