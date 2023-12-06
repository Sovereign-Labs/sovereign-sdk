//! Cache key/value definitions

use alloc::vec::Vec;
use core::fmt;

use sov_rollup_interface::maybestd::collections::hash_map::Entry;
use sov_rollup_interface::maybestd::collections::HashMap;
use sov_rollup_interface::maybestd::RefCount;

use crate::common::{MergeError, ReadError};
use crate::storage::{Storage, StorageKey, StorageValue};

/// A key for a cache set.
#[derive(Debug, Eq, PartialEq, Clone, Hash, PartialOrd, Ord)]
pub struct CacheKey {
    /// The key of the cache entry.
    pub key: RefCount<Vec<u8>>,
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO revisit how we display keys
        write!(f, "{:?}", self.key)
    }
}

/// A value stored in the cache.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct CacheValue {
    /// The value of the cache entry.
    pub value: RefCount<Vec<u8>>,
}

impl fmt::Display for CacheValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO revisit how we display values
        write!(f, "{:?}", self.value)
    }
}

/// `Access` represents a sequence of events on a particular value.
/// For example, a transaction might read a value, then take some action which causes it to be updated
/// The rules for defining causality are as follows:
/// 1. If a read is preceded by another read, check that the two reads match and discard one.
/// 2. If a read is preceded by a write, check that the value read matches the value written. Discard the read.
/// 3. Otherwise, retain the read.
/// 4. A write is retained unless it is followed by another write.
#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) enum Access {
    Read(Option<CacheValue>),
    ReadThenWrite {
        original: Option<CacheValue>,
        modified: Option<CacheValue>,
    },
    Write(Option<CacheValue>),
}

impl Access {
    pub fn last_value(&self) -> &Option<CacheValue> {
        match self {
            Access::Read(value) => value,
            Access::ReadThenWrite { modified, .. } => modified,
            Access::Write(value) => value,
        }
    }

    pub fn write_value(&mut self, new_value: Option<CacheValue>) {
        match self {
            // If we've already read this slot, turn it into a readThenWrite access
            Access::Read(original) => {
                // If we're resetting the key to its original value, we can just discard the write history
                if original == &new_value {
                    return;
                }
                // Otherwise, keep track of the original value and the new value
                *self = Access::ReadThenWrite {
                    original: original.take(),

                    modified: new_value,
                };
            }
            // For ReadThenWrite override the modified value with a new value
            Access::ReadThenWrite { original, modified } => {
                // If we're resetting the key to its original value, we can just discard the write history
                if original == &new_value {
                    *self = Access::Read(new_value)
                } else {
                    *modified = new_value
                }
            }
            // For Write override the original value with a new value
            // We can do this unconditionally, since overwriting a value with itself is a no-op
            Access::Write(value) => *value = new_value,
        }
    }

    pub fn merge(&mut self, rhs: Self) -> Result<(), MergeError> {
        // Pattern matching on (`self`, rhs) is a bit cleaner, but would move the `self` inside the tuple.
        // We need the `self` later on for *self = Access.. therefore the nested solution.
        match self {
            Access::Read(left_read) => match rhs {
                Access::Read(right_read) => {
                    if left_read != &right_read {
                        Err(MergeError::ReadThenRead {
                            left: left_read.clone(),
                            right: right_read,
                        })
                    } else {
                        Ok(())
                    }
                }
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                } => {
                    if left_read != &right_original {
                        Err(MergeError::ReadThenRead {
                            left: left_read.clone(),
                            right: right_original,
                        })
                    } else {
                        *self = Access::ReadThenWrite {
                            original: right_original,
                            modified: right_modified,
                        };

                        Ok(())
                    }
                }
                Access::Write(right_write) => {
                    *self = Access::ReadThenWrite {
                        original: left_read.take(),
                        modified: right_write,
                    };
                    Ok(())
                }
            },
            Access::ReadThenWrite {
                original: left_original,
                modified: left_modified,
            } => match rhs {
                Access::Read(right_read) => {
                    if left_modified != &right_read {
                        Err(MergeError::WriteThenRead {
                            write: left_modified.clone(),
                            read: right_read,
                        })
                    } else {
                        Ok(())
                    }
                }
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                } => {
                    if left_modified != &right_original {
                        Err(MergeError::WriteThenRead {
                            write: left_modified.clone(),
                            read: right_original,
                        })
                    } else {
                        *self = Access::ReadThenWrite {
                            original: left_original.take(),
                            modified: right_modified,
                        };
                        Ok(())
                    }
                }
                Access::Write(right_write) => {
                    *self = Access::ReadThenWrite {
                        original: left_original.take(),
                        modified: right_write,
                    };
                    Ok(())
                }
            },
            Access::Write(left_write) => match rhs {
                Access::Read(right_read) => {
                    if left_write != &right_read {
                        Err(MergeError::WriteThenRead {
                            write: left_write.clone(),
                            read: right_read,
                        })
                    } else {
                        Ok(())
                    }
                }
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                } => {
                    if left_write != &right_original {
                        Err(MergeError::WriteThenRead {
                            write: left_write.clone(),
                            read: right_original,
                        })
                    } else {
                        *self = Access::Write(right_modified);
                        Ok(())
                    }
                }
                Access::Write(right_write) => {
                    *self = Access::Write(right_write);
                    Ok(())
                }
            },
        }
    }
}

/// Cache entry can be in three states:
/// - Does not exists, a given key was never inserted in the cache:
///     ValueExists::No
/// - Exists but the value is empty.
///      ValueExists::Yes(None)
/// - Exists and contains a value:
///     ValueExists::Yes(Some(value))
pub enum ValueExists {
    /// The key exists in the cache.
    Yes(Option<CacheValue>),
    /// The key does not exist in the cache.
    No,
}

/// CacheLog keeps track of the original and current values of each key accessed.
/// By tracking original values, we can detect and eliminate write patterns where a key is
/// changed temporarily and then reset to its original value
#[derive(Default)]
pub struct CacheLog {
    log: HashMap<CacheKey, Access>,
}

impl CacheLog {
    /// Creates a cache log with the provided map capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            log: HashMap::with_capacity(capacity),
        }
    }
}

impl CacheLog {
    /// Returns the owned set of key/value pairs of the cache.
    pub fn take_writes(self) -> Vec<(CacheKey, Option<CacheValue>)> {
        self.log
            .into_iter()
            .filter_map(|(k, v)| match v {
                Access::Read(_) => None,
                Access::ReadThenWrite { modified, .. } => Some((k, modified)),
                Access::Write(write) => Some((k, write)),
            })
            .collect()
    }

    /// Returns a value corresponding to the key.
    pub fn get_value(&self, key: &CacheKey) -> ValueExists {
        match self.log.get(key) {
            Some(value) => ValueExists::Yes(value.last_value().clone()),
            None => ValueExists::No,
        }
    }

    /// The first read for a given key is inserted in the cache. For an existing cache entry
    /// checks if reads are consistent with previous reads/writes.
    pub fn add_read(&mut self, key: CacheKey, value: Option<CacheValue>) -> Result<(), ReadError> {
        match self.log.entry(key) {
            Entry::Occupied(existing) => {
                let last_value = existing.get().last_value().clone();

                if last_value != value {
                    return Err(ReadError::InconsistentRead {
                        expected: last_value,
                        found: value,
                    });
                }
                Ok(())
            }
            Entry::Vacant(vacancy) => {
                vacancy.insert(Access::Read(value));
                Ok(())
            }
        }
    }

    /// Adds a write entry to the cache.
    pub fn add_write(&mut self, key: CacheKey, value: Option<CacheValue>) {
        match self.log.entry(key) {
            Entry::Occupied(mut existing) => {
                existing.get_mut().write_value(value);
            }
            Entry::Vacant(vacancy) => {
                vacancy.insert(Access::Write(value));
            }
        }
    }

    /// Merges two cache logs in a way that preserves the first read (from self) and the last write (from rhs)
    /// for the same key in both caches.
    /// The merge succeeds if the first read in the right cache for a key 'k' is consistent with the last read/write
    /// in the self cache.
    ///
    /// Example:
    ///
    /// Cache1:        Cache2:
    ///     k1 => v1       k1 => v1'
    ///     k2 => v2       k3 => v3
    ///
    /// Merged Cache:
    ///     k1 => v1.merge(v1') <- preserves the first read and the last write for 'k1'
    ///     k2 => v2
    ///     k3 => v3
    pub fn merge_left(&mut self, rhs: Self) -> Result<(), MergeError> {
        self.merge_left_with_filter_map(rhs, Some)
    }

    /// Merges two cache logs in a way that preserves the first read (from self) and the last write (from rhs).
    pub fn merge_writes_left(&mut self, rhs: Self) -> Result<(), MergeError> {
        self.merge_left_with_filter_map(rhs, |(key, access)| match access {
            Access::Read(_) => None,
            Access::ReadThenWrite { modified, .. } => Some((key, Access::Write(modified))),
            Access::Write(w) => Some((key, Access::Write(w))),
        })
    }

    /// Merges two cache logs in a way that preserves the first read (from self) and the last write (from rhs).
    pub fn merge_reads_left(&mut self, rhs: Self) -> Result<(), MergeError> {
        self.merge_left_with_filter_map(rhs, |(key, access)| match access {
            Access::Read(read) => Some((key, Access::Read(read))),
            Access::ReadThenWrite { original, .. } => Some((key, Access::Read(original))),
            Access::Write(_) => None,
        })
    }

    fn merge_left_with_filter_map<F: FnMut((CacheKey, Access)) -> Option<(CacheKey, Access)>>(
        &mut self,
        rhs: Self,
        filter: F,
    ) -> Result<(), MergeError> {
        for (rhs_key, rhs_access) in rhs.log.into_iter().filter_map(filter) {
            match self.log.get_mut(&rhs_key) {
                Some(self_access) => self_access.merge(rhs_access)?,
                None => {
                    self.log.insert(rhs_key, rhs_access);
                }
            };
        }
        Ok(())
    }

    /// Returns the number of entries in the cache.
    pub fn len(&self) -> usize {
        self.log.len()
    }

    /// Returns `true` if the cache is empty, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }
}

/// Caches reads and writes for a (key, value) pair. On the first read the value is fetched
/// from an external source represented by the `ValueReader` trait. On following reads,
/// the cache checks if the value we read was inserted before.
#[derive(Default)]
pub struct StorageInternalCache {
    /// Transaction cache.
    pub tx_cache: CacheLog,
    /// Ordered reads and writes.
    pub ordered_db_reads: Vec<(CacheKey, Option<CacheValue>)>,
    /// Version for versioned usage with cache
    pub version: Option<u64>,
}

impl StorageInternalCache {
    /// Wrapper around default that can create the cache with knowledge of the version
    pub fn new_with_version(version: u64) -> Self {
        StorageInternalCache {
            version: Some(version),
            ..Default::default()
        }
    }

    /// Gets a value from the cache or reads it from the provided `ValueReader`.
    pub fn get_or_fetch<S: Storage>(
        &mut self,
        key: &StorageKey,
        value_reader: &S,
        witness: &S::Witness,
    ) -> Option<StorageValue> {
        let cache_key = key.to_cache_key_version(self.version);
        let cache_value = self.get_value_from_cache(&cache_key);

        match cache_value {
            ValueExists::Yes(cache_value_exists) => cache_value_exists.map(Into::into),
            // If the value does not exist in the cache, then fetch it from an external source.
            ValueExists::No => {
                let storage_value = value_reader.get(key, self.version, witness);
                let cache_value = storage_value.as_ref().map(|v| v.clone().into_cache_value());

                self.add_read(cache_key, cache_value);
                storage_value
            }
        }
    }

    /// Gets a keyed value from the cache, returning a wrapper on whether it exists.
    pub fn try_get(&self, key: &StorageKey) -> ValueExists {
        let cache_key = key.to_cache_key_version(self.version);
        self.get_value_from_cache(&cache_key)
    }

    /// Replaces the keyed value on the storage.
    pub fn set(&mut self, key: &StorageKey, value: StorageValue) {
        let cache_key = key.to_cache_key_version(self.version);
        let cache_value = value.into_cache_value();
        self.tx_cache.add_write(cache_key, Some(cache_value));
    }

    /// Deletes a keyed value from the cache.
    pub fn delete(&mut self, key: &StorageKey) {
        let cache_key = key.to_cache_key_version(self.version);
        self.tx_cache.add_write(cache_key, None);
    }

    fn get_value_from_cache(&self, cache_key: &CacheKey) -> ValueExists {
        self.tx_cache.get_value(cache_key)
    }

    /// Merges the provided `StorageInternalCache` into this one.
    pub fn merge_left(&mut self, rhs: Self) -> Result<(), MergeError> {
        self.tx_cache.merge_left(rhs.tx_cache)
    }

    /// Merges the reads of the provided `StorageInternalCache` into this one.
    pub fn merge_reads_left(&mut self, rhs: Self) -> Result<(), MergeError> {
        self.tx_cache.merge_reads_left(rhs.tx_cache)
    }

    /// Merges the writes of the provided `StorageInternalCache` into this one.
    pub fn merge_writes_left(&mut self, rhs: Self) -> Result<(), MergeError> {
        self.tx_cache.merge_writes_left(rhs.tx_cache)
    }

    fn add_read(&mut self, key: CacheKey, value: Option<CacheValue>) {
        self.tx_cache
            .add_read(key.clone(), value.clone())
            // It is ok to panic here, we must guarantee that the cache is consistent.
            .unwrap_or_else(|e| panic!("Inconsistent read from the cache: {e:?}"));
        self.ordered_db_reads.push((key, value))
    }
}

/// A struct that contains the values read from the DB and the values to be written, both in
/// deterministic order.
#[derive(Debug, Default)]
pub struct OrderedReadsAndWrites {
    /// Ordered reads.
    pub ordered_reads: Vec<(CacheKey, Option<CacheValue>)>,
    /// Ordered writes.
    pub ordered_writes: Vec<(CacheKey, Option<CacheValue>)>,
}

impl From<StorageInternalCache> for OrderedReadsAndWrites {
    fn from(val: StorageInternalCache) -> Self {
        let mut writes = val.tx_cache.take_writes();
        // TODO: Make this more efficient
        writes.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
        Self {
            ordered_reads: val.ordered_db_reads,
            ordered_writes: writes,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use sov_rollup_interface::maybestd::RefCount;

    use super::*;

    pub fn create_key(key: u8) -> CacheKey {
        CacheKey {
            key: RefCount::new(alloc::vec![key]),
        }
    }

    pub fn create_value(v: u8) -> Option<CacheValue> {
        Some(CacheValue {
            value: RefCount::new(alloc::vec![v]),
        })
    }

    impl ValueExists {
        fn get(self) -> Option<CacheValue> {
            match self {
                ValueExists::Yes(value) => value,
                ValueExists::No => unreachable!(),
            }
        }
    }

    #[test]
    fn test_cache_read_write() {
        let mut cache_log = CacheLog::default();
        let key = create_key(1);

        {
            let value = create_value(2);

            cache_log.add_read(key.clone(), value.clone()).unwrap();
            let value_from_cache = cache_log.get_value(&key).get();
            assert_eq!(value_from_cache, value);
        }

        {
            let value = create_value(3);

            cache_log.add_write(key.clone(), value.clone());

            let value_from_cache = cache_log.get_value(&key).get();
            assert_eq!(value_from_cache, value);

            cache_log.add_read(key.clone(), value.clone()).unwrap();

            let value_from_cache = cache_log.get_value(&key).get();
            assert_eq!(value_from_cache, value);
        }
    }

    #[derive(PartialEq, Eq, Clone, Debug)]
    pub(crate) struct CacheEntry {
        key: CacheKey,
        value: Option<CacheValue>,
    }

    impl CacheEntry {
        fn new(key: CacheKey, value: Option<CacheValue>) -> Self {
            Self { key, value }
        }
    }

    fn new_cache_entry(key: u8, value: u8) -> CacheEntry {
        CacheEntry::new(create_key(key), create_value(value))
    }

    #[derive(Clone)]
    enum ReadWrite {
        Read(CacheEntry),
        Write(CacheEntry),
    }

    impl ReadWrite {
        fn get_value(self) -> CacheEntry {
            match self {
                ReadWrite::Read(r) => r,
                ReadWrite::Write(w) => w,
            }
        }

        fn check_cache_consistency(self, rhs: Self, merged: &CacheLog) {
            match (self, rhs) {
                (ReadWrite::Read(left_read), ReadWrite::Read(right_read)) => {
                    assert_eq!(left_read, right_read);
                    let value = merged.get_value(&left_read.key).get();
                    assert_eq!(left_read.value, value)
                }
                (ReadWrite::Read(_), ReadWrite::Write(right_write)) => {
                    let value = merged.get_value(&right_write.key).get();
                    assert_eq!(right_write.value, value)
                }
                (ReadWrite::Write(left_write), ReadWrite::Read(right_write)) => {
                    assert_eq!(left_write, right_write);
                    let value = merged.get_value(&left_write.key).get();
                    assert_eq!(left_write.value, value)
                }
                (ReadWrite::Write(_), ReadWrite::Write(right_write)) => {
                    let value = merged.get_value(&right_write.key).get();
                    assert_eq!(right_write.value, value)
                }
            }
        }
    }

    impl CacheLog {
        fn add_to_cache(&mut self, rw: ReadWrite) -> Result<(), ReadError> {
            match rw {
                ReadWrite::Read(r) => self.add_read(r.key, r.value),
                ReadWrite::Write(w) => {
                    self.add_write(w.key, w.value);
                    Ok(())
                }
            }
        }
    }

    #[derive(Clone)]
    struct TestCase {
        left: Option<ReadWrite>,
        right: Option<ReadWrite>,
    }

    #[test]
    fn test_add_read() {
        let mut cache = CacheLog::default();

        let entry = new_cache_entry(1, 1);

        let res = cache.add_read(entry.key, entry.value);
        assert!(res.is_ok());

        let entry = new_cache_entry(2, 1);
        let res = cache.add_read(entry.key, entry.value);
        assert!(res.is_ok());

        let entry = new_cache_entry(1, 2);
        let res = cache.add_read(entry.key, entry.value);

        assert_eq!(
            res,
            Err(ReadError::InconsistentRead {
                expected: create_value(1),
                found: create_value(2)
            })
        )
    }

    #[test]
    fn test_merge_ok() {
        let test_cases = alloc::vec![
            TestCase {
                left: Some(ReadWrite::Read(new_cache_entry(1, 11))),
                right: Some(ReadWrite::Read(new_cache_entry(1, 11))),
            },
            TestCase {
                left: Some(ReadWrite::Read(new_cache_entry(2, 12))),
                right: Some(ReadWrite::Write(new_cache_entry(2, 22))),
            },
            TestCase {
                left: Some(ReadWrite::Write(new_cache_entry(3, 13))),
                right: Some(ReadWrite::Write(new_cache_entry(3, 23))),
            },
            TestCase {
                left: Some(ReadWrite::Write(new_cache_entry(4, 14))),
                right: None,
            },
            TestCase {
                left: None,
                right: Some(ReadWrite::Read(new_cache_entry(5, 25))),
            },
            TestCase {
                left: None,
                right: Some(ReadWrite::Write(new_cache_entry(6, 25))),
            },
            TestCase {
                left: Some(ReadWrite::Write(new_cache_entry(7, 17))),
                right: Some(ReadWrite::Read(new_cache_entry(7, 17))),
            },
        ];

        test_merge_ok_helper(test_cases);
    }

    #[test]
    fn test_merge_fail() {
        let test_cases = alloc::vec![
            TestCase {
                left: Some(ReadWrite::Read(new_cache_entry(1, 11))),
                // The read is inconsistent with the previous read.
                right: Some(ReadWrite::Read(new_cache_entry(1, 12))),
            },
            TestCase {
                left: Some(ReadWrite::Write(new_cache_entry(2, 12))),
                // The read is inconsistent with the previous write.
                right: Some(ReadWrite::Read(new_cache_entry(2, 22))),
            },
        ];

        let result = test_merge_helper(test_cases);
        assert!(result.is_err());
    }

    proptest! {
        #[test]
        fn test_merge_fuzz(s: u8) {
            let num_cases = 15;
            let mut testvec = Vec::with_capacity(num_cases);

            for i in 0..num_cases {
                testvec.push( s.wrapping_add(i as u8));
            }

            let test_cases = alloc::vec![
                TestCase {
                    left: Some(ReadWrite::Read(new_cache_entry(testvec[0], testvec[1]))),
                    right: Some(ReadWrite::Read(new_cache_entry(testvec[0], testvec[1]))),
                },
                TestCase {
                    left: Some(ReadWrite::Read(new_cache_entry(testvec[2], testvec[3]))),
                    right: Some(ReadWrite::Write(new_cache_entry(testvec[2], testvec[4]))),
                },
                TestCase {
                    left: Some(ReadWrite::Write(new_cache_entry(testvec[5], testvec[6]))),
                    right: Some(ReadWrite::Write(new_cache_entry(testvec[5], testvec[7]))),
                },
                TestCase {
                    left: Some(ReadWrite::Write(new_cache_entry(testvec[8], testvec[9]))),
                    right: None,
                },
                TestCase {
                    left: None,
                    right: Some(ReadWrite::Read(new_cache_entry(testvec[10], testvec[11]))),
                },
                TestCase {
                    left: None,
                    right: Some(ReadWrite::Write(new_cache_entry(testvec[12], testvec[11]))),
                },
                TestCase {
                    left: Some(ReadWrite::Write(new_cache_entry(testvec[13], testvec[14]))),
                    right: Some(ReadWrite::Read(new_cache_entry(testvec[13], testvec[14]))),
                },
            ];

            test_merge_ok_helper(test_cases);
        }
    }

    fn test_merge_ok_helper(test_cases: Vec<TestCase>) {
        let result = test_merge_helper(test_cases.clone());
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.log.len(), test_cases.len());

        for TestCase { left, right } in test_cases {
            match (left, right) {
                (None, None) => unreachable!(),
                (None, Some(rw)) => {
                    let entry = rw.get_value();
                    let value = merged.get_value(&entry.key).get();
                    assert_eq!(entry.value, value)
                }
                (Some(rw), None) => {
                    let entry = rw.get_value();
                    let value = merged.get_value(&entry.key).get();
                    assert_eq!(entry.value, value)
                }
                (Some(left_rw), Some(right_rw)) => {
                    left_rw.check_cache_consistency(right_rw, &merged);
                }
            }
        }
    }

    fn test_merge_helper(test_cases: Vec<TestCase>) -> Result<CacheLog, MergeError> {
        let mut left_cache = CacheLog::default();
        let mut right_cache = CacheLog::default();

        for TestCase { left, right } in test_cases {
            match (left, right) {
                (None, None) => {}
                (None, Some(rw)) => right_cache.add_to_cache(rw).unwrap(),
                (Some(rw), None) => left_cache.add_to_cache(rw).unwrap(),
                (Some(left_rw), Some(right_rw)) => {
                    left_cache.add_to_cache(left_rw).unwrap();
                    right_cache.add_to_cache(right_rw).unwrap();
                }
            }
        }

        left_cache.merge_left(right_cache)?;
        Ok(left_cache)
    }
    #[test]
    fn test_access_read_write() {
        let original_value = create_value(1);
        let mut access = Access::Read(original_value.clone());

        // Check: Read => ReadThenWrite transition
        {
            let new_value = create_value(2);
            access.write_value(new_value.clone());

            assert_eq!(access.last_value(), &new_value);
            assert_eq!(
                access,
                Access::ReadThenWrite {
                    original: original_value.clone(),
                    modified: new_value
                }
            );
        }

        // Check: ReadThenWrite => ReadThenWrite transition
        {
            let new_value = create_value(3);
            access.write_value(new_value.clone());

            assert_eq!(access.last_value(), &new_value);
            assert_eq!(
                access,
                Access::ReadThenWrite {
                    original: original_value,
                    modified: new_value
                }
            );
        }
    }

    #[test]
    fn test_access_write() {
        let original_value = create_value(1);
        let mut access = Access::Write(original_value.clone());

        // Check: Write => Write transition
        {
            assert_eq!(access.last_value(), &original_value);
            let new_value = create_value(3);
            access.write_value(new_value.clone());
            assert_eq!(access.last_value(), &new_value);
            assert_eq!(access, Access::Write(new_value));
        }
    }

    #[test]
    fn test_access_merge() {
        let first_read = 1;
        let mut value = create_value(first_read);
        let mut left = Access::Read(value.clone());

        let last_write = 10;
        for i in 2..last_write + 1 {
            left.merge(Access::Read(value.clone())).unwrap();

            value = create_value(i);
            left.merge(Access::Write(value.clone())).unwrap();
        }

        assert_eq!(
            left,
            Access::ReadThenWrite {
                original: create_value(first_read),
                modified: create_value(last_write)
            }
        )
    }

    #[test]
    fn test_err_merge_left_read_neq_right_read() {
        let first_read = 1;
        let value = create_value(first_read);
        let left = &mut Access::Read(value.clone());

        let second_read = 2;
        let value2 = create_value(second_read);

        assert_eq!(
            left.merge(Access::Read(value2.clone())),
            Err(MergeError::ReadThenRead {
                left: value,
                right: value2,
            })
        );
    }

    #[test]
    fn test_err_merge_left_read_neq_right_orig() {
        let first_read = 1;
        let value = create_value(first_read);
        let left = &mut Access::Read(value.clone());

        let second_read = 2;
        let value2 = create_value(second_read);
        let right = Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        assert_eq!(
            left.merge(right),
            Err(MergeError::ReadThenRead {
                left: value,
                right: value2,
            })
        );
    }

    #[test]
    fn test_err_merge_left_mod_neq_right_read() {
        let first_read = 1;
        let value = create_value(first_read);

        let second_read = 2;
        let value2 = create_value(second_read);

        let left = &mut Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        let right = Access::Read(value2.clone());

        assert_eq!(
            left.merge(right),
            Err(MergeError::WriteThenRead {
                write: value,
                read: value2,
            })
        )
    }

    #[test]
    fn test_err_merge_left_mod_neq_right_orig() {
        let first_read = 1;
        let value = create_value(first_read);

        let second_read = 2;
        let value2 = create_value(second_read);

        let left = &mut Access::ReadThenWrite {
            original: value.clone(),
            modified: value2.clone(),
        };

        let right = Access::ReadThenWrite {
            original: value.clone(),
            modified: value2.clone(),
        };

        assert_eq!(
            left.merge(right),
            Err(MergeError::WriteThenRead {
                write: value2,
                read: value,
            })
        )
    }

    #[test]
    fn test_err_merge_left_right_neq_right_orig() {
        let first_read = 1;
        let value = create_value(first_read);

        let second_read = 2;
        let value2 = create_value(second_read);

        let left = &mut Access::Write(value.clone());
        let right = Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        assert_eq!(
            left.merge(right),
            Err(MergeError::WriteThenRead {
                write: value,
                read: value2,
            })
        )
    }
}
