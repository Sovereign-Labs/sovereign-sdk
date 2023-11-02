use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use sov_schema_db::schema::{KeyCodec, ValueCodec};
use sov_schema_db::snapshot::{FrozenDbSnapshot, QueryManager, SnapshotId};
use sov_schema_db::{Operation, Schema};

/// Snapshot manager holds snapshots associated with particular DB and can traverse them backwards
/// down to DB level
/// Managed externally by [`NewProverStorageManager`]
pub struct SnapshotManager {
    db: sov_schema_db::DB,
    snapshots: HashMap<SnapshotId, FrozenDbSnapshot>,
    /// Hierarchical
    to_parent: Arc<RwLock<HashMap<SnapshotId, SnapshotId>>>,
}

impl SnapshotManager {
    pub(crate) fn new(
        db: sov_schema_db::DB,
        to_parent: Arc<RwLock<HashMap<SnapshotId, SnapshotId>>>,
    ) -> Self {
        Self {
            db,
            snapshots: HashMap::new(),
            to_parent,
        }
    }

    pub(crate) fn add_snapshot(&mut self, snapshot: FrozenDbSnapshot) {
        let snapshot_id = snapshot.get_id();
        if self.snapshots.insert(snapshot_id, snapshot).is_some() {
            panic!("Attempt to double save same snapshot");
        }
    }

    pub(crate) fn discard_snapshot(&mut self, snapshot_id: &SnapshotId) {
        self.snapshots
            .remove(snapshot_id)
            .expect("Attempt to remove unknown snapshot");
    }

    pub(crate) fn commit_snapshot(&mut self, snapshot_id: &SnapshotId) -> anyhow::Result<()> {
        if !self.snapshots.contains_key(snapshot_id) {
            anyhow::bail!("Attempt to commit unknown snapshot");
        }

        let snapshot = self.snapshots.remove(snapshot_id).unwrap();
        self.db.write_schemas(snapshot.into())
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

impl QueryManager for SnapshotManager {
    fn get<S: Schema>(
        &self,
        mut snapshot_id: SnapshotId,
        key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<S::Value>> {
        while let Some(parent_snapshot_id) = self.to_parent.read().unwrap().get(&snapshot_id) {
            let parent_snapshot = self
                .snapshots
                .get(parent_snapshot_id)
                .expect("Inconsistent snapshots tree");

            // Some operation has been found
            if let Some(operation) = parent_snapshot.get(key)? {
                return match operation {
                    Operation::Put { value } => Ok(Some(S::Value::decode_value(value)?)),
                    Operation::Delete => Ok(None),
                };
            }

            snapshot_id = *parent_snapshot_id;
        }

        self.db.get(key)
    }
}

#[cfg(test)]
mod tests {}
