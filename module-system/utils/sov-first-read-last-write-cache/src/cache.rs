use alloc::vec::Vec;

use sov_rollup_interface::maybestd::collections::hash_map::Entry;
use sov_rollup_interface::maybestd::collections::HashMap;

use crate::access::{Access, MergeError};
use crate::{CacheKey, CacheValue};

#[derive(Debug, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum ReadError {
    #[cfg_attr(
        feature = "std",
        error("inconsistent read, expected: {expected:?}, found: {found:?}")
    )]
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

/// CacheLog keeps track of the original and current values of each key accessed.
/// By tracking original values, we can detect and eliminate write patterns where a key is
/// changed temporarily and then reset to its original value
#[derive(Default)]
pub struct CacheLog {
    log: HashMap<CacheKey, Access>,
}

impl CacheLog {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            log: HashMap::with_capacity(capacity),
        }
    }
}

impl CacheLog {
    pub fn take_writes(self) -> Vec<(CacheKey, Option<CacheValue>)> {
        self.log
            .into_iter()
            .filter_map(|(k, v)| filter_writes(k, v))
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

    pub fn merge_writes_left(&mut self, rhs: Self) -> Result<(), MergeError> {
        self.merge_left_with_filter_map(rhs, |(key, access)| match access {
            Access::Read(_) => None,
            Access::ReadThenWrite { modified, .. } => Some((key, Access::Write(modified))),
            Access::Write(w) => Some((key, Access::Write(w))),
        })
    }

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

    pub fn len(&self) -> usize {
        self.log.len()
    }

    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }
}

fn filter_writes(k: CacheKey, access: Access) -> Option<(CacheKey, Option<CacheValue>)> {
    match access {
        Access::Read(_) => None,
        Access::ReadThenWrite { modified, .. } => Some((k, modified)),
        Access::Write(write) => Some((k, write)),
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use proptest::prelude::*;

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

        test_merge_ok_helper(test_cases);
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

            let test_cases = vec![
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
}
