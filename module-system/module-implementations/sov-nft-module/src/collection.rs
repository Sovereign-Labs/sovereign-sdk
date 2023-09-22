use anyhow::{anyhow, bail, Context as _};
use sov_modules_api::{Context, StateMap, WorkingSet};

use crate::address::CollectionAddress;
use crate::utils::get_collection_address;
use crate::CreatorAddress;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// Defines an nft collection
pub struct Collection<C: Context> {
    /// Name of the collection
    /// The name has to be unique in the scope of a creator. A single creator address cannot have
    /// duplicate collection names
    name: String,
    /// Address of the collection creator
    /// This is the only address that can mint new NFTs for the collection
    creator: CreatorAddress<C>,
    /// If a collection is frozen, then new NFTs
    /// cannot be minted and the supply is frozen
    frozen: bool,
    /// Supply of the collection. This is dynamic and changes
    /// with the number of NFTs created. It stops changing
    /// when frozen is set to true.
    supply: u64,
    /// collection metadata stored at this url
    collection_uri: String,
}

pub enum CollectionState<C: Context> {
    Frozen(Collection<C>),
    Mutable(MutableCollection<C>),
}

impl<C: Context> CollectionState<C> {
    pub fn get_mutable_or_bail(&self) -> anyhow::Result<MutableCollection<C>> {
        match self {
            CollectionState::Frozen(collection) => bail!(
                "Collection with name: {} , creator: {} is frozen",
                collection.get_name(),
                collection.get_creator()
            ),
            CollectionState::Mutable(mut_collection) => Ok(mut_collection.clone()),
        }
    }
}

impl<C: Context> Collection<C> {
    pub fn new(
        collection_name: &str,
        collection_uri: &str,
        collections: &StateMap<CollectionAddress<C>, Collection<C>>,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<(CollectionAddress<C>, Collection<C>)> {
        let creator = context.sender();
        let collection_address = get_collection_address(collection_name, creator.as_ref());
        let collection = collections.get(&collection_address, working_set);
        if collection.is_some() {
            Err(anyhow!(
                "Collection with name: {} already exists creator {}",
                collection_name,
                creator
            ))
        } else {
            Ok((
                collection_address,
                Collection {
                    name: collection_name.to_string(),
                    creator: CreatorAddress::new(creator),
                    frozen: false,
                    supply: 0,
                    collection_uri: collection_uri.to_string(),
                },
            ))
        }
    }

    pub fn get_owned_collection(
        collection_name: &str,
        collections: &StateMap<CollectionAddress<C>, Collection<C>>,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<(CollectionAddress<C>, CollectionState<C>)> {
        let creator = context.sender();
        let collection_address = get_collection_address(collection_name, creator.as_ref());
        let collection = collections.get(&collection_address, working_set);
        if let Some(collection) = collection {
            if collection.is_frozen() {
                Ok((collection_address, CollectionState::Frozen(collection)))
            } else {
                Ok((
                    collection_address,
                    CollectionState::Mutable(MutableCollection(collection)),
                ))
            }
        } else {
            Err(anyhow!("Collection not found")).with_context(|| {
                format!(
                    "Collection with name: {} does not exist for creator {}",
                    collection_name, creator
                )
            })
        }
    }

    // Allow dead code used to suppress warnings when native feature flag is not used
    // 1. The getters are primarily used by rpc which is not native
    // 2. The getters can still be used by other modules in the future

    #[allow(dead_code)]
    pub fn get_name(&self) -> &str {
        &self.name
    }
    #[allow(dead_code)]
    pub fn get_creator(&self) -> &CreatorAddress<C> {
        &self.creator
    }
    #[allow(dead_code)]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }
    #[allow(dead_code)]
    pub fn get_supply(&self) -> u64 {
        self.supply
    }
    #[allow(dead_code)]
    pub fn get_collection_uri(&self) -> &str {
        &self.collection_uri
    }
}

// We use a NewType instead of &mut on the Collection because we don't want all
// the members of the struct to be mutable
#[derive(Clone)]
/// NewType representing a mutable (or unfrozen) collection
pub struct MutableCollection<C: Context>(Collection<C>);

/// Member Functions to allow controlled mutability for the Collection struct
/// Can only freeze. Cannot unfreeze
/// Can modify collection_uri
/// Can increment supply. Cannot decrement
/// Cannot modify creator address
/// Cannot modify name
impl<C: Context> MutableCollection<C> {
    pub fn inner(&self) -> &Collection<C> {
        &self.0
    }
    pub fn freeze(&mut self) {
        self.0.frozen = true;
    }

    pub fn set_collection_uri(&mut self, collection_uri: &str) {
        self.0.collection_uri = collection_uri.to_string();
    }

    pub fn increment_supply(&mut self) {
        self.0.supply += 1;
    }
}
