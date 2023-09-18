use anyhow::{anyhow, ensure, Result};
use sov_modules_api::{CallResponse, Context};
use sov_state::WorkingSet;
use crate::utils::get_collection_address;
use crate::{
    Collection, CollectionAddress, Nft, NftIdentifier, NonFungibleToken, TokenId, UserAddress,
};
use crate::offchain::{track_collection, track_nft, update_top_owners};

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
        metadata_url: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let creator = context.sender();
        let collection_address = get_collection_address::<C>(collection_name, creator.as_ref());
        self.exit_if_collection_exists(collection_name, context, working_set)?;

        let c = Collection::<C> {
            name: collection_name.to_string(),
            creator: UserAddress(creator.clone()),
            frozen: false,
            supply: 0,
            collection_uri: metadata_url.to_string(),
        };
        self.collections.set(&collection_address, &c, working_set);
        track_collection(&collection_address.to_string(),
                         &collection_name.to_string(),
                         &creator.to_string(),
                         false,
                         &metadata_url.to_string(),
                         0);
        Ok(CallResponse::default())
    }

    pub(crate) fn update_collection(
        &self,
        collection_name: &str,
        collection_uri: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (collection_address, mut collection) =
            self.get_collection_by_name(collection_name, context, working_set)?;
        collection.exit_if_frozen()?;
        collection.collection_uri = collection_uri.to_string();
        self.collections
            .set(&collection_address, &collection, working_set);
        track_collection(&collection_address.to_string(),
                         &collection.name.to_string(),
                         &collection.creator.to_string(),
                         collection.frozen,
                         &collection.collection_uri,
                         collection.supply);
        Ok(CallResponse::default())
    }

    pub(crate) fn freeze_collection(
        &self,
        collection_name: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (collection_address, mut collection) =
            self.get_collection_by_name(collection_name, context, working_set)?;
        collection.exit_if_frozen()?;
        collection.frozen = true;
        self.collections
            .set(&collection_address, &collection, working_set);
        track_collection(&collection_address.to_string(),
                         &collection.name.to_string(),
                         &collection.creator.to_string(),
                         collection.frozen,
                         &collection.collection_uri,
                         collection.supply);
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
        let (collection_address, mut collection) =
            self.get_collection_by_name(collection_name, context, working_set)?;
        collection.exit_if_frozen()?;
        self.exit_if_nft_exists(token_id, &collection_address, working_set)?;

        let new_nft = Nft {
            token_id,
            collection_address: collection_address.clone(),
            owner: mint_to_address.clone(),
            frozen,
            token_uri: token_uri.to_string(),
        };

        self.nfts.set(
            &NftIdentifier(token_id, collection_address.clone()),
            &new_nft,
            working_set,
        );
        track_nft(&collection_address.to_string(),
                  token_id,
                  &mint_to_address.to_string(),
                  frozen,
                  &token_uri.to_string());
        collection.supply += 1;
        self.collections
            .set(&collection_address, &collection, working_set);
        track_collection(&collection_address.to_string(),
                         &collection.name.to_string(),
                         &collection.creator.to_string(),
                         collection.frozen,
                         &collection.collection_uri,
                         collection.supply);
        update_top_owners(&collection_address.to_string(),
                          Some(&[(mint_to_address.to_string(), 1)]),
                          None);

        Ok(CallResponse::default())
    }

    pub(crate) fn transfer_nft(
        &self,
        nft_id: u64,
        collection_address: &CollectionAddress<C>,
        to: &UserAddress<C>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.get_collection_by_address(collection_address, working_set)?;
        let token_identifier = NftIdentifier(nft_id, collection_address.clone());
        let mut nft = self.get_nft_by_id(&token_identifier, working_set)?;
        nft.exit_if_not_owned(context)?;
        let original_owner = nft.owner;
        nft.owner = to.clone();
        self.nfts.set(&token_identifier, &nft, working_set);
        track_nft(&collection_address.to_string(),
                  nft.token_id,
                  &nft.owner.to_string(),
                  nft.frozen,
                  &nft.token_uri);
        update_top_owners(&collection_address.to_string(),
                          Some(&[(to.to_string(), 1)]),
                          Some(&[(original_owner.to_string(), 1)]));
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
        let (collection_address, _) =
            self.get_collection_by_name(collection_name, context, working_set)?;
        let token_identifier = NftIdentifier(token_id, collection_address.clone());
        let mut nft = self.get_nft_by_id(&token_identifier, working_set)?;
        nft.exit_if_frozen()?;
        if let Some(val) = frozen {
            nft.frozen = val
        };
        if let Some(murl) = token_uri {
            nft.token_uri = murl
        };
        self.nfts.set(&token_identifier, &nft, working_set);
        track_nft(&collection_address.to_string(),
                  nft.token_id,
                  &nft.owner.to_string(),
                  nft.frozen,
                  &nft.token_uri);

        Ok(CallResponse::default())
    }

    fn exit_if_collection_exists(
        &self,
        collection_name: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let creator = context.sender();
        let ca = get_collection_address(collection_name, creator.as_ref());
        ensure!(
            self.collections.get(&ca, working_set).is_none(),
            format!(
                "Collection with name: {} already exists creator {}",
                collection_name, creator
            )
        );
        Ok(())
    }

    fn exit_if_nft_exists(
        &self,
        token_id: TokenId,
        collection_address: &CollectionAddress<C>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let msg = format!(
            "Collection with address {} already exists",
            collection_address.0
        );
        ensure!(
            self.nfts
                .get(
                    &NftIdentifier(token_id, collection_address.clone()),
                    working_set
                )
                .is_none(),
            msg
        );
        Ok(())
    }

    fn get_collection_by_address(
        &self,
        collection_address: &CollectionAddress<C>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<Collection<C>> {
        let c = self.collections.get(collection_address, working_set);
        if let Some(collection) = c {
            Ok(collection)
        } else {
            Err(anyhow!(
                "Collection with address: {} does not exist",
                collection_address.0
            ))
        }
    }

    fn get_collection_by_name(
        &self,
        collection_name: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(CollectionAddress<C>, Collection<C>)> {
        let creator = context.sender();
        let ca = get_collection_address(collection_name, creator.as_ref());
        let c = self.collections.get(&ca, working_set);
        if let Some(collection) = c {
            Ok((ca, collection))
        } else {
            Err(anyhow!(
                "Collection with name: {} does not exist for creator {}",
                collection_name,
                creator
            ))
        }
    }

    fn get_nft_by_id(
        &self,
        token_identifier: &NftIdentifier<C>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<Nft<C>> {
        let n = self.nfts.get(token_identifier, working_set);
        if let Some(nft) = n {
            Ok(nft)
        } else {
            Err(anyhow!(
                "Nft with token_id: {} in collection_address: {} does not exist",
                token_identifier.0,
                token_identifier.1 .0
            ))
        }
    }
}
