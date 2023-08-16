use jsonrpsee::core::RpcResult;
use jsonrpsee::types::error::ErrorCode;
use jsonrpsee::types::ErrorObject;
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
pub struct SequencerAddressResponse {
    pub address: Option<String>,
}

#[rpc_gen(client, server, namespace = "sequencer")]
impl<C: Context, A: AddressTrait + borsh::BorshSerialize + borsh::BorshDeserialize>
    SequencerRegistry<C, A>
// where
// A: AddressTrait + borsh::BorshSerialize + borsh::BorshDeserialize,
{
    /// Returns sequencer rollup address for given DA address
    /// Contains any data only if sequencer is allowed to produce batches
    #[rpc_method(name = "getSequencerAddress")]
    pub fn sequencer_address(
        &self,
        da_address: String,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<SequencerAddressResponse> {
        let da_address =
            A::from_str(&da_address).map_err(|_| ErrorObject::from(ErrorCode::InvalidRequest))?;
        Ok(SequencerAddressResponse {
            address: self
                .allowed_sequencers
                .get(&da_address, working_set)
                .map(|a| a.to_string()),
        })
    }
}
