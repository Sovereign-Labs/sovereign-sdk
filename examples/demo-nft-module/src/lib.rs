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
/// Utility functions.
pub mod utils;

#[cfg_attr(
feature = "native",
derive(serde::Serialize),
derive(serde::Deserialize),
derive(schemars::JsonSchema),
schemars(bound = "C::Address: ::schemars::JsonSchema", rename = "UserAddress"),
)]
#[cfg_attr(feature = "native", serde(transparent))]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize,Clone, Debug, PartialEq, Eq, Hash)]
/// A newtype that represents an owner address
/// (creator of collection, owner of an nft)
pub struct UserAddress<C: Context>(pub C::Address);

#[cfg(all(feature = "native"))]
#[derive(
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash
)]
#[cfg_attr(feature = "native", serde(transparent))]
#[cfg_attr(feature = "native",schemars(bound = "C::Address: ::schemars::JsonSchema",rename = "CollectionAddress"))]
/// Collection address is an address derived deterministically using
/// the collection name and the address of the creator (UserAddress)
pub struct CollectionAddress<C: Context>(pub C::Address);

#[cfg(not(feature = "native"))]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Clone, Debug, PartialEq, Eq, Hash)]
/// Collection address is an address derived deterministically using
/// the collection name and the address of the creator (UserAddress)
pub struct CollectionAddress<C: Context>(pub C::Address);


impl<C: Context> ToString for UserAddress<C>
    where
        C::Address: ToString,
{
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl<C: Context> ToString for CollectionAddress<C>
    where
        C::Address: ToString,
{
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl<C: Context> AsRef<[u8]> for UserAddress<C>
    where
        C::Address: AsRef<[u8]>,
{
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<C: Context> AsRef<[u8]> for CollectionAddress<C>
    where
        C::Address: AsRef<[u8]>,
{
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// tokenId for the NFT that's unique within the scope of the collection
pub type TokenId = u64;

#[cfg_attr(
feature = "native",
derive(serde::Serialize),
derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize,Clone, Debug, PartialEq, Eq, Hash)]
/// A simple wrapper struct to mark an NFT identifier as a combination of
/// a token id (u64) and a collection address
pub struct NftIdentifier<C: Context>(pub TokenId, pub CollectionAddress<C>);

#[cfg_attr(
feature = "native",
derive(serde::Serialize),
derive(serde::Deserialize),
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// Defines an nft collection
pub struct Collection<C: Context> {
    /// Name of the collection
    /// The name has to be unique in the scope of a creator. A single creator address cannot have
    /// duplicate collection names
    pub name: String,
    /// Address of the collection creator
    /// This is the only address that can mint new NFTs for the collection
    pub creator: UserAddress<C>,
    /// If a collection is frozen, then new NFTs
    /// cannot be minted and the supply is frozen
    pub frozen: bool,
    /// Supply of the collection. This is dynamic and changes
    /// with the number of NFTs created. It stops changing
    /// when frozen is set to true.
    pub supply: u64,
    /// collection metadata stored at this url
    pub collection_uri: String,
}

#[cfg_attr(
feature = "native",
derive(serde::Serialize),
derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// Defines an nft
pub struct Nft<C: Context> {
    /// A token id that uniquely identifies an NFT within the scope of a (collection name, creator)
    pub token_id: TokenId,
    /// A collection address that uniquely identifies a collection - derived from (collection name, creator)
    pub collection_address: CollectionAddress<C>,
    /// Owner address of a specific token_id within a collection
    pub owner: UserAddress<C>,
    /// A frozen NFT cannot have its data altered and is immutable
    /// Cannot be unfrozen. token_uri cannot be modified
    pub frozen: bool,
    /// A URI pointing to the offchain metadata
    pub token_uri: String,
}
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo, Clone)]
/// Module for non-fungible tokens (NFT).
/// Each token is represented by a unique ID.
pub struct NonFungibleToken<C: Context> {
    #[address]
    /// The address of the NonFungibleToken module.
    address: C::Address,

    #[state]
    /// Mapping of tokens to their owners
    collections: sov_state::StateMap<CollectionAddress<C>, Collection<C>>,

    #[state]
    /// Mapping of tokens to their owners
    nfts: sov_state::StateMap<NftIdentifier<C>, Nft<C>>,
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
                name, collection_uri
            } => self.create_collection(&name, &collection_uri, context, working_set),
            CallMessage::FreezeCollection {
                collection_name
            } => self.freeze_collection(&collection_name, context, working_set),
            CallMessage::MintNft {
                collection_name, token_uri, token_id,owner, frozen
            } => self.mint_nft(token_id, &collection_name, &token_uri, &owner, frozen, context, working_set),
            CallMessage::UpdateCollection { name, collection_uri } => {
                self.update_collection(&name, &collection_uri,context, working_set)
            },
            CallMessage::TransferNft { collection_address, token_id, to } => {
                self.transfer_nft(token_id, &collection_address, &to, context, working_set)
            },
            CallMessage::UpdateNft {collection_address, token_id, token_uri, frozen } => {
                self.update_nft(&collection_address, token_id, token_uri, frozen, context, working_set)
            },

        };
        Ok(call_result?)
    }
}
