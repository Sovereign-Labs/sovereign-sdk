use std::collections::HashMap;
use std::iter::Peekable;
use std::sync::{Arc, RwLock};

use sov_schema_db::schema::{KeyCodec, ValueCodec};
use sov_schema_db::snapshot::{FrozenDbSnapshot, QueryManager, SnapshotId};
use sov_schema_db::{Operation, Schema, SchemaBatchIterator, SchemaKey, SchemaValue};

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
        self.snapshots.remove(snapshot_id);
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

    pub(crate) fn contains_snapshot(&self, snapshot_id: &SnapshotId) -> bool {
        self.snapshots.contains_key(snapshot_id)
    }

    pub fn iter<S: Schema>(&self, mut snapshot_id: SnapshotId) -> SnapshotManagerIter<S> {
        let mut snapshot_iterators = vec![];
        let to_parent = self.to_parent.read().unwrap();
        while let Some(parent_snapshot_id) = to_parent.get(&snapshot_id) {
            let parent_snapshot = self
                .snapshots
                .get(parent_snapshot_id)
                .expect("Inconsistency between `self.snapshots` and `self.to_parent`");

            snapshot_iterators.push(parent_snapshot.iter::<S>());

            snapshot_id = *parent_snapshot_id;
        }

        SnapshotManagerIter::new(snapshot_iterators)
    }
}

/// [`Iterator`] over keys in given [`Schema`] in all snapshots in reverse lexicographical order
pub struct SnapshotManagerIter<'a, S: Schema> {
    // TODO: Plug DB iterator HERE.
    snapshot_iterators: Vec<Peekable<SchemaBatchIterator<'a, S>>>,
}

impl<'a, S: Schema> SnapshotManagerIter<'a, S> {
    pub(crate) fn new(snapshot_iterators: Vec<SchemaBatchIterator<'a, S>>) -> Self {
        Self {
            snapshot_iterators: snapshot_iterators
                .into_iter()
                .map(|iter| iter.peekable())
                .collect(),
        }
    }
}

impl<'a, S: Schema> Iterator for SnapshotManagerIter<'a, S> {
    type Item = (&'a SchemaKey, &'a SchemaValue);

    fn next(&mut self) -> Option<Self::Item> {
        // Find max value
        loop {
            let mut max_values: Vec<(usize, &&SchemaKey)> = vec![];
            for (idx, iter) in self.snapshot_iterators.iter_mut().enumerate() {
                if let Some((peeked_key, _)) = iter.peek() {
                    if max_values.is_empty() {
                        max_values.push((idx, peeked_key));
                    } else {
                        let (_, max_key) = max_values[0];
                        if peeked_key > max_key {
                            max_values.clear();
                            max_values.push((idx, peeked_key));
                        } else if peeked_key == max_key {
                            max_values.push((idx, peeked_key));
                        }
                    }
                }
            }

            if max_values.is_empty() {
                break;
            }

            let max_indices = max_values
                .into_iter()
                .map(|(idx, _)| idx)
                .collect::<Vec<_>>();

            let (key, operation) = self.snapshot_iterators[max_indices[0]].next().unwrap();

            // Progress all previous (older) snapshots to the next key
            for idx in max_indices.into_iter().skip(1) {
                let _ = self.snapshot_iterators[idx].next().unwrap();
            }

            match operation {
                Operation::Put { value } => return Some((key, value)),
                Operation::Delete => continue,
            }
        }

        None
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
                .expect("Inconsistency between `self.snapshots` and `self.to_parent`");

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
    use sov_schema_db::schema::{KeyDecoder, ValueCodec};
    use sov_schema_db::snapshot::{DbSnapshot, NoopQueryManager, QueryManager};
    use sov_schema_db::SchemaBatch;

    use crate::dummy_storage::{DummyField, DummyStateSchema, DUMMY_STATE_CF};
    use crate::snapshot_manager::SnapshotManager;

    type Schema = DummyStateSchema;

    fn create_test_db(path: &std::path::Path) -> sov_schema_db::DB {
        let tables = vec![DUMMY_STATE_CF.to_string()];
        sov_schema_db::DB::open(
            path,
            "test_db",
            tables,
            &gen_rocksdb_options(&Default::default(), false),
        )
        .unwrap()
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
    fn test_discard_unknown() {
        // Discarding unknown snapshots are fine.
        // As it possible that caller didn't save it previously.
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
    fn test_query_unknown_snapshot_id() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        assert_eq!(
            None,
            snapshot_manager.get::<Schema>(1, &DummyField(1)).unwrap()
        );
    }

    #[test]
    fn test_query_genesis_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));

        let one = DummyField(1);
        let two = DummyField(2);
        let three = DummyField(3);

        let mut db_data = SchemaBatch::new();
        db_data.put::<Schema>(&one, &one).unwrap();
        db_data.put::<Schema>(&three, &three).unwrap();
        db.write_schemas(db_data).unwrap();

        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let db_snapshot = DbSnapshot::new(1, query_manager.clone().into());
        db_snapshot.put::<Schema>(&two, &two).unwrap();
        db_snapshot.delete::<Schema>(&three).unwrap();

        snapshot_manager.add_snapshot(db_snapshot.into());

        // Effectively querying database:
        assert_eq!(Some(one), snapshot_manager.get::<Schema>(1, &one).unwrap());
        assert_eq!(None, snapshot_manager.get::<Schema>(1, &two).unwrap());
        assert_eq!(
            Some(three),
            snapshot_manager.get::<Schema>(1, &three).unwrap()
        );
    }

    #[test]
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

        let f1 = DummyField(1);
        let f2 = DummyField(2);
        let f3 = DummyField(3);
        let f4 = DummyField(4);
        let f5 = DummyField(5);
        let f6 = DummyField(6);
        let f7 = DummyField(7);
        let f8 = DummyField(8);

        let mut db_data = SchemaBatch::new();
        db_data.put::<Schema>(&f1, &f1).unwrap();
        db.write_schemas(db_data).unwrap();

        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        // Operations:
        // | snapshot_id | key | operation |
        // | DB          |   1 |  write(1) |
        // | 1           |   2 |  write(2) |
        // | 1           |   3 |  write(4) |
        // | 2           |   1 |  write(5) |
        // | 2           |   2 |   delete  |
        // | 4           |   3 |  write(6) |
        // | 6           |   1 |  write(7) |
        // | 6           |   2 |  write(8) |

        // 1
        let db_snapshot = DbSnapshot::new(1, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f2, &f2).unwrap();
        db_snapshot.put::<Schema>(&f3, &f4).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 2
        let db_snapshot = DbSnapshot::new(2, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f1, &f5).unwrap();
        db_snapshot.delete::<Schema>(&f2).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 3
        let db_snapshot = DbSnapshot::new(3, query_manager.clone().into());
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 4
        let db_snapshot = DbSnapshot::new(4, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f3, &f6).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 5
        let db_snapshot = DbSnapshot::new(5, query_manager.clone().into());
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 6
        let db_snapshot = DbSnapshot::new(6, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f1, &f7).unwrap();
        db_snapshot.put::<Schema>(&f2, &f8).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 7
        let db_snapshot = DbSnapshot::new(7, query_manager.clone().into());
        snapshot_manager.add_snapshot(db_snapshot.into());

        // View:
        // | from s_id   | key | value |
        // | 3           |   1 |     5 |
        // | 3           |   2 |  None |
        // | 3           |   3 |     4 |
        // | 5           |   1 |     1 |
        // | 5           |   2 |     2 |
        // | 5           |   3 |     6 |
        // | 7           |   1 |     7 |
        // | 7           |   2 |     8 |
        // | 7           |   3 |     4 |
        assert_eq!(Some(f5), snapshot_manager.get::<Schema>(3, &f1).unwrap());
        assert_eq!(None, snapshot_manager.get::<Schema>(3, &f2).unwrap());
        assert_eq!(Some(f4), snapshot_manager.get::<Schema>(3, &f3).unwrap());
        assert_eq!(Some(f1), snapshot_manager.get::<Schema>(5, &f1).unwrap());
        assert_eq!(Some(f2), snapshot_manager.get::<Schema>(5, &f2).unwrap());
        assert_eq!(Some(f6), snapshot_manager.get::<Schema>(5, &f3).unwrap());

        assert_eq!(Some(f7), snapshot_manager.get::<Schema>(7, &f1).unwrap());
        assert_eq!(Some(f8), snapshot_manager.get::<Schema>(7, &f2).unwrap());
        assert_eq!(Some(f4), snapshot_manager.get::<Schema>(7, &f3).unwrap());
    }

    #[test]
    fn test_iterator() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        {
            // DB -> 1 -> 2 -> 3
            let mut edit = to_parent.write().unwrap();
            edit.insert(2, 1);
            edit.insert(3, 2);
            edit.insert(4, 3);
        }

        let f1 = DummyField(1);
        let f2 = DummyField(2);
        let f3 = DummyField(3);
        let f4 = DummyField(4);
        let f5 = DummyField(5);
        let f6 = DummyField(6);
        let f7 = DummyField(7);
        let f8 = DummyField(8);
        let f9 = DummyField(9);
        let f10 = DummyField(10);
        let f12 = DummyField(12);

        let mut db_data = SchemaBatch::new();
        db_data.put::<Schema>(&f3, &f9).unwrap();
        db_data.put::<Schema>(&f2, &f1).unwrap();
        db_data.put::<Schema>(&f4, &f1).unwrap();
        db.write_schemas(db_data).unwrap();

        let mut snapshot_manager = SnapshotManager::new(db, to_parent.clone());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        // Operations:
        // | snapshot_id | key |  operation |
        // |           1 |   1 |   write(8) |
        // |           1 |   5 |   write(7) |
        // |           1 |   8 |   write(3) |
        // |           1 |   4 |   write(2) |
        // |           2 |  10 |   write(2) |
        // |           2 |   9 |   write(4) |
        // |           2 |   4 |     delete |
        // |           2 |   2 |   write(6) |
        // |           3 |   8 |   write(6) |
        // |           3 |   9 |     delete |
        // |           3 |  12 |   write(1) |
        // |           3 |   1 |   write(2) |

        // 1
        let db_snapshot = DbSnapshot::new(1, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f1, &f8).unwrap();
        db_snapshot.put::<Schema>(&f5, &f7).unwrap();
        db_snapshot.put::<Schema>(&f8, &f3).unwrap();
        db_snapshot.put::<Schema>(&f4, &f2).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 2
        let db_snapshot = DbSnapshot::new(2, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f10, &f2).unwrap();
        db_snapshot.put::<Schema>(&f9, &f4).unwrap();
        db_snapshot.delete::<Schema>(&f4).unwrap();
        db_snapshot.put::<Schema>(&f2, &f6).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 3
        let db_snapshot = DbSnapshot::new(3, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f8, &f6).unwrap();
        db_snapshot.delete::<Schema>(&f9).unwrap();
        db_snapshot.put::<Schema>(&f12, &f1).unwrap();
        db_snapshot.put::<Schema>(&f1, &f2).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // Expected Order
        // | key | value |
        // |  12 |     1 |
        // |  10 |     2 |
        // |   8 |     6 |
        // |   5 |     7 |
        // |  *4 |     1 |
        // |  *3 |     1 |
        // |   2 |     6 |
        // |   1 |     2 |

        println!("CHECKING VIEW");
        let i = snapshot_manager.iter::<Schema>(4);
        let mut v = vec![];
        for (key, value) in i {
            v.push((key, value));
        }

        for (key, value) in v {
            let key = <<DummyStateSchema as sov_schema_db::Schema>::Key as KeyDecoder<Schema>>::decode_key(&key).unwrap();
            let value = <<DummyStateSchema as sov_schema_db::Schema>::Value as ValueCodec<
                Schema,
            >>::decode_value(&value)
            .unwrap();
            // let value = DummyField::decode_value(&value).unwrap();
            println!("[I]: k={:?}: v={:?}", key, value);
        }
    }
}
