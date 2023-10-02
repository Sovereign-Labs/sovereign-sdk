use sov_state::StorageInternalCache;
use std::collections::{HashMap, VecDeque};

type Hash = [u8; 32];
type SnapshotId = u64;

struct StateCheckpoint {
    snapshot_manager: SnapshotManager,
}

// TODO garbage collection after finalization
struct SnapshotManager {
    // Unique id for a new snapshot.
    unique_id: SnapshotId,
    // Snapshot we are currently modifying via get/set.
    currently_executed_snap: SnapshotId,
    snapshots: HashMap<SnapshotId, StorageInternalCache>,
    // All ancestors of a given snapshot kid => parent.
    snapshot_ancestors: HashMap<SnapshotId, SnapshotId>,
    // Vew on the blockchain: parent => kids.
    forks: HashMap<Hash, Vec<Hash>>,
    // snapshot checkpointed to a given block.
    checkpointed_blocks_to_snapshots: HashMap<Hash, SnapshotId>,
    // Used to cerate a new checkpoint in checkpointed_blocks_to_snapshots.
    current_block_hash: Hash,
}

impl SnapshotManager {
    fn set_head(&mut self, parent_block_hash: Hash, current_block_hash: Hash) {
        let fresh_snapshot = StorageInternalCache::default();
        self.unique_id += 1;
        self.currently_executed_snap = self.unique_id;
        self.snapshots.insert(self.unique_id, fresh_snapshot);

        // push current_block_hash as a child of parent_block_hash
        self.forks
            .get_mut(&parent_block_hash)
            .unwrap()
            .push(current_block_hash);
    }

    fn checkpoint(mut self) -> Self {
        // This saves the snapshot
        self.checkpointed_blocks_to_snapshots
            .insert(self.current_block_hash, self.currently_executed_snap);

        self
    }

    fn to_revertable(mut self) -> WorkingSet {
        // Create a new snapshot
        let parent_snapshot_id = self.currently_executed_snap;

        let fresh_snapshot = StorageInternalCache::default();
        self.unique_id += 1;
        self.currently_executed_snap = self.unique_id;
        self.snapshots.insert(self.unique_id, fresh_snapshot);

        self.snapshot_ancestors
            .insert(self.currently_executed_snap, parent_snapshot_id);

        WorkingSet {
            snapshot_manager: self,
        }
    }

    fn set(&mut self, k: Key, v: Value) {
        // self.snapshots
        //    .get_mut(&self.currently_executed_snap)
        //    .unwrap().set(key, value)
    }

    fn get(&mut self, k: Key) -> Value {
        // in a lop until we reach db
        // 1. current = get_current_snapshot
        // 2. parent = get_parent_snapshot
        // current.get_or_fetch(k, parent, todo: witness)

        todo!()
    }

    fn finalize(&mut self, block_hash_to_finalize: Hash) {
        let mut snapshot_id_to_finalize = self
            .checkpointed_blocks_to_snapshots
            .remove(&block_hash_to_finalize)
            .unwrap();

        let mut all_snapshots_to_save_in_db = VecDeque::default();
        while let Some(snapshot) = self.snapshots.remove(&snapshot_id_to_finalize) {
            all_snapshots_to_save_in_db.push_front(snapshot);
        }

        let snapshot_to_save = &mut all_snapshots_to_save_in_db.pop_front().unwrap();

        while let Some(s) = all_snapshots_to_save_in_db.pop_front() {
            snapshot_to_save.merge_left(s);
        }

        // Save snapshot_to_save in the db
    }
}

type Key = Vec<u8>;
type Value = Vec<u8>;

struct WorkingSet {
    snapshot_manager: SnapshotManager,
}

impl WorkingSet {
    fn set_head(&mut self, parent_block_hash: Hash, current_block_hash: Hash) {
        self.snapshot_manager
            .set_head(parent_block_hash, current_block_hash)
    }

    fn checkpoint(self) -> StateCheckpoint {
        StateCheckpoint {
            snapshot_manager: self.snapshot_manager.checkpoint(),
        }
    }

    fn finalize(&mut self, block_hash_to_finalize: Hash) {
        self.snapshot_manager.finalize(block_hash_to_finalize);
    }

    fn get(&mut self, k: Key) -> Value {
        self.snapshot_manager.get(k)
    }

    fn set(&mut self, k: Key, v: Value) {
        self.snapshot_manager.set(k, v)
    }
}
