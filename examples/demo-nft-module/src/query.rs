use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::{Context, WorkingSet};

use crate::NonFungibleToken;

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
/// Response for `getOwner` method
pub struct OwnerResponse<C: Context> {
    /// Optional owner address
    pub owner: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "nft")]
impl<C: Context> NonFungibleToken<C> {
    #[rpc_method(name = "getOwner")]
    /// Get the owner of a token
    pub fn get_owner(
        &self,
        token_id: u64,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<OwnerResponse<C>> {
        Ok(OwnerResponse {
            owner: self.owners.get(&token_id, working_set),
        })
    }
}
