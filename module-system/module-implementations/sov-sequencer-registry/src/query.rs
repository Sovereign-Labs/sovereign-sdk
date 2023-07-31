use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::Context;
use sov_state::WorkingSet;

use crate::SequencerRegistry;

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize, Clone))]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAddressResponse<C: Context> {
    pub address: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "sequencer")]
impl<C: Context> SequencerRegistry<C> {
    /// Returns sequencer rollup address for given DA address
    /// Contains any data only if sequencer is allowed to produce batches
    #[rpc_method(name = "getSequencerAddress")]
    pub fn sequencer_address(
        &self,
        da_address: Vec<u8>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<SequencerAddressResponse<C>> {
        Ok(SequencerAddressResponse {
            address: self.allowed_sequencers.get(&da_address, working_set),
        })
    }
}
