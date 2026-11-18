//! Implementation of [`HierarchicalStorageManager`] based on NOMT

mod groups;
mod pruning;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

pub use crate::flat_db::FlatStateDb;
use anyhow::Context;
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
#[cfg(feature = "migration-script")]
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::reexports::digest;
use sov_rollup_interface::storage::HierarchicalStorageManager;

use crate::accessory_db::AccessoryDb;
use crate::config::{RollupDbConfig, RollupDbConfigWithCustomizations};
use crate::historical_state::{HistoricalStateReader, StateChanges};
use crate::metrics::nomt::StorageManagerFinalizationMetric;
use crate::state_db_nomt::{NomtSessionBuilder, StateOverlay};
use crate::storage_manager::nomt_based::groups::{DbGroup, SnapshotGroup};
use crate::storage_manager::nomt_based::pruning::PruningController;
pub use groups::PrunerJobOutput;
pub(crate) use groups::DEFAULT_MAX_PRUNING_BATCH_SIZE;

/// Controls witness generation for storage creation.
pub enum WitnessMode {
    /// Record witness hints for ZK proving.
    On,
    /// No witness generation.
    Off,
}

impl WitnessMode {
    /// Creates a [`WitnessMode::Off`] variant.
    pub fn off() -> Self {
        Self::Off
    }

    /// Returns `true` if witness generation is enabled.
    pub fn is_witness_enabled(&self) -> bool {
        matches!(self, Self::On)
    }

    /// Constructs a [`WitnessMode`] from a witness flag.
    pub fn from_bool(with_witness: bool) -> Self {
        if with_witness {
            Self::On
        } else {
            Self::Off
        }
    }
}

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
    pub historical_state: StateChanges,
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
        strict_mode: bool,
        witness_mode: WitnessMode,
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

    /// Owns the pruning schedule and runtime: the resolved policy, the in-flight background
    /// pruner job, and test-only observability counters.
    pruning: PruningController,

    /// When true, `create_state_for` will generate witness hints for ZK proving.
    witness_generation_enabled: bool,

    /// Finalized blocks being written to disk by the background commit thread.
    committing: Option<Committing<Da::SlotHash>>,

    _phantom_s: PhantomData<S>,
}

/// Finalized blocks whose commit runs on a background thread.
struct Committing<K> {
    /// Oldest first.
    blocks: Vec<K>,
    /// Live flat state version before this commit started. Storages are built on top of it
    /// while the commit runs, because the live version advances under them.
    base_version: Option<u64>,
    handle: std::thread::JoinHandle<()>,
}

impl<Da, H, S> NomtStorageManager<Da, H, S>
where
    Da: DaSpec,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    S: InitializableNativeNomtStorage<H, Da::SlotHash>,
{
    /// Create a new [` NomtStorageManager`].
    ///
    /// `witness_generation` controls whether `create_state_for` will generate witness hints
    /// for ZK proving.
    pub fn new(config: RollupDbConfig, witness_generation: bool) -> anyhow::Result<Self> {
        Self::new_with_custom_config(config.into(), witness_generation)
    }

    /// Create a new [` NomtStorageManager`] with runtime-only RocksDB customization.
    pub fn new_with_custom_config(
        custom_config: RollupDbConfigWithCustomizations,
        witness_generation: bool,
    ) -> anyhow::Result<Self> {
        let mut pruning = PruningController::new(custom_config.config().pruner());
        let mut db_group = DbGroup::new(custom_config)?;
        db_group.update_ledger_finalized_height()?;
        // Run the one-time startup prune now (a no-op unless `PrunerConfig::OnceAtStartup`), before
        // any state access, while there is no live read/write traffic to compete with.
        pruning.run_startup_prune(&mut db_group)?;

        Ok(Self {
            chain_forks: Default::default(),
            blocks_to_parent: Default::default(),
            rockbound_snapshots: Default::default(),
            nomt_snapshots: Arc::new(Default::default()),
            db_group,
            pruning,
            witness_generation_enabled: witness_generation,
            committing: None,
            _phantom_s: Default::default(),
        })
    }

    /// Waits for the background commit of finalized blocks. After it returns, everything
    /// finalized so far is on disk. A failed commit panics, and that panic is re-raised here.
    ///
    /// Must not be called by a thread that holds a live NOMT session:
    /// the commit waits for all sessions to end.
    pub fn flush(&mut self) {
        let Some(Committing { blocks, handle, .. }) = self.committing.take() else {
            return;
        };
        if let Err(panic) = handle.join() {
            std::panic::resume_unwind(panic);
        }
        for block_hash in &blocks {
            self.rockbound_snapshots.remove(block_hash);
            self.blocks_to_parent.remove(block_hash);
        }
    }

    // build a storage up to the given block_hash (inclusive).
    fn create_state_up_to(
        &self,
        block_hash: Da::SlotHash,
        strict_mode: bool,
        witness_mode: WitnessMode,
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

        self.create_storage(rev_references, strict_mode, witness_mode)
    }

    /// Builds a storage on top of the finalized state plus the snapshots of `rev_references`.
    /// `rev_references` are in reverse chronological order and must all be present in
    /// `rockbound_snapshots`.
    fn create_storage(
        &self,
        mut rev_references: Vec<Da::SlotHash>,
        strict_mode: bool,
        witness_mode: WitnessMode,
    ) -> anyhow::Result<(S, DeltaReader)> {
        let base_version = match &self.committing {
            Some(committing) => {
                // Blocks being committed are part of the finalized state, so storages always
                // include all of them, as if they were already on disk. Stopping at one of them is
                // not possible: the NOMT overlays of committed blocks are gone, so NOMT would read
                // past it while the flat state would not.
                rev_references.retain(|block_hash| !committing.blocks.contains(block_hash));
                rev_references.extend(committing.blocks.iter().rev().cloned());
                committing.base_version
            }
            None => self
                .db_group
                .latest_flat_state_version()?
                .map(|slot| slot.get()),
        };
        let snapshots = rev_references
            .into_iter()
            .map(|block_hash| {
                let snapshot = self
                    .rockbound_snapshots
                    .get(&block_hash)
                    .expect("Referenced block must have a rockbound snapshot");
                (block_hash, snapshot)
            })
            .collect();
        self.db_group.create_storage(
            snapshots,
            base_version,
            self.nomt_snapshots.clone(),
            strict_mode,
            witness_mode,
        )
    }

    #[cfg(feature = "migration-script")]
    /// Creates a strict storage view over the latest finalized state without mutating
    /// fork bookkeeping maps.
    pub fn create_state_for_migration(&self) -> anyhow::Result<(S, DeltaReader)> {
        self.create_storage(Vec::new(), true, WitnessMode::off())
    }

    #[cfg(feature = "migration-script")]
    /// Commits migration changes directly at the current head version.
    pub fn commit_migration_change_set_at_head(
        &mut self,
        head_slot: SlotNumber,
        stf_change_set: NomtChangeSet,
        ledger_change_set: SchemaBatch,
    ) -> anyhow::Result<()> {
        if !self.rockbound_snapshots.is_empty()
            || !self
                .nomt_snapshots
                .read()
                .expect("Failed to lock snapshots")
                .is_empty()
            || !self.blocks_to_parent.is_empty()
            || !self.chain_forks.is_empty()
        {
            anyhow::bail!(
                "migration commit requires an empty in-memory fork cache; restart with a fresh storage manager"
            );
        }

        let live_latest = self.db_group.latest_flat_state_version()?;

        let Some(live_latest) = live_latest else {
            anyhow::bail!("cannot run migration commit on empty state");
        };

        if live_latest != head_slot {
            anyhow::bail!(
                "head slot {} does not match flat-state latest version {}",
                head_slot,
                live_latest
            );
        }

        let NomtChangeSet {
            state,
            historical_state,
            accessory,
        } = stf_change_set;

        let state_overlay = state.into_state_overlay();
        let commit_group = groups::CommitGroup {
            nomt: state_overlay,
            rockbound: SnapshotGroup {
                historical_state,
                accessory: Arc::new(accessory),
                ledger: Arc::new(ledger_change_set),
            },
        };

        self.db_group
            .commit_at_latest_checked(commit_group, head_slot)
    }

    /// Whether `block_hash` is being committed in the background.
    /// The test helpers below report the state as if that commit has already been joined.
    #[cfg(test)]
    fn is_committing(&self, block_hash: &Da::SlotHash) -> bool {
        self.committing
            .as_ref()
            .is_some_and(|committing| committing.blocks.contains(block_hash))
    }

    /// Blocks until the background commit thread has finished, without joining it.
    #[cfg(test)]
    pub(crate) fn wait_for_commit_to_finish(&self) {
        if let Some(committing) = &self.committing {
            while !committing.handle.is_finished() {
                std::thread::yield_now();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.snapshots_count() == 0
            && self.blocks_to_parent_count() == 0
            && self.chain_forks.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn snapshots_count(&self) -> usize {
        let nomt_snapshots_count = {
            let nomt_snapshots = self.nomt_snapshots.read().unwrap();
            nomt_snapshots
                .keys()
                .filter(|block_hash| !self.is_committing(block_hash))
                .count()
        };
        let rockbound_snapshots_count = self
            .rockbound_snapshots
            .keys()
            .filter(|block_hash| !self.is_committing(block_hash))
            .count();
        assert_eq!(nomt_snapshots_count, rockbound_snapshots_count);
        nomt_snapshots_count
    }

    #[cfg(test)]
    pub(crate) fn blocks_to_parent_count(&self) -> usize {
        self.blocks_to_parent
            .keys()
            .filter(|block_hash| !self.is_committing(block_hash))
            .count()
    }

    /// Number of pruning batches committed so far. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn pruning_commits_count(&self) -> u64 {
        self.pruning.commits_count()
    }

    /// Whether the most recently committed pruning batch hit the size limit. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn last_pruning_hit_size_limit(&self) -> bool {
        self.pruning.last_hit_size_limit()
    }

    /// Whether a pruner thread is currently in flight. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn is_pruner_running(&self) -> bool {
        self.pruning.is_running()
    }

    /// `None` if no pruner job is in flight, otherwise `Some(finished)`. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn pruner_in_flight_finished(&self) -> Option<bool> {
        self.pruning.in_flight_finished()
    }

    /// Blocks the current thread until any in-flight pruner thread has finished. Does NOT commit
    /// the resulting batch (that happens on the next `finalize`). Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn wait_for_pruner_to_finish(&self) {
        self.pruning.wait_for_finish();
    }
}

impl<Da, H, S> HierarchicalStorageManager<Da> for NomtStorageManager<Da, H, S>
where
    Da: DaSpec,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync + 'static,
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
        // That's why it is created in strict mode.
        let witness_mode = WitnessMode::from_bool(self.witness_generation_enabled);
        let state = self.create_state_up_to(block_header.prev_hash(), true, witness_mode)?;

        Ok(state)
    }

    fn create_state_after(
        &mut self,
        block_header: &Da::BlockHeader,
    ) -> anyhow::Result<(Self::StfState, Self::LedgerState)> {
        // Storage created "after" a block is usually used outside of the node context,
        // So neither strict mode nor witness is needed.
        if !self.rockbound_snapshots.contains_key(&block_header.hash()) {
            tracing::debug!(block_header = %block_header.display(), "Creating new storage from finalized data as block header is not in the saved chain");
            self.create_storage(Vec::new(), false, WitnessMode::off())
        } else {
            self.create_state_up_to(block_header.hash(), false, WitnessMode::off())
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
            historical_state,
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

    /// Waits for the previous finalization to land on disk, then commits the newly finalized
    /// blocks on a background thread. Storages created meanwhile already include them.
    ///
    /// Must not be called by a thread that holds a live NOMT session:
    /// the commit waits for all sessions to end.
    fn finalize(&mut self, block_header: &Da::BlockHeader) -> anyhow::Result<()> {
        let wait_start = std::time::Instant::now();
        self.flush();
        let wait_for_previous_commit_time = wait_start.elapsed();
        tracing::trace!(block_hash = %block_header.hash(), "Finalizing changes");

        if !self.rockbound_snapshots.contains_key(&block_header.hash()) {
            anyhow::bail!(
                "No changes has been previously saved for block header prev_hash={} next_hash={}",
                block_header.prev_hash(),
                block_header.hash(),
            );
        }

        // Every fallible step runs before the bookkeeping maps are changed, so an error leaves
        // the storage manager as it was.

        // No commit is in flight, so the on-disk version is stable.
        let base_version = self
            .db_group
            .latest_flat_state_version()?
            .map(|slot| slot.get());

        let start_prep = std::time::Instant::now();
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
        tracing::trace!(
            ?all_discard_hashes_set,
            "Collected all hashes to be discarded"
        );

        // 1c. Collect the snapshots of the finalized blocks.
        let mut blocks = Vec::with_capacity(finalization_segments.len());
        let mut blocks_to_commit = Vec::with_capacity(finalization_segments.len());
        {
            let nomt_snapshots = self
                .nomt_snapshots
                .read()
                .expect("Failed to lock nomt_snapshots for finalization");
            for (parent_hash, child_hash_to_keep) in &finalization_segments {
                let Some(rockbound_snapshot) = self.rockbound_snapshots.get(child_hash_to_keep)
                else {
                    anyhow::bail!(
                        "Snapshot for block to be finalized {child_hash_to_keep} (child of {parent_hash}) not found during finalization"
                    );
                };
                anyhow::ensure!(
                    nomt_snapshots.contains_key(child_hash_to_keep),
                    "Inconsistent storage manager state: block to be finalized {child_hash_to_keep} has a rockbound snapshot but no NOMT snapshot"
                );
                blocks.push(child_hash_to_keep.clone());
                blocks_to_commit.push((child_hash_to_keep.clone(), rockbound_snapshot.clone()));
            }
        }
        let preparation_time = start_prep.elapsed();

        // Runs between commits, so pruning writes never overlap a commit.
        let pruning_commit_time = self
            .pruning
            .on_finalize(&mut self.db_group, block_header.height())
            .with_context(|| {
                format!(
                    "periodic pruning failed while finalizing block height={} hash={}",
                    block_header.height(),
                    block_header.hash()
                )
            })?;

        let apply_start = std::time::Instant::now();
        // --- Step 2: Apply changes. Snapshots and links of the finalized blocks are removed after
        // their commit is joined.
        {
            let mut nomt_snapshots = self
                .nomt_snapshots
                .write()
                .expect("Failed to lock nomt_snapshots for finalization");

            for (parent_hash, child_hash_to_keep) in &finalization_segments {
                tracing::trace!(%parent_hash, %child_hash_to_keep, "Finalizing segment");
                self.chain_forks.remove(parent_hash);
            }

            for discarded_hash in &all_discard_hashes_set {
                tracing::trace!(%discarded_hash, "Discarding block artifacts");
                let nomt_snapshot = nomt_snapshots.remove(discarded_hash);
                let rockbound_snapshot = self.rockbound_snapshots.remove(discarded_hash);
                assert_eq!(
                    nomt_snapshot.is_some(),
                    rockbound_snapshot.is_some(),
                    "Inconsistent storage manager state: discrepancy between rockbound and nomt snapshots for block hash {discarded_hash}"
                );
                self.blocks_to_parent.remove(discarded_hash);
                self.chain_forks.remove(discarded_hash);
            }
        }

        // Commit finalized blocks in the background.
        let handle = self
            .db_group
            .committer()
            .spawn_commit(blocks_to_commit, self.nomt_snapshots.clone());
        self.committing = Some(Committing {
            blocks,
            base_version,
            handle,
        });
        let apply_time = apply_start.elapsed();
        tracing::trace!(
            finalized_block_hash = %block_header.hash(),
            ?preparation_time,
            ?apply_time,
            "Finalization complete");

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(StorageManagerFinalizationMetric {
                wait_for_previous_commit_time,
                preparation_time,
                commit_time: apply_time,
                pruning_commit_time,
            });
        });
        Ok(())
    }
}

impl<Da, H, S> Drop for NomtStorageManager<Da, H, S>
where
    Da: DaSpec,
    S: InitializableNativeNomtStorage<H, Da::SlotHash>,
{
    fn drop(&mut self) {
        let Some(committing) = self.committing.take() else {
            return;
        };
        // The panic message has already been printed by the commit thread's panic hook.
        if committing.handle.join().is_err() {
            tracing::error!("Background storage commit has panicked during shutdown");
        }
    }
}
