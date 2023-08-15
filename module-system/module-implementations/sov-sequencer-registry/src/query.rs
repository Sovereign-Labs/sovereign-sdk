use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::Context;
use sov_rollup_interface::AddressTrait;
use sov_state::WorkingSet;

use crate::SequencerRegistry;

#[cfg_attr(
    feature = "native",
    derive(serde::Deserialize, serde::Serialize, Clone)
)]
#[derive(Debug, Eq, PartialEq)]
/// Rollup address for given DA address sequencer
pub struct SequencerAddressResponse<C: Context> {
    pub address: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "sequencer")]
impl<C: Context, A: AddressTrait + borsh::BorshSerialize + borsh::BorshDeserialize>
    SequencerRegistry<C, A>
{
    /// Returns sequencer rollup address for given DA address
    /// Contains any data only if sequencer is allowed to produce batches
    #[rpc_method(name = "getSequencerAddress")]
    pub fn sequencer_address(
        &self,
        da_address: &A,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<SequencerAddressResponse<C>> {
        Ok(SequencerAddressResponse {
            address: self.allowed_sequencers.get(da_address, working_set),
        })
    }
}
