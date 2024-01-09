use std::collections::btree_map;
use std::iter::Rev;

use crate::cache::SnapshotId;
use crate::{KeyCodec, Operation, Schema, SchemaBatch, SchemaBatchIterator, SchemaKey};

/// Collection of all writes with associated [`SnapshotId`]
#[derive(Debug, Clone)]
pub struct ChangeSet {
    id: SnapshotId,
    pub(crate) operations: SchemaBatch,
}

impl ChangeSet {
    pub(crate) fn new(id: SnapshotId) -> Self {
        Self {
            id,
            operations: SchemaBatch::default(),
        }
    }
    /// Get value from its own cache
    pub fn get<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<Option<&Operation>> {
        self.operations.read(key)
    }

    /// Get id of this ChangeSet
    pub fn get_id(&self) -> SnapshotId {
        self.id
    }

    /// Iterate over all operations in snapshot in reversed lexicographic order
    pub fn iter<S: Schema>(
        &self,
    ) -> SchemaBatchIterator<'_, S, Rev<btree_map::Iter<SchemaKey, Operation>>> {
        self.operations.iter::<S>()
    }

    /// Iterate over all operations in snapshot in reversed lexicographical order, starting from `upper_bound`
    pub fn iter_range<S: Schema>(
        &self,
        upper_bound: SchemaKey,
    ) -> SchemaBatchIterator<'_, S, Rev<btree_map::Range<SchemaKey, Operation>>> {
        self.operations.iter_range::<S>(upper_bound)
    }
}

impl From<ChangeSet> for SchemaBatch {
    fn from(value: ChangeSet) -> Self {
        value.operations
    }
}
