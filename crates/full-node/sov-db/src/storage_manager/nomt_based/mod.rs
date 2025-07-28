//! Implementation of [`HierarchicalStorageManager`] based on NOMT

mod groups;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::reexports::digest;
use sov_rollup_interface::storage::HierarchicalStorageManager;

use crate::accessory_db::AccessoryDb;
use crate::config::RollupDbConfig;
use crate::historical_state::HistoricalStateReader;
use crate::metrics::nomt::StorageManagerFinalizationMetric;
use crate::state_db_nomt::{NomtSessionBuilder, StateOverlay};
use crate::storage_manager::nomt_based::groups::{CommitGroup, DbGroup, PrunerJob, SnapshotGroup};

#[allow(missing_docs)]
pub struct StateFinishedSession {
    user: nomt::FinishedSession,
    kernel: nomt::FinishedSession,
}

impl StateFinishedSession {
    /// Creates a new instance of [`StateFinishedSession`] from individual nomt sessions.
    pub fn new(user: nomt::FinishedSession, kernel: nomt::FinishedSession) -> Self {
        Self { user, kernel }
    }

    /// Converts it into [`StateOverlay`] which can be committed to disk or used in new sessions.
    pub(crate) fn into_state_overlay(self) -> StateOverlay {
        let StateFinishedSession { user, kernel } = self;
        StateOverlay {
            user: user.into_overlay(),
            kernel: kernel.into_overlay(),
        }
    }
}

#[allow(missing_docs)]
pub struct NomtChangeSet {
    pub state: StateFinishedSession,
    pub historical_state: SchemaBatch,
    pub accessory: SchemaBatch,
}

#[cfg(test)]
fn generate_empty_finished_session() -> nomt::FinishedSession {
    let dir = tempfile::tempdir().unwrap();

    let mut opts = nomt::Options::new();
    opts.path(dir.path());
    let nomt = nomt::Nomt::<nomt::hasher::BinaryHasher<sha2::Sha256>>::open(opts).unwrap();
    let params = nomt::SessionParams::default().witness_mode(nomt::WitnessMode::read_write());
    nomt.begin_session(params).finish(Vec::new()).unwrap()
}

#[cfg(test)]
impl Default for NomtChangeSet {
    fn default() -> Self {
        Self {
            state: StateFinishedSession {
                user: generate_empty_finished_session(),
                kernel: generate_empty_finished_session(),
            },
            historical_state: Default::default(),
            accessory: Default::default(),
        }
    }
}

/// The only thing [`NomtStorageManager`] needs to know about the thing it builds.
pub trait InitializableNativeNomtStorage<H, K>: Sized + Send + Sync
where
    K: Clone,
{
    #[allow(missing_docs)]
    fn new(
        state_db: NomtSessionBuilder<H, K>,
        historical_state: HistoricalStateReader,
        accessory_db: AccessoryDb,
        use_strict_mode: bool,
    ) -> Self;
}

/// Implementation of [`HierarchicalStorageManager`] based on NOMT.
pub struct NomtStorageManager<Da: DaSpec, H, S: InitializableNativeNomtStorage<H, Da::SlotHash>> {
    // L1 forks representation
    // Chain: prev_block -> child_blocks
    chain_forks: HashMap<Da::SlotHash, Vec<Da::SlotHash>>,
    // Reverse: child_block -> parent
    blocks_to_parent: HashMap<Da::SlotHash, Da::SlotHash>,

    rockbound_snapshots: HashMap<Da::SlotHash, SnapshotGroup>,
    nomt_snapshots: Arc<RwLock<HashMap<Da::SlotHash, StateOverlay>>>,

    db_group: DbGroup<H, Da::SlotHash>,

    // If pruner is running.
    pruner: Option<PrunerJob>,
    last_pruner_run_at_height: Option<u64>,
    pruner_block_interval: u64,
    pruner_versions_to_keep: usize,

    _phantom_s: PhantomData<S>,
}

impl<Da, H, S> NomtStorageManager<Da, H, S>
where
    Da: DaSpec,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    S: InitializableNativeNomtStorage<H, Da::SlotHash>,
{
    /// Create a new [` NomtStorageManager`].
    pub fn new(config: RollupDbConfig) -> anyhow::Result<Self> {
        let pruner_block_interval = config.get_pruner_interval();
        let pruner_versions_to_keep = config.get_pruner_versions_to_keep();
        assert!(
            pruner_versions_to_keep >= 1,
            "Pruner versions to keep should be at least 1, got {pruner_versions_to_keep}",
        );
        let db_group = DbGroup::new(config)?;

        db_group.verify_and_fix_commited_root_hashes()?;
        db_group.update_ledger_finalized_height()?;

        Ok(Self {
            chain_forks: Default::default(),
            blocks_to_parent: Default::default(),
            rockbound_snapshots: Default::default(),
            nomt_snapshots: Arc::new(Default::default()),
            db_group,
            pruner: None,
            last_pruner_run_at_height: None,
            pruner_block_interval,
            pruner_versions_to_keep,
            _phantom_s: Default::default(),
        })
    }

    // build a storage up to the given block_hash (inclusive).
    fn create_state_up_to(
        &self,
        block_hash: Da::SlotHash,
        use_strict_mode: bool,
    ) -> anyhow::Result<(S, DeltaReader)> {
        tracing::trace!(%block_hash, "Creating storage up to block hash");
        // References are in reversed chronological order,
        // starting from the tip of the chain going back to the last finalized header

        let mut rev_references = Vec::new();

        let mut current_hash = block_hash.clone();

        {
            while self.rockbound_snapshots.contains_key(&current_hash) {
                rev_references.push(current_hash.clone());
                match self.blocks_to_parent.get(&current_hash) {
                    None => {
                        break;
                    }
                    Some(parent_hash) => {
                        current_hash = parent_hash.clone();
                    }
                }
            }
        }

        tracing::trace!(?rev_references, %block_hash, "Collected hashes storage up to block hash");

        self.db_group.create_storage(
            rev_references,
            &self.rockbound_snapshots,
            self.nomt_snapshots.clone(),
            use_strict_mode,
        )
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.rockbound_snapshots.is_empty()
            && self.blocks_to_parent.is_empty()
            && self.chain_forks.is_empty()
            // Lock at the end, so should be triggered last in case of non-empty.
            && self.nomt_snapshots.read().unwrap().is_empty()
    }

    #[cfg(test)]
    pub(crate) fn snapshots_count(&self) -> usize {
        let nomt_snapshots_count = {
            let nomt_snapshots = self.nomt_snapshots.read().unwrap();
            nomt_snapshots.len()
        };
        assert_eq!(nomt_snapshots_count, self.rockbound_snapshots.len());
        nomt_snapshots_count
    }

    #[cfg(test)]
    pub(crate) fn blocks_to_parent_count(&self) -> usize {
        self.blocks_to_parent.len()
    }
}

impl<Da, H, S> HierarchicalStorageManager<Da> for NomtStorageManager<Da, H, S>
where
    Da: DaSpec,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    S: InitializableNativeNomtStorage<H, Da::SlotHash>,
{
    type StfState = S;
    type StfChangeSet = NomtChangeSet;
    type LedgerState = DeltaReader;
    type LedgerChangeSet = SchemaBatch;

    fn create_state_for(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)> {
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

        // Storage created "for" a block implies node context,
        // and we expect a change set from this storage to be saved.
        // That's why it is created in a strict mode.
        let state = self.create_state_up_to(block_header.prev_hash(), true)?;

        Ok(state)
    }

    fn create_state_after(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)> {
        // Storage created "after" a block is usually used outside of the node context,
        // So strict mode is not needed.
        let use_strict_mode = false;
        if !self.rockbound_snapshots.contains_key(&block_header.hash()) {
            tracing::debug!(block_header = %block_header.display(), "Creating new storage from finalized data as block header is not in the saved chain");
            self.db_group.create_storage(
                Vec::new(),
                &self.rockbound_snapshots,
                self.nomt_snapshots.clone(),
                use_strict_mode,
            )
        } else {
            self.create_state_up_to(block_header.hash(), use_strict_mode)
        }
    }

    fn save_change_set(
        &mut self,
        block_header: &Da::BlockHeader,
        stf_change_set: Self::StfChangeSet,
        ledger_change_set: Self::LedgerChangeSet,
    ) -> anyhow::Result<()> {
        tracing::trace!(block_header = %block_header.display(), "Saving changes");

        if !self.chain_forks.contains_key(&block_header.prev_hash()) {
            anyhow::bail!(
                "Attempt to save changeset for unknown block header {}",
                block_header.display(),
            );
        }

        let block_hash = block_header.hash();
        if self.rockbound_snapshots.contains_key(&block_hash) {
            anyhow::bail!(
                "Attempt to save changes for the same block {} twice. Probably a bug.",
                block_header.display()
            )
        }

        let NomtChangeSet {
            state,
            historical_state,
            accessory,
        } = stf_change_set;

        let state_overlay = state.into_state_overlay();

        let rockbound_snapshot = SnapshotGroup {
            historical_state: Arc::new(historical_state),
            accessory: Arc::new(accessory),
            ledger: Arc::new(ledger_change_set),
        };

        // Deliberately keep lock till the end of the method to maintain internal consistency.
        let mut nomt_snapshots = self
            .nomt_snapshots
            .write()
            .expect("Failed to lock snapshots");
        nomt_snapshots.insert(block_hash.clone(), state_overlay);
        self.rockbound_snapshots
            .insert(block_hash, rockbound_snapshot);

        Ok(())
    }

    /// **Warning**: There should be no active storages by the time this method is called.
    /// From [NOMT documentation](https://github.com/thrumdev/nomt/blob/51a2a3559b2a3153244dda923daf7e38807a9427/nomt/src/lib.rs#L652):
    /// This function will block until all ongoing sessions and commits have finished.
    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()> {
        let start_prep = std::time::Instant::now();
        tracing::trace!(block_hash = %block_header.hash(), "Finalizing changes");

        if !self.rockbound_snapshots.contains_key(&block_header.hash()) {
            anyhow::bail!(
                "No changes has been previously saved for block header prev_hash={} next_hash={}",
                block_header.prev_hash(),
                block_header.hash(),
            );
        }

        // --- Step 1: Collect block hashes to be finalized and discarded ---
        let mut finalization_segments: Vec<(Da::SlotHash, Da::SlotHash)> = Vec::new(); // (parent, child_to_keep)
        let mut all_discard_hashes_set: std::collections::HashSet<Da::SlotHash> =
            std::collections::HashSet::new();

        // 1a. Collect finalization segments (parent, child_to_keep), ordered from oldest to newest
        {
            let mut current_child_hash = block_header.hash();
            let mut current_parent_hash = block_header.prev_hash();
            loop {
                finalization_segments
                    .push((current_parent_hash.clone(), current_child_hash.clone()));
                if let Some(grand_parent_hash) = self.blocks_to_parent.get(&current_parent_hash) {
                    current_child_hash = current_parent_hash;
                    current_parent_hash = grand_parent_hash.clone();
                } else {
                    // current_parent_hash is the oldest parent in the chain we're finalizing
                    break;
                }
            }
            // TODO: Maybe not reverse here, but just iterate in reverse order.
            finalization_segments.reverse();
        }
        tracing::trace!(?finalization_segments, "Collected finalization segments");

        // 1b. Collect all block hashes to be discarded
        {
            let mut discard_queue: std::collections::VecDeque<Da::SlotHash> =
                std::collections::VecDeque::new();

            // Seed the discard_queue with initial siblings to discard
            for (parent_hash, child_to_keep_hash) in &finalization_segments {
                if let Some(children) = self.chain_forks.get(parent_hash) {
                    for sibling_hash in children {
                        // Avoid re-queueing if already processed or queued
                        if sibling_hash != child_to_keep_hash
                            && !all_discard_hashes_set.contains(sibling_hash)
                        {
                            discard_queue.push_back(sibling_hash.clone());
                        }
                    }
                }
            }

            while let Some(block_to_discard) = discard_queue.pop_front() {
                if all_discard_hashes_set.insert(block_to_discard.clone()) {
                    // Process only if newly added
                    if let Some(children_of_discarded) = self.chain_forks.get(&block_to_discard) {
                        for child in children_of_discarded {
                            if !all_discard_hashes_set.contains(child) {
                                // Avoid re-queueing
                                discard_queue.push_back(child.clone());
                            }
                        }
                    }
                }
            }
        }
        let preparation_time = start_prep.elapsed();
        tracing::trace!(
            ?all_discard_hashes_set,
            "Collected all hashes to be discarded"
        );

        let apply_start = std::time::Instant::now();
        // --- Step 2: Apply changes.
        {
            let mut nomt_snapshots_guard = self
                .nomt_snapshots
                .write()
                .expect("Failed to lock nomt_snapshots for finalization");

            // Helper to remove snapshot data from both hashmaps
            let mut remove_snapshot_payloads_fn =
                |block_hash: &Da::SlotHash| -> Option<CommitGroup> {
                    let nomt_snapshot = nomt_snapshots_guard.remove(block_hash);
                    let rockbound_snapshot = self.rockbound_snapshots.remove(block_hash);
                    match (nomt_snapshot, rockbound_snapshot) {
                        (Some(nomt), Some(rockbound)) => Some(CommitGroup { rockbound, nomt }),
                        (None, None) => None,
                        _ => panic!(
                            "Inconsistent storage manager state: discrepancy between rockbound and nomt snapshots for block hash {block_hash}"
                        ),
                    }
                };
            // 2a. Process finalized blocks
            for (parent_hash, child_hash_to_keep) in &finalization_segments {
                tracing::trace!(%parent_hash, %child_hash_to_keep, "Finalizing segment");
                if let Some(snapshot_to_commit) = remove_snapshot_payloads_fn(child_hash_to_keep) {
                    self.db_group.commit(snapshot_to_commit)?;
                } else {
                    // This block was expected to have a snapshot
                    return Err(anyhow::anyhow!(
                        "Snapshot for block to be finalized {} (child of {}) not found during finalization",
                        child_hash_to_keep, parent_hash
                    ));
                }
                self.blocks_to_parent.remove(parent_hash);
                self.blocks_to_parent.remove(child_hash_to_keep);
                self.chain_forks.remove(parent_hash);
            }
            // 2b. Process discarded blocks
            for discarded_hash in &all_discard_hashes_set {
                tracing::trace!(%discarded_hash, "Discarding block artifacts");
                remove_snapshot_payloads_fn(discarded_hash);

                self.blocks_to_parent.remove(discarded_hash);
                self.chain_forks.remove(discarded_hash);
            }
        }
        let apply_time = apply_start.elapsed();
        tracing::trace!(
            finalized_block_hash = %block_header.hash(),
            ?preparation_time,
            ?apply_time,
            "Finalization complete");

        let is_pruner_ready = self
            .pruner
            .as_ref()
            .map(|p| p.is_finished())
            .unwrap_or(false);
        let mut pruning_commit_time = None;
        if is_pruner_ready {
            // UNWRAP: Checked above.
            let pruner = std::mem::take(&mut self.pruner).unwrap();
            let prune_group = pruner.join()?;
            let start = std::time::Instant::now();
            self.db_group.commit_pruning(prune_group)?;
            pruning_commit_time = Some(start.elapsed());
            self.last_pruner_run_at_height = Some(block_header.height());
        }

        let should_run_pruner = self.pruner.is_none()
            && self
                .last_pruner_run_at_height
                .map(|last_run_at_height| {
                    block_header.height().saturating_sub(last_run_at_height)
                        > self.pruner_block_interval
                })
                .unwrap_or(true);
        if should_run_pruner {
            let pruner = self.db_group.start_pruner(self.pruner_versions_to_keep);
            self.pruner = Some(pruner);
        }

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(StorageManagerFinalizationMetric {
                da_height: block_header.height(),
                preparation_time,
                commit_time: apply_time,
                pruning_commit_time,
            });
        });
        Ok(())
    }
}
