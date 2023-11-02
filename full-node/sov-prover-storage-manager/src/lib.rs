mod dummy_storage;
mod snapshot_manager;

use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_schema_db::snapshot::{DbSnapshot, FrozenDbSnapshot, ReadOnlyLock, SnapshotId};
use sov_state::MerkleProofSpec;

use crate::dummy_storage::NewProverStorage;
use crate::snapshot_manager::SnapshotManager;

struct NewProverStorageManager<Da: DaSpec, S: MerkleProofSpec> {
    // L1 forks representation
    // Chain: prev_block -> child_blocks
    chain_forks: HashMap<Da::SlotHash, Vec<Da::SlotHash>>,
    // Reverse: child_block -> parent
    blocks_to_parent: HashMap<Da::SlotHash, Da::SlotHash>,

    latest_snapshot_id: SnapshotId,
    block_hash_to_snapshot_id: HashMap<Da::SlotHash, SnapshotId>,
    // Same reference for individual managers
    snapshot_id_to_parent: Arc<RwLock<HashMap<SnapshotId, SnapshotId>>>,

    state_snapshot_manager: Arc<RwLock<SnapshotManager>>,
    native_snapshot_manager: Arc<RwLock<SnapshotManager>>,

    phantom_mp_spec: PhantomData<S>,
}

impl<Da: DaSpec, S: MerkleProofSpec> NewProverStorageManager<Da, S> {
    #[allow(dead_code)]
    pub fn new(state_db: sov_schema_db::DB, native_db: sov_schema_db::DB) -> Self {
        let snapshot_id_to_parent = Arc::new(RwLock::new(HashMap::new()));

        let state_snapshot_manager = SnapshotManager::new(state_db, snapshot_id_to_parent.clone());
        let native_snapshot_manager =
            SnapshotManager::new(native_db, snapshot_id_to_parent.clone());

        Self {
            chain_forks: Default::default(),
            blocks_to_parent: Default::default(),
            latest_snapshot_id: 0,
            block_hash_to_snapshot_id: Default::default(),
            snapshot_id_to_parent,
            state_snapshot_manager: Arc::new(RwLock::new(state_snapshot_manager)),
            native_snapshot_manager: Arc::new(RwLock::new(native_snapshot_manager)),
            phantom_mp_spec: Default::default(),
        }
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.chain_forks.is_empty()
            && self.blocks_to_parent.is_empty()
            && self.block_hash_to_snapshot_id.is_empty()
            && self.snapshot_id_to_parent.read().unwrap().is_empty()
            && self.state_snapshot_manager.read().unwrap().is_empty()
            && self.native_snapshot_manager.read().unwrap().is_empty()
    }
}

impl<Da: DaSpec, S: MerkleProofSpec> HierarchicalStorageManager<Da>
    for NewProverStorageManager<Da, S>
where
    Da::SlotHash: Hash,
{
    type NativeStorage = NewProverStorage<S, SnapshotManager>;
    type NativeChangeSet = NewProverStorage<S, SnapshotManager>;

    fn get_native_storage_on(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<Self::NativeStorage> {
        let current_block_hash = block_header.hash();
        let prev_block_hash = block_header.prev_hash();
        assert_ne!(
            current_block_hash, prev_block_hash,
            "Cannot provide storage for corrupt block"
        );

        let new_snapshot_id = match self.block_hash_to_snapshot_id.get(&current_block_hash) {
            // Storage for this block has been requested before
            Some(snapshot_id) => {
                // TODO: Do consistency checks here?

                *snapshot_id
            }
            // Storage requested first time
            None => {
                let new_snapshot_id = self.latest_snapshot_id + 1;
                if let Some(parent_snapshot_id) =
                    self.block_hash_to_snapshot_id.get(&prev_block_hash)
                {
                    let mut snapshot_id_to_parent = self.snapshot_id_to_parent.write().unwrap();
                    snapshot_id_to_parent.insert(new_snapshot_id, *parent_snapshot_id);
                }

                self.block_hash_to_snapshot_id
                    .insert(current_block_hash.clone(), new_snapshot_id);

                self.chain_forks
                    .entry(prev_block_hash.clone())
                    .or_default()
                    .push(current_block_hash.clone());

                self.blocks_to_parent
                    .insert(current_block_hash, prev_block_hash);

                // Update latest snapshot id
                self.latest_snapshot_id = new_snapshot_id;
                new_snapshot_id
            }
        };
        println!(
            "BLOCK HEIGHT={} SNAP_ID={}",
            block_header.height(),
            new_snapshot_id
        );

        let state_db_snapshot = DbSnapshot::new(
            new_snapshot_id,
            ReadOnlyLock::new(self.state_snapshot_manager.clone()),
        );

        let native_db_snapshot = DbSnapshot::new(
            new_snapshot_id,
            ReadOnlyLock::new(self.native_snapshot_manager.clone()),
        );

        Ok(NewProverStorage::with_db_handlers(
            state_db_snapshot,
            native_db_snapshot,
        ))
    }

    fn save_change_set(
        &mut self,
        block_header: &Da::BlockHeader,
        change_set: Self::NativeChangeSet,
    ) -> anyhow::Result<()> {
        if !self.chain_forks.contains_key(&block_header.prev_hash()) {
            anyhow::bail!("Attempt to save changeset for unknown block header");
        }
        let (state_db, native_db) = change_set.freeze();
        let state_snapshot: FrozenDbSnapshot = state_db.into();
        let native_snapshot: FrozenDbSnapshot = native_db.into();
        let snapshot_id = state_snapshot.get_id();
        if snapshot_id != native_snapshot.get_id() {
            anyhow::bail!(
                "State id={} and Native id={} snapshots have different are not matching",
                snapshot_id,
                native_snapshot.get_id()
            );
        }

        // Obviously alien
        println!("L={} S={}", self.latest_snapshot_id, snapshot_id);
        if snapshot_id > self.latest_snapshot_id {
            anyhow::bail!("Attempt to save unknown snapshot with id={}", snapshot_id);
        }

        {
            let existing_snapshot_id = self
                .block_hash_to_snapshot_id
                .get(&block_header.hash())
                .expect("Inconsistent block_hash_to_snapshot_id");
            if *existing_snapshot_id != snapshot_id {
                anyhow::bail!("Attempt to save unknown snapshot with id={}", snapshot_id);
            }
        }

        {
            let mut state_manager = self.state_snapshot_manager.write().unwrap();
            let mut native_manager = self.native_snapshot_manager.write().unwrap();

            state_manager.add_snapshot(state_snapshot);
            native_manager.add_snapshot(native_snapshot);
        }

        Ok(())
    }

    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()> {
        let current_block_hash = block_header.hash();
        let prev_block_hash = block_header.prev_hash();

        let snapshot_id = self
            .block_hash_to_snapshot_id
            .remove(&current_block_hash)
            .ok_or(anyhow::anyhow!("Attempt to finalize non existing snapshot"))?;

        let mut state_manager = self.state_snapshot_manager.write().unwrap();
        let mut native_manager = self.native_snapshot_manager.write().unwrap();

        // Return error here, as underlying database can return error
        state_manager.commit_snapshot(&snapshot_id)?;
        native_manager.commit_snapshot(&snapshot_id)?;

        // All siblings of current snapshot
        let mut to_discard: Vec<_> = self
            .chain_forks
            .remove(&prev_block_hash)
            .expect("Inconsistent chain_forks")
            .into_iter()
            .filter(|bh| bh != &current_block_hash)
            .collect();

        while let Some(block_hash) = to_discard.pop() {
            let child_block_hashes = self.chain_forks.remove(&block_hash).unwrap_or_default();
            self.blocks_to_parent.remove(&block_hash).unwrap();
            let snapshot_id = self.block_hash_to_snapshot_id.remove(&block_hash).unwrap();

            state_manager.discard_snapshot(&snapshot_id);
            native_manager.discard_snapshot(&snapshot_id);

            to_discard.extend(child_block_hashes);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path;

    use sov_db::rocks_db_config::gen_rocksdb_options;
    use sov_mock_da::{MockBlockHeader, MockHash};
    use sov_schema_db::snapshot::FrozenDbSnapshot;

    use super::*;
    use crate::dummy_storage::{
        DummyField, DummyNativeSchema, DummyStateSchema, DUMMY_NATIVE_CF, DUMMY_STATE_CF,
    };

    type Da = sov_mock_da::MockDaSpec;
    type S = sov_state::DefaultStorageSpec;

    fn build_dbs(
        state_path: &path::Path,
        native_path: &path::Path,
    ) -> (sov_schema_db::DB, sov_schema_db::DB) {
        let state_tables = vec![DUMMY_STATE_CF.to_string()];
        let state_db = sov_schema_db::DB::open(
            state_path,
            "state_db",
            state_tables,
            &gen_rocksdb_options(&Default::default(), false),
        )
        .unwrap();
        let native_tables = vec![DUMMY_NATIVE_CF.to_string()];
        let native_db = sov_schema_db::DB::open(
            native_path,
            "native_db",
            native_tables,
            &gen_rocksdb_options(&Default::default(), false),
        )
        .unwrap();

        (state_db, native_db)
    }

    #[test]
    fn initiate_new() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        let storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());
    }

    #[test]
    fn get_new_storage() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
        };

        let _storage = storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();

        assert!(!storage_manager.is_empty());
        assert!(!storage_manager.chain_forks.is_empty());
        assert!(!storage_manager.block_hash_to_snapshot_id.is_empty());
        assert!(storage_manager
            .snapshot_id_to_parent
            .read()
            .unwrap()
            .is_empty());
        assert!(storage_manager
            .state_snapshot_manager
            .read()
            .unwrap()
            .is_empty());
        assert!(storage_manager
            .native_snapshot_manager
            .read()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn try_get_new_storage_same_block() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
        };

        let storage_1 = storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();

        let storage_2 = storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();

        // We just check, that both storage have same underlying id.
        // This is more tight with implementation.
        // More black box way to check would be:
        //   - have some data in db
        //   - have some parent snapshots
        //   - make sure that writing to each individual storage do not propagate to another
        //   - both storage have same view of the previous state, for example they don't look into siblings
        let (state_db_1, native_db_1) = storage_1.freeze();
        let state_snapshot_1 = FrozenDbSnapshot::from(state_db_1);
        let native_snapshot_1 = FrozenDbSnapshot::from(native_db_1);
        let (state_db_2, native_db_2) = storage_2.freeze();
        let state_snapshot_2 = FrozenDbSnapshot::from(state_db_2);
        let native_snapshot_2 = FrozenDbSnapshot::from(native_db_2);

        assert_eq!(state_snapshot_1.get_id(), state_snapshot_2.get_id());
        assert_eq!(native_snapshot_1.get_id(), native_snapshot_2.get_id());
    }

    #[test]
    #[should_panic(expected = "Cannot provide storage for corrupt block")]
    fn try_get_new_storage_corrupt_block() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([1; 32]),
            height: 1,
        };

        let _storage_1 = storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();
    }

    #[test]
    #[ignore = "TBD"]
    fn save_change_set() {}

    #[test]
    fn try_save_unknown_changeset() {
        let state_tmpdir_1 = tempfile::tempdir().unwrap();
        let native_tmpdir_1 = tempfile::tempdir().unwrap();

        let state_tmpdir_2 = tempfile::tempdir().unwrap();
        let native_tmpdir_2 = tempfile::tempdir().unwrap();

        let block_a = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
        };

        let snapshot_1 = {
            let (state_db, native_db) = build_dbs(state_tmpdir_1.path(), native_tmpdir_1.path());
            let mut storage_manager_temp =
                NewProverStorageManager::<Da, S>::new(state_db, native_db);
            storage_manager_temp
                .get_native_storage_on(&block_a)
                .unwrap()
        };

        let (state_db, native_db) = build_dbs(state_tmpdir_2.path(), native_tmpdir_2.path());
        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);

        let unknown_id = storage_manager.save_change_set(&block_a, snapshot_1);
        assert!(unknown_id.is_err());
        assert!(unknown_id
            .err()
            .unwrap()
            .to_string()
            .starts_with("Attempt to save unknown snapshot with id="));

        // TODO: Unknown block

        // TODO: Block / snapshot_id mismatch
    }

    #[test]
    fn lifecycle_simulation() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        // State DB has following values initially:
        // x = 1
        // y = 2

        let x = DummyField(1);
        let y = DummyField(2);
        let _z = DummyField(3);

        state_db
            .put::<DummyStateSchema>(&x, &DummyField(1))
            .unwrap();
        state_db
            .put::<DummyStateSchema>(&y, &DummyField(2))
            .unwrap();

        // Native DB has following values initially
        // 10 = 10
        // 20 = 20

        native_db
            .put::<DummyNativeSchema>(&x, &DummyField(100))
            .unwrap();
        native_db
            .put::<DummyNativeSchema>(&y, &DummyField(200))
            .unwrap();

        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        //      / -> D
        // A -> B -> C -> D -> E
        // |    \ -> G -> H
        // \ -> F -> K
        //

        // Block A
        let block_a = MockBlockHeader {
            prev_hash: MockHash::from([0; 32]),
            hash: MockHash::from([1; 32]),
            height: 1,
        };

        let storage_a = storage_manager.get_native_storage_on(&block_a).unwrap();

        let state_x_actual = storage_a.read_state(x.0).unwrap();
        assert_eq!(Some(1), state_x_actual);

        let native_x_actual = storage_a.read_native(x.0).unwrap();
        assert_eq!(Some(100), native_x_actual);

        storage_a.write_state(x.0, 2).unwrap();
        storage_a.write_native(x.0, 20).unwrap();

        storage_manager
            .save_change_set(&block_a, storage_a)
            .unwrap();

        // Block B
        let block_b = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
        };

        let storage_b = storage_manager.get_native_storage_on(&block_b).unwrap();

        assert_eq!(Some(2), storage_b.read_state(x.0).unwrap());
        assert_eq!(Some(20), storage_b.read_native(x.0).unwrap());
    }
}
