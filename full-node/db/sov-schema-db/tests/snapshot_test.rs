use std::path::Path;
use std::sync::{Arc, RwLock};

use byteorder::{BigEndian, ReadBytesExt};
use rocksdb::DEFAULT_COLUMN_FAMILY_NAME;
use sov_schema_db::schema::{ColumnFamilyName, KeyCodec, KeyDecoder, KeyEncoder, ValueCodec};
use sov_schema_db::snapshot::{
    DbSnapshot, FrozenDbSnapshot, QueryManager, ReadOnlyLock, SnapshotId,
};
use sov_schema_db::{define_schema, CodecError, Operation, Schema, DB};

define_schema!(TestSchema1, TestField, TestField, "TestCF1");

#[derive(Debug, Eq, PartialEq, Clone)]
pub(crate) struct TestField(u32);

impl TestField {
    fn to_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().to_vec()
    }

    fn from_bytes(data: &[u8]) -> sov_schema_db::schema::Result<Self> {
        let mut reader = std::io::Cursor::new(data);
        Ok(TestField(
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
        ))
    }
}

impl KeyEncoder<TestSchema1> for TestField {
    fn encode_key(&self) -> sov_schema_db::schema::Result<Vec<u8>> {
        Ok(self.to_bytes())
    }
}

impl KeyDecoder<TestSchema1> for TestField {
    fn decode_key(data: &[u8]) -> sov_schema_db::schema::Result<Self> {
        Self::from_bytes(data)
    }
}

impl ValueCodec<TestSchema1> for TestField {
    fn encode_value(&self) -> sov_schema_db::schema::Result<Vec<u8>> {
        Ok(self.to_bytes())
    }

    fn decode_value(data: &[u8]) -> sov_schema_db::schema::Result<Self> {
        Self::from_bytes(data)
    }
}

#[derive(Default)]
struct LinearSnapshotManager {
    snapshots: Vec<FrozenDbSnapshot>,
}

impl LinearSnapshotManager {
    fn add_snapshot(&mut self, snapshot: FrozenDbSnapshot) {
        self.snapshots.push(snapshot);
    }
}

impl QueryManager for LinearSnapshotManager {
    fn get<S: Schema>(
        &self,
        snapshot_id: SnapshotId,
        key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<Operation>> {
        for snapshot in self.snapshots[..snapshot_id as usize].iter().rev() {
            if let Some(operation) = snapshot.get(key)? {
                return Ok(Some(operation));
            }
        }
        Ok(None)
    }
}

fn get_column_families() -> Vec<ColumnFamilyName> {
    vec![DEFAULT_COLUMN_FAMILY_NAME, TestSchema1::COLUMN_FAMILY_NAME]
}

fn open_db(dir: impl AsRef<Path>) -> DB {
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_if_missing(true);
    db_opts.create_missing_column_families(true);

    DB::open(dir, "test", get_column_families(), &db_opts).expect("Failed to open DB.")
}

#[test]
fn snapshot_lifecycle() {
    let tmpdir = tempfile::tempdir().unwrap();
    let db = Arc::new(open_db(&tmpdir));
    let manager = Arc::new(RwLock::new(LinearSnapshotManager::default()));

    let key_1 = TestField(1);
    let value_1 = TestField(1);
    // 1 => 1 in db
    db.put::<TestSchema1>(&key_1, &value_1).unwrap();

    // Snapshot 1: reads from DB first, then sets 1 => 2
    let snapshot_1 =
        DbSnapshot::<LinearSnapshotManager>::new(0, ReadOnlyLock::new(manager.clone()), db.clone());
    assert_eq!(
        Some(value_1.clone()),
        snapshot_1.read::<TestSchema1>(&key_1).unwrap(),
        "Incorrect value, should be fetched from DB"
    );
    // 1 => 2 in snapshot 1
    let value_2 = TestField(2);
    snapshot_1.put(&key_1, &value_2).unwrap();
    assert_eq!(
        Some(value_2.clone()),
        snapshot_1.read::<TestSchema1>(&key_1).unwrap(),
        "Incorrect value, should be fetched from local cache"
    );
    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_1.into());
    }

    // Snapshot 2: reads value from snapshot 1, then deletes it
    let snapshot_2 =
        DbSnapshot::<LinearSnapshotManager>::new(1, ReadOnlyLock::new(manager.clone()), db.clone());
    assert_eq!(
        Some(value_2.clone()),
        snapshot_2.read::<TestSchema1>(&key_1).unwrap()
    );
    snapshot_2.delete(&key_1).unwrap();
    assert_eq!(None, snapshot_2.read::<TestSchema1>(&key_1).unwrap());
    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_2.into());
    }

    // Snapshot 3: gets empty result, event value is still in DB, but it was deleted in previous snapshot
    let snapshot_3 =
        DbSnapshot::<LinearSnapshotManager>::new(2, ReadOnlyLock::new(manager.clone()), db.clone());
    assert_eq!(None, snapshot_3.read::<TestSchema1>(&key_1).unwrap());
}
