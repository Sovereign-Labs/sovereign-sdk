// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use rocksdb::DEFAULT_COLUMN_FAMILY_NAME;
use sov_schema_db::schema::{ColumnFamilyName, Result};
use sov_schema_db::test::TestField;
use sov_schema_db::{define_schema, Schema, SchemaBatch, DB};
use tempfile::TempDir;

// Creating two schemas that share exactly the same structure but are stored in different column
// families. Also note that the key and value are of the same type `TestField`. By implementing
// both the `KeyCodec<>` and `ValueCodec<>` traits for both schemas, we are able to use it
// everywhere.
define_schema!(TestSchema1, TestField, TestField, "TestCF1");
define_schema!(TestSchema2, TestField, TestField, "TestCF2");

fn get_column_families() -> Vec<ColumnFamilyName> {
    vec![
        DEFAULT_COLUMN_FAMILY_NAME,
        TestSchema1::COLUMN_FAMILY_NAME,
        TestSchema2::COLUMN_FAMILY_NAME,
    ]
}

fn open_db(dir: impl AsRef<Path>) -> DB {
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_if_missing(true);
    db_opts.create_missing_column_families(true);
    DB::open(dir, "test", get_column_families(), &db_opts).expect("Failed to open DB.")
}

fn open_db_read_only(dir: &TempDir) -> DB {
    DB::open_cf_readonly(
        &rocksdb::Options::default(),
        dir.path(),
        "test",
        get_column_families(),
    )
    .expect("Failed to open DB.")
}

fn open_db_as_secondary(dir: &TempDir, dir_sec: &TempDir) -> DB {
    DB::open_cf_as_secondary(
        &rocksdb::Options::default(),
        &dir.path(),
        &dir_sec.path(),
        "test",
        get_column_families(),
    )
    .expect("Failed to open DB.")
}

struct TestDB {
    _tmpdir: TempDir,
    db: DB,
}

impl TestDB {
    fn new() -> Self {
        let tmpdir = tempfile::tempdir().unwrap();
        let db = open_db(&tmpdir);

        TestDB {
            _tmpdir: tmpdir,
            db,
        }
    }
}

impl std::ops::Deref for TestDB {
    type Target = DB;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

#[test]
fn test_schema_put_get() {
    let db = TestDB::new();

    // Let's put more than 256 items in each to test RocksDB's lexicographic
    // ordering.
    for i in 0..300 {
        db.put::<TestSchema1>(&TestField(i), &TestField(i)).unwrap();
    }
    for i in 100..400 {
        db.put::<TestSchema2>(&TestField(i), &TestField(i + 1))
            .unwrap();
    }

    // `.get()`.
    assert_eq!(
        db.get::<TestSchema1>(&TestField(0)).unwrap(),
        Some(TestField(0)),
    );
    assert_eq!(
        db.get::<TestSchema1>(&TestField(1)).unwrap(),
        Some(TestField(1)),
    );
    assert_eq!(
        db.get::<TestSchema1>(&TestField(299)).unwrap(),
        Some(TestField(299)),
    );
    assert_eq!(
        db.get::<TestSchema2>(&TestField(102)).unwrap(),
        Some(TestField(103)),
    );
    assert_eq!(
        db.get::<TestSchema2>(&TestField(203)).unwrap(),
        Some(TestField(204)),
    );
    assert_eq!(
        db.get::<TestSchema2>(&TestField(399)).unwrap(),
        Some(TestField(400)),
    );

    // `collect_values()`.
    assert_eq!(
        collect_values::<TestSchema2>(&db),
        gen_expected_values(&(100..400).map(|i| (i, i + 1)).collect::<Vec<_>>()),
    );

    // Nonexistent keys.
    assert_eq!(db.get::<TestSchema1>(&TestField(300)).unwrap(), None);
    assert_eq!(db.get::<TestSchema2>(&TestField(99)).unwrap(), None);
    assert_eq!(db.get::<TestSchema2>(&TestField(400)).unwrap(), None);
}

fn collect_values<S: Schema>(db: &TestDB) -> Vec<(S::Key, S::Value)> {
    let mut iter = db.iter::<S>().expect("Failed to create iterator.");
    iter.seek_to_first();
    iter.map(|res| res.map(|item| item.into_tuple()))
        .collect::<Result<Vec<_>, anyhow::Error>>()
        .unwrap()
}

fn gen_expected_values(values: &[(u32, u32)]) -> Vec<(TestField, TestField)> {
    values
        .iter()
        .cloned()
        .map(|(x, y)| (TestField(x), TestField(y)))
        .collect()
}

#[test]
fn test_single_schema_batch() {
    let db = TestDB::new();

    let mut db_batch = SchemaBatch::new();
    db_batch
        .put::<TestSchema1>(&TestField(0), &TestField(0))
        .unwrap();
    db_batch
        .put::<TestSchema1>(&TestField(1), &TestField(1))
        .unwrap();
    db_batch
        .put::<TestSchema1>(&TestField(2), &TestField(2))
        .unwrap();
    db_batch
        .put::<TestSchema2>(&TestField(3), &TestField(3))
        .unwrap();
    db_batch.delete::<TestSchema2>(&TestField(4)).unwrap();
    db_batch.delete::<TestSchema2>(&TestField(3)).unwrap();
    db_batch
        .put::<TestSchema2>(&TestField(4), &TestField(4))
        .unwrap();
    db_batch
        .put::<TestSchema2>(&TestField(5), &TestField(5))
        .unwrap();

    db.write_schemas(db_batch).unwrap();

    assert_eq!(
        collect_values::<TestSchema1>(&db),
        gen_expected_values(&[(0, 0), (1, 1), (2, 2)]),
    );
    assert_eq!(
        collect_values::<TestSchema2>(&db),
        gen_expected_values(&[(4, 4), (5, 5)]),
    );
}

#[test]
fn test_two_schema_batches() {
    let db = TestDB::new();

    let mut db_batch1 = SchemaBatch::new();
    db_batch1
        .put::<TestSchema1>(&TestField(0), &TestField(0))
        .unwrap();
    db_batch1
        .put::<TestSchema1>(&TestField(1), &TestField(1))
        .unwrap();
    db_batch1
        .put::<TestSchema1>(&TestField(2), &TestField(2))
        .unwrap();
    db_batch1.delete::<TestSchema1>(&TestField(2)).unwrap();
    db.write_schemas(db_batch1).unwrap();

    assert_eq!(
        collect_values::<TestSchema1>(&db),
        gen_expected_values(&[(0, 0), (1, 1)]),
    );

    let mut db_batch2 = SchemaBatch::new();
    db_batch2.delete::<TestSchema2>(&TestField(3)).unwrap();
    db_batch2
        .put::<TestSchema2>(&TestField(3), &TestField(3))
        .unwrap();
    db_batch2
        .put::<TestSchema2>(&TestField(4), &TestField(4))
        .unwrap();
    db_batch2
        .put::<TestSchema2>(&TestField(5), &TestField(5))
        .unwrap();
    db.write_schemas(db_batch2).unwrap();

    assert_eq!(
        collect_values::<TestSchema1>(&db),
        gen_expected_values(&[(0, 0), (1, 1)]),
    );
    assert_eq!(
        collect_values::<TestSchema2>(&db),
        gen_expected_values(&[(3, 3), (4, 4), (5, 5)]),
    );
}

#[test]
fn test_reopen() {
    let tmpdir = tempfile::tempdir().unwrap();
    {
        let db = open_db(&tmpdir);
        db.put::<TestSchema1>(&TestField(0), &TestField(0)).unwrap();
        assert_eq!(
            db.get::<TestSchema1>(&TestField(0)).unwrap(),
            Some(TestField(0)),
        );
    }
    {
        let db = open_db(&tmpdir);
        assert_eq!(
            db.get::<TestSchema1>(&TestField(0)).unwrap(),
            Some(TestField(0)),
        );
    }
}

#[test]
fn test_open_read_only() {
    let tmpdir = tempfile::tempdir().unwrap();
    {
        let db = open_db(&tmpdir);
        db.put::<TestSchema1>(&TestField(0), &TestField(0)).unwrap();
    }
    {
        let db = open_db_read_only(&tmpdir);
        assert_eq!(
            db.get::<TestSchema1>(&TestField(0)).unwrap(),
            Some(TestField(0)),
        );
        assert!(db.put::<TestSchema1>(&TestField(1), &TestField(1)).is_err());
    }
}

#[test]
fn test_open_as_secondary() {
    let tmpdir = tempfile::tempdir().unwrap();
    let tmpdir_sec = tempfile::tempdir().unwrap();

    let db = open_db(&tmpdir);
    db.put::<TestSchema1>(&TestField(0), &TestField(0)).unwrap();

    let db_sec = open_db_as_secondary(&tmpdir, &tmpdir_sec);
    assert_eq!(
        db_sec.get::<TestSchema1>(&TestField(0)).unwrap(),
        Some(TestField(0)),
    );
}

#[test]
fn test_report_size() {
    let db = TestDB::new();

    for i in 0..1000 {
        let mut db_batch = SchemaBatch::new();
        db_batch
            .put::<TestSchema1>(&TestField(i), &TestField(i))
            .unwrap();
        db_batch
            .put::<TestSchema2>(&TestField(i), &TestField(i))
            .unwrap();
        db.write_schemas(db_batch).unwrap();
    }

    db.flush_cf("TestCF1").unwrap();
    db.flush_cf("TestCF2").unwrap();

    assert!(
        db.get_property("TestCF1", "rocksdb.estimate-live-data-size")
            .unwrap()
            > 0
    );
    assert!(
        db.get_property("TestCF2", "rocksdb.estimate-live-data-size")
            .unwrap()
            > 0
    );
    assert_eq!(
        db.get_property("default", "rocksdb.estimate-live-data-size")
            .unwrap(),
        0
    );
}

#[test]
fn test_checkpoint() {
    let tmpdir = tempfile::tempdir().unwrap();
    let checkpoint_parent = tempfile::tempdir().unwrap();
    let checkpoint = checkpoint_parent.path().join("checkpoint");
    {
        let db = open_db(&tmpdir);
        db.put::<TestSchema1>(&TestField(0), &TestField(0)).unwrap();
        db.create_checkpoint(&checkpoint).unwrap();
    }
    {
        let db = open_db(&tmpdir);
        assert_eq!(
            db.get::<TestSchema1>(&TestField(0)).unwrap(),
            Some(TestField(0)),
        );

        let cp = open_db(&checkpoint);
        assert_eq!(
            cp.get::<TestSchema1>(&TestField(0)).unwrap(),
            Some(TestField(0)),
        );
        cp.put::<TestSchema1>(&TestField(1), &TestField(1)).unwrap();
        assert_eq!(
            cp.get::<TestSchema1>(&TestField(1)).unwrap(),
            Some(TestField(1)),
        );
        assert_eq!(db.get::<TestSchema1>(&TestField(1)).unwrap(), None);
    }
}
