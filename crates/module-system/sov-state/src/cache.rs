//! Cache key/value definitions

use std::collections::hash_map::Entry;
use std::mem;

use crate::namespaces::ProvableCompileTimeNamespace;
use crate::storage::{SlotKey, SlotValue, Storage};
#[cfg(feature = "native")]
use crate::NativeStorage;
use crate::{NodeLeaf, NodeLeafAndMaybeValue, ReadType};

/// An enum that represents the temperature of a value in the storage.
/// Used in cached-structs to determine whether this is the first read of a value or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsValueCached {
    /// The value is cached plus the last access type. Note that writes superseed reads - if we write
    /// to a value, we will always return `IsValueCached::Yes(Write)` even if the value is read afterwards.
    Yes(AccessSize),
    /// The value is fetched from the storage and was never cached.
    No,
}

/// [`Access`] represents a sequence of events on a particular value.
/// For example, a transaction might read a value, then take some action which causes it to be updated
#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    /// Read access to a storage value.
    Read {
        original: Option<NodeLeafAndMaybeValue>,
    },
    /// Write access to a storage value.
    Write { modified: Option<SlotValue> },
}

/// [`AccessSize`] represents a cache event that occurred on a particular value with the size of the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessSize {
    /// Read access to a storage value.
    Read(u32),
    /// Write access to a storage value.
    Write(u32),
}

impl AccessSize {
    /// Return the size of the value contained in the access.
    pub fn size(&self) -> u32 {
        match self {
            AccessSize::Read(size) | AccessSize::Write(size) => *size,
        }
    }
}

impl Access {
    /// Return the size of the value contained in the access.
    pub fn as_access_size(&self) -> AccessSize {
        match self {
            Access::Read { original } => {
                AccessSize::Read(original.as_ref().map(|node| node.leaf.size).unwrap_or(0))
            }
            Access::Write { modified } => {
                AccessSize::Write(modified.as_ref().map(|v| v.size()).unwrap_or(0))
            }
        }
    }

    fn modified(&self) -> Option<Option<&SlotValue>> {
        match self {
            Access::Read { .. } => None,
            Access::Write { modified } => Some(modified.as_ref()),
        }
    }

    fn modified_mut(&mut self) -> Option<&mut Option<SlotValue>> {
        match self {
            Access::Read { .. } => None,
            Access::Write { modified } => Some(modified),
        }
    }

    fn add_write(&mut self, write: Option<SlotValue>) {
        match self {
            Access::Read { original: _ } => *self = Access::Write { modified: write },
            Access::Write { modified } => {
                // Simply override the modified value with the new modified
                // value.
                *modified = write;
            }
        }
    }
}

mod internal {
    use super::*;
    /// [`CacheLog`] keeps track of the original and current values of each key accessed.
    /// By tracking original values, we can detect and eliminate write patterns where a key is
    /// changed temporarily and then reset to its original value
    #[derive(Default, Debug, Clone)]
    pub(crate) struct CacheLog {
        revertable_log: std::collections::HashMap<SlotKey, Access>,
        log: std::collections::HashMap<SlotKey, Access>,
    }

    impl CacheLog {
        // This method is used to get all the changeset from the cache. The `revertable_log`
        // shoule be either merged or discarded before calling this method.
        pub(crate) fn iter(&self) -> impl Iterator<Item = (&SlotKey, &Access)> {
            assert!(
                self.revertable_log.is_empty(),
                "Revertable cache should be merged or discarded before calling `iter`"
            );
            self.log.iter()
        }

        // This method is used to take all the changeset from the cache. The `revertable_log`
        // shoule be either merged or discarded before calling this method.
        pub(crate) fn take_writes(self) -> Vec<(SlotKey, Option<SlotValue>)> {
            assert!(
                self.revertable_log.is_empty(),
                "Revertable cache should be merged or discarded before calling `take_writes`"
            );
            self.log
                .into_iter()
                .filter_map(|(k, mut access)| access.modified_mut().map(|value| (k, value.take())))
                .collect()
        }

        pub(crate) fn get(&self, key: &SlotKey) -> Option<&Access> {
            self.revertable_log.get(key).or_else(|| self.log.get(key))
        }

        pub(crate) fn get_mut(&mut self, key: &SlotKey) -> Option<&mut Access> {
            self.revertable_log
                .get_mut(key)
                .or_else(|| self.log.get_mut(key))
        }

        // Adds a read entry to the cache. Caller must guarantee that the key is not already present in the cache.
        pub(crate) fn add_read(&mut self, key: SlotKey, value: Option<NodeLeafAndMaybeValue>) {
            // We do the sanity check for `revertable_log` because it is free.
            match self.revertable_log.entry(key) {
                Entry::Occupied(existing) => {
                    // Sanity check.
                    panic!(
                        "Detected multiple calls to `add_read` for the same key.: {:?}",
                        existing.key()
                    );
                }
                Entry::Vacant(vacancy) => vacancy.insert(Access::Read { original: value }),
            };
        }

        // Adds a write entry to the cache.
        pub(crate) fn add_write(
            &mut self,
            key: SlotKey,
            value: Option<SlotValue>,
        ) -> IsValueCached {
            let out = IsValueCached::Yes(AccessSize::Write(
                value.as_ref().map(|v| v.size()).unwrap_or(0),
            ));

            match self.revertable_log.entry(key.clone()) {
                Entry::Occupied(mut existing) => {
                    existing.get_mut().add_write(value);
                    out
                }
                Entry::Vacant(vacancy) => {
                    let out = match self.log.entry(key) {
                        Entry::Occupied(_) => out,
                        Entry::Vacant(_) => IsValueCached::No,
                    };
                    // The write is added only to `revertable_log`.
                    // It will later be either committed or discarded.
                    vacancy.insert(Access::Write { modified: value });
                    out
                }
            }
        }

        pub(crate) fn commit_revertable_log(&mut self) {
            for (k, v) in self.revertable_log.drain() {
                match v {
                    // 1. merge reads
                    Access::Read { original: _ } => {
                        let is_new = self.log.insert(k, v).is_none();
                        assert!(is_new, "The read is already present in the log");
                    }
                    // 2. merge writes
                    Access::Write { modified } => match self.log.entry(k) {
                        Entry::Occupied(mut existing) => {
                            existing.get_mut().add_write(modified);
                        }
                        Entry::Vacant(vacancy) => {
                            vacancy.insert(Access::Write { modified });
                        }
                    },
                }
            }
        }

        pub(crate) fn discard_revertable_log(&mut self) {
            self.revertable_log.clear();
        }
    }
}

use internal::CacheLog;

/// Caches reads and writes for a (key, value) pair. On the first read the value is fetched
/// from an external source represented by the `ValueReader` trait. On following reads,
/// the cache checks if the value we read was inserted before.
#[derive(Default, Debug, Clone)]
pub struct ProvableStorageCache<N> {
    // Transaction cache.
    cache: CacheLog,
    //
    revertable_ordered_reads: Vec<(SlotKey, Option<NodeLeaf>)>,
    // Ordered reads and writes.
    ordered_db_reads: Vec<(SlotKey, Option<NodeLeaf>)>,
    phantom: core::marker::PhantomData<N>,
}

// We implement these methods only for *provable* state values because the internal cache
// Typical workflow for fetching a value from storage:
//
// In NATIVE execution:
// 1. The caller requests the value size by calling `get_size_or_fetch`.
// 2. If the value is accessed for the first time, it is fetched from the DB and stored in the cache as `GetSizeValueFetched(value)`.
// 3. If the caller then wants to read the full value, they call `get_or_fetch`, transitioning `GetSizeValueFetched` to `Read`.
//    The value is then passed to the witness, allowing ZK execution to access it.
//
// In ZK execution:
// 1. The caller requests the value size by calling `get_size_or_fetch`.
// 2. If the value is accessed for the first time, only the `NodeLeaf` is fetched from the witness.
//    This avoids passing the entire value to the witness just to determine its size.
//    Unlike native execution, the full value is *not* stored in the cache when requesting its size.
// 3. If the caller then wants to read the full value, they call `get_or_fetch`, and the value is fetched from the witness.
//
// The key difference is that in ZK mode, values are loaded lazily, only when needed, whereas in native mode, values are eagerly
// fetched and cached—even when only requesting the size. This is because, in native execution, it's acceptable to cache the full
// value, but in ZK execution, arbitrary large values cannot be stored as hints in the witness.
impl<N: ProvableCompileTimeNamespace> ProvableStorageCache<N> {
    /// Commit the revertable part of the `ProvableStorageCache`.
    pub fn commit_revertable_storage_cache(&mut self) {
        let revertable_ordered_reads = mem::take(&mut self.revertable_ordered_reads);
        self.ordered_db_reads.extend(revertable_ordered_reads);
        self.cache.commit_revertable_log();
    }

    /// Discards the revertable part of the `ProvableStorageCache`.
    pub fn discard_revertable_storage_cache(&mut self) {
        self.revertable_ordered_reads.clear();
        self.cache.discard_revertable_log();
    }

    /// Returns an iterator over the writes
    pub fn get_writes(&self) -> impl Iterator<Item = (&SlotKey, Option<&SlotValue>)> {
        self.cache
            .iter()
            .filter_map(|(k, access)| access.modified().map(|v| (k, v)))
    }

    /// Converts the `ProvableStorageCache` into `OrderedReadsAndWrites`.
    pub fn to_ordered_writes_and_reads(mut self) -> OrderedReadsAndWrites {
        self.commit_revertable_storage_cache();
        let mut writes = self.cache.take_writes();
        //This TODO is for performance enhancement, not a security concern.
        // TODO: Make this more efficient
        writes.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
        OrderedReadsAndWrites {
            ordered_reads: self.ordered_db_reads,
            ordered_writes: writes,
        }
    }

    /// Checks if a value corresponding to a given key is cached.
    pub fn is_value_cached(&self, key: &SlotKey) -> IsValueCached {
        if let Some(access) = self.cache.get(key) {
            IsValueCached::Yes(access.as_access_size())
        } else {
            IsValueCached::No
        }
    }

    /// Get the size of the value.
    #[cfg(feature = "native")]
    pub fn get_size_or_fetch_historical<S: Storage + NativeStorage>(
        &mut self,
        key: &SlotKey,
        storage: &S,
        witness: &S::Witness,
        version: Option<sov_rollup_interface::common::SlotNumber>,
    ) -> Option<u32> {
        match self.cache.get(key) {
            Some(Access::Read { original }) => original.as_ref().map(|node| node.leaf.size),
            Some(Access::Write { modified }) => modified.as_ref().map(SlotValue::size),
            None => {
                let maybe_leaf = storage.get_leaf_historical::<N>(key, version, witness);
                let size = maybe_leaf.as_ref().map(|leaf| leaf.leaf.size);
                self.add_read(key.clone(), maybe_leaf);
                size
            }
        }
    }

    /// Get the size of the value.
    pub fn get_size_or_fetch<S: Storage>(
        &mut self,
        key: &SlotKey,
        storage: &S,
        witness: &S::Witness,
    ) -> Option<u32> {
        match self.cache.get(key) {
            Some(Access::Read { original }) => original.as_ref().map(|node| node.leaf.size),
            Some(Access::Write { modified }) => modified.as_ref().map(SlotValue::size),
            None => {
                let maybe_leaf = storage.get_leaf::<N>(key, witness);
                let size = maybe_leaf.as_ref().map(|leaf| leaf.leaf.size);
                self.add_read(key.clone(), maybe_leaf);
                size
            }
        }
    }

    /// Gets a value from the cache or reads it from the provided `ValueReader`.
    pub fn get_or_fetch<S: Storage>(
        &mut self,
        key: &SlotKey,
        storage: &S,
        witness: &S::Witness,
    ) -> Option<SlotValue> {
        self.get_or_fetch_with_fn(
            key,
            storage,
            witness,
            |key, witness, _args| storage.get::<N>(key, witness),
            (),
        )
    }

    fn get_or_fetch_with_fn<S: Storage, F, Args>(
        &mut self,
        key: &SlotKey,
        storage: &S,
        witness: &S::Witness,
        fetch_fn: F,
        args: Args,
    ) -> Option<SlotValue>
    where
        F: Fn(&SlotKey, &S::Witness, Args) -> Option<SlotValue>,
    {
        if let Some(access) = self.cache.get_mut(key) {
            match access {
                Access::Read {
                    original: Some(node),
                } => match node.value.clone() {
                    ReadType::GetSizeValueNotFetched => {
                        let slot_value = fetch_fn(key, witness, args)
                            // This unwrap is justified because in the `ReadType::GetSizeValueFetched` branch,
                            // we inserted `Some(slot_value)`.
                            .unwrap_or_else(|| {
                                panic!("Invalid read for {key:?}, provided witness is invalid")
                            });

                        let node_leaf = NodeLeaf::make_leaf::<S::Hasher>(&slot_value);
                        assert_eq!(node.leaf, node_leaf);

                        node.value = ReadType::Read(slot_value.clone());
                        Some(slot_value)
                    }
                    ReadType::GetSizeValueFetched(slot_value) => {
                        // Insert `slot_value` in the witness
                        storage.put_in_witness(Some(slot_value.clone()), witness);
                        node.value = ReadType::Read(slot_value.clone());
                        Some(slot_value.clone())
                    }
                    ReadType::Read(slot_value) => Some(slot_value),
                },
                Access::Read { original: None } => None,
                Access::Write { modified } => modified.clone(),
            }
        } else {
            let storage_value = fetch_fn(key, witness, args);
            let read = storage_value.clone().map(|v| NodeLeafAndMaybeValue {
                leaf: NodeLeaf::make_leaf::<S::Hasher>(&v),
                value: ReadType::Read(v),
            });
            self.add_read(key.clone(), read);
            storage_value
        }
    }

    #[cfg(feature = "native")]
    /// Gets a value from the cache or reads it from the provided `ValueReader`.
    pub fn get_or_fetch_historical<S: Storage + NativeStorage>(
        &mut self,
        key: &SlotKey,
        storage: &S,
        witness: &S::Witness,
        version: Option<sov_rollup_interface::common::SlotNumber>,
    ) -> Option<SlotValue> {
        self.get_or_fetch_with_fn(
            key,
            storage,
            witness,
            |key, witness, version: Option<sov_rollup_interface::common::SlotNumber>| {
                storage.get_historical::<N>(key, version, witness)
            },
            version,
        )
    }

    /// Replaces the keyed value on the storage.
    pub fn set(&mut self, key: &SlotKey, value: SlotValue) {
        self.cache.add_write(key.clone(), Some(value));
    }

    /// Deletes a keyed value from the cache.
    pub fn delete(&mut self, key: &SlotKey) {
        self.cache.add_write(key.clone(), None);
    }

    // This method can be called only once per given key.
    fn add_read(&mut self, key: SlotKey, node: Option<NodeLeafAndMaybeValue>) {
        self.revertable_ordered_reads
            .push((key.clone(), node.as_ref().map(|n| n.leaf)));

        self.cache.add_read(key, node);
    }
}

/// A struct that contains the values read from the DB and the values to be written, both in
/// deterministic order.
#[derive(Debug, Default)]
#[cfg_attr(feature = "test-utils", derive(Clone))]
pub struct OrderedReadsAndWrites {
    /// Ordered reads.
    pub ordered_reads: Vec<(SlotKey, Option<NodeLeaf>)>,
    /// Ordered writes.
    pub ordered_writes: Vec<(SlotKey, Option<SlotValue>)>,
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for OrderedReadsAndWrites {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        const MAX_LEN: usize = 100;
        let writes_len = u.int_in_range(1..=MAX_LEN)?;

        let mut ordered_writes = Vec::with_capacity(writes_len);
        for _ in 0..writes_len {
            let key = u.arbitrary::<SlotKey>()?;
            let value = u.arbitrary::<Option<SlotValue>>()?;
            ordered_writes.push((key, value));
        }
        Ok(OrderedReadsAndWrites {
            ordered_reads: Vec::new(),
            ordered_writes,
        })
    }
}

/// A struct that contains the read/write sets for the user and kernel namespaces.
#[derive(Debug, Default)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "test-utils", derive(Clone))]
pub struct StateAccesses {
    /// The reads and writes to the user namespace
    pub user: OrderedReadsAndWrites,
    /// The reads and writes to the user namespace
    pub kernel: OrderedReadsAndWrites,
}

#[cfg(test)]
mod tests {
    // Testing `ProvableStorageCache` requires higher-level types from `sov-modules-api`.
    // While adding `sov-modules-api` as a dev-dependency is an option, we chose to place the relevant tests directly in `sov-modules-api` for the following reasons:
    // 1. The tests rely on concepts and types that are more closely related to `sov-modules-api`.
    // 2. The `UniversalStateAccessor` trait is sealed and cannot be exported from `sov-modules-api`.
    use super::*;

    pub fn create_key(key: u8) -> SlotKey {
        SlotKey::from(vec![key])
    }

    pub fn create_value(v: u8) -> Option<SlotValue> {
        Some(SlotValue::from(vec![v]))
    }

    #[test]
    fn test_cache_read_write() {
        let key = create_key(1);

        // Test read.
        {
            let mut cache_log = CacheLog::default();
            let value = create_value(2).map(|v| NodeLeafAndMaybeValue {
                leaf: NodeLeaf::make_leaf::<sha2::Sha256>(&v),
                value: ReadType::Read(v),
            });
            cache_log.add_read(key.clone(), value);

            cache_log.commit_revertable_log();
            let writes = cache_log.take_writes();
            assert_eq!(writes.len(), 0);
        }

        // Test write.
        {
            let mut cache_log = CacheLog::default();
            let value = create_value(3);
            cache_log.add_write(key.clone(), value.clone());

            cache_log.commit_revertable_log();
            let writes = cache_log.take_writes();
            assert_eq!(writes.len(), 1);
            assert_eq!((key.clone(), value), writes[0]);
        }

        // Test that write overrides read.
        {
            let mut cache_log = CacheLog::default();
            let value = create_value(4).map(|v| NodeLeafAndMaybeValue {
                leaf: NodeLeaf::make_leaf::<sha2::Sha256>(&v),
                value: ReadType::Read(v),
            });

            cache_log.add_read(key.clone(), value.clone());

            let next_value = create_value(5);
            cache_log.add_write(key.clone(), next_value.clone());

            cache_log.commit_revertable_log();
            let writes = cache_log.take_writes();
            assert_eq!(writes.len(), 1);
            assert_eq!((key.clone(), next_value), writes[0]);
        }

        // Test that write overrides another write
        {
            let mut cache_log = CacheLog::default();
            let value = create_value(4);
            cache_log.add_write(key.clone(), value.clone());

            let next_value = create_value(5);
            cache_log.add_write(key.clone(), next_value.clone());

            cache_log.commit_revertable_log();
            let writes = cache_log.take_writes();
            assert_eq!(writes.len(), 1);
            assert_eq!((key.clone(), next_value), writes[0]);
        }

        // Test discarded write.
        {
            let mut cache_log = CacheLog::default();
            let value = create_value(3);
            cache_log.add_write(key.clone(), value.clone());

            cache_log.discard_revertable_log();
            let writes = cache_log.take_writes();
            assert_eq!(writes.len(), 0);
        }
    }
}
