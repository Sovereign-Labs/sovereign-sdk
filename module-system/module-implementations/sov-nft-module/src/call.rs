use anyhow::{bail, Result};
use sov_modules_api::WorkingSet;
use sov_modules_api::{CallResponse, Context};

use crate::address::UserAddress;
use crate::{
    Collection, CollectionAddress, CollectionState, Nft, NftIdentifier, NonFungibleToken, TokenId,
};

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema),
    schemars(bound = "C::Address: ::schemars::JsonSchema", rename = "CallMessage")
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// A transaction handled by the NFT module. Mints, Transfers, or Burns an NFT by id
pub enum CallMessage<C: Context> {
    /// Create a new collection
    CreateCollection {
        /// Name of the collection
        name: String,
        /// meta data url for collection
        collection_uri: String,
    },
    /// update collection metadata
    UpdateCollection {
        /// Name of the collection
        name: String,
        /// meta data url for collection
        collection_uri: String,
    },
    /// Freeze a collection that is unfrozen.
    /// This prevents new NFTs from being minted.
    FreezeCollection {
        /// collection name
        collection_name: String,
    },
    /// mint a new nft
    MintNft {
        /// Name of the collection
        collection_name: String,
        /// Meta data url for collection
        token_uri: String,
        /// nft id. a unique identifier for each NFT
        token_id: TokenId,
        /// Address that the NFT should be minted to
        owner: UserAddress<C>,
        /// A frozen nft cannot have its metadata_url modified or be unfrozen
        /// Setting this to true makes the nft immutable
        frozen: bool,
    },
    /// Update nft metadata url or frozen status
    UpdateNft {
        /// Name of the collection
        collection_name: String,
        /// nft id
        token_id: TokenId,
        /// Meta data url for collection
        token_uri: Option<String>,
        /// Frozen status
        frozen: Option<bool>,
    },
    /// Transfer an NFT from an owned address to another address
    TransferNft {
        /// Collection Address
        collection_address: CollectionAddress<C>,
        /// NFT id of the owned token to be transferred
        token_id: u64,
        /// Target address of the user to transfer the NFT to
        to: UserAddress<C>,
    },
}

impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn create_collection(
        &self,
        collection_name: &str,
        collection_uri: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (collection_address, collection) = Collection::new(
            collection_name,
            collection_uri,
            &self.collections,
            context,
            working_set,
        )?;
        self.collections
            .set(&collection_address, &collection, working_set);
        Ok(CallResponse::default())
    }

    pub(crate) fn update_collection(
        &self,
        collection_name: &str,
        collection_uri: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (collection_address, collection_state) = Collection::get_owned_collection(
            collection_name,
            &self.collections,
            context,
            working_set,
        )?;
        match collection_state {
            CollectionState::Frozen(c) => bail!(
                "Collection with name: {} , creator: {} is frozen",
                c.get_name(),
                c.get_creator()
            ),
            CollectionState::Mutable(mut mut_collection) => {
                mut_collection.set_collection_uri(collection_uri);
                self.collections
                    .set(&collection_address, &mut_collection.0, working_set);
            }
        }
        Ok(CallResponse::default())
    }

    pub(crate) fn freeze_collection(
        &self,
        collection_name: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (collection_address, collection_state) = Collection::get_owned_collection(
            collection_name,
            &self.collections,
            context,
            working_set,
        )?;
        match collection_state {
            CollectionState::Frozen(c) => bail!(
                "Collection with name: {} , creator: {} is frozen",
                c.get_name(),
                c.get_creator()
            ),
            CollectionState::Mutable(mut collection) => {
                collection.freeze();
                self.collections
                    .set(&collection_address, &collection.0, working_set);
            }
        }
        Ok(CallResponse::default())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mint_nft(
        &self,
        token_id: u64,
        collection_name: &str,
        token_uri: &str,
        mint_to_address: &UserAddress<C>,
        frozen: bool,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (collection_address, collection_state) = Collection::get_owned_collection(
            collection_name,
            &self.collections,
            context,
            working_set,
        )?;
        match collection_state {
            CollectionState::Frozen(c) => bail!(
                "Collection with name: {} , creator: {} is frozen",
                c.get_name(),
                c.get_creator()
            ),
            CollectionState::Mutable(mut collection) => {
                let new_nft = Nft::new(
                    token_id,
                    token_uri,
                    mint_to_address,
                    frozen,
                    &collection_address,
                    &self.nfts,
                    working_set,
                )?;
                self.nfts.set(
                    &NftIdentifier(token_id, collection_address.clone()),
                    &new_nft,
                    working_set,
                );
                collection.increment_supply();
                self.collections
                    .set(&collection_address, &collection.0, working_set);

                Ok(CallResponse::default())
            }
        }
    }

    pub(crate) fn transfer_nft(
        &self,
        nft_id: u64,
        collection_address: &CollectionAddress<C>,
        to: &UserAddress<C>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let mut owned_nft =
            Nft::get_owned_nft(nft_id, collection_address, &self.nfts, context, working_set)?;
        owned_nft.set_owner(to);
        self.nfts.set(
            &NftIdentifier(nft_id, collection_address.clone()),
            &owned_nft.0,
            working_set,
        );
        Ok(CallResponse::default())
    }

    pub(crate) fn update_nft(
        &self,
        collection_name: &str,
        token_id: u64,
        token_uri: Option<String>,
        frozen: Option<bool>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (collection_address, mut mutable_nft) = Nft::get_mutable_nft(
            token_id,
            collection_name,
            &self.nfts,
            &self.collections,
            context,
            working_set,
        )?;
        if let Some(true) = frozen {
            mutable_nft.freeze()
        }
        if let Some(uri) = token_uri {
            mutable_nft.update_token_uri(&uri);
        }
        self.nfts.set(
            &NftIdentifier(token_id, collection_address),
            &mutable_nft.0,
            working_set,
        );
        Ok(CallResponse::default())
    }
}
