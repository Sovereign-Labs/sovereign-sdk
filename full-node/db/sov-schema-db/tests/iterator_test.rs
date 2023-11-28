// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, RwLock};

use anyhow::Result;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use rocksdb::DEFAULT_COLUMN_FAMILY_NAME;
use sov_schema_db::schema::{KeyDecoder, KeyEncoder, Schema, ValueCodec};
use sov_schema_db::snapshot::{DbSnapshot, ReadOnlyLock};
use sov_schema_db::{
    define_schema, CodecError, Operation, SchemaBatch, SchemaIterator, SeekKeyEncoder, DB,
};
use tempfile::TempDir;

use crate::liner_snapshot_manager::LinearSnapshotManager;

mod liner_snapshot_manager;

define_schema!(TestSchema, TestKey, TestValue, "TestCF");

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TestKey(u32, u32, u32);

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TestValue(u32);

impl KeyEncoder<TestSchema> for TestKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        let mut bytes = vec![];
        bytes
            .write_u32::<BigEndian>(self.0)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        bytes
            .write_u32::<BigEndian>(self.1)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        bytes
            .write_u32::<BigEndian>(self.2)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        Ok(bytes)
    }
}

impl KeyDecoder<TestSchema> for TestKey {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        let mut reader = std::io::Cursor::new(data);
        Ok(TestKey(
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
        ))
    }
}

impl SeekKeyEncoder<TestSchema> for TestKey {
    fn encode_seek_key(&self) -> sov_schema_db::schema::Result<Vec<u8>> {
        self.encode_key()
    }
}

impl ValueCodec<TestSchema> for TestValue {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0.to_be_bytes().to_vec())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        let mut reader = std::io::Cursor::new(data);
        Ok(TestValue(
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
        ))
    }
}

pub struct KeyPrefix1(u32);

impl SeekKeyEncoder<TestSchema> for KeyPrefix1 {
    fn encode_seek_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0.to_be_bytes().to_vec())
    }
}

pub struct KeyPrefix2(u32, u32);

impl SeekKeyEncoder<TestSchema> for KeyPrefix2 {
    fn encode_seek_key(&self) -> Result<Vec<u8>, CodecError> {
        let mut bytes = vec![];
        bytes
            .write_u32::<BigEndian>(self.0)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        bytes
            .write_u32::<BigEndian>(self.1)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        Ok(bytes)
    }
}

fn collect_values(iter: SchemaIterator<TestSchema>) -> Vec<u32> {
    iter.map(|row| (row.unwrap().1).0).collect()
}

struct TestDB {
    _tmpdir: TempDir,
    db: DB,
}

impl TestDB {
    fn new() -> Self {
        let tmpdir = tempfile::tempdir().unwrap();
        let column_families = vec![DEFAULT_COLUMN_FAMILY_NAME, TestSchema::COLUMN_FAMILY_NAME];
        let mut db_opts = rocksdb::Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        let db = DB::open(tmpdir.path(), "test", column_families, &db_opts).unwrap();

        db.put::<TestSchema>(&TestKey(1, 0, 0), &TestValue(100))
            .unwrap();
        db.put::<TestSchema>(&TestKey(1, 0, 2), &TestValue(102))
            .unwrap();
        db.put::<TestSchema>(&TestKey(1, 0, 4), &TestValue(104))
            .unwrap();
        db.put::<TestSchema>(&TestKey(1, 1, 0), &TestValue(110))
            .unwrap();
        db.put::<TestSchema>(&TestKey(1, 1, 2), &TestValue(112))
            .unwrap();
        db.put::<TestSchema>(&TestKey(1, 1, 4), &TestValue(114))
            .unwrap();
        db.put::<TestSchema>(&TestKey(2, 0, 0), &TestValue(200))
            .unwrap();
        db.put::<TestSchema>(&TestKey(2, 0, 2), &TestValue(202))
            .unwrap();

        TestDB {
            _tmpdir: tmpdir,
            db,
        }
    }
}

impl TestDB {
    fn iter(&self) -> SchemaIterator<TestSchema> {
        self.db.iter().expect("Failed to create iterator.")
    }

    fn rev_iter(&self) -> SchemaIterator<TestSchema> {
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
    iter.seek(&TestKey(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek(&TestKey(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 104, 102, 100]);
}

#[test]
fn test_seek_by_nonexistent_key() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek(&TestKey(1, 1, 1)).unwrap();
    assert_eq!(collect_values(iter), [112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek(&TestKey(1, 1, 1)).unwrap();
    assert_eq!(collect_values(iter), [112, 110, 104, 102, 100]);
}

#[test]
fn test_seek_for_prev_by_existing_key() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_for_prev(&TestKey(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek_for_prev(&TestKey(1, 1, 0)).unwrap();
    assert_eq!(collect_values(iter), [110, 104, 102, 100]);
}

#[test]
fn test_seek_for_prev_by_nonexistent_key() {
    let db = TestDB::new();

    let mut iter = db.iter();
    iter.seek_for_prev(&TestKey(1, 1, 1)).unwrap();
    assert_eq!(collect_values(iter), [110, 112, 114, 200, 202]);

    let mut iter = db.rev_iter();
    iter.seek_for_prev(&TestKey(1, 1, 1)).unwrap();
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
        (TestKey(2, 0, 0), TestValue(600)),
        (TestKey(1, 3, 4), TestValue(500)),
        (TestKey(1, 3, 3), TestValue(400)),
        (TestKey(1, 3, 2), TestValue(300)),
        (TestKey(1, 3, 0), TestValue(200)),
        (TestKey(1, 2, 0), TestValue(100)),
    ];

    // Insert them out of order
    for i in [4, 2, 0, 1, 3, 5] {
        let (key, value) = &operations[i];
        batch.put::<TestSchema>(key, value).unwrap();
    }

    let iter = batch.iter::<TestSchema>();
    let collected: Vec<_> = iter
        .filter_map(|(key, value)| match value {
            Operation::Put { value } => Some((
                TestKey::decode_key(key).unwrap(),
                TestValue::decode_value(value).unwrap(),
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
        .put::<TestSchema>(&TestKey(8, 0, 0), &TestValue(6))
        .unwrap();
    batch.delete::<TestSchema>(&TestKey(9, 0, 0)).unwrap();
    batch
        .put::<TestSchema>(&TestKey(12, 0, 0), &TestValue(1))
        .unwrap();
    batch
        .put::<TestSchema>(&TestKey(1, 0, 0), &TestValue(2))
        .unwrap();
    let mut iter = batch.iter::<TestSchema>().peekable();
    let first1 = iter.peek().unwrap();
    assert_eq!(first1.0, &TestKey(12, 0, 0).encode_key().unwrap(),);
    assert_eq!(
        first1.1,
        &Operation::Put {
            value: TestValue(1).encode_value().unwrap()
        }
    );
    let collected: Vec<_> = iter.collect();
    assert_eq!(4, collected.len());
}

#[test]
fn test_db_snapshot_iteration() {}

#[test]
fn test_db_snapshot_get_last_value() {
    let manager = Arc::new(RwLock::new(LinearSnapshotManager::default()));

    let snapshot_1 =
        DbSnapshot::<LinearSnapshotManager>::new(0, ReadOnlyLock::new(manager.clone()));

    assert!(snapshot_1.get_largest::<TestSchema>().unwrap().is_none());

    snapshot_1
        .put::<TestSchema>(&TestKey(8, 2, 3), &TestValue(6))
        .unwrap();

    {
        let latest = snapshot_1.get_largest::<TestSchema>().unwrap();
        assert_eq!(Some(TestValue(6)), latest);
    }

    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_1.into());
    }

    let snapshot_2 =
        DbSnapshot::<LinearSnapshotManager>::new(1, ReadOnlyLock::new(manager.clone()));

    {
        let latest = snapshot_2.get_largest::<TestSchema>().unwrap();
        assert_eq!(Some(TestValue(6)), latest);
    }

    snapshot_2.put(&TestKey(8, 1, 3), &TestValue(7)).unwrap();
    {
        let latest = snapshot_2.get_largest::<TestSchema>().unwrap();
        assert_eq!(Some(TestValue(6)), latest);
    }
    snapshot_2.put(&TestKey(8, 3, 1), &TestValue(8)).unwrap();
    {
        let latest = snapshot_2.get_largest::<TestSchema>().unwrap();
        assert_eq!(Some(TestValue(8)), latest);
    }
}

#[test]
fn test_db_snapshot_get_prev_value() {}
