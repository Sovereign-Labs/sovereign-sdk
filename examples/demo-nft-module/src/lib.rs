#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod call;
pub use call::CallMessage;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(feature = "native")]
pub use query::{NonFungibleTokenRpcImpl, NonFungibleTokenRpcServer, CollectionResponse};
use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo};
use sov_state::WorkingSet;
mod offchain;
/// Utility functions.
pub mod utils;

#[cfg_attr(
feature = "native",
derive(serde::Serialize),
derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// Defines an nft collection
pub struct Collection<C: Context> {
    /// name of the collection
    pub name: String,
    /// collection address
    pub creator: C::Address,
    /// frozen or not
    pub frozen: bool,
    /// supply
    pub supply: u64,
    /// collection metadata
    pub metadata_url: String,
}

#[cfg_attr(
feature = "native",
derive(serde::Serialize),
derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// Defines an nft
pub struct Nft<C: Context> {
    /// name of the collection
    pub id: u64,
    /// creator of the nft collection
    pub collection_address: C::Address,
    /// owner of nft
    pub owner: C::Address,
    /// frozen or not
    pub frozen: bool,
    /// supply
    pub metadata_url: String,
}

#[derive(ModuleInfo, Clone)]
/// Module for non-fungible tokens (NFT).
/// Each token is represented by a unique ID.
pub struct NonFungibleToken<C: Context> {
    #[address]
    /// The address of the NonFungibleToken module.
    address: C::Address,

    #[state]
    /// Mapping of tokens to their owners
    collections: sov_state::StateMap<C::Address, Collection<C>>,

    #[state]
    /// Mapping of tokens to their owners
    nfts: sov_state::StateMap<(u64, C::Address), Nft<C>>,
}

/// Config for the NonFungibleToken module.
/// Sets admin and existing owners.
pub struct NonFungibleTokenConfig {
}

impl<C: Context> Module for NonFungibleToken<C> {
    type Context = C;

    type Config = NonFungibleTokenConfig;

    type CallMessage = CallMessage<C>;

    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse, Error> {
        let call_result = match msg {
            CallMessage::CreateCollection {
                name, metadata_url
            } => self.create_collection(&name, &metadata_url, context, working_set),
            CallMessage::FreezeCollection {
                collection_name
            } => self.freeze_collection(&collection_name, context, working_set),
            CallMessage::MintNft {
                collection_name, metadata_url, id,mint_to_address, frozen
            } => self.mint_nft(id, &collection_name, &metadata_url, &mint_to_address, frozen, context, working_set),
            CallMessage::UpdateCollection { name, metadata_url } => {
                self.update_collection(&name, &metadata_url,context, working_set)
            },
            CallMessage::TransferNft { collection_address, id, to } => {
                self.transfer_nft(id, &collection_address, &to, context, working_set)
            },
            CallMessage::UpdateNft {collection_address, id, metadata_url, frozen } => {
                self.update_nft(&collection_address, id, metadata_url, frozen, context, working_set)
            },

        };
        Ok(call_result?)
    }
}
