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
    accessory_snapshot_manager: Arc<RwLock<SnapshotManager>>,

    phantom_mp_spec: PhantomData<S>,
}

impl<Da: DaSpec, S: MerkleProofSpec> NewProverStorageManager<Da, S>
where
    Da::SlotHash: Hash,
{
    #[allow(dead_code)]
    pub fn new(state_db: sov_schema_db::DB, native_db: sov_schema_db::DB) -> Self {
        let snapshot_id_to_parent = Arc::new(RwLock::new(HashMap::new()));

        let state_snapshot_manager = SnapshotManager::new(state_db, snapshot_id_to_parent.clone());
        let accessory_snapshot_manager =
            SnapshotManager::new(native_db, snapshot_id_to_parent.clone());

        Self {
            chain_forks: Default::default(),
            blocks_to_parent: Default::default(),
            latest_snapshot_id: 0,
            block_hash_to_snapshot_id: Default::default(),
            snapshot_id_to_parent,
            state_snapshot_manager: Arc::new(RwLock::new(state_snapshot_manager)),
            accessory_snapshot_manager: Arc::new(RwLock::new(accessory_snapshot_manager)),
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
            && self.accessory_snapshot_manager.read().unwrap().is_empty()
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
            "Cannot provide storage for corrupt block: prev_hash == current_hash"
        );
        if let Some(prev_snapshot_id) = self.block_hash_to_snapshot_id.get(&prev_block_hash) {
            let state_snapshot_manager = self.state_snapshot_manager.read().unwrap();
            if !state_snapshot_manager.contains_snapshot(prev_snapshot_id) {
                anyhow::bail!("Snapshot for previous block has been saved yet");
            }
        }

        let new_snapshot_id = match self.block_hash_to_snapshot_id.get(&current_block_hash) {
            // Storage for this block has been requested before
            Some(snapshot_id) => *snapshot_id,
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

        let state_db_snapshot = DbSnapshot::new(
            new_snapshot_id,
            ReadOnlyLock::new(self.state_snapshot_manager.clone()),
        );

        let native_db_snapshot = DbSnapshot::new(
            new_snapshot_id,
            ReadOnlyLock::new(self.accessory_snapshot_manager.clone()),
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
            anyhow::bail!(
                "Attempt to save changeset for unknown block header {:?}",
                block_header
            );
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
            let mut native_manager = self.accessory_snapshot_manager.write().unwrap();

            state_manager.add_snapshot(state_snapshot);
            native_manager.add_snapshot(native_snapshot);
        }

        Ok(())
    }

    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()> {
        let current_block_hash = block_header.hash();
        let prev_block_hash = block_header.prev_hash();

        self.blocks_to_parent.remove(&prev_block_hash);
        self.blocks_to_parent.remove(&current_block_hash);

        // Removing previous
        self.block_hash_to_snapshot_id.remove(&prev_block_hash);
        let snapshot_id = &self
            .block_hash_to_snapshot_id
            .remove(&current_block_hash)
            .ok_or(anyhow::anyhow!("Attempt to finalize non existing snapshot"))?;

        let mut state_manager = self.state_snapshot_manager.write().unwrap();
        let mut native_manager = self.accessory_snapshot_manager.write().unwrap();
        let mut snapshot_id_to_parent = self.snapshot_id_to_parent.write().unwrap();
        snapshot_id_to_parent.remove(snapshot_id);

        // Return error here, as underlying database can return error
        state_manager.commit_snapshot(snapshot_id)?;
        native_manager.commit_snapshot(snapshot_id)?;

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
            snapshot_id_to_parent.remove(&snapshot_id);
            state_manager.discard_snapshot(&snapshot_id);
            native_manager.discard_snapshot(&snapshot_id);

            to_discard.extend(child_block_hashes);
        }

        // Removing snapshot id pointers for children of this one
        for child_block_hash in self.chain_forks.get(&current_block_hash).unwrap_or(&vec![]) {
            let child_snapshot_id = self
                .block_hash_to_snapshot_id
                .get(child_block_hash)
                .unwrap();
            snapshot_id_to_parent.remove(child_snapshot_id);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path;

    use sov_db::rocks_db_config::gen_rocksdb_options;
    use sov_mock_da::{MockBlockHeader, MockHash};
    use sov_rollup_interface::da::Time;
    use sov_schema_db::snapshot::FrozenDbSnapshot;

    use super::*;
    use crate::dummy_storage::{
        DummyField, DummyNativeSchema, DummyStateSchema, DUMMY_NATIVE_CF, DUMMY_STATE_CF,
    };

    type Da = sov_mock_da::MockDaSpec;
    type S = sov_state::DefaultStorageSpec;

    fn validate_internal_consistency(storage_manager: &NewProverStorageManager<Da, S>) {
        let snapshot_id_to_parent = storage_manager.snapshot_id_to_parent.read().unwrap();
        let state_snapshot_manager = storage_manager.state_snapshot_manager.read().unwrap();
        let native_snapshot_manager = storage_manager.state_snapshot_manager.read().unwrap();

        for (block_hash, parent_block_hash) in storage_manager.blocks_to_parent.iter() {
            // For each block hash there should be snapshot id
            let snapshot_id = storage_manager
                .block_hash_to_snapshot_id
                .get(block_hash)
                .expect("Missing snapshot_id");

            // For each snapshot id, that is not head, there should be parent snapshot id
            if !storage_manager
                .chain_forks
                .get(block_hash)
                .unwrap_or(&vec![])
                .is_empty()
            {
                assert!(
                    state_snapshot_manager.contains_snapshot(snapshot_id),
                    "snapshot id={} is missing in state_snapshot_manager",
                    snapshot_id
                );
                assert!(
                    native_snapshot_manager.contains_snapshot(snapshot_id),
                    "snapshot id={} is missing in native_snapshot_manager",
                    snapshot_id
                );
            } else {
                assert_eq!(
                    state_snapshot_manager.contains_snapshot(snapshot_id),
                    native_snapshot_manager.contains_snapshot(snapshot_id),
                );
            }

            // If there's reference to parent snapshot id, it should be consistent with block hash i
            match snapshot_id_to_parent.get(snapshot_id) {
                None => {
                    assert!(storage_manager
                        .block_hash_to_snapshot_id
                        .get(parent_block_hash)
                        .is_none());
                }
                Some(parent_snapshot_id) => {
                    let parent_snapshot_id_from_block_hash = storage_manager
                        .block_hash_to_snapshot_id
                        .get(parent_block_hash)
                        .unwrap_or_else(|| panic!(
                            "Missing parent snapshot_id for block_hash={:?}, parent_block_hash={:?}, snapshot_id={}, expected_parent_snapshot_id={}",
                            block_hash, parent_block_hash, snapshot_id, parent_snapshot_id,
                        ));
                    assert_eq!(parent_snapshot_id, parent_snapshot_id_from_block_hash);
                }
            }
        }
    }

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
        validate_internal_consistency(&storage_manager);
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
            time: Time::now(),
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
            .accessory_snapshot_manager
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
            prev_hash: MockHash::from([0; 32]),
            hash: MockHash::from([1; 32]),
            height: 1,
            time: Time::now(),
        };

        let storage_1 = storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();

        let storage_2 = storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();

        // We just check, that both storage have same underlying id.
        // This is more tight with implementation.
        let (state_db_1, native_db_1) = storage_1.freeze();
        let state_snapshot_1 = FrozenDbSnapshot::from(state_db_1);
        let native_snapshot_1 = FrozenDbSnapshot::from(native_db_1);
        let (state_db_2, native_db_2) = storage_2.freeze();
        let state_snapshot_2 = FrozenDbSnapshot::from(state_db_2);
        let native_snapshot_2 = FrozenDbSnapshot::from(native_db_2);

        assert_eq!(state_snapshot_1.get_id(), state_snapshot_2.get_id());
        assert_eq!(native_snapshot_1.get_id(), native_snapshot_2.get_id());

        // TODO: Do more checks
        // More black box way to check would be:
        //   - have some data in db
        //   - have some parent snapshots
        //   - make sure that writing to each individual storage do not propagate to another
        //   - both storage have same view of the previous state, for example they don't look into siblings
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
            time: Time::now(),
        };

        storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();
    }

    #[test]
    fn read_state_before_parent_is_added() {
        // Blocks A -> B
        // create snapshot A from block A
        // create snapshot B from block B
        // query data from block B, before adding snapshot A back to the manager!
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_a = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
            time: Time::now(),
        };
        let block_b = MockBlockHeader {
            prev_hash: MockHash::from([2; 32]),
            hash: MockHash::from([1; 32]),
            height: 2,
            time: Time::now(),
        };

        let _storage_a = storage_manager.get_native_storage_on(&block_a).unwrap();

        // new storage can be crated only on top of saved snapshot.
        let result = storage_manager.get_native_storage_on(&block_b);
        assert!(result.is_err());
        assert_eq!(
            "Snapshot for previous block has been saved yet",
            result.err().unwrap().to_string()
        );
    }

    #[test]
    fn save_change_set() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
            time: Time::now(),
        };

        assert!(storage_manager.is_empty());
        let storage = storage_manager
            .get_native_storage_on(&block_header)
            .unwrap();
        assert!(!storage_manager.is_empty());

        // We can save empty storage as well
        storage_manager
            .save_change_set(&block_header, storage)
            .unwrap();

        assert!(!storage_manager.is_empty());
    }

    #[test]
    fn try_save_unknown_block_header() {
        let state_tmpdir_1 = tempfile::tempdir().unwrap();
        let native_tmpdir_1 = tempfile::tempdir().unwrap();

        let state_tmpdir_2 = tempfile::tempdir().unwrap();
        let native_tmpdir_2 = tempfile::tempdir().unwrap();

        let block_a = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
            time: Time::now(),
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

        let result = storage_manager.save_change_set(&block_a, snapshot_1);
        assert!(result.is_err());
        let expected_error_msg = format!(
            "Attempt to save changeset for unknown block header {:?}",
            &block_a
        );
        assert_eq!(expected_error_msg, result.err().unwrap().to_string());
    }

    #[test]
    fn try_save_unknown_snapshot() {
        // This test we create 2 snapshot managers and try to save snapshots from first manager
        // in another
        // First it checks for yet unknown id 2. It is larger that last known snapshot 1.
        // Then we commit own snapshot 1, and then try to save alien snapshot with id 1
        let state_tmpdir_1 = tempfile::tempdir().unwrap();
        let native_tmpdir_1 = tempfile::tempdir().unwrap();

        let state_tmpdir_2 = tempfile::tempdir().unwrap();
        let native_tmpdir_2 = tempfile::tempdir().unwrap();

        let block_a = MockBlockHeader {
            prev_hash: MockHash::from([0; 32]),
            hash: MockHash::from([1; 32]),
            height: 1,
            time: Time::now(),
        };

        let block_b = MockBlockHeader {
            prev_hash: MockHash::from([2; 32]),
            hash: MockHash::from([3; 32]),
            height: 2,
            time: Time::now(),
        };

        let (snapshot_alien_1, snapshot_alien_2) = {
            let (state_db, native_db) = build_dbs(state_tmpdir_1.path(), native_tmpdir_1.path());
            let mut storage_manager_temp =
                NewProverStorageManager::<Da, S>::new(state_db, native_db);
            // ID = 1
            let snapshot_a = storage_manager_temp
                .get_native_storage_on(&block_a)
                .unwrap();
            // ID = 2
            let snapshot_b = storage_manager_temp
                .get_native_storage_on(&block_b)
                .unwrap();
            (snapshot_a, snapshot_b)
        };

        let (state_db, native_db) = build_dbs(state_tmpdir_2.path(), native_tmpdir_2.path());
        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);

        let snapshot_own_a = storage_manager.get_native_storage_on(&block_a).unwrap();
        let _snapshot_own_b = storage_manager.get_native_storage_on(&block_b).unwrap();

        let result = storage_manager.save_change_set(&block_a, snapshot_alien_2);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert_eq!("Attempt to save unknown snapshot with id=2", err_msg);

        storage_manager
            .save_change_set(&block_a, snapshot_own_a)
            .unwrap();

        storage_manager.finalize(&block_a).unwrap();

        let result = storage_manager.save_change_set(&block_b, snapshot_alien_1);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert_eq!("Attempt to save unknown snapshot with id=1", err_msg);
    }

    #[test]
    fn linear_progression() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());
        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_from_i = |i: u8| MockBlockHeader {
            prev_hash: MockHash::from([i; 32]),
            hash: MockHash::from([i + 1; 32]),
            height: i as u64 + 1,
            time: Time::now(),
        };

        for i in 0u8..4 {
            let block = block_from_i(i);
            let storage = storage_manager.get_native_storage_on(&block).unwrap();
            storage_manager.save_change_set(&block, storage).unwrap();
        }

        for i in 0u8..4 {
            let block = block_from_i(i);
            storage_manager.finalize(&block).unwrap();
            validate_internal_consistency(&storage_manager);
        }
        assert!(storage_manager.is_empty());
    }

    #[test]
    fn parallel_forks() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());
        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        // 1    2    3
        // / -> D -> E
        // A -> B -> C
        // \ -> F -> G

        // (height, prev_hash, current_hash)
        let blocks: Vec<(u8, u8, u8)> = vec![
            (1, 0, 1),   // A
            (2, 1, 2),   // B
            (2, 1, 12),  // D
            (2, 1, 22),  // F
            (3, 2, 3),   // C
            (3, 12, 13), // E
            (3, 22, 23), // G
        ];

        for (height, prev_hash, next_hash) in blocks {
            let block = MockBlockHeader {
                prev_hash: MockHash::from([prev_hash; 32]),
                hash: MockHash::from([next_hash; 32]),
                height: height as u64,
                time: Time::now(),
            };
            let storage = storage_manager.get_native_storage_on(&block).unwrap();
            storage_manager.save_change_set(&block, storage).unwrap();
        }

        for prev_hash in 0..3 {
            let block = MockBlockHeader {
                prev_hash: MockHash::from([prev_hash; 32]),
                hash: MockHash::from([prev_hash + 1; 32]),
                height: prev_hash as u64 + 1,
                time: Time::now(),
            };
            storage_manager.finalize(&block).unwrap();
            validate_internal_consistency(&storage_manager);
        }

        assert!(storage_manager.is_empty());
    }

    #[test]
    #[ignore = "TBD"]
    fn finalize_non_earliest_block() {
        // All previous states should be finalized
    }

    #[test]
    fn lifecycle_simulation() {
        let state_tmpdir = tempfile::tempdir().unwrap();
        let native_tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(state_tmpdir.path(), native_tmpdir.path());

        // State DB has following values initially:
        // 1 = 1
        // 2 = 2
        let one = DummyField(1);
        let two = DummyField(2);

        state_db.put::<DummyStateSchema>(&one, &one).unwrap();
        state_db.put::<DummyStateSchema>(&two, &two).unwrap();

        // Native DB has following values initially
        // 1 = 100
        // 2 = 200

        native_db
            .put::<DummyNativeSchema>(&one, &DummyField(100))
            .unwrap();
        native_db
            .put::<DummyNativeSchema>(&two, &DummyField(200))
            .unwrap();

        let mut storage_manager = NewProverStorageManager::<Da, S>::new(state_db, native_db);
        assert!(storage_manager.is_empty());

        // Chains:
        // 1    2    3    4    5
        //      / -> L -> M
        // A -> B -> C -> D -> E
        // |    \ -> G -> H
        // \ -> F -> K
        // M, E, H, K: Observability snapshots.

        let block_a = MockBlockHeader {
            prev_hash: MockHash::from([0; 32]),
            hash: MockHash::from([1; 32]),
            height: 1,
            time: Time::now(),
        };
        let block_b = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 2,
            time: Time::now(),
        };
        let block_c = MockBlockHeader {
            prev_hash: MockHash::from([2; 32]),
            hash: MockHash::from([3; 32]),
            height: 3,
            time: Time::now(),
        };
        let block_d = MockBlockHeader {
            prev_hash: MockHash::from([3; 32]),
            hash: MockHash::from([4; 32]),
            height: 4,
            time: Time::now(),
        };
        let block_e = MockBlockHeader {
            prev_hash: MockHash::from([4; 32]),
            hash: MockHash::from([5; 32]),
            height: 5,
            time: Time::now(),
        };
        let block_f = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([32; 32]),
            height: 2,
            time: Time::now(),
        };
        let block_g = MockBlockHeader {
            prev_hash: MockHash::from([2; 32]),
            hash: MockHash::from([23; 32]),
            height: 3,
            time: Time::now(),
        };
        let block_h = MockBlockHeader {
            prev_hash: MockHash::from([23; 32]),
            hash: MockHash::from([24; 32]),
            height: 4,
            time: Time::now(),
        };
        let block_k = MockBlockHeader {
            prev_hash: MockHash::from([32; 32]),
            hash: MockHash::from([33; 32]),
            height: 3,
            time: Time::now(),
        };
        let block_l = MockBlockHeader {
            prev_hash: MockHash::from([2; 32]),
            hash: MockHash::from([13; 32]),
            height: 3,
            time: Time::now(),
        };
        let block_m = MockBlockHeader {
            prev_hash: MockHash::from([13; 32]),
            hash: MockHash::from([14; 32]),
            height: 4,
            time: Time::now(),
        };

        // Data
        // | Block |    DB  | Key |  Operation |
        // |     A |  state |   1 |   write(3) |
        // |     A |  state |   3 |   write(4) |
        // |     A | native |   3 | write(400) |
        // |     B |  state |   3 |   write(4) |
        // |     B | native |   3 | write(500) |
        // |     C |  state |   1 |     delete |
        // |     C |  state |   4 |   write(5) |
        // |     C | native |   1 | write(600) |
        // |     D |  state |   3 |   write(6) |
        // |     F |  state |   1 |   write(7) |
        // |     F | native |   3 | write(700) |
        // |     F |  state |   3 |     delete |
        // |     F | native |   1 |     delete |
        // |     G |  state |   1 |   write(8) |
        // |     G | native |   2 |   write(9) |
        // |     L |  state |   1 |  write(10) |

        // A
        let storage_a = storage_manager.get_native_storage_on(&block_a).unwrap();
        storage_a.write_state(1, 3).unwrap();
        storage_a.write_state(3, 4).unwrap();
        storage_a.write_native(3, 400).unwrap();
        storage_manager
            .save_change_set(&block_a, storage_a)
            .unwrap();
        // B
        let storage_b = storage_manager.get_native_storage_on(&block_b).unwrap();
        storage_b.write_state(3, 4).unwrap();
        storage_b.write_native(3, 500).unwrap();
        storage_manager
            .save_change_set(&block_b, storage_b)
            .unwrap();
        // C
        let storage_c = storage_manager.get_native_storage_on(&block_c).unwrap();
        storage_c.delete_state(1).unwrap();
        storage_c.write_state(4, 5).unwrap();
        storage_c.write_native(1, 600).unwrap();
        storage_manager
            .save_change_set(&block_c, storage_c)
            .unwrap();
        // D
        let storage_d = storage_manager.get_native_storage_on(&block_d).unwrap();
        storage_d.write_state(3, 6).unwrap();
        storage_manager
            .save_change_set(&block_d, storage_d)
            .unwrap();
        // F
        let storage_f = storage_manager.get_native_storage_on(&block_f).unwrap();
        storage_f.write_state(1, 7).unwrap();
        storage_f.write_native(3, 700).unwrap();
        storage_f.delete_state(3).unwrap();
        storage_f.delete_native(1).unwrap();
        storage_manager
            .save_change_set(&block_f, storage_f)
            .unwrap();
        // G
        let storage_g = storage_manager.get_native_storage_on(&block_g).unwrap();
        storage_g.write_state(1, 8).unwrap();
        storage_g.write_native(2, 9).unwrap();
        storage_manager
            .save_change_set(&block_g, storage_g)
            .unwrap();
        // L
        let storage_l = storage_manager.get_native_storage_on(&block_l).unwrap();
        storage_l.write_state(1, 10).unwrap();
        storage_manager
            .save_change_set(&block_l, storage_l)
            .unwrap();

        // VIEW: Before finalization of A
        // | snapshot |    DB  | Key |  Value |
        // |        E |  state |   1 |   None |
        // |        E |  state |   2 |      2 |
        // |        E |  state |   3 |      6 |
        // |        E |  state |   4 |      5 |
        // |        E | native |   1 |    600 |
        // |        E | native |   2 |    200 |
        // |        E | native |   3 |    500 |
        // |        E | native |   4 |   None |
        // |        M |  state |   1 |     10 |
        // |        M |  state |   2 |      2 |
        // |        M |  state |   3 |      4 |
        // |        M |  state |   4 |   None |
        // |        M | native |   1 |    100 |
        // |        M | native |   2 |    200 |
        // |        M | native |   3 |    500 |
        // |        M | native |   4 |   None |
        // |        H |  state |   1 |      8 |
        // |        H |  state |   2 |      2 |
        // |        H |  state |   3 |      4 |
        // |        H |  state |   4 |   None |
        // |        H | native |   1 |    100 |
        // |        H | native |   2 |      9 |
        // |        H | native |   3 |    500 |
        // |        H | native |   4 |   None |
        // |        K |  state |   1 |      7 |
        // |        K |  state |   2 |      2 |
        // |        K |  state |   3 |   None |
        // |        K |  state |   4 |   None |
        // |        K | native |   1 |   None |
        // |        K | native |   2 |    200 |
        // |        K | native |   3 |    700 |
        // |        K | native |   4 |   None |

        let storage_e = storage_manager.get_native_storage_on(&block_e).unwrap();
        let storage_m = storage_manager.get_native_storage_on(&block_m).unwrap();
        let storage_h = storage_manager.get_native_storage_on(&block_h).unwrap();
        let storage_k = storage_manager.get_native_storage_on(&block_k).unwrap();

        let assert_main_fork = || {
            assert_eq!(None, storage_e.read_state(1).unwrap());
            assert_eq!(Some(2), storage_e.read_state(2).unwrap());
            assert_eq!(Some(6), storage_e.read_state(3).unwrap());
            assert_eq!(Some(5), storage_e.read_state(4).unwrap());
            assert_eq!(Some(600), storage_e.read_native(1).unwrap());
            assert_eq!(Some(200), storage_e.read_native(2).unwrap());
            assert_eq!(Some(500), storage_e.read_native(3).unwrap());
            assert_eq!(None, storage_e.read_native(4).unwrap());

            assert_eq!(Some(10), storage_m.read_state(1).unwrap());
            assert_eq!(Some(2), storage_m.read_state(2).unwrap());
            assert_eq!(Some(4), storage_m.read_state(3).unwrap());
            assert_eq!(None, storage_m.read_state(4).unwrap());
            assert_eq!(Some(100), storage_m.read_native(1).unwrap());
            assert_eq!(Some(200), storage_m.read_native(2).unwrap());
            assert_eq!(Some(500), storage_m.read_native(3).unwrap());
            assert_eq!(None, storage_m.read_native(4).unwrap());

            assert_eq!(Some(8), storage_h.read_state(1).unwrap());
            assert_eq!(Some(2), storage_h.read_state(2).unwrap());
            assert_eq!(Some(4), storage_h.read_state(3).unwrap());
            assert_eq!(None, storage_h.read_state(4).unwrap());
            assert_eq!(Some(100), storage_h.read_native(1).unwrap());
            assert_eq!(Some(9), storage_h.read_native(2).unwrap());
            assert_eq!(Some(500), storage_h.read_native(3).unwrap());
            assert_eq!(None, storage_h.read_native(4).unwrap());
        };
        assert_main_fork();
        assert_eq!(Some(7), storage_k.read_state(1).unwrap());
        assert_eq!(Some(2), storage_k.read_state(2).unwrap());
        assert_eq!(None, storage_k.read_state(3).unwrap());
        assert_eq!(None, storage_k.read_state(4).unwrap());
        assert_eq!(None, storage_k.read_native(1).unwrap());
        assert_eq!(Some(200), storage_k.read_native(2).unwrap());
        assert_eq!(Some(700), storage_k.read_native(3).unwrap());
        assert_eq!(None, storage_k.read_native(4).unwrap());

        validate_internal_consistency(&storage_manager);
        // After finalization of A
        storage_manager.finalize(&block_a).unwrap();
        validate_internal_consistency(&storage_manager);
        assert_main_fork();
        // Finalizing the rest
        storage_manager.finalize(&block_b).unwrap();
        validate_internal_consistency(&storage_manager);
        storage_manager.finalize(&block_c).unwrap();
        validate_internal_consistency(&storage_manager);
        storage_manager.finalize(&block_d).unwrap();
        validate_internal_consistency(&storage_manager);
        // TODO: Check that values are in the database
    }
}
