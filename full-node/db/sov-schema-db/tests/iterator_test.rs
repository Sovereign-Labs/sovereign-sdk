// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, RwLock};

use rocksdb::DEFAULT_COLUMN_FAMILY_NAME;
use sov_schema_db::schema::{KeyDecoder, KeyEncoder, ValueCodec};
use sov_schema_db::snapshot::{DbSnapshot, ReadOnlyLock};
use sov_schema_db::test::{KeyPrefix1, KeyPrefix2, TestCompositeField, TestField};
use sov_schema_db::{define_schema, Operation, Schema, SchemaBatch, SchemaIterator, DB};
use tempfile::TempDir;

use crate::liner_snapshot_manager::LinearSnapshotManager;

mod liner_snapshot_manager;

define_schema!(TestSchema, TestCompositeField, TestField, "TestCF");

type S = TestSchema;

fn collect_values(iter: SchemaIterator<S>) -> Vec<u32> {
    iter.map(|row| row.unwrap().value.0).collect()
}

fn decode_key(key: &[u8]) -> TestCompositeField {
    <TestCompositeField as KeyDecoder<S>>::decode_key(key).unwrap()
}

fn encode_key(key: &TestCompositeField) -> Vec<u8> {
    <TestCompositeField as KeyEncoder<S>>::encode_key(key).unwrap()
}

fn encode_value(value: &TestField) -> Vec<u8> {
    <TestField as ValueCodec<S>>::encode_value(value).unwrap()
}

struct TestDB {
    _tmpdir: TempDir,
    db: DB,
}

impl TestDB {
    fn new() -> Self {
        let tmpdir = tempfile::tempdir().unwrap();
        let column_families = vec![DEFAULT_COLUMN_FAMILY_NAME, S::COLUMN_FAMILY_NAME];
        let mut db_opts = rocksdb::Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        let db = DB::open(tmpdir.path(), "test", column_families, &db_opts).unwrap();

        db.put::<S>(&TestCompositeField(1, 0, 0), &TestField(100))
            .unwrap();
        db.put::<S>(&TestCompositeField(1, 0, 2), &TestField(102))
            .unwrap();
        db.put::<S>(&TestCompositeField(1, 0, 4), &TestField(104))
            .unwrap();
        db.put::<S>(&TestCompositeField(1, 1, 0), &TestField(110))
            .unwrap();
        db.put::<S>(&TestCompositeField(1, 1, 2), &TestField(112))
            .unwrap();
        db.put::<S>(&TestCompositeField(1, 1, 4), &TestField(114))
            .unwrap();
        db.put::<S>(&TestCompositeField(2, 0, 0), &TestField(200))
            .unwrap();
        db.put::<S>(&TestCompositeField(2, 0, 2), &TestField(202))
            .unwrap();

        TestDB {
            _tmpdir: tmpdir,
            db,
        }
    }
}

impl TestDB {
    fn iter(&self) -> SchemaIterator<S> {
        self.db.iter().expect("Failed to create iterator.")
    }

    fn rev_iter(&self) -> SchemaIterator<S> {
        self.db.iter().expect("Failed to create iterator.").rev()
    }
}

impl std::ops::Deref for TestDB {
    type Target = DB;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

#[test]
fn test_seek_to_first() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_to_first();
    assert_eq!(
        collect_values(iter),
        [100, 102, 104, 110, 112, 114, 200, 202]
    );

    let mut iter = db.rev_iter();
    iter.seek_to_first();
    assert_eq!(collect_values(iter), [100]);
}

#[test]
fn test_seek_to_last() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_to_last();
    assert_eq!(collect_values(iter), [202]);

    let mut iter = db.rev_iter();
    iter.seek_to_last();
    assert_eq!(
        collect_values(iter),
        [202, 200, 114, 112, 110, 104, 102, 100]
    );
}

#[test]
fn test_seek_by_existing_key() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek(&TestCompositeField(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek(&TestCompositeField(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 104, 102, 100]);
}

#[test]
fn test_seek_by_nonexistent_key() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek(&TestCompositeField(1, 1, 1)).unwrap();
    assert_eq!(collect_values(iter), [112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek(&TestCompositeField(1, 1, 1)).unwrap();
    assert_eq!(collect_values(iter), [112, 110, 104, 102, 100]);
}

#[test]
fn test_seek_for_prev_by_existing_key() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_for_prev(&TestCompositeField(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek_for_prev(&TestCompositeField(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 104, 102, 100]);
}

#[test]
fn test_seek_for_prev_by_nonexistent_key() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_for_prev(&TestCompositeField(1, 1, 1)).unwrap();
    assert_eq!(collect_values(iter), [110, 112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek_for_prev(&TestCompositeField(1, 1, 1)).unwrap();
    assert_eq!(collect_values(iter), [110, 104, 102, 100]);
}

#[test]
fn test_seek_by_1prefix() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek(&KeyPrefix1(2)).unwrap();
    assert_eq!(collect_values(iter), [200, 202]);

    let mut iter = db.rev_iter();
    iter.seek(&KeyPrefix1(2)).unwrap();
    assert_eq!(collect_values(iter), [200, 114, 112, 110, 104, 102, 100]);
}

#[test]
fn test_seek_for_prev_by_1prefix() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_for_prev(&KeyPrefix1(2)).unwrap();
    assert_eq!(collect_values(iter), [114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek_for_prev(&KeyPrefix1(2)).unwrap();
    assert_eq!(collect_values(iter), [114, 112, 110, 104, 102, 100]);
}

#[test]
fn test_seek_by_2prefix() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek(&KeyPrefix2(2, 0)).unwrap();
    assert_eq!(collect_values(iter), [200, 202]);

    let mut iter = db.rev_iter();
    iter.seek(&KeyPrefix2(2, 0)).unwrap();
    assert_eq!(collect_values(iter), [200, 114, 112, 110, 104, 102, 100]);
}

#[test]
fn test_seek_for_prev_by_2prefix() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_for_prev(&KeyPrefix2(2, 0)).unwrap();
    assert_eq!(collect_values(iter), [114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek_for_prev(&KeyPrefix2(2, 0)).unwrap();
    assert_eq!(collect_values(iter), [114, 112, 110, 104, 102, 100]);
}

#[test]
fn test_schema_batch_iteration_order() {
    let mut batch = SchemaBatch::new();

    // Operations in expected order
    let operations = vec![
        (TestCompositeField(2, 0, 0), TestField(600)),
        (TestCompositeField(1, 3, 4), TestField(500)),
        (TestCompositeField(1, 3, 3), TestField(400)),
        (TestCompositeField(1, 3, 2), TestField(300)),
        (TestCompositeField(1, 3, 0), TestField(200)),
        (TestCompositeField(1, 2, 0), TestField(100)),
    ];

    // Insert them out of order
    for i in [4, 2, 0, 1, 3, 5] {
        let (key, value) = &operations[i];
        batch.put::<S>(key, value).unwrap();
    }

    let iter = batch.iter::<S>();
    let collected: Vec<_> = iter
        .filter_map(|(key, value)| match value {
            Operation::Put { value } => Some((
                decode_key(key),
                <TestField as ValueCodec<S>>::decode_value(value).unwrap(),
            )),
            Operation::Delete => None,
        })
        .collect();
    assert_eq!(operations, collected);
}

#[test]
fn test_schema_batch_iteration_with_deletions() {
    let mut batch = SchemaBatch::new();

    batch
        .put::<S>(&TestCompositeField(8, 0, 0), &TestField(6))
        .unwrap();
    batch.delete::<S>(&TestCompositeField(9, 0, 0)).unwrap();
    batch
        .put::<S>(&TestCompositeField(12, 0, 0), &TestField(1))
        .unwrap();
    batch
        .put::<S>(&TestCompositeField(1, 0, 0), &TestField(2))
        .unwrap();
    let mut iter = batch.iter::<S>().peekable();
    let first1 = iter.peek().unwrap();
    assert_eq!(first1.0, &encode_key(&TestCompositeField(12, 0, 0)));
    assert_eq!(
        first1.1,
        &Operation::Put {
            value: encode_value(&TestField(1)),
        }
    );
    let collected: Vec<_> = iter.collect();
    assert_eq!(4, collected.len());
}

#[test]
fn test_db_snapshot_get_last_value() {
    let manager = Arc::new(RwLock::new(LinearSnapshotManager::default()));

    let snapshot_1 =
        DbSnapshot::<LinearSnapshotManager>::new(0, ReadOnlyLock::new(manager.clone()));

    assert!(snapshot_1.get_largest::<S>().unwrap().is_none());

    snapshot_1
        .put::<S>(&TestCompositeField(8, 2, 3), &TestField(6))
        .unwrap();

    {
        let latest = snapshot_1.get_largest::<S>().unwrap();
        assert_eq!(Some(TestField(6)), latest);
    }

    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_1.into());
    }

    let snapshot_2 =
        DbSnapshot::<LinearSnapshotManager>::new(1, ReadOnlyLock::new(manager.clone()));

    {
        let latest = snapshot_2.get_largest::<S>().unwrap();
        assert_eq!(Some(TestField(6)), latest);
    }

    snapshot_2
        .put::<S>(&TestCompositeField(8, 1, 3), &TestField(7))
        .unwrap();
    {
        let latest = snapshot_2.get_largest::<S>().unwrap();
        assert_eq!(Some(TestField(6)), latest);
    }
    // Largest value from local is picked up
    snapshot_2
        .put::<S>(&TestCompositeField(8, 3, 1), &TestField(8))
        .unwrap();
    {
        let latest = snapshot_2.get_largest::<S>().unwrap();
        assert_eq!(Some(TestField(8)), latest);
    }

    // Deletion: Previous "largest" value is returned
    snapshot_2
        .delete::<S>(&TestCompositeField(8, 3, 1))
        .unwrap();
    {
        let latest = snapshot_2.get_largest::<S>().unwrap();
        assert_eq!(Some(TestField(6)), latest);
    }
}

#[test]
fn test_db_snapshot_get_prev_value() {
    let manager = Arc::new(RwLock::new(LinearSnapshotManager::default()));

    // Snapshots 1 and 2 are to black box usages of parents iterator
    let snapshot_1 =
        DbSnapshot::<LinearSnapshotManager>::new(0, ReadOnlyLock::new(manager.clone()));

    assert!(snapshot_1
        .get_prev::<S>(&TestCompositeField(8, 2, 3))
        .unwrap()
        .is_none());

    snapshot_1
        .put::<S>(&TestCompositeField(8, 2, 0), &TestField(10))
        .unwrap();
    snapshot_1
        .put::<S>(&TestCompositeField(8, 2, 3), &TestField(1))
        .unwrap();
    snapshot_1
        .put::<S>(&TestCompositeField(8, 1, 3), &TestField(11))
        .unwrap();
    snapshot_1
        .put::<S>(&TestCompositeField(7, 2, 3), &TestField(12))
        .unwrap();
    snapshot_1
        .put::<S>(&TestCompositeField(8, 2, 5), &TestField(13))
        .unwrap();
    snapshot_1
        .put::<S>(&TestCompositeField(8, 3, 2), &TestField(14))
        .unwrap();

    // Equal:
    assert_eq!(
        Some(TestField(1)),
        snapshot_1
            .get_prev::<S>(&TestCompositeField(8, 2, 3))
            .unwrap()
    );
    // Previous: value from 8.2.0
    assert_eq!(
        Some(TestField(10)),
        snapshot_1
            .get_prev::<S>(&TestCompositeField(8, 2, 1))
            .unwrap()
    );

    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_1.into());
    }

    let snapshot_2 =
        DbSnapshot::<LinearSnapshotManager>::new(1, ReadOnlyLock::new(manager.clone()));
    // Equal:
    assert_eq!(
        Some(TestField(1)),
        snapshot_2
            .get_prev::<S>(&TestCompositeField(8, 2, 3))
            .unwrap()
    );
    // Previous: value from 8.2.0
    assert_eq!(
        Some(TestField(10)),
        snapshot_2
            .get_prev::<S>(&TestCompositeField(8, 2, 1))
            .unwrap()
    );
    snapshot_2
        .put::<S>(&TestCompositeField(8, 2, 0), &TestField(20))
        .unwrap();
    snapshot_2
        .put::<S>(&TestCompositeField(8, 2, 3), &TestField(2))
        .unwrap();
    // Updated values are higher priority
    assert_eq!(
        Some(TestField(2)),
        snapshot_2
            .get_prev::<S>(&TestCompositeField(8, 2, 3))
            .unwrap()
    );
    assert_eq!(
        Some(TestField(20)),
        snapshot_2
            .get_prev::<S>(&TestCompositeField(8, 2, 1))
            .unwrap()
    );
    snapshot_2
        .delete::<S>(&TestCompositeField(8, 2, 3))
        .unwrap();
    assert_eq!(
        Some(TestField(20)),
        snapshot_2
            .get_prev::<S>(&TestCompositeField(8, 2, 3))
            .unwrap()
    );
    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_2.into());
    }
    let snapshot_3 =
        DbSnapshot::<LinearSnapshotManager>::new(2, ReadOnlyLock::new(manager.clone()));
    assert_eq!(
        Some(TestField(20)),
        snapshot_3
            .get_prev::<S>(&TestCompositeField(8, 2, 1))
            .unwrap()
    );
    assert_eq!(
        Some(TestField(20)),
        snapshot_3
            .get_prev::<S>(&TestCompositeField(8, 2, 3))
            .unwrap()
    );
    assert_eq!(
        Some(TestField(14)),
        snapshot_3
            .get_prev::<S>(&TestCompositeField(8, 3, 4))
            .unwrap()
    );
}
