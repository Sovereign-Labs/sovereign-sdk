//! A storage cache for items that are "pinned" in RAM - meaning that we can always fetch their values without touching disk. 
//! 
//! In the current implementation of the SDK, we maintain two copies of the currently pinned state - one each for the sequencer and the node.
//! Because the set of pinned items is expected to be large, we have to be careful not to clone or create these copies unnecessarily.
//! 
//! Currently, accessors are responsible for updating their local copy of the pinned cache when they perform writes. This means 
//! that we can avoid any synchronization overhead when reading/writing the pinned cache, but requires us to keep separate copies for
//! each executor. We instantiate the pinned cache once in the full node and pass it between executors through storage. At the beginning of `apply_slot`,
//! we check whether the current executors parent (i.e. the one from the previous block) left a pinned cache in storage. If so, we take it. If not,
//! we populate by iterating over the database to find all pinned keys. (Caches are currently put silently into storage during `materialize_slot`).
//! 
//! In the full node, we create a pinned cache after startup/resync and pass it back and forth between executors through the shared StateCheckpoint.
//! When we create the checkpoint at initialization, we create the cache once. With each successive block, we `take` the cache from the previous block
//! executor and pass it to the next one. 
//! 
//! Note that the semantics of the pinned cache are such that it is always guaranteed to match the database *plus* any writes from the current statecheckpoint.
//! This means that if we don't want to give a particular checkpoint a copy of the pinned cache, we can simply not populate it and the results will be correct.


// Add `Option<Map<BucketId<BucketStorage>>>`
// 	- BucketId is a prefix of the slotkey
// 	- `BucketStorage` is `{ items: HashmapMap<Key, Value>, size: usize}`

use std::collections::{BTreeMap, HashMap};

use crate::{NativeStorage, Prefix, SlotKey, SlotValue};


#[derive(Debug)]
struct StateItemCache {
    bucket_key_length: usize,
    // Maps the first `bucket_key_length` bytes of a slot key to a bucket of k/v pairs. 
    // Buckets have separate size limits; a common use case would be to have one bucket for each EVM smart contract.
    items: BTreeMap<BucketId, BucketStorage>
}

/// A cache for state items that are pinned in RAM. When querying for pinned state, we know that the item
/// is never both present on disk and absent from this cache, so we never have to fall down to disk. 
/// 
/// State is "Pinned" in buckets, where buckets are simply prefixes of a slot key. for example, you might pin the bucket
/// [0, 2] || 0x1234567890abcdef. Then any queries for keys that start with `0x1234567890abcdef` in the statemap with id [0, 2] will be served from RAM
/// without checking disk.
/// 
/// This cache has a 3-level hierarchy. 
/// - Each statemap has a separate StateItemCache.
/// - Each StateItemCache has a number of different buckets for grouping related state. (for example, each EVM contract gets its own bucket)
/// - Individual buckets contain actual k/v pairs. Each bucket has its own size limit.
#[derive(Debug, Default)]
pub struct PinnedCache {
    // Maps the prefix (which uniquely identifies a statemap) to the state item cache. We need a separate item cache
    // for each statemap because different statemaps may have different key lengths.
    // For example, EVM contract storage uses the contract address (20 bytes) as the bucket key,
    // while standard sov state maps will use a 32 byte module ID as the bucket key.
    item_caches: BTreeMap<Prefix, StateItemCache>,
}

impl PinnedCache {
    /// Get a mutable reference to the bucket for a given key.
    pub fn bucket_for_mut(&mut self, key: &SlotKey) -> Option<&mut BucketStorage> {
        let prefix = key.prefix();
        let state_item_cache = self.item_caches.get_mut(&prefix)?;
        let key_length = state_item_cache.bucket_key_length;
        let relevant_key = BucketId(key.truncate_to(key_length as usize)?);
        state_item_cache.items.get_mut(&relevant_key)
    }

    /// Get a reference to the bucket for a given key.
    pub fn bucket_for(&self, key: &SlotKey) -> Option<&BucketStorage> {
        let prefix = key.prefix();
        let state_item_cache = self.item_caches.get(&prefix)?;
        let key_length = state_item_cache.bucket_key_length;
        let relevant_key = BucketId(key.truncate_to(key_length as usize)?);
        state_item_cache.items.get(&relevant_key)
    }
   
}

/// A unique identifier for a bucket in the pinned cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct BucketId(SlotKey);

/// A size-limited storage bucket for key-value pairs.
#[derive(Debug, Default)]
pub struct BucketStorage {
    items: HashMap<SlotKey, SlotValue>,
    current_size: usize,
    max_size: usize,
}

impl BucketStorage {
    /// Get a value from the bucket.
    pub fn get(&self, key: &SlotKey) -> Option<&SlotValue> {
        self.items.get(key)
    }

    /// Try to insert a key-value pair into the bucket. Returns false if the item could not be inserted (due to exceeding the size limit)
    pub fn try_insert(&mut self, key: SlotKey, value: SlotValue) -> bool {
        let size: usize = value.size().try_into().expect("Unable to cast u32 to usize. This is impossible on 64-bit platforms.");
        if self.current_size.saturating_add(size) > self.max_size {
            tracing::warn!("Bucket {} is over its size limit {} after adding item with size {}. Dropping the bucket.", key, self.max_size, size);
            return false;
        }
        self.items.insert(key, value);
        self.current_size += size;
        return true;
    }

    /// Delete a key from the bucket.
    pub fn delete(&mut self, key: &SlotKey) {
        let size = self.items.remove(key).map(|v| v.size()).unwrap_or(0);
        let size: usize = size.try_into().expect("Unable to cast u32 to usize. This is impossible on 64-bit platforms.");
        self.current_size.checked_sub(size).expect("Bucket size underflowed! This is a bug in the implementaiton of the pinned cache.");
    }
}

/// The outcome of trying to load a bucket into the pinned cache.
pub enum LoadBucketOutcome {
    /// The bucket was loaded successfully.
    Loaded,
    /// The bucket was not loaded because it exceeded the size limit.
    OverSizeLimit,
    /// The bucket was not loaded because it is already present in the cache.
    AlreadyPresent,
    /// The bucket was not loaded because the provided storage doesn't support iteration.
    NotSupportedByStorage,
}

impl PinnedCache {
    #[cfg(feature = "native")]
    /// Try to load a bucket from storage into the pinned cache if it is not already present.
    pub fn try_load_bucket_if_absent<S: NativeStorage>(&mut self, bucket_id: BucketId, storage: &S, max_size: usize) -> anyhow::Result<LoadBucketOutcome> {

        // If we've already loaded the bucket, we can return early.
        if self.bucket_for(&bucket_id.0).is_some() {
            return Ok(LoadBucketOutcome::AlreadyPresent);
        }
        
        // If the storage doesn't support iteration, we can't load the bucket. Return early.
        let Some(iter) = storage.maybe_iter_user_values_with_prefix(bucket_id.0.clone())? else {
            return Ok(LoadBucketOutcome::NotSupportedByStorage);
        };

        let mut bucket_storage = BucketStorage {
            items: HashMap::new(),
            current_size: 0,
            max_size,
        };
        let key_length = bucket_id.0.len().checked_sub(2).ok_or_else(|| anyhow::anyhow!("Bucket ID is too short to have a prefix"))?;
       
        let state_item_cache = self.item_caches.entry(bucket_id.0.prefix()).or_insert(StateItemCache { bucket_key_length: key_length, items: Default::default()});

        for (key, value) in  iter {
            assert!(key.as_ref().starts_with(bucket_id.0.as_ref()), "Key {} does not fall under bucket {:?}. This is a bug in the implementaiton of maybe_iter_user_values_with_prefix; please report it.", key, bucket_id);
            if !bucket_storage.try_insert(key, value) {
                return Ok(LoadBucketOutcome::OverSizeLimit);
            }
        }
        state_item_cache.items.insert(bucket_id, bucket_storage);

        Ok(LoadBucketOutcome::Loaded)

    }
}
