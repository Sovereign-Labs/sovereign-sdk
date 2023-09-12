use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::Context;
use sov_state::WorkingSet;
use crate::utils::get_collection_address;

use crate::{NonFungibleToken, CollectionAddress, UserAddress};


#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
/// Response for `getCollection` method
pub struct CollectionResponse<C: Context> where C::Address: serde::Serialize {
    /// Collection name
    pub name: String,
    /// Creator Address
    pub creator: UserAddress<C>,
    /// frozen or not
    pub frozen: bool,
    /// supply
    pub supply: u64,
    /// Collection metadata uri
    pub collection_uri: String
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
/// Response for `getCollectionAddress` method
pub struct CollectionAddressResponse<C: Context> {
    pub collection_address: CollectionAddress<C>
}

#[rpc_gen(client, server, namespace = "nft")]
impl<C: Context> NonFungibleToken<C> {
    #[rpc_method(name = "getCollection")]
    /// Get the collection details
    pub fn get_collection(
        &self,
        collection_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<CollectionResponse<C>> {
        let c = self.collections
            .get(&CollectionAddress(collection_address), working_set).unwrap();

        Ok(
            CollectionResponse {
                name: c.name.to_string(),
                creator: c.creator.clone(),
                frozen: c.frozen,
                supply: c.supply,
                collection_uri: c.collection_uri.to_string(),
            })
    }
    #[rpc_method(name = "getCollectionAddress")]
    /// Get the collection address
    pub fn get_collection_address(
        &self,
        creator: UserAddress<C>,
        collection_name: String,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<CollectionAddressResponse<C>> {
        let ca = get_collection_address::<C>(&collection_name, creator.as_ref());
        Ok(
            CollectionAddressResponse {
                collection_address: ca,
            })
    }
}
