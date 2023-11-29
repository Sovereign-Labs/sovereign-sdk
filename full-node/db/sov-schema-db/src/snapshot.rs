//! Snapshot related logic

use std::sync::{Arc, LockResult, Mutex, RwLock, RwLockReadGuard};

use crate::schema::{KeyCodec, ValueCodec};
use crate::schema_batch::SchemaBatchIterator;
use crate::{Operation, Schema, SchemaBatch, SchemaKey, SchemaValue, SeekKeyEncoder};

/// Id of database snapshot
pub type SnapshotId = u64;

/// A trait to make nested calls to several [`SchemaBatch`]s and eventually [`crate::DB`]
pub trait QueryManager {
    /// Iterator over key-value pairs in reverse lexicographic order in given [`Schema`]
    type Iter<'a, S: Schema>: Iterator<Item = (SchemaKey, SchemaValue)>
    where
        Self: 'a;
    /// Get a value from parents of given [`SnapshotId`]
    /// In case of unknown [`SnapshotId`] return `Ok(None)`
    fn get<S: Schema>(
        &self,
        snapshot_id: SnapshotId,
        key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<S::Value>>;

    /// Returns an iterator over all key-value pairs in given [`Schema`] in reverse lexicographic order
    /// Starting from given [`SnapshotId`]
    fn iter<S: Schema>(&self, snapshot_id: SnapshotId) -> anyhow::Result<Self::Iter<'_, S>>;
}

/// Simple wrapper around `RwLock` that only allows read access.
#[derive(Debug)]
pub struct ReadOnlyLock<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> ReadOnlyLock<T> {
    /// Create new [`ReadOnlyLock`] from [`Arc<RwLock<T>>`].
    pub fn new(lock: Arc<RwLock<T>>) -> Self {
        Self { lock }
    }

    /// Acquires a read lock on the underlying `RwLock`.
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        self.lock.read()
    }
}

impl<T> From<Arc<RwLock<T>>> for ReadOnlyLock<T> {
    fn from(value: Arc<RwLock<T>>) -> Self {
        Self::new(value)
    }
}

/// Wrapper around [`QueryManager`] that allows to read from snapshots
#[derive(Debug)]
pub struct DbSnapshot<Q> {
    id: SnapshotId,
    cache: Mutex<SchemaBatch>,
    parents_manager: ReadOnlyLock<Q>,
}

impl<Q: QueryManager> DbSnapshot<Q> {
    /// Create new [`DbSnapshot`]
    pub fn new(id: SnapshotId, manager: ReadOnlyLock<Q>) -> Self {
        Self {
            id,
            cache: Mutex::new(SchemaBatch::default()),
            parents_manager: manager,
        }
    }

    /// Get a value from current snapshot, its parents or underlying database
    pub fn read<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<Option<S::Value>> {
        // Some(Operation) means that key was touched,
        // but in case of deletion we early return None
        // Only in case of not finding operation for key,
        // we go deeper

        // Hold local cache lock explicitly, so reads are atomic
        let local_cache = self
            .cache
            .lock()
            .expect("SchemaBatch lock should not be poisoned");

        // 1. Check in cache
        if let Some(operation) = local_cache.read(key)? {
            return decode_operation::<S>(operation);
        }

        // 2. Check parent
        let parent = self
            .parents_manager
            .read()
            .expect("Parent lock must not be poisoned");
        parent.get::<S>(self.id, key)
    }

    /// Store a value in snapshot
    pub fn put<S: Schema>(
        &self,
        key: &impl KeyCodec<S>,
        value: &impl ValueCodec<S>,
    ) -> anyhow::Result<()> {
        self.cache
            .lock()
            .expect("SchemaBatch lock must not be poisoned")
            .put(key, value)
    }

    /// Delete given key from snapshot
    pub fn delete<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<()> {
        self.cache
            .lock()
            .expect("SchemaBatch lock must not be poisoned")
            .delete(key)
    }

    /// Get value of largest key written value for given [`Schema`]
    pub fn get_largest<S: Schema>(&self) -> anyhow::Result<Option<S::Value>> {
        let local_cache = self
            .cache
            .lock()
            .expect("SchemaBatch lock must not be poisoned");
        let local_cache_iter = local_cache.iter::<S>();

        let parent = self
            .parents_manager
            .read()
            .expect("Parent lock must not be poisoned");

        let parent_iter = parent.iter::<S>(self.id)?;

        let mut combined_iter: SnapshotIter<'_, Q, S> = SnapshotIter {
            local_cache_iter: local_cache_iter.peekable(),
            parent_iter: parent_iter.peekable(),
        };

        if let Some((_, value)) = combined_iter.next() {
            let value = S::Value::decode_value(&value)?;
            return Ok(Some(value));
        }

        Ok(None)
    }

    /// Get value in [`Schema`] that is smaller or equal than give `seek_key`
    pub fn get_prev<S: Schema>(
        &self,
        seek_key: &impl SeekKeyEncoder<S>,
    ) -> anyhow::Result<Option<S::Value>> {
        let seek_key = seek_key.encode_seek_key()?;

        let local_cache = self
            .cache
            .lock()
            .expect("Local cache lock must not be poisoned");

        let local_cache_iter = local_cache.iter::<S>();

        let parent = self
            .parents_manager
            .read()
            .expect("Parent snapshots lock must not be poisoned");

        let parent_iter = parent.iter::<S>(self.id)?;

        let combined_iter: SnapshotIter<'_, Q, S> = SnapshotIter {
            local_cache_iter: local_cache_iter.peekable(),
            parent_iter: parent_iter.peekable(),
        };

        if let Some((_, value)) = combined_iter
            .filter_map(|(key, value)| {
                if key <= seek_key {
                    return Some((key, value));
                }
                None
            })
            .next()
        {
            return Ok(Some(S::Value::decode_value(&value)?));
        }

        Ok(None)
    }
}

struct SnapshotIter<'a, Q: QueryManager + 'a, S: Schema> {
    local_cache_iter: std::iter::Peekable<SchemaBatchIterator<'a, S>>,
    parent_iter: std::iter::Peekable<Q::Iter<'a, S>>,
}

impl<'a, Q: QueryManager + 'a, S: Schema> Iterator for SnapshotIter<'a, Q, S> {
    type Item = (SchemaKey, SchemaValue);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let local_cache_peeked = self.local_cache_iter.peek();
            let parent_peeked = self.parent_iter.peek();

            match (local_cache_peeked, parent_peeked) {
                // Both iterators exhausted
                (None, None) => break,
                // Parent exhausted (just like me on friday)
                (Some(&(key, operation)), None) => {
                    self.local_cache_iter.next();
                    let next = put_or_none(key, operation);
                    if next.is_none() {
                        continue;
                    }
                    return next;
                }
                // Local exhausted
                (None, Some((_key, _value))) => {
                    return self.parent_iter.next();
                }
                // Both are active, need to compare keys
                (Some(&(local_key, local_operation)), Some((parent_key, _parent_value))) => {
                    return if local_key < parent_key {
                        self.parent_iter.next()
                    } else {
                        // Local is preferable, as it is the latest
                        // But both operators must succeed
                        if local_key == parent_key {
                            self.parent_iter.next();
                        }
                        self.local_cache_iter.next();
                        let next = put_or_none(local_key, local_operation);
                        if next.is_none() {
                            continue;
                        }
                        next
                    };
                }
            }
        }

        None
    }
}

/// Read only version of [`DbSnapshot`], for usage inside [`QueryManager`]
pub struct FrozenDbSnapshot {
    id: SnapshotId,
    cache: SchemaBatch,
}

impl FrozenDbSnapshot {
    /// Get value from its own cache
    pub fn get<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<Option<&Operation>> {
        self.cache.read(key)
    }

    /// Get id of this Snapshot
    pub fn get_id(&self) -> SnapshotId {
        self.id
    }

    /// Iterate over all operations in snapshot in reversed lexicographic order
    pub fn iter<S: Schema>(&self) -> SchemaBatchIterator<'_, S> {
        self.cache.iter::<S>()
    }
}

impl<Q> From<DbSnapshot<Q>> for FrozenDbSnapshot {
    fn from(snapshot: DbSnapshot<Q>) -> Self {
        Self {
            id: snapshot.id,
            cache: snapshot
                .cache
                .into_inner()
                .expect("SchemaBatch lock must not be poisoned"),
        }
    }
}

impl From<FrozenDbSnapshot> for SchemaBatch {
    fn from(value: FrozenDbSnapshot) -> Self {
        value.cache
    }
}

fn decode_operation<S: Schema>(operation: &Operation) -> anyhow::Result<Option<S::Value>> {
    match operation {
        Operation::Put { value } => {
            let value = S::Value::decode_value(value)?;
            Ok(Some(value))
        }
        Operation::Delete => Ok(None),
    }
}

fn put_or_none(key: &SchemaKey, operation: &Operation) -> Option<(SchemaKey, SchemaValue)> {
    if let Operation::Put { value } = operation {
        return Some((key.to_vec(), value.to_vec()));
    }
    None
}

/// QueryManager, which never returns any values
pub struct NoopQueryManager;

impl QueryManager for NoopQueryManager {
    type Iter<'a, S: Schema> = std::iter::Empty<(SchemaKey, SchemaValue)>;

    fn get<S: Schema>(
        &self,
        _snapshot_id: SnapshotId,
        _key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<S::Value>> {
        Ok(None)
    }

    fn iter<S: Schema>(&self, _snapshot_id: SnapshotId) -> anyhow::Result<Self::Iter<'_, S>> {
        Ok(std::iter::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::define_schema;
    use crate::schema::KeyEncoder;
    use crate::test::{TestCompositeField, TestField};

    define_schema!(TestSchema, TestCompositeField, TestField, "TestCF");

    struct SingleSnapshotQueryManager {
        cache: SchemaBatch,
    }

    impl QueryManager for SingleSnapshotQueryManager {
        type Iter<'a, S: Schema> = std::vec::IntoIter<(SchemaKey, SchemaValue)>;

        fn get<S: Schema>(
            &self,
            _snapshot_id: SnapshotId,
            key: &impl KeyCodec<S>,
        ) -> anyhow::Result<Option<S::Value>> {
            if let Some(Operation::Put { value }) = self.cache.read(key)? {
                let value = S::Value::decode_value(value)?;
                return Ok(Some(value));
            }
            Ok(None)
        }

        fn iter<S: Schema>(&self, _snapshot_id: SnapshotId) -> anyhow::Result<Self::Iter<'_, S>> {
            let collected: Vec<(SchemaKey, SchemaValue)> = self
                .cache
                .iter::<S>()
                .filter_map(|(k, op)| match op {
                    Operation::Put { value } => Some((k.to_vec(), value.to_vec())),
                    Operation::Delete => None,
                })
                .collect();

            Ok(collected.into_iter())
        }
    }

    fn encode_key(key: &TestCompositeField) -> Vec<u8> {
        <TestCompositeField as KeyEncoder<TestSchema>>::encode_key(key).unwrap()
    }

    fn encode_value(value: &TestField) -> Vec<u8> {
        <TestField as ValueCodec<TestSchema>>::encode_value(value).unwrap()
    }

    #[test]
    fn test_db_snapshot_iterator_empty() {
        let local_cache = SchemaBatch::new();
        let parent_values = SchemaBatch::new();

        let manager = SingleSnapshotQueryManager {
            cache: parent_values,
        };

        let local_cache_iter = local_cache.iter::<TestSchema>().peekable();
        let manager_iter = manager.iter::<TestSchema>(0).unwrap().peekable();

        let snapshot_iter = SnapshotIter::<'_, SingleSnapshotQueryManager, TestSchema> {
            local_cache_iter,
            parent_iter: manager_iter,
        };

        let values: Vec<(SchemaKey, SchemaValue)> = snapshot_iter.collect();

        assert!(values.is_empty());
    }

    #[test]
    fn test_db_snapshot_iterator_values() {
        let k1 = TestCompositeField(0, 1, 0);
        let k2 = TestCompositeField(0, 1, 2);
        let k3 = TestCompositeField(3, 1, 0);
        let k4 = TestCompositeField(3, 2, 0);

        let mut parent_values = SchemaBatch::new();
        parent_values.put::<TestSchema>(&k2, &TestField(2)).unwrap();
        parent_values.put::<TestSchema>(&k1, &TestField(1)).unwrap();
        parent_values.put::<TestSchema>(&k4, &TestField(4)).unwrap();
        parent_values.put::<TestSchema>(&k3, &TestField(3)).unwrap();

        let mut local_cache = SchemaBatch::new();
        local_cache.delete::<TestSchema>(&k3).unwrap();
        local_cache.put::<TestSchema>(&k1, &TestField(10)).unwrap();
        local_cache.put::<TestSchema>(&k2, &TestField(20)).unwrap();

        let manager = SingleSnapshotQueryManager {
            cache: parent_values,
        };

        let local_cache_iter = local_cache.iter::<TestSchema>().peekable();
        let manager_iter = manager.iter::<TestSchema>(0).unwrap().peekable();

        let snapshot_iter = SnapshotIter::<'_, SingleSnapshotQueryManager, TestSchema> {
            local_cache_iter,
            parent_iter: manager_iter,
        };

        let actual_values: Vec<(SchemaKey, SchemaValue)> = snapshot_iter.collect();
        let expected_values = vec![
            (encode_key(&k4), encode_value(&TestField(4))),
            (encode_key(&k2), encode_value(&TestField(20))),
            (encode_key(&k1), encode_value(&TestField(10))),
        ];

        assert_eq!(expected_values, actual_values);
    }
}
