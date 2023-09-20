use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::{Context, WorkingSet};

use crate::utils::get_collection_address;
use crate::{
    CollectionAddress, CreatorAddress, NftIdentifier, NonFungibleToken, OwnerAddress, TokenId,
};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "CreatorAddress<C>: serde::Serialize",
    deserialize = "CreatorAddress<C>: serde::Deserialize<'de>"
))]
/// Response for `getCollection` method
pub struct CollectionResponse<C: Context> {
    /// Collection name
    pub name: String,
    /// Creator Address
    pub creator: CreatorAddress<C>,
    /// frozen or not
    pub frozen: bool,
    /// supply
    pub supply: u64,
    /// Collection metadata uri
    pub collection_uri: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "OwnerAddress<C>: serde::Serialize, CollectionAddress<C>: serde::Serialize",
    deserialize = "OwnerAddress<C>: serde::Deserialize<'de>, CollectionAddress<C>: serde::Deserialize<'de>"
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
    pub owner: OwnerAddress<C>,
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
    /// Address of the collection
    pub collection_address: CollectionAddress<C>,
}

#[rpc_gen(client, server, namespace = "nft")]
impl<C: Context> NonFungibleToken<C> {
    #[rpc_method(name = "getCollection")]
    /// Get the collection details
    pub fn get_collection(
        &self,
        collection_address: CollectionAddress<C>,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<CollectionResponse<C>> {
        let c = self
            .collections
            .get(&collection_address, working_set)
            .unwrap();

        Ok(CollectionResponse {
            name: c.get_name().to_string(),
            creator: c.get_creator().clone(),
            frozen: c.is_frozen(),
            supply: c.get_supply(),
            collection_uri: c.get_collection_uri().to_string(),
        })
    }
    #[rpc_method(name = "getCollectionAddress")]
    /// Get the collection address
    pub fn get_collection_address(
        &self,
        creator: CreatorAddress<C>,
        collection_name: &str,
        _working_set: &mut WorkingSet<C>,
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
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<NftResponse<C>> {
        let nft_id = NftIdentifier(token_id, collection_address);
        let n = self.nfts.get(&nft_id, working_set).unwrap();
        Ok(NftResponse {
            token_id: n.get_token_id(),
            token_uri: n.get_token_uri().to_string(),
            frozen: n.is_frozen(),
            owner: n.get_owner().clone(),
            collection_address: n.get_collection_address().clone(),
        })
    }
}
