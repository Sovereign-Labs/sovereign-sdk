use crate::flat_db::DbCache;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Context;
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::versioned_db::VersionedDeltaReader;
use rockbound::SchemaBatch;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::reexports::digest;

use crate::accessory_db::AccessoryDb;
use crate::config::{RocksDbKind, RollupDbConfigWithCustomizations};
use crate::flat_db::FlatStateDb;
use crate::historical_state::{HistoricalStateReader, StateChanges};
use crate::ledger_db::LedgerDb;
use crate::metrics::nomt::CommitDetailedMetric;
use crate::namespaces::{KernelNamespace, UserNamespace};
use crate::pruner::Pruner;
use crate::schema::namespace::NomtStateValues;
use crate::schema::tables::ModuleAccessoryState;
use crate::state_db_nomt::{NomtSessionBuilder, NomtStateDb, StateOverlay, StateRootHashes};
use crate::storage_manager::{
    update_ledger_finalized_height, InitializableNativeNomtStorage, WitnessMode,
};

const GIGABYTE: usize = 1024 * 1024 * 1024;

// 300 thousand keys * 32 bytes is about 10 MB. This should be a large enough batch size to keep up with state growth,
// without consuming excessive memory.
pub(crate) const DEFAULT_MAX_PRUNING_BATCH_SIZE: usize = 300_000;

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
    pub(crate) fn new(custom_config: RollupDbConfigWithCustomizations) -> anyhow::Result<Self> {
        let config = custom_config.config().clone();
        let path = config.path.clone();
        let ledger_db_path = config.ledger_db_path.clone();
        let state_cache_size = config.state_cache_size.unwrap_or(GIGABYTE);

        let merklized_state = Arc::new(NomtStateDb::<H>::new(config.clone())?);
        let flat_state = FlatStateDb::new_with_customizations(
            path.clone(),
            state_cache_size,
            Some(&custom_config),
        )?;
        let ledger_db_options = custom_config.get_rocksdb_options(RocksDbKind::Ledger);
        let ledger = Arc::new(if let Some(ledger_db_path) = ledger_db_path {
            LedgerDb::get_rockbound_options().setup_db_with_options_and_cfs(
                ledger_db_path,
                &ledger_db_options,
                |cf_name, builder| {
                    custom_config.customize_rocksdb_cf(RocksDbKind::Ledger, cf_name, None, builder);
                },
            )?
        } else {
            LedgerDb::get_rockbound_options().setup_db_as_subdir_with_options_and_cfs(
                &path,
                &ledger_db_options,
                |cf_name, builder| {
                    custom_config.customize_rocksdb_cf(RocksDbKind::Ledger, cf_name, None, builder);
                },
            )?
        });

        let accessory_db_options = custom_config.get_rocksdb_options(RocksDbKind::Accessory);
        let accessory = Arc::new(
            AccessoryDb::get_rockbound_options().setup_db_as_subdir_with_options_and_cfs(
                &path,
                &accessory_db_options,
                |cf_name, builder| {
                    custom_config.customize_rocksdb_cf(
                        RocksDbKind::Accessory,
                        cf_name,
                        None,
                        builder,
                    );
                },
            )?,
        );

        // Validate the commit state.
        Self::validate_commit_flag_and_rollback_if_necessary(
            &merklized_state,
            ledger.clone(),
            accessory.clone(),
            &flat_state,
        )?;

        Ok(Self {
            merklized_state,
            flat_state,
            accessory,
            ledger,
            phantom_ref: Default::default(),
        })
    }

    pub(crate) fn commit(&mut self, group: CommitGroup) -> anyhow::Result<()> {
        self.commit_helper(group, None)
    }

    fn commit_helper(
        &mut self,
        group: CommitGroup,
        expected_latest: Option<SlotNumber>,
    ) -> anyhow::Result<()> {
        tracing::trace!("Commiting a group...");
        // The last commit had to be successful.
        let CommitGroup {
            nomt: state,
            rockbound:
                SnapshotGroup {
                    historical_state,
                    accessory,
                    ledger,
                },
        } = group;

        // ======================= DANGER ZONE ======================
        // The commit order here is relied on by NomtProverStorage::get_with_proof
        // If you change the order, you'll need to update merkle proof generation.

        // NOMT
        tracing::trace!("Commiting NOMT DBs...");
        let merklized_commit = self.merklized_state.commit(state)?;

        // Ledger
        tracing::trace!("Committing Ledger DB...");
        #[cfg(feature = "test-utils")]
        crate::test_utils::CommitFaultInjectionLocation::BeforeCommittingLedger
            .inject_fault_if_configured();
        let ledger_commit = self.commit_ledger(&ledger)?;

        // Accessory
        tracing::trace!("Commiting Accessory DB...");
        #[cfg(feature = "test-utils")]
        crate::test_utils::CommitFaultInjectionLocation::BeforeCommittingAccessory
            .inject_fault_if_configured();
        let accessory_commit =
            self.commit_accessory(&accessory, &historical_state.root_hash_batch)?;

        // Flat State
        tracing::trace!("Committing Flat DB..");
        #[cfg(feature = "migration-script")]
        let flat_metrics = if let Some(expected_latest) = expected_latest {
            self.flat_state
                .commit_at_latest_checked(historical_state, expected_latest)?
        } else {
            self.flat_state.commit(historical_state)?
        };

        #[cfg(not(feature = "migration-script"))]
        let flat_metrics = {
            assert!(
                expected_latest.is_none(),
                "expected_latest must be none when not in migration script"
            );
            self.flat_state.commit(historical_state)?
        };

        // ======================= END DANGER ZONE ======================

        // Metrics
        let merklized_commit_from_caller = merklized_commit.total;
        let commit_detailed_metrics = CommitDetailedMetric {
            merklized_commit,
            merklized_commit_from_caller,
            flat: flat_metrics,
            accessory_commit,
            ledger_commit,
        };
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(commit_detailed_metrics);
        });
        self.merklized_state.send_metrics();

        Ok(())
    }

    #[cfg(feature = "migration-script")]
    pub(crate) fn commit_at_latest_checked(
        &mut self,
        group: CommitGroup,
        expected_latest: SlotNumber,
    ) -> anyhow::Result<()> {
        self.commit_helper(group, Some(expected_latest))
    }

    #[cfg(feature = "migration-script")]
    pub(crate) fn latest_flat_state_version(&self) -> anyhow::Result<Option<SlotNumber>> {
        Ok(self
            .flat_state
            .latest_version_and_root_hash_live_db()?
            .map(|(v, _)| SlotNumber::new(v)))
    }

    fn validate_commit_flag_and_rollback_if_necessary(
        merklized_state: &NomtStateDb<H>,
        ledger_db: Arc<rockbound::DB>,
        accessory_db: Arc<rockbound::DB>,
        flat_state_db: &FlatStateDb,
    ) -> anyhow::Result<()> {
        let state_roots = AllDBsStateRoots::from_dbs(
            merklized_state,
            ledger_db.clone(),
            accessory_db.clone(),
            flat_state_db,
        )?;

        state_roots.info("before validation");

        if state_roots.is_kernel_nomt_root_newer() {
            merklized_state.kernel.rollback(1)?;
        }

        if state_roots.is_user_nomt_root_newer() {
            merklized_state.user.rollback(1)?;
        }

        if state_roots.is_ledger_db_root_newer() {
            LedgerDb::rollback_head_slot(ledger_db.clone())?;
        }

        if state_roots.is_accessory_db_root_newer() {
            AccessoryDb::rollback(accessory_db.clone())?;
        }

        if state_roots.is_archival_db_root_newer() {
            flat_state_db.validate_and_rollback_archival()?;
        }

        let state_roots =
            AllDBsStateRoots::from_dbs(merklized_state, ledger_db, accessory_db, flat_state_db)?;
        state_roots.info("after validation");

        state_roots.check_all();

        Ok(())
    }

    fn commit_accessory(
        &self,
        accessory: &SchemaBatch,
        root_hash_batch: &SchemaBatch,
    ) -> anyhow::Result<Duration> {
        let accessory_start = std::time::Instant::now();
        AccessoryDb::commit(&self.accessory, accessory, root_hash_batch)?;
        Ok(accessory_start.elapsed())
    }

    fn commit_ledger(&self, ledger: &SchemaBatch) -> anyhow::Result<Duration> {
        let ledger_start = std::time::Instant::now();
        // Ledger goes after last, as its data is used during the start.
        // So if ledger save failed, state and accessory will be synced from DA
        self.ledger.write_schemas(ledger)?;
        Ok(ledger_start.elapsed())
    }

    // Flush pruning schema batches to disk.
    pub(crate) fn commit_pruning(&mut self, group: PruneGroup) -> anyhow::Result<()> {
        self.flat_state
            .archival_db
            .write_schemas(&group.historical_state.pruning_batch)?;
        self.accessory
            .write_schemas(&group.accessory.pruning_batch)?;
        Ok(())
    }

    /// Runs a full RocksDB compaction on the column families that pruning deletes from, to
    /// drop the resulting tombstones and reclaim disk space. Intended only for the one-time
    /// startup prune (`PrunerConfig::OnceAtStartup` with `compact_after`), where rewriting
    /// the affected column families is paid up front with no live read/write traffic.
    ///
    /// Compacts all three pruned regions: the accessory state column family (where the
    /// accessory pruner's `delete_range` tombstones live) and the user and kernel archival
    /// historical + pruning column families (where rockbound's `VersionedDB` writes its
    /// pruning tombstones — point deletes for the scattered historical rows, a range delete
    /// for the version-prefixed pruning CF). `VersionedDB::trigger_compaction` compacts the
    /// user/kernel pair.
    pub(crate) fn compact_pruned_cfs(&self) -> anyhow::Result<()> {
        tracing::info!("Compacting pruned column families to reclaim disk space");
        self.accessory
            .trigger_compaction::<ModuleAccessoryState>()?;
        self.flat_state.user.trigger_compaction()?;
        self.flat_state.kernel.trigger_compaction()?;
        Ok(())
    }

    pub(crate) fn create_storage<S: InitializableNativeNomtStorage<H, K>>(
        &self,
        // Snapshot refs are in reveresed chronological order.
        relevant_snapshot_refs: Vec<K>,
        rockbound_snapshots: &HashMap<K, SnapshotGroup>,
        nomt_snapshots: Arc<RwLock<HashMap<K, StateOverlay>>>,
        strict_mode: bool,
        witness_mode: WitnessMode,
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
            historical_state_snapshots.push(snapshot.historical_state.root_hash_batch.clone());
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
            DeltaReader::new(self.flat_state.live_db.clone(), historical_state_snapshots);
        let version = self
            .flat_state
            .latest_version_and_root_hash_live_db()?
            .map(|(v, _)| v);

        let user_state_reader =
            VersionedDeltaReader::<NomtStateValues<UserNamespace>, DbCache>::new(
                self.flat_state.user.clone(),
                version,
                user_state_snapshots,
            );
        let kernel_state_reader =
            VersionedDeltaReader::<NomtStateValues<KernelNamespace>, DbCache>::new(
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
            strict_mode,
            witness_mode,
        );
        Ok((storage, ledger_reader))
    }

    pub(crate) fn update_ledger_finalized_height(&self) -> anyhow::Result<()> {
        update_ledger_finalized_height(self.ledger.clone())
    }

    pub(crate) fn start_pruner(&self, versions_to_keep: usize, max_batch_size: usize) -> PrunerJob {
        tracing::info!(versions_to_keep, "Starting pruner task iteration");
        // User and kernel state are versioned by rockbound's `VersionedDB`, which owns
        // the historical / pruning / metadata column families. We ask it for a multi-CF
        // delete batch and write it into the archival DB at commit time.
        let user_db = self.flat_state.user.clone();
        let kernel_db = self.flat_state.kernel.clone();
        let accessory_pruner = Pruner::new(self.accessory.clone(), Some(max_batch_size));

        // Spawn historical-state pruner thread (user + kernel, sequential).
        let historical_state: JoinHandle<Result<PrunerJobOutput, anyhow::Error>> =
            std::thread::spawn(move || -> anyhow::Result<PrunerJobOutput> {
                let keep = versions_to_keep as u64;
                let user_output = user_db.collect_pruning_batch(keep, Some(max_batch_size))?;
                let remaining = max_batch_size.saturating_sub(user_output.keys_to_prune);

                let mut pruning_batch = user_output.batch;
                let mut hit_size_limit = user_output.hit_size_limit;
                if remaining > 0 {
                    let kernel_output = kernel_db.collect_pruning_batch(keep, Some(remaining))?;
                    pruning_batch.merge(kernel_output.batch);
                    hit_size_limit |= kernel_output.hit_size_limit;
                }

                Ok(PrunerJobOutput {
                    pruning_batch,
                    hit_size_limit,
                })
            });

        // Spawn accessory pruner thread.
        let accessory_state = std::thread::spawn(move || -> anyhow::Result<PrunerJobOutput> {
            accessory_pruner.collect_pruning_batch::<ModuleAccessoryState>(versions_to_keep as u64)
        });

        PrunerJob {
            historical_state,
            accessory_state,
        }
    }
}
pub(crate) struct SnapshotGroup {
    pub(crate) historical_state: StateChanges,
    pub(crate) accessory: Arc<SchemaBatch>,
    pub(crate) ledger: Arc<SchemaBatch>,
}

pub(crate) struct PruneGroup {
    historical_state: PrunerJobOutput,
    accessory: PrunerJobOutput,
}

impl PruneGroup {
    pub(crate) fn hit_size_limit(&self) -> bool {
        self.historical_state.hit_size_limit || self.accessory.hit_size_limit
    }
}

pub(crate) struct CommitGroup {
    // State
    pub(crate) nomt: StateOverlay,
    // The rest.
    pub(crate) rockbound: SnapshotGroup,
}

// Collection of 2 handles to pruner threads for each database.
pub(crate) struct PrunerJob {
    historical_state: JoinHandle<anyhow::Result<PrunerJobOutput>>,
    accessory_state: JoinHandle<anyhow::Result<PrunerJobOutput>>,
}

/// The output of a pruner job
pub struct PrunerJobOutput {
    /// The batch of keys to delete from the database.
    pub pruning_batch: SchemaBatch,
    /// Whether the pruner hit the batch size limit. If this is true, the pruner should be spawned again when possible to continue its work.
    pub hit_size_limit: bool,
}

impl PrunerJob {
    pub(crate) fn is_finished(&self) -> bool {
        self.historical_state.is_finished() && self.accessory_state.is_finished()
    }

    pub(crate) fn join(self) -> anyhow::Result<PruneGroup> {
        let historical_state = self
            .historical_state
            .join()
            .map_err(|e| anyhow::anyhow!("Historical state pruner panicked: {:?}", e))?
            .context("historical state")?;
        let accessory_state = self
            .accessory_state
            .join()
            .map_err(|e| anyhow::anyhow!("Accessory state pruner panicked: {:?}", e))?
            .context("accessory state")?;
        tracing::info!(%historical_state.hit_size_limit, %accessory_state.hit_size_limit, "Pruner task has completed");
        Ok(PruneGroup {
            historical_state,
            accessory: accessory_state,
        })
    }
}

// Root hash for empty nomt state.
fn pre_genesis_root() -> [u8; 64] {
    let mut pre_genesis_root = [0u8; 64];
    pre_genesis_root[..32].copy_from_slice(&nomt::trie::TERMINATOR);
    pre_genesis_root[32..].copy_from_slice(&nomt::trie::TERMINATOR);
    pre_genesis_root
}

struct AllDBsStateRoots {
    // The `live_db` is committed last. We can use `root_hash_from_live_db` to verify
    // whether all other databases were committed in the previous run.
    root_hash_from_live_db: [u8; 64],
    root_hash_from_archival_db: [u8; 64],
    root_hash_from_accessory_db: [u8; 64],
    root_hash_from_ledger_db: [u8; 64],
    root_hash_nomt: StateRootHashes,
}

impl AllDBsStateRoots {
    fn from_dbs<H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync>(
        merklized_state: &NomtStateDb<H>,
        ledger_db: Arc<rockbound::DB>,
        accessory_db: Arc<rockbound::DB>,
        flat_state_db: &FlatStateDb,
    ) -> anyhow::Result<AllDBsStateRoots> {
        let root_hash_nomt = merklized_state.get_root_hashes();

        let root_hash_from_live_db = match flat_state_db.root_hash_from_live_db()? {
            Some(root_hash_from_live_db) => root_hash_from_live_db,
            None => {
                // The merklized_state is committed first:
                // root_hash_from_live_db == None and root_hash_nomt is empty. This indicates that the rollup is being run for the first time.
                if root_hash_nomt.is_empty() {
                    pre_genesis_root()
                } else {
                    // root_hash_nomt is not empty but root_hash_from_live_db is empty.
                    // This means that the rollup ran before for the first time but crashed before saving the live DB.
                    //
                    // In this case, we should manually remove all databases and start again.
                    // This happens only in the following scenario:
                    // 1. The rollup started from genesis.
                    // 2. It crashed before finishing the first commit, and the live DB was not saved.
                    //
                    // In this case, it is safe to delete all the databases.
                    tracing::error!(
                        "Rollup instantiation error: Delete the rollup databases and start again."
                    );
                    anyhow::bail!(
                        "Live db not found. Delete the rollup databases and start again."
                    );
                }
            }
        };

        let root_hash_from_archival_db = flat_state_db
            .root_hash_from_archival_db()?
            .unwrap_or_else(pre_genesis_root);

        let root_hash_from_ledger_db =
            LedgerDb::get_head_root_hash(ledger_db.clone())?.unwrap_or_else(pre_genesis_root);

        let root_hash_from_accessory_db =
            AccessoryDb::latest_version_and_root_hash_archival_db(accessory_db)?
                .map(|(_, r)| r)
                .unwrap_or_else(pre_genesis_root);

        Ok(AllDBsStateRoots {
            root_hash_from_live_db,
            root_hash_from_archival_db,
            root_hash_from_accessory_db,
            root_hash_from_ledger_db,
            root_hash_nomt,
        })
    }

    fn is_kernel_nomt_root_newer(&self) -> bool {
        self.root_hash_nomt.kernel != self.root_hash_from_live_db[32..]
    }

    fn is_user_nomt_root_newer(&self) -> bool {
        self.root_hash_nomt.user != self.root_hash_from_live_db[0..32]
    }

    fn is_ledger_db_root_newer(&self) -> bool {
        self.root_hash_from_ledger_db != self.root_hash_from_live_db
    }

    fn is_accessory_db_root_newer(&self) -> bool {
        self.root_hash_from_accessory_db != self.root_hash_from_live_db
    }

    fn is_archival_db_root_newer(&self) -> bool {
        self.root_hash_from_archival_db != self.root_hash_from_live_db
    }

    fn check_all(&self) {
        Self::check_hashes(
            &self.root_hash_from_archival_db,
            "root_hash_from_archival_db",
            &self.root_hash_from_live_db,
        );

        Self::check_hashes(
            &self.root_hash_from_accessory_db,
            "root_hash_from_accessory_db",
            &self.root_hash_from_live_db,
        );

        Self::check_hashes(
            &self.root_hash_from_ledger_db,
            "root_hash_from_ledger_db",
            &self.root_hash_from_live_db,
        );

        Self::check_hashes(
            &self.root_hash_nomt.user,
            "self.root_hash_nomt.user",
            &self.root_hash_from_live_db[0..32],
        );

        Self::check_hashes(
            &self.root_hash_nomt.kernel,
            "self.root_hash_nomt.kernel",
            &self.root_hash_from_live_db[32..],
        );
    }

    fn check_hashes(root_hash: &[u8], root_hash_name: &str, root_hash_from_live_db: &[u8]) {
        let root_hash = hex::encode(root_hash);
        let root_hash_from_live_db = hex::encode(root_hash_from_live_db);

        if root_hash != root_hash_from_live_db {
            panic!("{root_hash_name}: {root_hash} does not match root_hash_from_live_db: {root_hash_from_live_db}");
        }
    }

    fn info(&self, msg: &str) {
        tracing::info!(
            root_hash_from_live_db = hex::encode(self.root_hash_from_live_db),
            root_hash_from_archival_db = hex::encode(self.root_hash_from_archival_db),
            root_hash_from_ledger_db = hex::encode(self.root_hash_from_ledger_db),
            root_hash_nomt_user = hex::encode(self.root_hash_nomt.user),
            root_hash_nomt_kernel = hex::encode(self.root_hash_nomt.kernel),
            "State roots on startup {msg}"
        );
    }
}
