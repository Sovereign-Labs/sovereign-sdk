use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use sov_db::native_db::NativeDB;
use sov_db::state_db::StateDB;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_schema_db::snapshot::{DbSnapshot, ReadOnlyLock, SnapshotId};
use sov_state::{MerkleProofSpec, ProverStorage};

pub use crate::snapshot_manager::SnapshotManager;

mod snapshot_manager;

/// Implementation of [`HierarchicalStorageManager`] that handles relation between snapshots
/// And reorgs on Data Availability layer.
pub struct ProverStorageManager<Da: DaSpec, S: MerkleProofSpec> {
    // L1 forks representation
    // Chain: prev_block -> child_blocks
    chain_forks: HashMap<Da::SlotHash, Vec<Da::SlotHash>>,
    // Reverse: child_block -> parent
    blocks_to_parent: HashMap<Da::SlotHash, Da::SlotHash>,

    latest_snapshot_id: SnapshotId,
    block_hash_to_snapshot_id: HashMap<Da::SlotHash, SnapshotId>,

    // This is for tracking "finalized" storage and detect errors
    // TODO: Should be removed after https://github.com/Sovereign-Labs/sovereign-sdk/issues/1218
    orphaned_snapshots: HashSet<SnapshotId>,

    // Same reference for individual managers
    snapshot_id_to_parent: Arc<RwLock<HashMap<SnapshotId, SnapshotId>>>,

    state_snapshot_manager: Arc<RwLock<SnapshotManager>>,
    accessory_snapshot_manager: Arc<RwLock<SnapshotManager>>,

    phantom_mp_spec: PhantomData<S>,
}

impl<Da: DaSpec, S: MerkleProofSpec> ProverStorageManager<Da, S>
where
    Da::SlotHash: Hash,
{
    fn with_db_handles(state_db: sov_schema_db::DB, native_db: sov_schema_db::DB) -> Self {
        let snapshot_id_to_parent = Arc::new(RwLock::new(HashMap::new()));

        let state_snapshot_manager = SnapshotManager::new(state_db, snapshot_id_to_parent.clone());
        let accessory_snapshot_manager =
            SnapshotManager::new(native_db, snapshot_id_to_parent.clone());

        Self {
            chain_forks: Default::default(),
            blocks_to_parent: Default::default(),
            latest_snapshot_id: 0,
            block_hash_to_snapshot_id: Default::default(),
            orphaned_snapshots: Default::default(),
            snapshot_id_to_parent,
            state_snapshot_manager: Arc::new(RwLock::new(state_snapshot_manager)),
            accessory_snapshot_manager: Arc::new(RwLock::new(accessory_snapshot_manager)),
            phantom_mp_spec: Default::default(),
        }
    }

    /// Create new [`ProverStorageManager`] from state config
    pub fn new(config: sov_state::config::Config) -> anyhow::Result<Self> {
        let path = config.path;
        let state_db = StateDB::<SnapshotManager>::setup_schema_db(&path)?;
        let native_db = NativeDB::<SnapshotManager>::setup_schema_db(&path)?;

        Ok(Self::with_db_handles(state_db, native_db))
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

    fn get_storage_with_snapshot_id(
        &self,
        snapshot_id: SnapshotId,
    ) -> anyhow::Result<ProverStorage<S, SnapshotManager>> {
        let state_db_snapshot = DbSnapshot::new(
            snapshot_id,
            ReadOnlyLock::new(self.state_snapshot_manager.clone()),
        );

        let state_db = StateDB::with_db_snapshot(state_db_snapshot)?;

        let native_db_snapshot = DbSnapshot::new(
            snapshot_id,
            ReadOnlyLock::new(self.accessory_snapshot_manager.clone()),
        );

        let native_db = NativeDB::with_db_snapshot(native_db_snapshot)?;
        Ok(ProverStorage::with_db_handles(state_db, native_db))
    }

    fn finalize_by_hash_pair(
        &mut self,
        prev_block_hash: Da::SlotHash,
        current_block_hash: Da::SlotHash,
    ) -> anyhow::Result<()> {
        tracing::debug!(
            "Finalizing block prev_hash={:?}; current_hash={:?}",
            prev_block_hash,
            current_block_hash
        );
        // Check if this is the oldest block
        if self
            .block_hash_to_snapshot_id
            .contains_key(&prev_block_hash)
        {
            if let Some(grand_parent) = self.blocks_to_parent.remove(&prev_block_hash) {
                self.finalize_by_hash_pair(grand_parent, prev_block_hash.clone())?;
            }
        }
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
            tracing::debug!("Discarding snapshot={}", snapshot_id);
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

impl<Da: DaSpec, S: MerkleProofSpec> HierarchicalStorageManager<Da> for ProverStorageManager<Da, S>
where
    Da::SlotHash: Hash,
{
    type NativeStorage = ProverStorage<S, SnapshotManager>;
    type NativeChangeSet = ProverStorage<S, SnapshotManager>;

    fn create_storage_on(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<Self::NativeStorage> {
        tracing::trace!("Requested native storage for block {:?} ", block_header);
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
        tracing::debug!(
            "Requested native storage for block {:?}, giving snapshot id={}",
            block_header,
            new_snapshot_id
        );

        self.get_storage_with_snapshot_id(new_snapshot_id)
    }

    fn create_finalized_storage(&mut self) -> anyhow::Result<Self::NativeStorage> {
        self.latest_snapshot_id += 1;
        let snapshot_id = self.latest_snapshot_id;
        tracing::debug!("Giving 'finalized' storage ref with id {}", snapshot_id);
        self.orphaned_snapshots.insert(snapshot_id);
        let state_db_snapshot = DbSnapshot::new(
            snapshot_id,
            ReadOnlyLock::new(self.state_snapshot_manager.clone()),
        );

        let state_db = StateDB::with_db_snapshot(state_db_snapshot)?;
        state_db.max_out_next_version();

        let native_db_snapshot = DbSnapshot::new(
            snapshot_id,
            ReadOnlyLock::new(self.accessory_snapshot_manager.clone()),
        );

        let native_db = NativeDB::with_db_snapshot(native_db_snapshot)?;
        Ok(ProverStorage::with_db_handles(state_db, native_db))
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
        let (state_snapshot, native_snapshot) = change_set.freeze()?;
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

        if self.orphaned_snapshots.remove(&snapshot_id) {
            tracing::debug!(
                "Discarded reference to 'finalized' snapshot={}",
                snapshot_id
            );
            return Ok(());
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
        tracing::debug!(
            "Snapshot id={} for block={:?} has been saved to StorageManager",
            snapshot_id,
            block_header
        );
        Ok(())
    }

    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()> {
        tracing::debug!("Finalizing block: {:?}", block_header);
        let current_block_hash = block_header.hash();
        let prev_block_hash = block_header.prev_hash();
        self.finalize_by_hash_pair(prev_block_hash, current_block_hash)
    }
}

/// Creates orphan [`ProverStorage`] which just points directly to the underlying database for previous data
/// Should be used only in tests
#[cfg(feature = "test-utils")]
pub fn new_orphan_storage<S: MerkleProofSpec>(
    path: impl AsRef<std::path::Path>,
) -> anyhow::Result<ProverStorage<S, SnapshotManager>> {
    let state_db_raw = StateDB::<SnapshotManager>::setup_schema_db(path.as_ref())?;
    let state_db_sm = Arc::new(RwLock::new(SnapshotManager::orphan(state_db_raw)));
    let state_db_snapshot = DbSnapshot::<SnapshotManager>::new(0, state_db_sm.into());
    let state_db = StateDB::with_db_snapshot(state_db_snapshot)?;
    let native_db_raw = NativeDB::<SnapshotManager>::setup_schema_db(path.as_ref())?;
    let native_db_sm = Arc::new(RwLock::new(SnapshotManager::orphan(native_db_raw)));
    let native_db_snapshot = DbSnapshot::<SnapshotManager>::new(0, native_db_sm.into());
    let native_db = NativeDB::with_db_snapshot(native_db_snapshot)?;
    Ok(ProverStorage::with_db_handles(state_db, native_db))
}

#[cfg(test)]
mod tests {
    use sov_mock_da::{MockBlockHeader, MockHash};
    use sov_rollup_interface::da::Time;
    use sov_state::storage::{CacheKey, CacheValue};
    use sov_state::{ArrayWitness, OrderedReadsAndWrites, Storage};

    use super::*;

    type Da = sov_mock_da::MockDaSpec;
    type S = sov_state::DefaultStorageSpec;

    fn validate_internal_consistency(storage_manager: &ProverStorageManager<Da, S>) {
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

    fn build_dbs(path: &std::path::Path) -> (sov_schema_db::DB, sov_schema_db::DB) {
        let state_db = StateDB::<SnapshotManager>::setup_schema_db(path).unwrap();
        let native_db = NativeDB::<SnapshotManager>::setup_schema_db(path).unwrap();

        (state_db, native_db)
    }

    #[test]
    fn initiate_new() {
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let storage_manager = ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
        assert!(storage_manager.is_empty());
        validate_internal_consistency(&storage_manager);
    }

    #[test]
    fn get_new_storage() {
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
            time: Time::now(),
        };

        let _storage = storage_manager.create_storage_on(&block_header).unwrap();

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
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([0; 32]),
            hash: MockHash::from([1; 32]),
            height: 1,
            time: Time::now(),
        };

        let storage_1 = storage_manager.create_storage_on(&block_header).unwrap();

        let storage_2 = storage_manager.create_storage_on(&block_header).unwrap();

        // We just check, that both storage have same underlying id.
        // This is more tight with implementation.
        let (state_snapshot_1, native_snapshot_1) = storage_1.freeze().unwrap();
        // let state_snapshot_1 = FrozenDbSnapshot::from(state_db_1);
        // let native_snapshot_1 = FrozenDbSnapshot::from(native_db_1);
        let (state_snapshot_2, native_snapshot_2) = storage_2.freeze().unwrap();
        // let state_snapshot_2 = FrozenDbSnapshot::from(state_db_2);
        // let native_snapshot_2 = FrozenDbSnapshot::from(native_db_2);

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
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([1; 32]),
            height: 1,
            time: Time::now(),
        };

        storage_manager.create_storage_on(&block_header).unwrap();
    }

    #[test]
    fn read_state_before_parent_is_added() {
        // Blocks A -> B
        // create snapshot A from block A
        // create snapshot B from block B
        // query data from block B, before adding snapshot A back to the manager!
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
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

        let _storage_a = storage_manager.create_storage_on(&block_a).unwrap();

        // new storage can be crated only on top of saved snapshot.
        let result = storage_manager.create_storage_on(&block_b);
        assert!(result.is_err());
        assert_eq!(
            "Snapshot for previous block has been saved yet",
            result.err().unwrap().to_string()
        );
    }

    #[test]
    fn save_change_set() {
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_header = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
            time: Time::now(),
        };

        assert!(storage_manager.is_empty());
        let storage = storage_manager.create_storage_on(&block_header).unwrap();
        assert!(!storage_manager.is_empty());

        // We can save empty storage as well
        storage_manager
            .save_change_set(&block_header, storage)
            .unwrap();

        assert!(!storage_manager.is_empty());
    }

    #[test]
    fn try_save_unknown_block_header() {
        let tmpdir_1 = tempfile::tempdir().unwrap();

        let tmpdir_2 = tempfile::tempdir().unwrap();

        let block_a = MockBlockHeader {
            prev_hash: MockHash::from([1; 32]),
            hash: MockHash::from([2; 32]),
            height: 1,
            time: Time::now(),
        };

        let snapshot_1 = {
            let (state_db, native_db) = build_dbs(tmpdir_1.path());
            let mut storage_manager_temp =
                ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
            storage_manager_temp.create_storage_on(&block_a).unwrap()
        };

        let (state_db, native_db) = build_dbs(tmpdir_2.path());
        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);

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
        let tmpdir_1 = tempfile::tempdir().unwrap();

        let tmpdir_2 = tempfile::tempdir().unwrap();

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
            let (state_db, native_db) = build_dbs(tmpdir_1.path());
            let mut storage_manager_temp =
                ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
            // ID = 1
            let snapshot_a = storage_manager_temp.create_storage_on(&block_a).unwrap();
            // ID = 2
            let snapshot_b = storage_manager_temp.create_storage_on(&block_b).unwrap();
            (snapshot_a, snapshot_b)
        };

        let (state_db, native_db) = build_dbs(tmpdir_2.path());
        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);

        let snapshot_own_a = storage_manager.create_storage_on(&block_a).unwrap();
        let _snapshot_own_b = storage_manager.create_storage_on(&block_b).unwrap();

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

    fn key_from(value: u64) -> CacheKey {
        let x = value.to_be_bytes().to_vec();
        CacheKey { key: Arc::new(x) }
    }

    fn value_from(value: u64) -> CacheValue {
        let x = value.to_be_bytes().to_vec();
        CacheValue { value: Arc::new(x) }
    }

    fn write_op(key: u64, value: u64) -> (CacheKey, Option<CacheValue>) {
        (key_from(key), Some(value_from(value)))
    }

    fn delete_op(key: u64) -> (CacheKey, Option<CacheValue>) {
        (key_from(key), None)
    }

    #[test]
    fn linear_progression() {
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
        assert!(storage_manager.is_empty());

        let block_from_i = |i: u8| MockBlockHeader {
            prev_hash: MockHash::from([i; 32]),
            hash: MockHash::from([i + 1; 32]),
            height: i as u64 + 1,
            time: Time::now(),
        };

        for i in 0u8..4 {
            let block = block_from_i(i);
            let storage = storage_manager.create_storage_on(&block).unwrap();
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
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
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
            let storage = storage_manager.create_storage_on(&block).unwrap();
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
    fn finalize_non_earliest_block() {
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
        assert!(storage_manager.is_empty());

        // Blocks A -> B -> C
        let block_a = MockBlockHeader::from_height(1);
        let block_b = MockBlockHeader::from_height(2);
        let block_c = MockBlockHeader::from_height(3);

        let storage_a = storage_manager.create_storage_on(&block_a).unwrap();
        let witness = ArrayWitness::default();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(1, 2));
            let mut native_operations = OrderedReadsAndWrites::default();
            native_operations.ordered_writes.push(write_op(30, 40));
            let (_, state_update) = storage_a
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_a.commit(&state_update, &native_operations);
        }
        storage_manager
            .save_change_set(&block_a, storage_a)
            .unwrap();

        let storage_b = storage_manager.create_storage_on(&block_b).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(3, 4));
            let mut native_operations = OrderedReadsAndWrites::default();
            native_operations.ordered_writes.push(write_op(50, 60));
            let (_, state_update) = storage_b
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_b.commit(&state_update, &native_operations);
        }
        storage_manager
            .save_change_set(&block_b, storage_b)
            .unwrap();

        let storage_c = storage_manager.create_storage_on(&block_c).unwrap();
        // Then finalize B
        storage_manager.finalize(&block_b).unwrap();

        assert_eq!(
            Some(value_from(2).into()),
            storage_c.get(&key_from(1).into(), None, &witness)
        );
        assert_eq!(
            Some(value_from(4).into()),
            storage_c.get(&key_from(3).into(), None, &witness)
        );
        assert_eq!(
            Some(value_from(40).into()),
            storage_c.get_accessory(&key_from(30).into(), None)
        );
        assert_eq!(
            Some(value_from(60).into()),
            storage_c.get_accessory(&key_from(50).into(), None)
        );

        // Finalize C now
        storage_manager
            .save_change_set(&block_c, storage_c)
            .unwrap();
        storage_manager.finalize(&block_c).unwrap();
        assert!(storage_manager.is_empty());
    }

    #[test]
    fn lifecycle_simulation() {
        let tmpdir = tempfile::tempdir().unwrap();

        let (state_db, native_db) = build_dbs(tmpdir.path());

        let mut storage_manager =
            ProverStorageManager::<Da, S>::with_db_handles(state_db, native_db);
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
        // |     A |    aux |   3 |  write(40) |
        // |     B |  state |   3 |   write(2) |
        // |     B |    aux |   3 |  write(50) |
        // |     C |  state |   1 |     delete |
        // |     C |  state |   4 |   write(5) |
        // |     C |    aux |   1 |  write(60) |
        // |     D |  state |   3 |   write(6) |
        // |     F |  state |   1 |   write(7) |
        // |     F |    aux |   3 |  write(70) |
        // |     F |  state |   3 |     delete |
        // |     F |    aux |   1 |     delete |
        // |     G |  state |   1 |   write(8) |
        // |     G |    aux |   2 |   write(9) |
        // |     L |  state |   1 |  write(10) |

        let witness = ArrayWitness::default();
        // A
        let storage_a = storage_manager.create_storage_on(&block_a).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(1, 3));
            state_operations.ordered_writes.push(write_op(3, 4));
            let mut native_operations = OrderedReadsAndWrites::default();
            native_operations.ordered_writes.push(write_op(3, 40));

            let (_, state_update) = storage_a
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_a.commit(&state_update, &native_operations);
        }

        storage_manager
            .save_change_set(&block_a, storage_a)
            .unwrap();
        // B
        let storage_b = storage_manager.create_storage_on(&block_b).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(3, 2));
            let mut native_operations = OrderedReadsAndWrites::default();
            native_operations.ordered_writes.push(write_op(3, 50));
            let (_, state_update) = storage_b
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_b.commit(&state_update, &native_operations);
        }
        storage_manager
            .save_change_set(&block_b, storage_b)
            .unwrap();
        // C
        let storage_c = storage_manager.create_storage_on(&block_c).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(delete_op(1));
            state_operations.ordered_writes.push(write_op(4, 5));
            let mut native_operations = OrderedReadsAndWrites::default();
            native_operations.ordered_writes.push(write_op(1, 60));
            let (_, state_update) = storage_c
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_c.commit(&state_update, &native_operations);
        }
        storage_manager
            .save_change_set(&block_c, storage_c)
            .unwrap();
        // D
        let storage_d = storage_manager.create_storage_on(&block_d).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(3, 6));
            let (_, state_update) = storage_d
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_d.commit(&state_update, &OrderedReadsAndWrites::default());
        }
        storage_manager
            .save_change_set(&block_d, storage_d)
            .unwrap();
        // F
        let storage_f = storage_manager.create_storage_on(&block_f).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(1, 7));
            state_operations.ordered_writes.push(delete_op(3));
            let mut native_operations = OrderedReadsAndWrites::default();
            native_operations.ordered_writes.push(delete_op(1));
            native_operations.ordered_writes.push(write_op(3, 70));
            let (_, state_update) = storage_f
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_f.commit(&state_update, &native_operations);
        }
        storage_manager
            .save_change_set(&block_f, storage_f)
            .unwrap();
        // G
        let storage_g = storage_manager.create_storage_on(&block_g).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(1, 8));
            let mut native_operations = OrderedReadsAndWrites::default();
            native_operations.ordered_writes.push(write_op(2, 9));
            let (_, state_update) = storage_g
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_g.commit(&state_update, &native_operations);
        }
        storage_manager
            .save_change_set(&block_g, storage_g)
            .unwrap();
        // L
        let storage_l = storage_manager.create_storage_on(&block_l).unwrap();
        {
            let mut state_operations = OrderedReadsAndWrites::default();
            state_operations.ordered_writes.push(write_op(1, 10));
            let (_, state_update) = storage_l
                .compute_state_update(state_operations, &witness)
                .unwrap();
            storage_l.commit(&state_update, &OrderedReadsAndWrites::default());
        }
        storage_manager
            .save_change_set(&block_l, storage_l)
            .unwrap();

        // VIEW: Before finalization of A
        // | snapshot |    DB  | Key |  Value |
        // |        E |  state |   1 |   None |
        // |        E |  state |   2 |   None |
        // |        E |  state |   3 |      6 |
        // |        E |  state |   4 |      5 |
        // |        E |    aux |   1 |     60 |
        // |        E |    aux |   2 |   None |
        // |        E |    aux |   3 |     50 |
        // |        M |  state |   1 |     10 |
        // |        M |  state |   2 |   None |
        // |        M |  state |   3 |      2 |
        // |        M |  state |   4 |   None |
        // |        M |    aux |   1 |   None |
        // |        M |    aux |   2 |   None |
        // |        M |    aux |   3 |     50 |
        // |        H |  state |   1 |      8 |
        // |        H |  state |   2 |   None |
        // |        H |  state |   3 |      2 |
        // |        H |  state |   4 |   None |
        // |        H |    aux |   1 |   None |
        // |        H |    aux |   2 |      9 |
        // |        H |    aux |   3 |     50 |
        // |        K |  state |   1 |      7 |
        // |        K |  state |   2 |   None |
        // |        K |  state |   3 |   None |
        // |        K |  state |   4 |   None |
        // |        K |    aux |   1 |   None |
        // |        K |    aux |   2 |   None |
        // |        K |    aux |   3 |     70 |

        let storage_e = storage_manager.create_storage_on(&block_e).unwrap();
        let storage_m = storage_manager.create_storage_on(&block_m).unwrap();
        let storage_h = storage_manager.create_storage_on(&block_h).unwrap();
        let storage_k = storage_manager.create_storage_on(&block_k).unwrap();

        let assert_main_fork = || {
            assert_eq!(None, storage_e.get(&key_from(1).into(), None, &witness));
            assert_eq!(None, storage_e.get(&key_from(2).into(), None, &witness));
            assert_eq!(
                Some(value_from(6).into()),
                storage_e.get(&key_from(3).into(), None, &witness)
            );
            assert_eq!(
                Some(value_from(5).into()),
                storage_e.get(&key_from(4).into(), None, &witness)
            );
            assert_eq!(
                Some(value_from(60).into()),
                storage_e.get_accessory(&key_from(1).into(), None)
            );
            assert_eq!(None, storage_e.get_accessory(&key_from(2).into(), None));
            assert_eq!(
                Some(value_from(50).into()),
                storage_e.get_accessory(&key_from(3).into(), None)
            );
        };
        // Storage M
        let assert_storage_m = || {
            assert_eq!(
                Some(value_from(10).into()),
                storage_m.get(&key_from(1).into(), None, &witness)
            );
            assert_eq!(None, storage_m.get(&key_from(2).into(), None, &witness));
            assert_eq!(
                Some(value_from(2).into()),
                storage_m.get(&key_from(3).into(), None, &witness)
            );
            assert_eq!(None, storage_m.get(&key_from(4).into(), None, &witness));
            assert_eq!(None, storage_m.get_accessory(&key_from(1).into(), None));
            assert_eq!(None, storage_m.get_accessory(&key_from(2).into(), None));
            assert_eq!(
                Some(value_from(50).into()),
                storage_m.get_accessory(&key_from(3).into(), None)
            );
        };
        // Storage H
        let assert_storage_h = || {
            assert_eq!(
                Some(value_from(8).into()),
                storage_h.get(&key_from(1).into(), None, &witness)
            );
            assert_eq!(None, storage_h.get(&key_from(2).into(), None, &witness));
            assert_eq!(
                Some(value_from(2).into()),
                storage_h.get(&key_from(3).into(), None, &witness)
            );
            assert_eq!(None, storage_h.get(&key_from(4).into(), None, &witness));
            assert_eq!(None, storage_h.get_accessory(&key_from(1).into(), None));
            assert_eq!(
                Some(value_from(9).into()),
                storage_h.get_accessory(&key_from(2).into(), None)
            );
            assert_eq!(
                Some(value_from(50).into()),
                storage_h.get_accessory(&key_from(3).into(), None)
            );
        };
        assert_main_fork();
        assert_storage_m();
        assert_storage_h();
        // Storage K
        assert_eq!(
            Some(value_from(7).into()),
            storage_k.get(&key_from(1).into(), None, &witness)
        );
        assert_eq!(None, storage_k.get(&key_from(2).into(), None, &witness));
        assert_eq!(None, storage_k.get(&key_from(3).into(), None, &witness));
        assert_eq!(None, storage_k.get(&key_from(4).into(), None, &witness));
        assert_eq!(None, storage_k.get_accessory(&key_from(1).into(), None));
        assert_eq!(None, storage_k.get_accessory(&key_from(2).into(), None));
        assert_eq!(
            Some(value_from(70).into()),
            storage_k.get_accessory(&key_from(3).into(), None)
        );
        validate_internal_consistency(&storage_manager);
        storage_manager
            .save_change_set(&block_k, storage_k)
            .unwrap();
        storage_manager.finalize(&block_a).unwrap();
        validate_internal_consistency(&storage_manager);
        assert_main_fork();
        assert_storage_m();
        assert_storage_h();

        // Finalizing the rest
        storage_manager.finalize(&block_b).unwrap();
        validate_internal_consistency(&storage_manager);
        assert_main_fork();
        storage_manager.finalize(&block_c).unwrap();
        validate_internal_consistency(&storage_manager);
        assert_main_fork();
        storage_manager.finalize(&block_d).unwrap();
        validate_internal_consistency(&storage_manager);
        assert_main_fork();
        storage_manager
            .save_change_set(&block_e, storage_e)
            .unwrap();
        storage_manager.finalize(&block_e).unwrap();
        assert!(storage_manager.is_empty());
        // Check that values are in the database.
        // Storage manager is empty, as checked before,
        // so new storage should read from database
        let new_block_after_e = MockBlockHeader {
            prev_hash: MockHash::from([5; 32]),
            hash: MockHash::from([6; 32]),
            height: 6,
            time: Time::now(),
        };
        let storage_last = storage_manager
            .create_storage_on(&new_block_after_e)
            .unwrap();
        assert_eq!(
            Some(value_from(6).into()),
            storage_last.get(&key_from(3).into(), None, &witness)
        );
        assert_eq!(
            Some(value_from(50).into()),
            storage_last.get_accessory(&key_from(3).into(), None)
        );
    }
}
