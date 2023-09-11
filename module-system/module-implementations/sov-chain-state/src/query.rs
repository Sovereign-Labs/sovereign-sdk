use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_state::WorkingSet;

use crate::{ChainState, TransitionHeight};

#[rpc_gen(client, server, namespace = "chainState")]
impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> ChainState<C, Da> {
    /// Get the height of the current slot.
    /// Panics if the slot height is not set
    #[rpc_method(name = "getSlotHeight")]
    pub fn get_slot_height_rpc(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<TransitionHeight> {
        Ok(self.get_slot_height(working_set))
    }
}
