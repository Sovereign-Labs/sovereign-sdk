use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

use anyhow::Context;
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::versioned_db::{VersionedDeltaReader, VersionedTableMetadataKey};
use rockbound::SchemaBatch;
use sov_rollup_interface::reexports::digest;

use crate::accessory_db::AccessoryDb;
use crate::config::RollupDbConfig;
use crate::flat_db::FlatStateDb;
use crate::historical_state::{HistoricalStateReader, StateChanges};
use crate::ledger_db::LedgerDb;
use crate::metrics::nomt::PrunerMetric;
use crate::namespaces::{KernelNamespace, UserNamespace};
use crate::pruner::Pruner;
use crate::schema::namespace::{
    NomtCommittedVersion, NomtHistoricalState, NomtPruningState, NomtStateValues,
};
use crate::schema::tables::ModuleAccessoryState;
use crate::state_db_nomt::{NomtSessionBuilder, NomtStateDb, StateOverlay};
use crate::storage_manager::{update_ledger_finalized_height, InitializableNativeNomtStorage};

pub(crate) struct DbGroup<H, K> {
    merklized_state: Arc<NomtStateDb<H>>,
    flat_state: FlatStateDb,
    accessory: Arc<rockbound::DB>,
    ledger: Arc<rockbound::DB>,
    phantom_ref: PhantomData<K>,
}

impl<H, K> DbGroup<H, K>
where
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    K: Eq + std::hash::Hash + Clone + std::fmt::Debug,
{
    pub(crate) fn new(config: RollupDbConfig) -> anyhow::Result<Self> {
        let path = config.path.clone();
        let state_db = NomtStateDb::<H>::new(config)?;
        let accessory_rocksdb =
            AccessoryDb::get_rockbound_options().default_setup_db_in_path(&path)?;
        let ledger_rocksdb = LedgerDb::get_rockbound_options().default_setup_db_in_path(&path)?;
        let flat_state = FlatStateDb::new(path)?;
        Ok(Self {
            merklized_state: Arc::new(state_db),
            flat_state,
            accessory: Arc::new(accessory_rocksdb),
            ledger: Arc::new(ledger_rocksdb),
            phantom_ref: Default::default(),
        })
    }

    pub(crate) fn commit(&mut self, group: CommitGroup) -> anyhow::Result<()> {
        let CommitGroup {
            nomt: state,
            rockbound:
                SnapshotGroup {
                    historical_state,
                    accessory,
                    ledger,
                },
        } = group;

        // Note: failure handling and data recovery will be implemented later.
        self.merklized_state.commit(state)?;
        // Historical data is committed after merklized state, as in case of failure, it can be synced from the normal state,
        // as it duplicates the last written data to `self.state`.
        self.flat_state.commit(historical_state)?;
        self.accessory
            .write_schemas(Arc::unwrap_or_clone(accessory))?;
        // Ledger goes after last, as its data is used during the start.
        // So if ledger save failed, state and accessory will be synced from DA
        self.ledger.write_schemas(Arc::unwrap_or_clone(ledger))?;

        self.merklized_state.send_metrics();

        Ok(())
    }

    // Flush pruning schema batches to disk.
    pub(crate) fn commit_pruning(&mut self, group: PruneGroup) -> anyhow::Result<()> {
        self.flat_state
            .other
            .write_schemas(group.historical_state)?;
        self.accessory.write_schemas(group.accessory)?;
        Ok(())
    }

    pub(crate) fn create_storage<S: InitializableNativeNomtStorage<H, K>>(
        &self,
        // Snapshot refs are in reveresed chronological order.
        relevant_snapshot_refs: Vec<K>,
        rockbound_snapshots: &HashMap<K, SnapshotGroup>,
        nomt_snapshots: Arc<RwLock<HashMap<K, StateOverlay>>>,
        use_strict_mode: bool,
    ) -> anyhow::Result<(S, DeltaReader)> {
        let mut historical_state_snapshots = Vec::with_capacity(relevant_snapshot_refs.len());
        let mut user_state_snapshots = Vec::with_capacity(relevant_snapshot_refs.len());
        let mut kernel_state_snapshots = Vec::with_capacity(relevant_snapshot_refs.len());
        let mut accessory_snapshots = Vec::with_capacity(relevant_snapshot_refs.len());
        let mut ledger_snapshots = Vec::with_capacity(relevant_snapshot_refs.len());

        // rockbound-based readers expect snapshots in chronological order,
        // so we iterate in reverse of the passed parameter
        // (in normal chronological order).
        for snapshot_ref in relevant_snapshot_refs.iter().rev() {
            let snapshot = rockbound_snapshots.get(snapshot_ref).unwrap();
            historical_state_snapshots.push(snapshot.historical_state.other.clone());
            user_state_snapshots.push(snapshot.historical_state.user.clone());
            kernel_state_snapshots.push(snapshot.historical_state.kernel.clone());
            accessory_snapshots.push(snapshot.accessory.clone());
            ledger_snapshots.push(snapshot.ledger.clone());
        }

        // NOMT-based readers expect snapshots in reversed chronological order,
        // the same as it was passed to the function.
        let state_session_builder = NomtSessionBuilder::new(
            self.merklized_state.clone(),
            relevant_snapshot_refs,
            nomt_snapshots,
        );
        let historical_state_reader =
            DeltaReader::new(self.flat_state.other.clone(), historical_state_snapshots);
        let version = self.flat_state.get_kernel_db().get_committed_version()?;

        let user_state_reader = VersionedDeltaReader::<NomtStateValues<UserNamespace>>::new(
            self.flat_state.user.clone(),
            version,
            user_state_snapshots,
        );
        let kernel_state_reader = VersionedDeltaReader::<NomtStateValues<KernelNamespace>>::new(
            self.flat_state.kernel.clone(),
            version,
            kernel_state_snapshots,
        );
        let historical_state_mapper = HistoricalStateReader::new(
            user_state_reader,
            kernel_state_reader,
            historical_state_reader,
        );

        let accessory_reader = DeltaReader::new(self.accessory.clone(), accessory_snapshots);
        let accessory_db = AccessoryDb::with_reader(accessory_reader)?;
        let ledger_reader = DeltaReader::new(self.ledger.clone(), ledger_snapshots);

        let storage = S::new(
            state_session_builder,
            historical_state_mapper,
            accessory_db,
            use_strict_mode,
        );
        Ok((storage, ledger_reader))
    }

    pub(crate) fn update_ledger_finalized_height(&self) -> anyhow::Result<()> {
        update_ledger_finalized_height(self.ledger.clone())
    }

    pub(crate) fn start_pruner(&self, versions_to_keep: usize) -> PrunerJob {
        tracing::info!(versions_to_keep, "Starting pruner task");
        let user = self.flat_state.get_user_db().clone();
        let kernel = self.flat_state.get_kernel_db().clone();
        let accessory_pruner = Pruner::new(self.accessory.clone());

        // Spawn historical state pruner thread
        let historical_state = std::thread::spawn(move || -> anyhow::Result<SchemaBatch> {
            let pruning_time = std::time::Instant::now();
            let current_user_version = user.get_committed_version()?;
            let current_kernel_version = kernel.get_committed_version()?;

            let mut batch = SchemaBatch::new();
            let mut keys_to_prune = 0;
            let mut keys_inspected = 0;

            if let Some(user_version) =
                current_user_version.and_then(|v| v.checked_sub(versions_to_keep as u64))
            {
                let prunable_keys = user.iter_pruning_keys_up_to_version(user_version)?;
                for key in prunable_keys {
                    // Prune the pruning table.
                    let key = key?;
                    batch.delete::<NomtPruningState<UserNamespace>>(&key)?;
                    keys_to_prune += 1;
                    keys_inspected += 1;
                    // Prune the historical state table. This is the main table that we want to prune.
                    // We want to make sure that the the newest version of the key is accessible. The pruning table
                    // records that we wrote key K at time T, so delete key K at time T-1. Recursively, this will ensure
                    // that no keys are pruned that are still live, and all old keys are pruned as soon as possible.
                    let mut key = key.into_versioned_key();
                    let previous_version = key.1.saturating_sub(1);
                    let prev_written_version =
                        user.get_version_for_key(&key.0, previous_version)?;
                    keys_inspected += 1;
                    if let Some(previous_version) = prev_written_version {
                        key.1 = previous_version;
                        batch.delete::<NomtHistoricalState<UserNamespace>>(&key)?;
                        keys_to_prune += 1;
                    }
                }
                batch.put::<NomtCommittedVersion<UserNamespace>>(
                    &VersionedTableMetadataKey::PrunedVersion,
                    &user_version,
                )?;
            }
            if let Some(kernel_version) =
                current_kernel_version.and_then(|v| v.checked_sub(versions_to_keep as u64))
            {
                let prunable_keys = kernel.iter_pruning_keys_up_to_version(kernel_version)?;
                for key in prunable_keys {
                    // Prune the pruning table.
                    let key = key?;
                    batch.delete::<NomtPruningState<KernelNamespace>>(&key)?;
                    keys_to_prune += 1;
                    keys_inspected += 1;
                    // Prune the historical state table.
                    let mut key = key.into_versioned_key();
                    let previous_version = key.1.saturating_sub(1);
                    let prev_written_version =
                        kernel.get_version_for_key(&key.0, previous_version)?;
                    keys_inspected += 1;
                    if let Some(previous_version) = prev_written_version {
                        key.1 = previous_version;
                        batch.delete::<NomtHistoricalState<KernelNamespace>>(&key)?;
                        keys_to_prune += 1;
                    }
                }
                batch.put::<NomtCommittedVersion<KernelNamespace>>(
                    &VersionedTableMetadataKey::PrunedVersion,
                    &kernel_version,
                )?;
            }

            let pruning_time = pruning_time.elapsed();
            sov_metrics::track_metrics(|tracker| {
                tracker.submit(PrunerMetric {
                    db: "versioned_dbs",
                    keys_inspected,
                    keys_to_prune,
                    time: pruning_time,
                });
            });
            Ok(batch)
        });

        // Spawn accessory pruner thread
        let accessory_state = std::thread::spawn(move || -> anyhow::Result<SchemaBatch> {
            accessory_pruner
                .collect_pruning_batch::<ModuleAccessoryState>(versions_to_keep as u64)?;
            Ok(SchemaBatch::new())
        });

        PrunerJob {
            historical_state,
            accessory_state,
        }
    }

    fn are_root_hashes_match(&self) -> anyhow::Result<bool> {
        let historical_state_delta_reader =
            DeltaReader::new(self.flat_state.other.clone(), Vec::new());

        let nomt_root_hashes = self.merklized_state.get_root_hashes();
        let last_version =
            HistoricalStateReader::last_version_from_reader(&historical_state_delta_reader)?;

        match last_version {
            None => {
                let is_kernel_empty = nomt_root_hashes.kernel.is_empty();
                let is_user_empty = nomt_root_hashes.user.is_empty();
                tracing::trace!(
                    ?is_kernel_empty,
                    ?is_user_empty,
                    "Historical root hash is empty, user and kernel must be too"
                );
                Ok(is_kernel_empty && is_user_empty)
            }
            Some(latest_version) => {
                let Some(state_root_rocksdb) =
                    HistoricalStateReader::get_serialized_root_hash_from_reader(
                        &historical_state_delta_reader,
                        latest_version,
                    )?
                else {
                    anyhow::bail!(
                        "Missing root hash for the latest version {}",
                        latest_version
                    );
                };
                tracing::trace!(
                    histrocial_root_hash = %hex::encode(&state_root_rocksdb),
                    nomt_state_roots = ?nomt_root_hashes,
                    %latest_version,
                    "Historical root hash is not empty");
                Ok(nomt_root_hashes.included_in_raw(&state_root_rocksdb))
            }
        }
    }

    pub(crate) fn verify_and_fix_commited_root_hashes(&self) -> anyhow::Result<()> {
        if !self.are_root_hashes_match()? {
            tracing::warn!("Historical state root hashes are not equal to NOMT state root hashes, attempt to fix it");
            self.merklized_state.full_rollback()?;
            if !self.are_root_hashes_match()? {
                return Err(anyhow::anyhow!("Fix didn't help, historical state root hashes are not equal to NOMT state root hashes. Manual intervention is required."));
            }
            tracing::info!(
                "Historical state root hashes are equal to NOMT state root hashes, fix applied"
            );
        }
        Ok(())
    }
}

pub(crate) struct SnapshotGroup {
    pub(crate) historical_state: StateChanges,
    pub(crate) accessory: Arc<SchemaBatch>,
    pub(crate) ledger: Arc<SchemaBatch>,
}

pub(crate) struct PruneGroup {
    historical_state: SchemaBatch,
    accessory: SchemaBatch,
}

pub(crate) struct CommitGroup {
    // State
    pub(crate) nomt: StateOverlay,
    // The rest.
    pub(crate) rockbound: SnapshotGroup,
}

// Collection of 2 handles to pruner threads for each database.
pub(crate) struct PrunerJob {
    historical_state: JoinHandle<anyhow::Result<SchemaBatch>>,
    accessory_state: JoinHandle<anyhow::Result<SchemaBatch>>,
}

impl PrunerJob {
    pub(crate) fn is_finished(&self) -> bool {
        self.historical_state.is_finished() && self.accessory_state.is_finished()
    }

    pub(crate) fn join(self) -> anyhow::Result<PruneGroup> {
        let historical_state = self
            .historical_state
            .join()
            .map_err(|e| anyhow::anyhow!("Historical state pruner panicked: {:?}", e))?;
        let accessory_state = self
            .accessory_state
            .join()
            .map_err(|e| anyhow::anyhow!("Accessory state pruner panicked: {:?}", e))?;
        tracing::info!("Pruner task has completed");
        Ok(PruneGroup {
            historical_state: historical_state.context("historical state")?,
            accessory: accessory_state.context("accessory state")?,
        })
    }
}
