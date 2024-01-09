use std::cmp::Ordering;
use std::collections::{btree_map, HashMap};
use std::iter::{Peekable, Rev};

use crate::cache::cache_container::DataLocation::Snapshot;
use crate::cache::change_set::ChangeSet;
use crate::cache::SnapshotId;
use crate::schema::{KeyCodec, ValueCodec};
use crate::{
    Operation, RawDbReverseIterator, ReadOnlyLock, Schema, SchemaBatchIterator, SchemaKey,
    SchemaValue, DB,
};

/// Holds collection of [`ChangeSet`] associated with particular Snapshot
/// and knows how to traverse them.
/// Managed externally.
/// Should be managed carefully, because discrepancy between `snapshots` and `to_parent` leads to panic
/// Ideally owner of writable reference to parent nad owner of cache container manages both correctly.
#[derive(Debug)]
pub struct CacheContainer {
    db: DB,
    /// Set of [`ChangeSet`]s of data per individual database per snapshot
    snapshots: HashMap<SnapshotId, ChangeSet>,
    /// Hierarchical
    /// Shared between all SnapshotManagers and managed by StorageManager
    to_parent: ReadOnlyLock<HashMap<SnapshotId, SnapshotId>>,
}

// 4 SnapshotManager

// sov-prover-storage-manager:
//   - sov-state::ProverStorage

impl CacheContainer {
    /// Create CacheContainer pointing go given DB and Snapshot ID relations
    pub fn new(db: DB, to_parent: ReadOnlyLock<HashMap<SnapshotId, SnapshotId>>) -> Self {
        Self {
            db,
            snapshots: HashMap::new(),
            to_parent,
        }
    }

    /// Create instance of snapshot manager, when it does not have connection to snapshots tree
    /// So it only reads from database.
    #[cfg(feature = "test-utils")]
    pub fn orphan(db: DB) -> Self {
        Self {
            db,
            snapshots: HashMap::new(),
            to_parent: std::sync::Arc::new(std::sync::RwLock::new(Default::default())).into(),
        }
    }

    /// Adds Snapshot to the collection.
    /// Please note that caller must update its own reference of `to_parent`
    /// After adding snapshot.
    /// Ideally these operations should be atomic
    pub fn add_snapshot(&mut self, snapshot: ChangeSet) {
        let snapshot_id = snapshot.get_id();
        if self.snapshots.insert(snapshot_id, snapshot).is_some() {
            panic!("Attempt to double save same snapshot");
        }
    }

    /// Removes snapshot from collection
    /// This should happen **after** `to_parent` is updated
    pub fn discard_snapshot(&mut self, snapshot_id: &SnapshotId) {
        self.snapshots.remove(snapshot_id);
    }

    /// Writes snapshot to the underlying database
    /// Snapshot id should be removed from `to_parent` atomically.
    pub fn commit_snapshot(&mut self, snapshot_id: &SnapshotId) -> anyhow::Result<()> {
        if !self.snapshots.contains_key(snapshot_id) {
            anyhow::bail!("Attempt to commit unknown snapshot");
        }

        let snapshot = self.snapshots.remove(snapshot_id).unwrap();
        self.db.write_schemas(snapshot.into())
    }

    /// Indicates, if CacheContainer has any snapshots stored.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Helper method to check if snapshot has been saved
    pub fn contains_snapshot(&self, snapshot_id: &SnapshotId) -> bool {
        self.snapshots.contains_key(snapshot_id)
    }

    /// Returns iterator over keys in given [`Schema`] among all snapshots and DB in reverse lexicographical order
    pub(crate) fn iter<S: Schema>(
        &self,
        mut snapshot_id: SnapshotId,
    ) -> anyhow::Result<SnapshotManagerIter<S, Rev<btree_map::Iter<SchemaKey, Operation>>>> {
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

        snapshot_iterators.reverse();
        let db_iter = self.db.raw_iter::<S>()?;
        Ok(SnapshotManagerIter::new(db_iter, snapshot_iterators))
    }

    pub(crate) fn get<S: Schema>(
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

    pub(crate) fn iter_range<S: Schema>(
        &self,
        mut snapshot_id: SnapshotId,
        upper_bound: SchemaKey,
    ) -> anyhow::Result<SnapshotManagerIter<S, Rev<btree_map::Range<SchemaKey, Operation>>>> {
        let mut snapshot_iterators = vec![];
        let to_parent = self.to_parent.read().unwrap();
        while let Some(parent_snapshot_id) = to_parent.get(&snapshot_id) {
            let parent_snapshot = self
                .snapshots
                .get(parent_snapshot_id)
                .expect("Inconsistency between `self.snapshots` and `self.to_parent`");

            snapshot_iterators.push(parent_snapshot.iter_range::<S>(upper_bound.clone()));

            snapshot_id = *parent_snapshot_id;
        }

        snapshot_iterators.reverse();
        let mut db_iter = self.db.raw_iter::<S>()?;
        db_iter.seek(upper_bound)?;

        Ok(SnapshotManagerIter::new(db_iter, snapshot_iterators))
    }
}

/// [`Iterator`] over keys in given [`Schema`] in all snapshots in reverse lexicographical order
pub struct SnapshotManagerIter<'a, S, I>
where
    S: Schema,
    I: Iterator<Item = (&'a SchemaKey, &'a Operation)>,
{
    db_iter: Peekable<RawDbReverseIterator<'a>>,
    snapshot_iterators: Vec<Peekable<SchemaBatchIterator<'a, S, I>>>,
    max_value_locations: Vec<DataLocation>,
}

impl<'a, S, I> SnapshotManagerIter<'a, S, I>
where
    S: Schema,
    I: Iterator<Item = (&'a SchemaKey, &'a Operation)>,
{
    fn new(
        db_iter: RawDbReverseIterator<'a>,
        snapshot_iterators: Vec<SchemaBatchIterator<'a, S, I>>,
    ) -> Self {
        let max_values_size = snapshot_iterators.len().checked_add(1).unwrap_or_default();
        Self {
            db_iter: db_iter.peekable(),
            snapshot_iterators: snapshot_iterators
                .into_iter()
                .map(|iter| iter.peekable())
                .collect(),
            max_value_locations: Vec::with_capacity(max_values_size),
        }
    }
}

#[derive(Debug)]
enum DataLocation {
    Db,
    // Index inside `snapshot_iterators`
    Snapshot(usize),
}

impl<'a, S, I> Iterator for SnapshotManagerIter<'a, S, I>
where
    S: Schema,
    I: Iterator<Item = (&'a SchemaKey, &'a Operation)>,
{
    type Item = (SchemaKey, SchemaValue);

    fn next(&mut self) -> Option<Self::Item> {
        // Find max value
        loop {
            let mut max_value: Option<&SchemaKey> = None;
            self.max_value_locations.clear();
            let max_db_value = self.db_iter.peek();
            if let Some((db_key, _)) = max_db_value {
                self.max_value_locations.push(DataLocation::Db);
                max_value = Some(db_key);
            };

            for (idx, iter) in self.snapshot_iterators.iter_mut().enumerate() {
                if let Some(&(peeked_key, _)) = iter.peek() {
                    match max_value {
                        None => {
                            self.max_value_locations.push(Snapshot(idx));
                            max_value = Some(peeked_key);
                        }
                        Some(max_key) => match peeked_key.cmp(max_key) {
                            Ordering::Greater => {
                                max_value = Some(peeked_key);
                                self.max_value_locations.clear();
                                self.max_value_locations.push(Snapshot(idx));
                            }
                            Ordering::Equal => {
                                self.max_value_locations.push(Snapshot(idx));
                            }
                            Ordering::Less => {}
                        },
                    };
                }
            }

            if let Some(last_max_location) = self.max_value_locations.pop() {
                // Move all iterators to next value
                for location in &self.max_value_locations {
                    match location {
                        DataLocation::Db => {
                            let _ = self.db_iter.next().unwrap();
                        }
                        Snapshot(idx) => {
                            let _ = self.snapshot_iterators[*idx].next().unwrap();
                        }
                    }
                }

                // Handle next value
                match last_max_location {
                    DataLocation::Db => {
                        let (key, value) = self.db_iter.next().unwrap();
                        return Some((key, value));
                    }
                    Snapshot(idx) => {
                        let (key, operation) = self.snapshot_iterators[idx].next().unwrap();
                        match operation {
                            Operation::Put { value } => {
                                return Some((key.to_vec(), value.to_vec()))
                            }
                            Operation::Delete => continue,
                        }
                    }
                };
            } else {
                break;
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    use sov_db::rocks_db_config::gen_rocksdb_options;

    use crate::cache::{CacheDb, NoopQueryManager, QueryManager};
    use crate::cache_container::CacheContainer;
    use crate::schema::{KeyDecoder, ValueCodec};
    use crate::test::TestField;
    use crate::{define_schema, SchemaBatch};

    const DUMMY_STATE_CF: &str = "DummyStateCF";

    define_schema!(DummyStateSchema, TestField, TestField, DUMMY_STATE_CF);
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
        let snapshot_manager =
            CacheContainer::new(db, Arc::new(RwLock::new(HashMap::new())).into());
        assert!(snapshot_manager.is_empty());
    }

    #[test]
    fn test_add_and_discard_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let snapshot_id = 1;
        let db_snapshot = CacheDb::new(snapshot_id, query_manager.clone().into());

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
        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let snapshot_id = 1;
        // Both share the same ID
        let db_snapshot_1 = CacheDb::new(snapshot_id, query_manager.clone().into());
        let db_snapshot_2 = CacheDb::new(snapshot_id, query_manager.clone().into());

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
        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());

        snapshot_manager.commit_snapshot(&1).unwrap();
    }

    #[test]
    fn test_discard_unknown() {
        // Discarding unknown snapshots are fine.
        // As it possible that caller didn't save it previously.
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());

        snapshot_manager.discard_snapshot(&1);
    }

    #[test]
    fn test_commit_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));
        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let snapshot_id = 1;
        let db_snapshot = CacheDb::new(snapshot_id, query_manager.clone().into());

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
        let snapshot_manager = CacheContainer::new(db, to_parent.clone().into());
        assert_eq!(
            None,
            snapshot_manager.get::<Schema>(1, &TestField(1)).unwrap()
        );
    }

    #[test]
    fn test_query_genesis_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        let db = create_test_db(tempdir.path());
        let to_parent = Arc::new(RwLock::new(HashMap::new()));

        let one = TestField(1);
        let two = TestField(2);
        let three = TestField(3);

        let mut db_data = SchemaBatch::new();
        db_data.put::<Schema>(&one, &one).unwrap();
        db_data.put::<Schema>(&three, &three).unwrap();
        db.write_schemas(db_data).unwrap();

        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());
        let query_manager = Arc::new(RwLock::new(NoopQueryManager));

        let db_snapshot = CacheDb::new(1, query_manager.clone().into());
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

        let f1 = TestField(1);
        let f2 = TestField(2);
        let f3 = TestField(3);
        let f4 = TestField(4);
        let f5 = TestField(5);
        let f6 = TestField(6);
        let f7 = TestField(7);
        let f8 = TestField(8);

        let mut db_data = SchemaBatch::new();
        db_data.put::<Schema>(&f1, &f1).unwrap();
        db.write_schemas(db_data).unwrap();

        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());
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
        let db_snapshot = CacheDb::new(1, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f2, &f2).unwrap();
        db_snapshot.put::<Schema>(&f3, &f4).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 2
        let db_snapshot = CacheDb::new(2, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f1, &f5).unwrap();
        db_snapshot.delete::<Schema>(&f2).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 3
        let db_snapshot = CacheDb::new(3, query_manager.clone().into());
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 4
        let db_snapshot = CacheDb::new(4, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f3, &f6).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 5
        let db_snapshot = CacheDb::new(5, query_manager.clone().into());
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 6
        let db_snapshot = CacheDb::new(6, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f1, &f7).unwrap();
        db_snapshot.put::<Schema>(&f2, &f8).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 7
        let db_snapshot = CacheDb::new(7, query_manager.clone().into());
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

        let f1 = TestField(1);
        let f2 = TestField(2);
        let f3 = TestField(3);
        let f4 = TestField(4);
        let f5 = TestField(5);
        let f6 = TestField(6);
        let f7 = TestField(7);
        let f8 = TestField(8);
        let f9 = TestField(9);
        let f10 = TestField(10);
        let f12 = TestField(12);

        let mut db_data = SchemaBatch::new();
        db_data.put::<Schema>(&f3, &f9).unwrap();
        db_data.put::<Schema>(&f2, &f1).unwrap();
        db_data.put::<Schema>(&f4, &f1).unwrap();
        db.write_schemas(db_data).unwrap();

        let mut snapshot_manager = CacheContainer::new(db, to_parent.clone().into());
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
        let db_snapshot = CacheDb::new(1, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f1, &f8).unwrap();
        db_snapshot.put::<Schema>(&f5, &f7).unwrap();
        db_snapshot.put::<Schema>(&f8, &f3).unwrap();
        db_snapshot.put::<Schema>(&f4, &f2).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 2
        let db_snapshot = CacheDb::new(2, query_manager.clone().into());
        db_snapshot.put::<Schema>(&f10, &f2).unwrap();
        db_snapshot.put::<Schema>(&f9, &f4).unwrap();
        db_snapshot.delete::<Schema>(&f4).unwrap();
        db_snapshot.put::<Schema>(&f2, &f6).unwrap();
        snapshot_manager.add_snapshot(db_snapshot.into());

        // 3
        let db_snapshot = CacheDb::new(3, query_manager.clone().into());
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
        // |   3 |     9 |
        // |   2 |     6 |
        // |   1 |     2 |

        let expected_fields = vec![
            (f12, f1),
            (f10, f2),
            (f8, f6),
            (f5, f7),
            (f3, f9),
            (f2, f6),
            (f1, f2),
        ];

        let i = snapshot_manager.iter::<Schema>(4).unwrap();
        let actual_fields: Vec<_> = i
            .into_iter()
            .map(|(k, v)| {
                let key = <<DummyStateSchema as sov_schema_db::Schema>::Key as KeyDecoder<
                    Schema,
                >>::decode_key(&k)
                .unwrap();
                let value = <<DummyStateSchema as sov_schema_db::Schema>::Value as ValueCodec<
                    Schema,
                >>::decode_value(&v)
                .unwrap();
                (key, value)
            })
            .collect();

        assert_eq!(actual_fields, expected_fields);
    }
}
