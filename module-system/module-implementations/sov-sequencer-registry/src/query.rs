//! Defines rpc queries exposed by the sequencer registry module, along with the relevant types
use std::str::FromStr;

use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::{BlobReaderTrait, Context, DaSpec};
use sov_state::WorkingSet;

use crate::{DaAddressSpec, SequencerRegistry};

/// The response type to the `getSequencerDaAddress` RPC method.
#[cfg_attr(
    feature = "native",
    derive(serde::Deserialize, serde::Serialize, Clone)
)]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAddressResponse<C: Context> {
    /// The rollup address of the requested sequencer.
    pub address: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "sequencer")]
impl<C: Context, Da: DaSpec> SequencerRegistry<C, Da>
where
    <<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address:
        borsh::BorshSerialize + borsh::BorshDeserialize,
{
    /// Returns the rollup address of the sequencer with the given DA address.
    ///
    /// The response only contains data if the sequencer is registered.
    #[rpc_method(name = "getSequencerAddress")]
    pub fn sequencer_address(
        &self,
        da_address: String,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<SequencerAddressResponse<C>> {
        let da_address = DaAddressSpec::<Da>::from_str(&da_address)
            .map_err(|_| anyhow::anyhow!("Failed to deserialize DA Address from string"))?;
        Ok(SequencerAddressResponse {
            address: self.allowed_sequencers.get(&da_address, working_set),
        })
    }
}
