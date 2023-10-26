//! Snapshot related logic

use std::sync::{Arc, LockResult, Mutex, RwLock, RwLockReadGuard};

use crate::schema::{KeyCodec, ValueCodec};
use crate::{Operation, Schema, SchemaBatch, DB};

/// Id of database snapshot
pub type SnapshotId = u64;

/// A trait to make nested calls to several [`Schema`]
pub trait QueryManager {
    /// Get a value from snapshot or its parents
    fn get<S: Schema>(
        &self,
        snapshot_id: SnapshotId,
        key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<Operation>>;
}

/// Simple wrapper around `RwLock` that only allows read access.
pub struct ReadOnlyLock<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> ReadOnlyLock<T> {
    #[allow(dead_code)]
    /// Create new [`ReadOnlyLock`] from [`Arc<RwLock<T>>`].
    pub fn new(lock: Arc<RwLock<T>>) -> Self {
        Self { lock }
    }

    /// Acquires a read lock on the underlying `RwLock`.
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        self.lock.read()
    }
}

/// Wrapper around [`DB`] that allows to read from snapshots
#[allow(dead_code)]
pub struct DbSnapshot<Q> {
    id: SnapshotId,
    cache: Mutex<SchemaBatch>,
    manager: ReadOnlyLock<Q>,
    db_reader: Arc<DB>,
}

#[allow(dead_code)]
impl<Q: QueryManager> DbSnapshot<Q> {
    /// Create new [`DbSnapshot`]
    pub fn new(id: SnapshotId, manager: ReadOnlyLock<Q>, db_reader: Arc<DB>) -> Self {
        Self {
            id,
            cache: Mutex::new(SchemaBatch::default()),
            manager,
            db_reader,
        }
    }

    /// Get a value from current snapshot, its parents or underlying database
    pub fn read<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<Option<S::Value>> {
        // Some(Operation) means that key was touched,
        // but in case of deletion we early return None
        // Only in case of not finding operation for key,
        // we go deeper

        // 1. Check in cache
        if let Some(operation) = self
            .cache
            .lock()
            .expect("SchemaBatch lock should not be poisoned")
            .read(key)?
        {
            return decode_operation::<S>(operation);
        }

        // Check parent
        {
            let parent = self
                .manager
                .read()
                .expect("Parent lock must not be poisoned");
            if let Some(operation) = parent.get(self.id, key)? {
                return decode_operation::<S>(operation);
            }
        }

        // Check db

        self.db_reader.get(key)
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
            .put(key, value)?;
        Ok(())
    }
}

/// Read only version of [`DbSnapshot`], for usage inside [`QueryManager`]
pub struct FrozenDbSnapshot {
    id: SnapshotId,
    cache: SchemaBatch,
}

impl FrozenDbSnapshot {
    /// Get value from its own cache
    pub fn get<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<Option<S::Value>> {
        if let Some(operation) = self.cache.read(key)? {
            return decode_operation::<S>(operation);
        }

        Ok(None)
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
