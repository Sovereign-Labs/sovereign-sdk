//! Snapshot related logic

use std::sync::{Arc, LockResult, Mutex, RwLock, RwLockReadGuard};

use crate::schema::{KeyCodec, ValueCodec};
use crate::{Operation, Schema, SchemaBatch};

/// Id of database snapshot
pub type SnapshotId = u64;

/// A trait to make nested calls to several [`SchemaBatch`]s and eventually [`crate::DB`]
pub trait QueryManager {
    /// Get a value from snapshot or its parents
    fn get<S: Schema>(
        &self,
        snapshot_id: SnapshotId,
        key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<S::Value>>;
}

/// Simple wrapper around `RwLock` that only allows read access.
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

/// Wrapper around [`QueryManager`] that allows to read from snapshots
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
}

/// Read only version of [`DbSnapshot`], for usage inside [`QueryManager`]
pub struct FrozenDbSnapshot {
    id: SnapshotId,
    cache: SchemaBatch,
}

impl FrozenDbSnapshot {
    /// Get value from its own cache
    pub fn get<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<Option<Operation>> {
        self.cache.read(key)
    }

    /// Get id of this Snapshot
    pub fn get_id(&self) -> SnapshotId {
        self.id
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

fn decode_operation<S: Schema>(operation: Operation) -> anyhow::Result<Option<S::Value>> {
    match operation {
        Operation::Put { value } => {
            let value = S::Value::decode_value(&value)?;
            Ok(Some(value))
        }
        Operation::Delete => Ok(None),
    }
}
