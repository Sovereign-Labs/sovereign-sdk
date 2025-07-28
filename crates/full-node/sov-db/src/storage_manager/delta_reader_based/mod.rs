//! Storage Manager that works with [`DeltaReader`].
mod groups;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;

use groups::{DbGroup, SnapshotGroup};
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::storage::HierarchicalStorageManager;

use crate::accessory_db::AccessoryDb;
use crate::state_db::StateDb;

/// Container that can be used for building actual storage from [`StateDb`] and [`AccessoryDb`].
#[derive(Debug, Clone)]
pub struct StfStorageHandlers {
    #[allow(missing_docs)]
    pub state: StateDb,
    #[allow(missing_docs)]
    pub accessory: AccessoryDb,
}

/// The only thing [`NativeStorageManager`] needs to know about thing it builds.
pub trait InitializableNativeStorage: Sized + Send + Sync {
    #[allow(missing_docs)]
    fn new(db: StateDb, accessory_db: AccessoryDb) -> Self;
}

/// Change produced in native execution.
#[derive(Debug, Default, Clone)]
pub struct NativeChangeSet {
    /// [`SchemaBatch`] associated with provable state updates.
    pub state_change_set: SchemaBatch,
    /// [`SchemaBatch`] associated with non-provable accessory updates.
    pub accessory_change_set: SchemaBatch,
}

/// Storage manager handles StateDb,
/// AccessoryDb and LedgerDb lifecycle in relation to the Data Availability layer.
pub struct NativeStorageManager<Da: DaSpec, S: InitializableNativeStorage> {
    // L1 forks representation
    // Chain: prev_block -> child_blocks
    chain_forks: HashMap<Da::SlotHash, Vec<Da::SlotHash>>,
    // Reverse: child_block -> parent
    blocks_to_parent: HashMap<Da::SlotHash, Da::SlotHash>,
    snapshots: HashMap<Da::SlotHash, SnapshotGroup>,

    // For writing committed changes.
    db_group: DbGroup,
    phantom_storage: PhantomData<S>,
}

impl<Da: DaSpec, S: InitializableNativeStorage> NativeStorageManager<Da, S> {
    /// Create new [`NativeStorageManager`] in a given path.
    pub fn new(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let db_group = DbGroup::new_write(path.as_ref().to_path_buf())?;

        // Updating ledger at startup
        db_group.update_ledger_finalized_height()?;

        Ok(Self {
            chain_forks: Default::default(),
            blocks_to_parent: Default::default(),
            snapshots: Default::default(),
            db_group,
            phantom_storage: Default::default(),
        })
    }

    fn finalize_by_hash_pair(
        &mut self,
        prev_block_hash: Da::SlotHash,
        current_block_hash: Da::SlotHash,
    ) -> anyhow::Result<()> {
        tracing::trace!(
            %prev_block_hash,
            %current_block_hash,
            "Finalizing block by pair of hashes"
        );

        if let Some(grand_parent) = self.blocks_to_parent.remove(&prev_block_hash) {
            self.finalize_by_hash_pair(grand_parent, prev_block_hash.clone())?;
        }

        let snapshot = self.snapshots.remove(&current_block_hash).ok_or_else(|| {
            anyhow::anyhow!(
                "No changes has been previously saved for block header prev_hash={} next_hash={}",
                prev_block_hash,
                current_block_hash,
            )
        })?;

        self.db_group.commit(snapshot)?;

        self.blocks_to_parent.remove(&current_block_hash);

        // All siblings of the current snapshot
        let mut to_discard: Vec<_> = self
            .chain_forks
            .remove(&prev_block_hash)
            .expect("Inconsistent chain_forks")
            .into_iter()
            .filter(|bh| bh != &current_block_hash)
            .collect();

        while let Some(block_hash) = to_discard.pop() {
            let child_block_hashes = self.chain_forks.remove(&block_hash).unwrap_or_default();
            self.blocks_to_parent
                .remove(&block_hash)
                .expect("Chain map inconsistency in `blocks_to_parent`");
            if self.snapshots.remove(&block_hash).is_some() {
                tracing::trace!(%block_hash, "Discarding snapshot");
            }
            to_discard.extend(child_block_hashes);
        }
        Ok(())
    }

    // build a storage up to given block_hash (inclusive).
    fn create_state_up_to(&self, block_hash: Da::SlotHash) -> anyhow::Result<(S, DeltaReader)> {
        // Snapshots are in reversed order
        let mut rev_snapshots = Vec::new();

        let mut current_hash = block_hash;
        while let Some(snapshot) = self.snapshots.get(&current_hash) {
            rev_snapshots.push(snapshot.clone());
            match self.blocks_to_parent.get(&current_hash) {
                None => {
                    break;
                }
                Some(parent_hash) => {
                    current_hash = parent_hash.clone();
                }
            }
        }

        self.db_group.create_storage(rev_snapshots)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.snapshots.is_empty() && self.blocks_to_parent.is_empty() && self.chain_forks.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn snapshots_count(&self) -> usize {
        self.snapshots.len()
    }

    #[cfg(test)]
    pub(crate) fn blocks_to_parent_count(&self) -> usize {
        self.blocks_to_parent.len()
    }

    /// Extensive checks about internal consistency of the internal maps storage manager.
    /// Panics if something is off.
    pub(crate) fn dbg_validate_internal_consistency(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        for (slot_hash, child_hashes) in self.chain_forks.iter() {
            let mut unique_child_hashes =
                std::collections::HashSet::<Da::SlotHash>::with_capacity(child_hashes.len());
            // Check of chain forks and to parent.
            for child_hash in child_hashes {
                unique_child_hashes.insert(child_hash.clone());
                assert_eq!(
                    Some(slot_hash),
                    self.blocks_to_parent.get(child_hash),
                    "missing entry in blocks_to_parent for {child_hash}"
                );
            }
            // There is no duplicates in child hashes.
            assert_eq!(
                unique_child_hashes.len(),
                child_hashes.len(),
                "Duplicate data in `chain_forks`"
            );

            // Check that all "inner" hashes have snapshots attached.
            let has_children = !child_hashes.is_empty();
            let has_parent = self.blocks_to_parent.contains_key(slot_hash);
            if has_parent && has_children {
                assert!(
                    self.snapshots.contains_key(slot_hash),
                    "missing snapshot for 'inner' block"
                );
            }
        }

        // All parent forks contains given child
        for (child_hash, parent_hash) in self.blocks_to_parent.iter() {
            let parent_forks = self.chain_forks.get(parent_hash).unwrap();
            assert!(
                parent_forks.contains(child_hash),
                "Parent doesn't contain reference to child."
            );
        }

        // No "unmapped" snapshots.
        for slot_hash in self.snapshots.keys() {
            // For each snapshot, there should be either:
            // Entry in chain forks.
            // Means it is the oldest block
            let has_children = self.chain_forks.contains_key(slot_hash);
            // Entry in blocks to parent.
            // Meaning it is a leaf block.
            let has_parent = self.blocks_to_parent.contains_key(slot_hash);

            assert!(
                has_parent || has_children,
                "snapshot for {slot_hash} has no parents({has_parent}) or children({has_children})"
            );
        }
    }
}

impl<Da: DaSpec, S: InitializableNativeStorage> HierarchicalStorageManager<Da>
    for NativeStorageManager<Da, S>
where
    Da::SlotHash: Hash,
{
    type StfState = S;
    type StfChangeSet = NativeChangeSet;
    type LedgerState = DeltaReader;
    type LedgerChangeSet = SchemaBatch;

    fn create_state_for(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)> {
        self.dbg_validate_internal_consistency();

        tracing::trace!(block_header = %block_header.display(), "Requested native storage");
        let prev_hash = block_header.prev_hash();
        let current_hash = block_header.hash();
        if let std::collections::hash_map::Entry::Vacant(e) =
            self.blocks_to_parent.entry(current_hash.clone())
        {
            self.chain_forks
                .entry(prev_hash.clone())
                .or_default()
                .push(current_hash.clone());
            e.insert(prev_hash);
        }
        let state = self.create_state_up_to(block_header.prev_hash())?;

        self.dbg_validate_internal_consistency();
        Ok(state)
    }

    fn create_state_after(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)> {
        self.dbg_validate_internal_consistency();

        let state = if !self.snapshots.contains_key(&block_header.hash()) {
            tracing::trace!(block_header = %block_header.display(), "Creating new storage from finalized data as block header is not in the saved chain");
            self.db_group.create_storage(Vec::new())?
        } else {
            self.create_state_up_to(block_header.hash())?
        };

        self.dbg_validate_internal_consistency();
        Ok(state)
    }

    fn save_change_set(
        &mut self,
        block_header: &Da::BlockHeader,
        stf_change_set: Self::StfChangeSet,
        ledger_change_set: Self::LedgerChangeSet,
    ) -> anyhow::Result<()> {
        self.dbg_validate_internal_consistency();

        tracing::trace!(block_header = %block_header.display(), "Saving changes");
        if !self.chain_forks.contains_key(&block_header.prev_hash()) {
            anyhow::bail!(
                "Attempt to save changeset for unknown block header {}",
                block_header.display(),
            );
        }
        let block_hash = block_header.hash();
        if self.snapshots.contains_key(&block_hash) {
            anyhow::bail!(
                "Attempt to save changes for the same block {} twice. Probably a bug.",
                block_header.display()
            )
        }

        let NativeChangeSet {
            state_change_set,
            accessory_change_set,
        } = stf_change_set;

        let snapshot =
            SnapshotGroup::new(state_change_set, accessory_change_set, ledger_change_set);
        self.snapshots.insert(block_hash, snapshot);

        self.dbg_validate_internal_consistency();
        Ok(())
    }

    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()> {
        self.dbg_validate_internal_consistency();

        tracing::trace!(block_hash = %block_header.hash(), "Finalizing changes");
        self.finalize_by_hash_pair(block_header.prev_hash(), block_header.hash())?;

        self.dbg_validate_internal_consistency();
        Ok(())
    }
}
