use anyhow::{bail, Result};
use sov_modules_api::{CallResponse, Context};
use sov_state::WorkingSet;
use crate::{Collection, Nft, NonFungibleToken};
use crate::offchain::{track_collection, track_nft, update_top_owners};
use crate::utils::get_collection_address;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// A transaction handled by the NFT module. Mints, Transfers, or Burns an NFT by id
pub enum CallMessage<C: Context> {
    /// Create a new collection
    CreateCollection {
        /// Name of the collection
        name: String,
        /// meta data url for collection
        metadata_url: String,
    },
    /// update collection metadata
    UpdateCollection {
        /// Name of the collection
        name: String,
        /// meta data url for collection
        metadata_url: String,
    },
    /// freeze a collection
    FreezeCollection {
        /// collection name
        collection_name: String,
    },
    /// mint a new nft
    MintNft {
        /// Name of the collection
        collection_name: String,
        /// Meta data url for collection
        metadata_url: String,
        /// nft id
        id: u64,
        /// mint_to address
        mint_to_address: C::Address,
        /// is nft frozen after mint
        frozen: bool,
    },
    /// Update nft metadata url or frozen status
    UpdateNft {
        /// Name of the collection
        collection_address: C::Address,
        /// nft id
        id: u64,
        /// Meta data url for collection
        metadata_url: Option<String>,
        /// Frozen status
        frozen: Option<bool>,

    },
    /// freeze an nft
    TransferNft {
        /// collection name
        collection_address: C::Address,
        /// nft id
        id: u64,
        /// nft id
        to: C::Address,
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
        if self.collections.get(&collection_address, working_set).is_some() {
            bail!("Collection with name {} by sender {} already exists",collection_name, creator.to_string());
        }
        let c = Collection::<C> {
            name: collection_name.to_string(),
            creator: creator.clone(),
            frozen: false,
            supply: 0,
            metadata_url: metadata_url.to_string(),
        };
        self.collections.set(&collection_address, &c, working_set);
        track_collection(&collection_address.to_string(),
                         collection_name, &creator.to_string(), false, metadata_url, 0);
        Ok(CallResponse::default())
    }

    pub(crate) fn update_collection(
        &self,
        collection_name: &str,
        metadata_url: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let creator = context.sender();
        let collection_address = get_collection_address::<C>(collection_name, creator.as_ref());
        if let Some(mut c) = self.collections.get(&collection_address, working_set) {
            if c.frozen == true {
                bail!("Collection with name {} by sender {} is frozen and cannot be updated",collection_name, creator.to_string());
            } else {
                c.metadata_url = metadata_url.to_string();
                self.collections.set(&collection_address, &c, working_set);
                track_collection(&collection_address.to_string(),
                                 collection_name, &creator.to_string(), false, metadata_url, 0);
            }
        } else {
            bail!("Collection with name {} by sender {} does not exist",collection_name, creator.to_string());
        }
        Ok(CallResponse::default())
    }

    pub(crate) fn freeze_collection(
        &self,
        collection_name: &str,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let creator = context.sender();
        let collection_address = get_collection_address::<C>(collection_name, creator.as_ref());

        if let Some(mut c) = self.collections.get(&collection_address, working_set) {
            if c.frozen == true {
                bail!("Collection with name {} by sender {} is already frozen",collection_name, creator.to_string())
            } else {
                c.frozen = true;
                self.collections.set(&collection_address, &c, working_set);
            }
        } else {
            bail!("Collection with name {} by sender {} does not exist",collection_name, creator.to_string());
        }


        Ok(CallResponse::default())
    }

    pub(crate) fn mint_nft(
        &self,
        nft_id: u64,
        collection_name: &str,
        metadata_url: &str,
        mint_to_address: &C::Address,
        frozen: bool,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let creator = context.sender();
        let collection_address = get_collection_address::<C>(collection_name, creator.as_ref());

        if let Some(mut c) = self.collections.get(&collection_address, working_set) {
            let nft_identifier = (nft_id,collection_address.clone());
            if c.frozen == true {
                bail!("Collection with name {} by sender {} is already frozen",collection_name, creator.to_string())
            } else {
                if let Some(_) = self.nfts.get(&nft_identifier,working_set) {
                    bail!("NFT id {} in Collection with name {}, creator {} already exists",nft_id, collection_name, creator.to_string());
                } else {
                    let new_nft = Nft{
                        id: nft_id,
                        collection_address: collection_address.clone(),
                        owner: mint_to_address.clone(),
                        frozen,
                        metadata_url: metadata_url.to_string(),
                    };
                    self.nfts.set(&nft_identifier, &new_nft, working_set);
                    c.supply+=1;
                    self.collections.set(&collection_address, &c, working_set);
                }
            }
        } else {
            bail!("Collection with name {} by sender {} does not exist",collection_name, creator.to_string());
        }

        Ok(CallResponse::default())
    }

    pub(crate) fn transfer_nft(
        &self,
        nft_id: u64,
        collection_address: &C::Address,
        to: &C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let owner = context.sender();

        if self.collections.get(collection_address, working_set).is_some() {
            let nft_identifier = (nft_id, collection_address.clone());
            if let Some(mut n) = self.nfts.get(&nft_identifier,working_set) {
                if owner.as_ref() == n.owner.as_ref() {
                    n.owner = to.clone();
                    self.nfts.set(&nft_identifier, &n, working_set);
                } else {
                    bail!("NFT id {} in Collection with address {} does not exist",nft_id, collection_address.to_string());
                }
            } else {
                bail!("NFT id {} in Collection with address {} does not exist",nft_id, collection_address.to_string());
            }
        } else {
            bail!("Collection with address {} does not exist",collection_address.to_string());
        }

        Ok(CallResponse::default())
    }

    pub(crate) fn update_nft(
        &self,
        collection_address: &C::Address,
        nft_id: u64,
        metadata_url: Option<String>,
        frozen: Option<bool>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let creator = context.sender();

        if let Some(c) = self.collections.get(collection_address, working_set) {
            let nft_identifier = (nft_id,collection_address.clone());
            if c.creator.as_ref() == creator.as_ref() {
                if let Some(mut n) = self.nfts.get(&nft_identifier,working_set) {
                    if n.frozen == false {
                        if Some(true) == frozen {
                            n.frozen = true;
                        }
                        if let Some(murl) = metadata_url {
                            n.metadata_url = murl;
                        }
                        self.nfts.set(&nft_identifier, &n, working_set);
                    } else {
                        bail!("NFT id {} in Collection with address {} is frozen",nft_id, collection_address.to_string());
                    }
                } else {
                    bail!("NFT id {} in Collection with address {} does not exist",nft_id, collection_address.to_string());
                }
            } else {
                bail!("Nfts in collection name:{} collection_address:{} cannot be frozen by {} .collection owner is {}",
                    c.name,collection_address.to_string(),creator, c.creator.to_string());
            }

        } else {
            bail!("Collection with address {} does not exist",collection_address.to_string());
        }

        Ok(CallResponse::default())
    }


}
