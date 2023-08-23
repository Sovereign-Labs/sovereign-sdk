#![allow(missing_docs)]

use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_rollup_interface::da::BlobReaderTrait;
use sov_state::WorkingSet;

use crate::{DaAddress, SequencerRegistry};

/// The response type to the `getSequencerDddress` RPC method.
#[cfg_attr(
    feature = "native",
    derive(serde::Deserialize, serde::Serialize, Clone)
)]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAddressResponse<C: sov_modules_api::Context> {
    /// The rollup address of the requested sequencer.
    pub address: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "sequencer")]
impl<C: sov_modules_api::Context, B: BlobReaderTrait> SequencerRegistry<C, B>
// where
//     B::Address: borsh::BorshSerialize + borsh::BorshDeserialize,
{
    /// Returns the rollup address of the sequencer with the given DA address.
    ///
    /// The response only contains data if the sequencer is registered.
    #[rpc_method(name = "getSequencerAddress")]
    pub fn sequencer_address(
        &self,
        da_address: DaAddress,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<SequencerAddressResponse<C>>
    where
        <B as BlobReaderTrait>::Address: borsh::BorshSerialize + borsh::BorshDeserialize,
    {
        // TODO: Remove after first iteration
        let a = B::Address::try_from(&da_address)?;
        Ok(SequencerAddressResponse {
            address: self.allowed_sequencers.get(&a, working_set),
        })
    }
}
