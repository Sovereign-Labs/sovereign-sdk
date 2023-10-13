use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard};

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_state::storage::{StorageKey, StorageValue};

pub type SnapshotId = u64;

pub struct ReadOnlyLock<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> ReadOnlyLock<T> {
    pub fn new(lock: Arc<RwLock<T>>) -> Self {
        Self { lock }
    }

    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        self.lock.read()
    }
}

/// Snapshot of the state
/// It can give a value that has been written/created on given state
/// [`ForkManager`] suppose to operate over those
pub trait Snapshot {
    /// Get own value, value from its own cache
    fn get_value(&self, key: &StorageKey) -> Option<StorageValue>;

    /// Helper method for mapping
    fn get_id(&self) -> SnapshotId;
}

pub trait QuerySnapshotLayers {
    fn get_value_recursively(
        &self,
        snapshot_block_hash: &SnapshotId,
        key: &StorageKey,
    ) -> Option<StorageValue>;
}

pub struct TreeQuery<S, Q>
where
    S: sov_state::Storage,
    Q: QuerySnapshotLayers,
{
    id: SnapshotId,
    storage: S,
    manager: ReadOnlyLock<Q>,
}

impl<S, Q> TreeQuery<S, Q>
where
    S: sov_state::Storage,
    Q: QuerySnapshotLayers,
{
    pub fn new(id: SnapshotId, storage: S, manager: ReadOnlyLock<Q>) -> Self {
        Self {
            id,
            storage,
            manager,
        }
    }

    pub fn get_id(&self) -> SnapshotId {
        self.id
    }
}

impl<S, Q> TreeQuery<S, Q>
where
    S: sov_state::Storage,
    Q: QuerySnapshotLayers,
{
    pub fn query_value(&self, key: &StorageKey) -> Option<StorageValue> {
        let manager = self.manager.read().unwrap();
        let value_from_cache = manager.get_value_recursively(&self.id, key);
        if value_from_cache.is_some() {
            return value_from_cache;
        }

        // TODO: What about witness
        self.storage.get(key, &Default::default())
    }
}

#[derive(Debug)]
pub struct ForkManager<S: sov_state::Storage, Sn: Snapshot, Da: DaSpec> {
    storage: S,

    // TODO: Ugly, fix this with higher lever struct
    self_ref: Option<Arc<RwLock<ForkManager<S, Sn, Da>>>>,

    snapshots: HashMap<Da::SlotHash, Sn>,

    // L1 forks representation
    // Chain: prev_block -> child_blocks
    chain_forks: HashMap<Da::SlotHash, Vec<Da::SlotHash>>,
    // Reverse: child_block -> parent
    blocks_to_parent: HashMap<Da::SlotHash, Da::SlotHash>,

    // Helper mappings
    latest_snapshot_id: SnapshotId,
    snapshot_id_to_block_hash: HashMap<SnapshotId, Da::SlotHash>,
}

impl<S, Sn, Da> QuerySnapshotLayers for ForkManager<S, Sn, Da>
where
    S: sov_state::Storage,
    Sn: Snapshot,
    Da: DaSpec,
    Da::SlotHash: Hash,
{
    fn get_value_recursively(
        &self,
        snapshot_id: &SnapshotId,
        key: &StorageKey,
    ) -> Option<StorageValue> {
        let snapshot_block_hash = self.snapshot_id_to_block_hash.get(snapshot_id)?;
        let parent_block_hash = self.blocks_to_parent.get(snapshot_block_hash)?;
        let mut parent_snapshot = self.snapshots.get(parent_block_hash);
        while parent_snapshot.is_some() {
            let snapshot = parent_snapshot.unwrap();
            let value = snapshot.get_value(key);
            if value.is_some() {
                return value;
            }
            let current_block_hash = self.snapshot_id_to_block_hash.get(&snapshot.get_id())?;
            let parent_block_hash = self.blocks_to_parent.get(current_block_hash)?;
            parent_snapshot = self.snapshots.get(parent_block_hash);
        }
        None
    }
}

impl<S, Sn, Da> ForkManager<S, Sn, Da>
where
    S: sov_state::Storage,
    Sn: Snapshot,
    Da: DaSpec,
    Da::SlotHash: Hash,
{
    pub fn new_locked(storage: S) -> Arc<RwLock<Self>> {
        let block_state_manager = Arc::new(RwLock::new(Self {
            storage,
            chain_forks: Default::default(),
            blocks_to_parent: Default::default(),
            snapshots: Default::default(),
            self_ref: None,
            snapshot_id_to_block_hash: Default::default(),
            latest_snapshot_id: Default::default(),
        }));
        let self_ref = block_state_manager.clone();
        {
            let mut bm = block_state_manager.write().unwrap();
            bm.self_ref = Some(self_ref);
        }
        block_state_manager
    }

    pub fn stop(mut self) {
        self.self_ref = None
    }

    pub fn is_empty(&self) -> bool {
        self.chain_forks.is_empty()
            && self.blocks_to_parent.is_empty()
            && self.snapshots.is_empty()
            && self.snapshot_id_to_block_hash.is_empty()
    }

    pub fn get_new_ref(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> TreeQuery<S, ForkManager<S, Sn, Da>> {
        self.latest_snapshot_id += 1;
        let new_snapshot_ref = TreeQuery {
            id: self.latest_snapshot_id,
            storage: self.storage.clone(),
            manager: ReadOnlyLock::new(self.self_ref.clone().unwrap().clone()),
        };

        let current_block_hash = block_header.hash();
        let prev_block_hash = block_header.prev_hash();

        self.snapshot_id_to_block_hash
            .insert(self.latest_snapshot_id, current_block_hash.clone());

        let c = self
            .blocks_to_parent
            .insert(current_block_hash.clone(), prev_block_hash.clone());
        // TODO: Maybe assert that parent is the same? Then
        assert!(
            c.is_none(),
            "current block hash has already snapshot requested"
        );
        self.chain_forks
            .entry(prev_block_hash)
            .or_insert(Vec::new())
            .push(current_block_hash);

        new_snapshot_ref
    }

    pub fn add_snapshot(&mut self, snapshot: Sn) {
        let snapshot_block_hash = self
            .snapshot_id_to_block_hash
            .get(&snapshot.get_id())
            .unwrap();
        self.snapshots.insert(snapshot_block_hash.clone(), snapshot);
    }

    fn remove_snapshot(&mut self, block_hash: &Da::SlotHash) -> Sn {
        let snapshot = self
            .snapshots
            .remove(&block_hash)
            .expect("Tried to remove non-existing snapshot: self.snapshots");
        let _removed_block_hash = self
            .snapshot_id_to_block_hash
            .remove(&snapshot.get_id())
            .unwrap();
        debug_assert_eq!(&_removed_block_hash, block_hash, "database is inconsistent");
        snapshot
    }

    pub fn finalize_snapshot(&mut self, block_hash: &Da::SlotHash) {
        let _snapshot = self.remove_snapshot(block_hash);
        // let payload = snapshot.into();
        // {
        //     let mut db = self.db.lock().unwrap();
        //     db.commit(payload);
        // }

        if let Some(parent_block_hash) = self.blocks_to_parent.remove(block_hash) {
            let mut to_discard: Vec<_> = self
                .chain_forks
                .remove(&parent_block_hash)
                .expect("Inconsistent chain_forks")
                .into_iter()
                .filter(|bh| bh != block_hash)
                .collect();
            while !to_discard.is_empty() {
                let next_to_discard = to_discard.pop().unwrap();
                let next_children_to_discard = self
                    .chain_forks
                    .remove(&next_to_discard)
                    .unwrap_or(Default::default());
                to_discard.extend(next_children_to_discard);

                self.blocks_to_parent.remove(&next_to_discard).unwrap();
                self.remove_snapshot(&next_to_discard);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;
    // use crate::db::Database;
    // use crate::state::{StateCheckpoint, DB};
    // use crate::BlockHash;
    //
    // fn write_values(
    //     db: DB,
    //     snapshot_ref: TreeQuery<Database, BlockStateManager<Database, FrozenSnapshot, BlockHash>>,
    //     values: &[(&str, &str)],
    // ) -> FrozenSnapshot {
    //     let checkpoint = StateCheckpoint::new(snapshot_ref);
    //     let mut working_set = checkpoint.into_revertable();
    //     for (key, value) in values {
    //         let key = Key::from(key.to_string());
    //         let value = Value::from(value.to_string());
    //         working_set.set(&key, value);
    //     }
    //     let checkpoint = working_set.commit();
    //     let (_witness, snapshot) = checkpoint.freeze();
    //     snapshot
    // }
    //
    // mod fork_tree_manager {
    //     use super::*;
    //
    //     #[test]
    //     fn new() {
    //         let db = DB::default();
    //         let state_manager =
    //             BlockStateManager::<Database, FrozenSnapshot, BlockHash>::new_locked(db.clone());
    //         let state_manager = state_manager.write().unwrap();
    //         assert!(state_manager.self_ref.is_some());
    //         {
    //             let db = db.lock().unwrap();
    //             assert!(db.data.is_empty());
    //         }
    //         assert!(state_manager.is_empty());
    //     }
    //
    //     #[test]
    //     fn linear_progression_with_2_blocks_delay() {
    //         let db = DB::default();
    //         let state_manager = BlockStateManager::new_locked(db.clone());
    //         let mut state_manager = state_manager.write().unwrap();
    //         assert!(state_manager.is_empty());
    //         let genesis_block = "genesis".to_string();
    //         let block_a = "a".to_string();
    //         let block_b = "b".to_string();
    //         let block_c = "c".to_string();
    //
    //         // Block A
    //         let block_a_values = vec![("x", "1"), ("y", "2")];
    //         let snapshot_ref = state_manager.get_new_ref(&genesis_block, &block_a);
    //
    //         assert!(!state_manager.is_empty());
    //         let snapshot = write_values(db.clone(), snapshot_ref, &block_a_values);
    //         state_manager.add_snapshot(snapshot);
    //         assert!(!state_manager.is_empty());
    //         {
    //             assert!(db.lock().unwrap().data.is_empty());
    //         }
    //
    //         // Block B
    //         let block_b_values = vec![("x", "3"), ("z", "4")];
    //         let snapshot_ref = state_manager.get_new_ref(&block_a, &block_b);
    //         let snapshot = write_values(db.clone(), snapshot_ref, &block_b_values);
    //         let snapshot_id_b = snapshot.get_id().clone();
    //         state_manager.add_snapshot(snapshot);
    //         assert_eq!(
    //             Some(CacheValue::from(Value::from("1".to_string()))),
    //             state_manager.get_value_recursively(
    //                 &snapshot_id_b,
    //                 &CacheKey::from(Key::from("x".to_string()))
    //             )
    //         );
    //         {
    //             assert!(db.lock().unwrap().data.is_empty());
    //         }
    //         println!("AFTER B: {:?}", state_manager);
    //         // Finalizing A
    //         state_manager.finalize_snapshot(&block_a);
    //         {
    //             let db = db.lock().unwrap();
    //             assert!(!db.data.is_empty());
    //             assert_eq!(Some("1".to_string()), db.get("x"));
    //             assert_eq!(Some("2".to_string()), db.get("y"));
    //             assert_eq!(None, db.get("z"));
    //         }
    //         println!("AFTER FINALIZING A: {:?}", state_manager);
    //
    //         // Block C
    //         let block_c_values = vec![("x", "5"), ("z", "6")];
    //         let snapshot_ref = state_manager.get_new_ref(&block_b, &block_c);
    //         let snapshot = write_values(db.clone(), snapshot_ref, &block_c_values);
    //         state_manager.add_snapshot(snapshot);
    //         println!("AFTER C: {:?}", state_manager);
    //         // Finalizing B
    //         state_manager.finalize_snapshot(&block_b);
    //         assert!(!state_manager.is_empty());
    //         {
    //             let db = db.lock().unwrap();
    //             assert!(!db.data.is_empty());
    //             assert_eq!(Some("3".to_string()), db.get("x"));
    //             assert_eq!(Some("2".to_string()), db.get("y"));
    //             assert_eq!(Some("4".to_string()), db.get("z"));
    //         }
    //
    //         state_manager.finalize_snapshot(&block_c);
    //         // TODO: Finalize everything, it should be clean
    //         println!("AFTER FINALIZING C: {:?}", state_manager);
    //         assert!(state_manager.is_empty());
    //     }
    //
    //     #[test]
    //     #[ignore = "TBD"]
    //     fn fork_added() {}
    //
    //     #[test]
    //     #[ignore = "TBD"]
    //     fn adding_alien_snapshot() {}
    //
    //     #[test]
    //     #[ignore = "TBD"]
    //     fn finalizing_alien_block() {}
    //
    //     #[test]
    //     #[ignore = "TBD"]
    //     fn finalizing_same_block_hash_twice() {}
    //
    //     #[test]
    //     #[ignore = "TBD"]
    //     fn requesting_ref_from_same_block_twice() {}
    // }
}
