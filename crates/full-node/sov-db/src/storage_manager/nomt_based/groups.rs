use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

use anyhow::Context;
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_rollup_interface::reexports::digest;

use crate::accessory_db::AccessoryDb;
use crate::config::RollupDbConfig;
use crate::historical_state::HistoricalStateReader;
use crate::ledger_db::LedgerDb;
use crate::namespaces::{KernelNamespace, UserNamespace};
use crate::pruner::Pruner;
use crate::schema::namespace::StateValues;
use crate::schema::tables::ModuleAccessoryState;
use crate::state_db_nomt::{NomtSessionBuilder, NomtStateDb, StateOverlay};
use crate::storage_manager::{update_ledger_finalized_height, InitializableNativeNomtStorage};

pub(crate) struct DbGroup<H, K> {
    state: Arc<NomtStateDb<H>>,
    historical_state: Arc<rockbound::DB>,
    accessory: Arc<rockbound::DB>,
    ledger: Arc<rockbound::DB>,
    phantom_ref: PhantomData<K>,
}

impl<H, K> DbGroup<H, K>
where
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    K: Eq + std::hash::Hash + Clone,
{
    pub(crate) fn new(config: RollupDbConfig) -> anyhow::Result<Self> {
        let path = config.path.clone();
        let state_db = NomtStateDb::<H>::new(config)?;
        let historical_state =
            HistoricalStateReader::get_rockbound_options().default_setup_db_in_path(&path)?;
        let accessory_rocksdb =
            AccessoryDb::get_rockbound_options().default_setup_db_in_path(&path)?;
        let ledger_rocksdb = LedgerDb::get_rockbound_options().default_setup_db_in_path(&path)?;
        Ok(Self {
            state: Arc::new(state_db),
            historical_state: Arc::new(historical_state),
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
        self.state.commit(state)?;
        self.accessory.write_schemas(&accessory)?;
        // Ledger goes last, as its data is used during the start.
        // So if ledger save failed, state and accessory will be synced from DA
        self.ledger.write_schemas(&ledger)?;
        // Historical data is committed the last, as in case of failure, it can be synced from the normal state,
        // as it duplicates the last written data to `self.state`.
        self.historical_state.write_schemas(&historical_state)?;

        self.state.send_metrics();

        Ok(())
    }

    // Flush pruning schema batches to disk.
    pub(crate) fn commit_pruning(&mut self, group: PruneGroup) -> anyhow::Result<()> {
        self.historical_state
            .write_schemas(&group.historical_state)?;
        self.accessory.write_schemas(&group.accessory)?;
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
        let mut accessory_snapshots = Vec::with_capacity(relevant_snapshot_refs.len());
        let mut ledger_snapshots = Vec::with_capacity(relevant_snapshot_refs.len());

        // rockbound-based readers expect snapshots in chronological order,
        // so we iterate in reverse of the passed parameter
        // (in normal chronological order).
        for snapshot_ref in relevant_snapshot_refs.iter().rev() {
            let snapshot = rockbound_snapshots.get(snapshot_ref).unwrap();
            historical_state_snapshots.push(snapshot.historical_state.clone());
            accessory_snapshots.push(snapshot.accessory.clone());
            ledger_snapshots.push(snapshot.ledger.clone());
        }

        // NOMT-based readers expect snapshots in reversed chronological order,
        // the same as it was passed to the function.
        let state_session_builder =
            NomtSessionBuilder::new(self.state.clone(), relevant_snapshot_refs, nomt_snapshots);
        let historical_state_reader =
            DeltaReader::new(self.historical_state.clone(), historical_state_snapshots);
        let historical_state_mapper =
            HistoricalStateReader::with_delta_reader(historical_state_reader)?;

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
        let state_pruner = Pruner::new(self.historical_state.clone());
        let accessory_pruner = Pruner::new(self.accessory.clone());

        // Spawn historical state pruner thread
        let historical_state = std::thread::spawn(move || -> anyhow::Result<SchemaBatch> {
            let mut kernel_prune_batch = state_pruner
                .collect_pruning_batch::<StateValues<KernelNamespace>>(versions_to_keep as u64)?;
            let user_prune_batch = state_pruner
                .collect_pruning_batch::<StateValues<UserNamespace>>(versions_to_keep as u64)?;
            kernel_prune_batch.merge(user_prune_batch);
            Ok(kernel_prune_batch)
        });

        // Spawn accessory pruner thread
        let accessory_state = std::thread::spawn(move || -> anyhow::Result<SchemaBatch> {
            accessory_pruner.collect_pruning_batch::<ModuleAccessoryState>(versions_to_keep as u64)
        });

        PrunerJob {
            historical_state,
            accessory_state,
        }
    }

    fn are_root_hashes_match(&self) -> anyhow::Result<bool> {
        let historical_state_delta_reader =
            DeltaReader::new(self.historical_state.clone(), Vec::new());
        let historical_state_reader =
            HistoricalStateReader::with_delta_reader(historical_state_delta_reader)?;

        let nomt_root_hashes = self.state.get_root_hashes();

        match historical_state_reader.last_version() {
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
                    historical_state_reader.get_serialized_root_hash(latest_version)?
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
            self.state.full_rollback()?;
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
    pub(crate) historical_state: Arc<SchemaBatch>,
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
