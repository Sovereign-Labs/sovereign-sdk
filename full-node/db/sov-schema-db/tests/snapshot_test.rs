use std::sync::{Arc, RwLock};

use sov_schema_db::define_schema;
use sov_schema_db::snapshot::{DbSnapshot, ReadOnlyLock, SingleSnapshotQueryManager};
use sov_schema_db::test::TestField;

define_schema!(TestSchema1, TestField, TestField, "TestCF1");

type S = TestSchema1;

#[test]
fn snapshot_lifecycle() {
    let manager = Arc::new(RwLock::new(SingleSnapshotQueryManager::default()));

    let key = TestField(1);
    let value = TestField(1);

    let snapshot_1 = DbSnapshot::new(0, ReadOnlyLock::new(manager.clone()));
    assert_eq!(
        None,
        snapshot_1.read::<S>(&key).unwrap(),
        "Incorrect value, should find nothing"
    );

    snapshot_1.put::<S>(&key, &value).unwrap();
    assert_eq!(
        Some(value),
        snapshot_1.read::<S>(&key).unwrap(),
        "Incorrect value, should be fetched from local cache"
    );
    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_1.into());
    }

    // Snapshot 2: reads value from snapshot 1, then deletes it
    let snapshot_2 = DbSnapshot::new(1, ReadOnlyLock::new(manager.clone()));
    assert_eq!(Some(value), snapshot_2.read::<S>(&key).unwrap());
    snapshot_2.delete::<S>(&key).unwrap();
    assert_eq!(None, snapshot_2.read::<S>(&key).unwrap());
    {
        let mut manager = manager.write().unwrap();
        manager.add_snapshot(snapshot_2.into());
    }

    // Snapshot 3: gets empty result, event value is in some previous snapshots
    let snapshot_3 = DbSnapshot::new(2, ReadOnlyLock::new(manager.clone()));
    assert_eq!(None, snapshot_3.read::<S>(&key).unwrap());
}
