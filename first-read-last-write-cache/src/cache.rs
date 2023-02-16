use crate::access::{Access, MergeError};
use crate::{CacheKey, CacheValue};
use std::collections::{hash_map::Entry, HashMap};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug, Eq, PartialEq)]
pub enum ReadError {
    #[error("inconsistent read, expected: {expected:?}, found: {found:?}")]
    InconsistentRead {
        expected: Option<CacheValue>,
        found: Option<CacheValue>,
    },
}

/// Cache entry can be in three states:
/// - Does not exists, a given key was never inserted in the cache:             
///     ValueExists::No
/// - Exists but the value is empty.
///      ValueExists::Yes(None)
/// - Exists and contains a value:
///     ValueExists::Yes(Some(value))
pub enum ValueExists {
    Yes(Option<CacheValue>),
    No,
}

/// CacheLog keeps track of the first write and the last read for a given key.
#[derive(Default)]
pub struct CacheLog {
    log: HashMap<CacheKey, Access>,
}

/// Represents all reads from a CacheLog.
#[derive(Default, Clone, Debug)]
pub struct FirstReads {
    reads: Arc<HashMap<CacheKey, Option<CacheValue>>>,
}

impl FirstReads {
    pub fn new(reads: HashMap<CacheKey, Option<CacheValue>>) -> Self {
        Self {
            reads: Arc::new(reads),
        }
    }

    /// Returns a value corresponding to the key.
    pub fn get(&self, key: &CacheKey) -> ValueExists {
        match self.reads.get(key) {
            Some(read) => ValueExists::Yes(read.clone()),
            None => ValueExists::No,
        }
    }
}

impl CacheLog {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            log: HashMap::with_capacity(capacity),
        }
    }
}

impl CacheLog {
    /// Returns all reads from the CacheLog.
    pub fn get_first_reads(&self) -> FirstReads {
        let reads = self
            .log
            .iter()
            .filter_map(|(k, v)| filter_first_reads(k.clone(), v.clone()))
            .collect::<HashMap<_, _>>();

        FirstReads::new(reads)
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
    pub fn merge(self, mut rhs: Self) -> Result<Self, MergeError> {
        let mut new_cache = CacheLog::with_capacity(self.log.len() + rhs.log.len());

        for (self_key, self_access) in self.log {
            match rhs.log.remove(&self_key) {
                Some(rhs_access) => {
                    let merged = self_access.merge(rhs_access)?;
                    new_cache.log.insert(self_key, merged);
                }
                None => {
                    new_cache.log.insert(self_key, self_access);
                }
            }
        }

        // Insert remaining entries from the rhs to the new_cache.
        new_cache.log.extend(rhs.log);
        Ok(new_cache)
    }
}

fn filter_first_reads(k: CacheKey, access: Access) -> Option<(CacheKey, Option<CacheValue>)> {
    match access {
        Access::Read(read) => Some((k, read)),
        Access::ReadThenWrite { original, .. } => Some((k, original)),
        Access::Write(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_util::{create_key, create_value};

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
        let test_cases = vec![
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

        let mut left_cache = CacheLog::default();
        let mut right_cache = CacheLog::default();

        for TestCase { left, right } in test_cases.clone() {
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

        let merged = left_cache.merge(right_cache).unwrap();
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

    #[test]
    fn test_merge_fail() {
        let test_cases = vec![
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

        for TestCase { left, right } in test_cases {
            let mut left_cache = CacheLog::default();
            let mut right_cache = CacheLog::default();

            match (left, right) {
                (None, None) => {}
                (None, Some(rw)) => right_cache.add_to_cache(rw).unwrap(),
                (Some(rw), None) => left_cache.add_to_cache(rw).unwrap(),
                (Some(left_rw), Some(right_rw)) => {
                    left_cache.add_to_cache(left_rw).unwrap();
                    right_cache.add_to_cache(right_rw).unwrap();
                }
            }

            let result = left_cache.merge(right_cache);

            // Assert that merge failed
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_first_reads() {
        let mut cache = CacheLog::default();
        let entries = vec![
            new_cache_entry(1, 11),
            new_cache_entry(2, 22),
            new_cache_entry(3, 33),
        ];

        for entry in entries.clone() {
            cache.add_read(entry.key, entry.value).unwrap();
        }

        let first_reads = cache.get_first_reads();

        for entry in entries {
            match first_reads.get(&entry.key) {
                ValueExists::Yes(value) => assert_eq!(entry.value, value),
                ValueExists::No => unreachable!(),
            }
        }
    }
}
