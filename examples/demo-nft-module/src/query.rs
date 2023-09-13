use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::Context;
use sov_state::WorkingSet;

use crate::utils::get_collection_address;
use crate::{CollectionAddress, NftIdentifier, NonFungibleToken, TokenId, UserAddress};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "UserAddress<C>: serde::Serialize",
    deserialize = "UserAddress<C>: serde::Deserialize<'de>"
))]
/// Response for `getCollection` method
pub struct CollectionResponse<C: Context> {
    /// Collection name
    pub name: String,
    /// Creator Address
    pub creator: UserAddress<C>,
    /// frozen or not
    pub frozen: bool,
    /// supply
    pub supply: u64,
    /// Collection metadata uri
    pub collection_uri: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "UserAddress<C>: serde::Serialize, CollectionAddress<C>: serde::Serialize",
    deserialize = "UserAddress<C>: serde::Deserialize<'de>, CollectionAddress<C>: serde::Deserialize<'de>"
))]
/// Response for `getNft` method
pub struct NftResponse<C: Context> {
    /// Unique token id scoped to the collection
    pub token_id: TokenId,
    /// URI pointing to offchain metadata
    pub token_uri: String,
    /// frozen status (token_uri mutable or not)
    pub frozen: bool,
    /// Owner of the NFT
    pub owner: UserAddress<C>,
    /// Collection address that the NFT belongs to
    pub collection_address: CollectionAddress<C>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "CollectionAddress<C>: serde::Serialize",
    deserialize = "CollectionAddress<C>: serde::Deserialize<'de>"
))]
/// Response for `getCollectionAddress` method
pub struct CollectionAddressResponse<C: Context> {
    pub collection_address: CollectionAddress<C>,
}

#[rpc_gen(client, server, namespace = "nft")]
impl<C: Context> NonFungibleToken<C> {
    #[rpc_method(name = "getCollection")]
    /// Get the collection details
    pub fn get_collection(
        &self,
        collection_address: CollectionAddress<C>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<CollectionResponse<C>> {
        let c = self
            .collections
            .get(&collection_address, working_set)
            .unwrap();

        Ok(CollectionResponse {
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
        collection_name: &str,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<CollectionAddressResponse<C>> {
        let ca = get_collection_address::<C>(collection_name, creator.as_ref());
        Ok(CollectionAddressResponse {
            collection_address: ca,
        })
    }
    #[rpc_method(name = "getNft")]
    /// Get the NFT details
    pub fn get_nft(
        &self,
        collection_address: CollectionAddress<C>,
        token_id: TokenId,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<NftResponse<C>> {
        let nft_id = NftIdentifier(token_id, collection_address);
        let n = self.nfts.get(&nft_id, working_set).unwrap();
        Ok(NftResponse {
            token_id: n.token_id,
            token_uri: n.token_uri,
            frozen: n.frozen,
            owner: n.owner,
            collection_address: n.collection_address,
        })
    }
}
