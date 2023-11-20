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
            .expect("Attempt to discard unknown snapshot");
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
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    use sov_db::rocks_db_config::gen_rocksdb_options;
    use sov_schema_db::snapshot::{DbSnapshot, NoopQueryManager};

    use crate::dummy_storage::DUMMY_STATE_CF;
    use crate::snapshot_manager::SnapshotManager;

    fn create_test_db(path: &std::path::Path) -> sov_schema_db::DB {
        let tables = vec![DUMMY_STATE_CF.to_string()];
        let db = sov_schema_db::DB::open(
            path,
            "test_db",
            tables,
            &gen_rocksdb_options(&Default::default(), false),
        )
        .unwrap();
        db
    }

    #[test]
    fn test_empty() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let snapshot_manager = SnapshotManager::new(db, Arc::new(RwLock::new(HashMap::new())));
        assert!(snapshot_manager.is_empty());
    }

    #[test]
    fn test_add_and_discard_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let snapshot_id = 1;
        let db_snapshot = DbSnapshot::new(snapshot_id, query_manager.clone().into());

        snapshot_manager.add_snapshot(db_snapshot.into());
        assert!(!snapshot_manager.is_empty());
        snapshot_manager.discard_snapshot(&snapshot_id);
        assert!(snapshot_manager.is_empty());
    }

    #[test]
    #[should_panic(expected = "Attempt to double save same snapshot")]
    fn test_add_twice() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let snapshot_id = 1;
        // Both share the same ID
        let db_snapshot_1 = DbSnapshot::new(snapshot_id, query_manager.clone().into());
        let db_snapshot_2 = DbSnapshot::new(snapshot_id, query_manager.clone().into());

        snapshot_manager.add_snapshot(db_snapshot_1.into());
        assert!(!snapshot_manager.is_empty());
        snapshot_manager.add_snapshot(db_snapshot_2.into());
    }

    #[test]
    #[should_panic(expected = "Attempt to commit unknown snapshot")]
    fn test_commit_unknown() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());

        snapshot_manager.commit_snapshot(&1).unwrap();
    }

    #[test]
    #[should_panic(expected = "Attempt to discard unknown snapshot")]
    fn test_discard_unknown() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());

        snapshot_manager.discard_snapshot(&1);
    }

    #[test]
    fn test_commit_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let snapshot_id = 1;
        let db_snapshot = DbSnapshot::new(snapshot_id, query_manager.clone().into());

        snapshot_manager.add_snapshot(db_snapshot.into());
        let result = snapshot_manager.commit_snapshot(&snapshot_id);
        assert!(result.is_ok());
        assert!(snapshot_manager.is_empty());
    }

    #[test]
    #[ignore = "TBD"]
    fn test_query_lifecycle() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        {
            //            / -> 6 -> 7
            // DB -> 1 -> 2 -> 3
            //       \ -> 4 -> 5
            let mut edit = to_parent.write().unwrap();
            edit.insert(3, 2);
            edit.insert(2, 1);
            edit.insert(4, 1);
            edit.insert(5, 4);
            edit.insert(6, 2);
            edit.insert(7, 6);
        }
        let _snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        let _query_manager = Arc::new(RwLock::new(NoopQueryManager));

        // Operations:
        // | snapshot_id | key | operation |
        // | DB          |   1 |  write(1) |
        // | 1           |   1 |  write(2) |
        // | 1           |   2 |  write(3) |
        // | 2           |   1 |  delete   |
        // | 2           |   2 |  write(4) |
        // | 4           |   1 |  write(5) |
        // | 4           |   3 |  write(6) |
        // | 6           |   1 |  write(7) |

        // View:
        // | from s_id   | key | value |
        // | 3           |   1 |  None |
        // | 3           |   2 |     3 |
        // | 3           |   3 |  None |
        // | 5           |   1 |     5 |
        // | 5           |   2 |     4 |
        // | 5           |   3 |     6 |
        // | 7           |   1 |     7 |
        // | 7           |   2 |     4 |
        // | 7           |   3 |  None |
    }
}
