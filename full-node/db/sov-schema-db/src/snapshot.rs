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

    /// Get last written value for given [`Schema`]
    pub fn get_last<S: Schema>(&self) -> anyhow::Result<Option<S::Value>> {
        let local_cache = self
            .cache
            .lock()
            .expect("SchemaBatch lock must not be poisoned");
        let mut local_cache_iter = local_cache.iter::<S>().filter_map(|(_key, op)| match op {
            Operation::Put { value } => Some(value),
            Operation::Delete => None,
        });

        if let Some(last_written_value) = local_cache_iter.next() {
            let value = S::Value::decode_value(last_written_value)?;
            return Ok(Some(value));
        }

        let parent = self
            .parents_manager
            .read()
            .expect("Parent lock must not be poisoned");

        let mut parent_iter = parent.iter::<S>(self.id)?.map(|(_key, value)| value);

        if let Some(last_written_value) = parent_iter.next() {
            let value = S::Value::decode_value(&last_written_value)?;
            return Ok(Some(value));
        }

        Ok(None)
    }

    /// Get value in [`Schema`] that is smaller or equal than give `seek_key`
    pub fn find_prev<S: Schema>(
        &mut self,
        seek_key: &impl SeekKeyEncoder<S>,
    ) -> anyhow::Result<Option<S::Value>> {
        let seek_key = seek_key.encode_seek_key()?;

        let local_cache = self
            .cache
            .lock()
            .expect("Local cache lock must not be poisoned");

        let mut local_cache_iter = local_cache.iter::<S>().peekable();

        let parent = self
            .parents_manager
            .read()
            .expect("Parent snapshots lock must not be poisoned");

        let mut parent_iter = parent.iter::<S>(self.id)?.peekable();

        let handle_key_match =
            |key: &SchemaKey, value: &SchemaValue| -> anyhow::Result<Option<S::Value>> {
                if key <= &seek_key {
                    return Ok(Some(S::Value::decode_value(value)?));
                }
                Ok(None)
            };

        loop {
            let local_cache_peeked = local_cache_iter.peek();
            let parent_peeked = parent_iter.peek();

            match (local_cache_peeked, parent_peeked) {
                // Both iterators exhausted
                (None, None) => break,
                // Parent exhausted (just like me on friday)
                (Some(&(key, operation)), None) => {
                    local_cache_iter.next();
                    if let Operation::Put { value } = operation {
                        if let Some(value) = handle_key_match(key, value)? {
                            return Ok(Some(value));
                        }
                    }
                }
                // Local exhausted
                (None, Some((key, value))) => {
                    if let Some(value) = handle_key_match(key, value)? {
                        return Ok(Some(value));
                    }
                    parent_iter.next();
                }
                // Both are active, need to compare keys
                (Some(&(local_key, local_operation)), Some((parent_key, parent_value))) => {
                    if local_key < parent_key {
                        if let Some(value) = handle_key_match(parent_key, parent_value)? {
                            return Ok(Some(value));
                        }
                        parent_iter.next();
                    } else {
                        // Local is preferable, as it is the latest
                        // But both operators must succeed
                        if local_key == parent_key {
                            parent_iter.next();
                        }
                        local_cache_iter.next();
                        if let Operation::Put { value: local_value } = local_operation {
                            if let Some(value) = handle_key_match(local_key, local_value)? {
                                return Ok(Some(value));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
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
        todo!()
    }
}
