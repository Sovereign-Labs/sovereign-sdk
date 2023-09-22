use anyhow::{anyhow, bail, Context as _};
use sov_modules_api::{Context, StateMap, WorkingSet};

use crate::address::{AuthorizedMinterAddress, CollectionAddress};
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
    creator: CreatorAddress<C>,
    /// Supply of the collection. This is dynamic and changes
    /// with the number of NFTs created. It stops changing
    /// when the collection is frozen
    supply: u64,
    /// Collection metadata stored at this url
    collection_uri: String,
    /// Authorized minters for the collection.
    /// If authorized minters is empty, then new NFTs cannot be minted
    authorized_minters: Vec<AuthorizedMinterAddress<C>>,
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
        authorized_minters: &[AuthorizedMinterAddress<C>],
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
                    authorized_minters: authorized_minters.to_vec(),
                    supply: 0,
                    collection_uri: collection_uri.to_string(),
                },
            ))
        }
    }

    pub fn get_authorized_collection(
        collection_address: &CollectionAddress<C>,
        collections: &StateMap<CollectionAddress<C>, Collection<C>>,
        authorized_address: &AuthorizedMinterAddress<C>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CollectionState<C>> {
        let collection = collections.get(collection_address, working_set);
        if let Some(collection) = collection {
            if collection.is_frozen() {
                Ok(CollectionState::Frozen(collection))
            } else {
                if !collection.authorized_minters.contains(authorized_address) {
                    return Err(anyhow!("sender not authorized")).with_context(|| {
                        format!(
                            "Sender with address: {} not authorized for collection with address: {}",
                            authorized_address, collection_address
                        )
                    });
                }
                Ok(CollectionState::Mutable(MutableCollection(collection)))
            }
        } else {
            Err(anyhow!("Collection not found")).with_context(|| {
                format!(
                    "Collection with address: {} does not exist",
                    collection_address
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
        self.authorized_minters.is_empty()
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
        self.0.authorized_minters = vec![];
    }

    pub fn set_collection_uri(&mut self, collection_uri: &str) {
        self.0.collection_uri = collection_uri.to_string();
    }

    pub fn increment_supply(&mut self) {
        self.0.supply += 1;
    }
}
