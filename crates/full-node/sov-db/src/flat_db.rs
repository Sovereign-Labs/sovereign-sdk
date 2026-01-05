//! A database to store the flat state of the rollup (i.e. the raw key-value pairs)
//! used by NOMT.

use crate::commit_flag::{CommitFlag, CommitStatus};
use crate::historical_state::{HistoricalStateReader, STATE_ROOT_HASH_SINGLETON};
use crate::metrics::nomt::FlatStateCommitMetric;
use crate::{
    historical_state::StateChanges,
    namespaces::{KernelNamespace, UserNamespace},
    schema::{namespace::NomtStateValues, tables::StateRootHashes},
    DbOptions,
};
use parking_lot::{MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock};
use parking_lot::{RwLockReadGuard, RwLockWriteGuard};
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::versioned_db::CacheForVersionedDB;
use rockbound::versioned_db::SchemaWithVersion;
use rockbound::{
    default_cf_descriptor, rocksdb::ColumnFamilyDescriptor, versioned_db::VersionedDB,
};
use rockbound::{rocksdb, SchemaBatch, DB};
use sov_rollup_interface::common::SlotNumber;
use std::sync::Arc;

/// A database to store the flat state of the rollup (i.e. the raw key-value pairs)
pub struct FlatStateDb {
    pub(crate) user: Arc<VersionedDB<NomtStateValues<UserNamespace>, DbCache>>,
    pub(crate) kernel: Arc<VersionedDB<NomtStateValues<KernelNamespace>, DbCache>>,
    pub(crate) live_db: Arc<rockbound::DB>,
    pub(crate) archival_db: Arc<rockbound::DB>,
    db_cache: DbCache,
}

impl FlatStateDb {
    const DB_NAME: &'static str = "state";
    const DB_PATH_SUFFIX: &'static str = "state-db";
    const ARCHIVAL_DB_PATH_SUFFIX: &'static str = "archival-state-db";

    /// Create a new [`FlatStateDb`] from a path.
    pub fn new(path: std::path::PathBuf, cache_size: usize) -> anyhow::Result<Self> {
        let live_db = {
            let mut live_columns = vec![default_cf_descriptor(StateRootHashes::table_name())];

            VersionedDB::<NomtStateValues<UserNamespace>, DbCache>::add_live_db_column_families(
                &mut live_columns,
            )?;
            VersionedDB::<NomtStateValues<KernelNamespace>, DbCache>::add_live_db_column_families(
                &mut live_columns,
            )?;

            let live = Self::get_rockbound_options(live_columns)
                .setup_db_in_path_with_column_descriptors(path.clone())?;
            Arc::new(live)
        };

        let archival_db = {
            let archival_path = path.join(Self::ARCHIVAL_DB_PATH_SUFFIX);

            let mut archival_columns = vec![default_cf_descriptor(StateRootHashes::table_name())];

            VersionedDB::<NomtStateValues<UserNamespace>, DbCache>::add_archival_db_column_families(
                &mut archival_columns,
            )?;

            VersionedDB::<NomtStateValues<KernelNamespace>, DbCache>::add_archival_db_column_families(
                &mut archival_columns,
            )?;

            let archival = Self::get_rockbound_options(archival_columns)
                .setup_db_in_path_with_column_descriptors(archival_path)?;

            Arc::new(archival)
        };

        let inner = Arc::new(RwLock::new(Inner {
            user_cache: rockbound::new_cache_for_schema::<NomtStateValues<UserNamespace>>(
                cache_size,
            ),
            kernel_cache: rockbound::new_cache_for_schema::<NomtStateValues<KernelNamespace>>(
                100_000,
            ),
        }));

        let db_cache = DbCache { inner };

        let user = Arc::new(
            VersionedDB::<NomtStateValues<UserNamespace>, DbCache>::from_dbs(
                live_db.clone(),
                archival_db.clone(),
                db_cache.clone(),
            )?,
        );

        let kernel = Arc::new(
            VersionedDB::<NomtStateValues<KernelNamespace>, DbCache>::from_dbs(
                live_db.clone(),
                archival_db.clone(),
                db_cache.clone(),
            )?,
        );

        Ok(Self {
            db_cache,
            user,
            kernel,
            live_db,
            archival_db,
        })
    }

    pub(crate) fn root_hash_from_live_db(&self) -> anyhow::Result<Option<[u8; 64]>> {
        let Some((delta_reader, version)) = Self::latest_reader_and_version(self.live_db.clone())?
        else {
            return Ok(None);
        };

        Self::root_hash_from_db(&delta_reader, version)
    }

    pub(crate) fn root_hash_from_archival_db(&self) -> anyhow::Result<Option<[u8; 64]>> {
        let Some((delta_reader, version)) =
            Self::latest_reader_and_version(self.archival_db.clone())?
        else {
            return Ok(None);
        };

        Self::root_hash_from_db(&delta_reader, version)
    }

    /// Latest state root hash from live db
    pub fn latest_version_and_root_hash_live_db(&self) -> anyhow::Result<Option<(u64, [u8; 64])>> {
        let Some((delta_reader, version)) = Self::latest_reader_and_version(self.live_db.clone())?
        else {
            return Ok(None);
        };
        let root_hash = Self::root_hash_from_db(&delta_reader, version)?.unwrap();
        Ok(Some((version.get(), root_hash)))
    }

    /// Latest state root hash from archival db.
    pub fn latest_version_and_root_hash_archival_db(
        &self,
    ) -> anyhow::Result<Option<(u64, [u8; 64])>> {
        let Some((delta_reader, version)) =
            Self::latest_reader_and_version(self.archival_db.clone())?
        else {
            return Ok(None);
        };
        let root_hash = Self::root_hash_from_db(&delta_reader, version)?.unwrap();
        Ok(Some((version.get(), root_hash)))
    }

    /// Historical state root hash from archival db.
    pub(crate) fn root_hash_from_archival_db_for_version(
        &self,
        version: SlotNumber,
    ) -> anyhow::Result<Option<[u8; 64]>> {
        let Some((delta_reader, _)) = Self::latest_reader_and_version(self.archival_db.clone())?
        else {
            return Ok(None);
        };

        Self::root_hash_from_db(&delta_reader, version)
    }

    fn root_hash_from_db(
        delta_reader: &DeltaReader,
        version: SlotNumber,
    ) -> anyhow::Result<Option<[u8; 64]>> {
        let state_root_hash =
            HistoricalStateReader::get_serialized_root_hash_from_reader(delta_reader, version)?
                .unwrap_or_else(|| {
                    // If `last_version` is present we must always have root hash.
                    panic!("Root hash missing for the latest DB version {version}",);
                });

        Ok(Some(state_root_hash.try_into().unwrap()))
    }

    fn latest_reader_and_version(
        db: Arc<rockbound::DB>,
    ) -> anyhow::Result<Option<(DeltaReader, SlotNumber)>> {
        let delta_reader = DeltaReader::new(db, Vec::new());
        let last_version = HistoricalStateReader::last_version_from_reader(&delta_reader)?;
        Ok(last_version.map(|v| (delta_reader, v)))
    }

    /// Get the underlying [`rockbound::DB`] for the historical state. Used for testing only.
    pub fn get_db(&self) -> Arc<rockbound::DB> {
        self.live_db.clone()
    }

    /// Get the underlying [`VersionedDB`] for the user state.
    pub fn get_user_db(&self) -> &Arc<VersionedDB<NomtStateValues<UserNamespace>, DbCache>> {
        &self.user
    }

    /// Get the underlying [`VersionedDB`] for the kernel state.
    pub fn get_kernel_db(&self) -> &Arc<VersionedDB<NomtStateValues<KernelNamespace>, DbCache>> {
        &self.kernel
    }

    /// [`DbOptions`] for [`HistoricalStateReader`].
    pub fn get_rockbound_options(
        columns: Vec<ColumnFamilyDescriptor>,
    ) -> DbOptions<ColumnFamilyDescriptor> {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns,
        }
    }

    /// Commit the `state_changes`.
    pub fn commit(
        &self,
        state_changes: StateChanges,
        commit_flag: Option<&CommitFlag>,
    ) -> anyhow::Result<FlatStateCommitMetric> {
        self.commit_internal(state_changes, commit_flag, true)
    }

    fn commit_internal(
        &self,
        state_changes: StateChanges,
        commit_flag: Option<&CommitFlag>,
        commit_live_db: bool,
    ) -> anyhow::Result<FlatStateCommitMetric> {
        let start_prepare = std::time::Instant::now();
        let prepare = start_prepare.elapsed();
        let start_write = std::time::Instant::now();

        let version = self
            .latest_version_and_root_hash_live_db()?
            .and_then(|(v, _)| v.checked_add(1))
            .unwrap_or(0);

        let mut live_db_batch = rocksdb::WriteBatch::default();
        let mut archival_db_batch = rocksdb::WriteBatch::default();

        // 1. Lock caches.
        let inner = self.db_cache.inner.write();

        let kernel_cache = &inner.kernel_cache;
        let user_cache = &inner.user_cache;

        // 2. Update batches with kernel values.
        let metrics_kernel = VersionedDB::<_, DbCache>::update_versioned_db_batch(
            &mut live_db_batch,
            &mut archival_db_batch,
            &state_changes.kernel,
            version,
            kernel_cache,
            &self.live_db,
            &self.archival_db,
        )?;

        let serialized_size_kernel = live_db_batch.size_in_bytes();
        let archival_serialized_size_kernel = archival_db_batch.size_in_bytes();

        // 3. Update batches with user values.
        let metrics_user = VersionedDB::<_, DbCache>::update_versioned_db_batch(
            &mut live_db_batch,
            &mut archival_db_batch,
            &state_changes.user,
            version,
            user_cache,
            &self.live_db,
            &self.archival_db,
        )?;

        let serialized_size_user = live_db_batch.size_in_bytes() - serialized_size_kernel;
        let archival_serialized_size_user =
            archival_db_batch.size_in_bytes() - archival_serialized_size_kernel;

        // 4.Update root hash.
        DB::update_db_batch_with_schema_data(
            &mut archival_db_batch,
            &state_changes.root_hash_batch,
            &self.archival_db,
        )?;

        DB::update_db_batch_with_schema_data(
            &mut live_db_batch,
            &state_changes.root_hash_batch,
            &self.live_db,
        )?;

        #[cfg(feature = "test-utils")]
        crate::test_utils::CrashLocation::BeforeSavingArchival.crash_if_env_set();

        // Write archival db batches.
        if let Some(commit_flag) = commit_flag {
            commit_flag.save_commit_status(&CommitStatus::CommittingArchivalUserAndKernel)?;
        }

        #[cfg(feature = "test-utils")]
        crate::test_utils::CrashLocation::BeforeCommittingArchival.crash_if_env_set();
        self.archival_db.write_db_batch(archival_db_batch)?;

        // rockbound requirement:  `store_committed_archival_version` has to be called before before writing `live_db_batch`.
        self.kernel.store_committed_archival_version(version);
        self.user.store_committed_archival_version(version);
        // Write live db batch.

        if commit_live_db {
            #[cfg(feature = "test-utils")]
            crate::test_utils::CrashLocation::BeforeSavingLive.crash_if_env_set();
            if let Some(commit_flag) = commit_flag {
                commit_flag.save_commit_status(&CommitStatus::CommittingLiveUserAndKernel)?;
            }

            #[cfg(feature = "test-utils")]
            crate::test_utils::CrashLocation::BeforeCommittingLive.crash_if_env_set();
            self.live_db.write_db_batch(live_db_batch)?;
        }

        // 5. Release caches.
        drop(inner);

        // Update metrics once the cache lock is released.
        self.kernel.update_metrics(
            metrics_kernel,
            serialized_size_kernel,
            archival_serialized_size_kernel,
        );

        self.user.update_metrics(
            metrics_user,
            serialized_size_user,
            archival_serialized_size_user,
        );

        let write = start_write.elapsed();
        Ok(FlatStateCommitMetric { prepare, write })
    }

    /// Validate consistency between live and archival DBs, rollback archival changes if necessary.
    pub fn validate_and_rollback_archival(&self) -> anyhow::Result<()> {
        let live_version_and_root_hash = self.latest_version_and_root_hash_live_db()?;

        let Some((current_version_archival, root_hash_archival)) =
            self.latest_version_and_root_hash_archival_db()?
        else {
            assert!(live_version_and_root_hash.is_none());
            tracing::info!("Roolup Archival & Live DBs are empty, nothing to rollback");
            return Ok(());
        };

        let Some((current_version_live, root_hash_live)) = live_version_and_root_hash else {
            assert_eq!(current_version_archival, 0);
            anyhow::bail!("The Live DB is empty; please remove the Archival DB manually.")
        };

        if current_version_archival == current_version_live {
            assert_eq!(root_hash_archival, root_hash_live);
            return Ok(());
        }

        assert_eq!(current_version_archival, current_version_live + 1);

        let prev_root_hash_archival = self
            .root_hash_from_archival_db_for_version(SlotNumber::new(current_version_live))?
            .unwrap_or_else(|| {
                panic!("Missing archival root hash for version {current_version_live}")
            });

        assert_eq!(prev_root_hash_archival, root_hash_live);

        self.rollback_archival(
            current_version_archival,
            current_version_live,
            root_hash_live,
        )
    }

    fn rollback_archival(
        &self,
        current_version_archival: u64,
        current_version_live: u64,
        root_hash_live: [u8; 64],
    ) -> anyhow::Result<()> {
        let user_changes = keys_and_values_for_rollback(&self.user, current_version_archival)?;
        let kernel_changes = keys_and_values_for_rollback(&self.kernel, current_version_archival)?;

        let mut state_changes = HistoricalStateReader::materialize_values(
            user_changes,
            kernel_changes,
            root_hash_live.to_vec(),
            SlotNumber::new(current_version_live),
        )?;

        // Remove the most recent (current_version_archival) root hash from the `StateRootHashes` table.
        let mut root_hash_batch = SchemaBatch::default();
        root_hash_batch.merge(state_changes.root_hash_batch.as_ref().clone());
        root_hash_batch.delete(&(
            SlotNumber::new(current_version_archival),
            STATE_ROOT_HASH_SINGLETON,
        ))?;

        state_changes.root_hash_batch = Arc::new(root_hash_batch);
        // We rollback only archival db.
        self.commit_internal(state_changes, None, false)?;

        Ok(())
    }
}

#[allow(clippy::type_complexity)]
fn keys_and_values_for_rollback<V, C>(
    db: &VersionedDB<V, C>,
    version_to_rollback: u64,
) -> anyhow::Result<Vec<(V::Key, Option<V::Value>)>>
where
    C: CacheForVersionedDB<V>,
    V::Key: Ord + Clone + std::hash::Hash + AsRef<[u8]>,
    V::Value: Clone + AsRef<[u8]>,
    V: SchemaWithVersion + Ord,
{
    // Get all keys committed at `version_to_rollback`.
    // Then retrieve the values for those keys from the previous state version.
    // Values that changed between `version_to_rollback` and `version_to_rollback - 1` are overridden.
    // Values that were inserted at `version_to_rollback` are deleted.
    // Values that were deleted at `version_to_rollback` are reinserted.
    let prunable_keys = db.iter_pruning_keys_at_version(version_to_rollback)?;
    let mut data_to_materialize = Vec::new();

    for key in prunable_keys {
        let (version, key) = key.version_and_key();
        assert_eq!(version, version_to_rollback);

        let value = db.get_historical_value(&key, version_to_rollback - 1)?;
        data_to_materialize.push((key, value));
    }

    Ok(data_to_materialize)
}

struct Inner {
    user_cache: rockbound::CacheForSchema<NomtStateValues<UserNamespace>>,
    kernel_cache: rockbound::CacheForSchema<NomtStateValues<KernelNamespace>>,
}

/// Cache for the database.
#[derive(Clone)]
pub struct DbCache {
    inner: Arc<RwLock<Inner>>,
}

impl CacheForVersionedDB<NomtStateValues<UserNamespace>> for DbCache {
    fn write(
        &self,
    ) -> MappedRwLockWriteGuard<'_, rockbound::CacheForSchema<NomtStateValues<UserNamespace>>> {
        RwLockWriteGuard::map(self.inner.write(), |c: &mut Inner| &mut c.user_cache)
    }

    fn read(
        &self,
    ) -> MappedRwLockReadGuard<'_, rockbound::CacheForSchema<NomtStateValues<UserNamespace>>> {
        RwLockReadGuard::map(self.inner.read(), |c: &Inner| &c.user_cache)
    }

    fn try_read(
        &self,
    ) -> Option<MappedRwLockReadGuard<'_, rockbound::CacheForSchema<NomtStateValues<UserNamespace>>>>
    {
        let lock = self.inner.try_read()?;
        Some(RwLockReadGuard::map(lock, |c: &Inner| &c.user_cache))
    }
}

impl CacheForVersionedDB<NomtStateValues<KernelNamespace>> for DbCache {
    fn write(
        &self,
    ) -> MappedRwLockWriteGuard<'_, rockbound::CacheForSchema<NomtStateValues<KernelNamespace>>>
    {
        RwLockWriteGuard::map(self.inner.write(), |c: &mut Inner| &mut c.kernel_cache)
    }

    fn read(
        &self,
    ) -> MappedRwLockReadGuard<'_, rockbound::CacheForSchema<NomtStateValues<KernelNamespace>>>
    {
        RwLockReadGuard::map(self.inner.read(), |c: &Inner| &c.kernel_cache)
    }

    fn try_read(
        &self,
    ) -> Option<
        MappedRwLockReadGuard<'_, rockbound::CacheForSchema<NomtStateValues<KernelNamespace>>>,
    > {
        let lock = self.inner.try_read()?;
        Some(RwLockReadGuard::map(lock, |c: &Inner| &c.kernel_cache))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::CrashLocation;
    use sov_db_types::{SlotKey, SlotValue};
    use std::{collections::HashMap, panic::AssertUnwindSafe, vec};
    use tempfile::TempDir;

    type Changes = Vec<(SlotKey, Option<SlotValue>)>;

    #[test]
    fn test_rollback_crash_before_saving_archival() -> anyhow::Result<()> {
        test_rollback(CrashLocation::BeforeSavingArchival, 0)
    }

    #[test]
    fn test_rollback_crash_before_commiting_archival() -> anyhow::Result<()> {
        test_rollback(CrashLocation::BeforeCommittingArchival, 0)
    }

    #[test]
    fn test_rollback_crash_before_saving_live() -> anyhow::Result<()> {
        test_rollback(CrashLocation::BeforeSavingLive, 1)
    }

    #[test]
    fn test_rollback_crash_before_comitting_live() -> anyhow::Result<()> {
        test_rollback(CrashLocation::BeforeCommittingLive, 1)
    }

    // This test commits data for version 0 of the rollup state and panics at various points during the commit.
    // Afterward, it checks whether the rollback logic correctly reverted the archival state.
    fn test_rollback(crash_location: CrashLocation, archival_version: u64) -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path();
        let data = data_to_insert_per_verson();

        // Commit version 0.
        {
            let version = 0;
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();

            let state_changes = data.change_set(version);
            flat_db.commit(state_changes, None).unwrap();
            assert_flat_state(version, version, &flat_db);
        }

        unlock_dbs(&tempdir);
        crash_location.set_crash_env();

        // Crash during commit of version 1.
        {
            let version = 1;
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();

            let state_changes = data.change_set(version);
            let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
                flat_db.commit(state_changes, None).unwrap();
            }));

            assert!(res.is_err());
            assert_flat_state(0, archival_version, &flat_db);
        }

        // Rollback to version 0.
        {
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();

            flat_db.validate_and_rollback_archival().unwrap();
            assert_flat_state(0, 0, &flat_db);

            // After rollback, the latest archival state must match version 0.
            for (k, v) in data.get_user_data_for_version(0).iter() {
                let value_from_archival_db = flat_db.user.get_historical_value(k, 99).unwrap();
                assert_eq!(v, &value_from_archival_db);
            }

            for (k, v) in data.get_kernel_data_for_version(0).iter() {
                let value_from_archival_db = flat_db.kernel.get_historical_value(k, 99).unwrap();
                assert_eq!(v, &value_from_archival_db);
            }
        }

        Ok(())
    }

    /// This test rolls back the archival state regardless of the version of the live state.
    #[test]
    fn test_rollback_skip_validation() -> anyhow::Result<()> {
        let separate_archival = true;

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path();

        let data = data_to_insert_per_verson();

        {
            let version = 0;
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();

            let state_changes = data.change_set(version);
            flat_db.commit(state_changes, None).unwrap();
            assert_flat_state(version, version, &flat_db);
        }

        unlock_dbs(&tempdir);

        {
            let version = 1;
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();

            let state_changes = data.change_set(version);
            flat_db.commit(state_changes, None).unwrap();

            assert_flat_state(version, version, &flat_db);
        }

        unlock_dbs(&tempdir);

        {
            let version = 2;
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();

            let state_changes = data.change_set(version);
            flat_db.commit(state_changes, None).unwrap();

            assert_flat_state(version, version, &flat_db);
        }

        unlock_dbs(&tempdir);

        // Rollbacks

        {
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();

            flat_db.rollback_archival_one_slot().unwrap();

            let version = 2;
            assert_flat_state(version, version - 1, &flat_db);
        }

        unlock_dbs(&tempdir);

        {
            let flat_db = FlatStateDb::new(db_path.to_path_buf(), 1_000_000).unwrap();
            flat_db.rollback_archival_one_slot().unwrap();

            let version = 2;
            assert_flat_state(version, version - 2, &flat_db);

            for (k, v) in data.get_user_data_for_version(0).iter() {
                let value_from_archival_db = flat_db.user.get_historical_value(k, 99).unwrap();
                assert_eq!(v, &value_from_archival_db);
            }

            for (k, v) in data.get_kernel_data_for_version(0).iter() {
                let value_from_archival_db = flat_db.kernel.get_historical_value(k, 99).unwrap();
                assert_eq!(v, &value_from_archival_db);
            }
        }

        Ok(())
    }

    impl FlatStateDb {
        fn rollback_archival_one_slot(&self) -> anyhow::Result<()> {
            let (current_version_archival, _) =
                self.latest_version_and_root_hash_archival_db()?.unwrap();

            let prev_root_hash_archival = self
                .root_hash_from_archival_db_for_version(SlotNumber::new(
                    current_version_archival - 1,
                ))?
                .unwrap();

            self.rollback_archival(
                current_version_archival,
                current_version_archival - 1,
                prev_root_hash_archival,
            )
        }
    }

    fn assert_flat_state(live_db_version: u64, archival_db_version: u64, flat_db: &FlatStateDb) {
        let expected_root_hash_live_db = [live_db_version as u8; 64].to_vec();
        let root_hash_from_live_db = flat_db.root_hash_from_live_db().unwrap().unwrap();
        let root_hash_from_archival_db = flat_db.root_hash_from_archival_db().unwrap().unwrap();

        assert_eq!(expected_root_hash_live_db, root_hash_from_live_db);

        let expected_root_hash_archival_db = [archival_db_version as u8; 64].to_vec();
        assert_eq!(expected_root_hash_archival_db, root_hash_from_archival_db);

        assert_eq!(
            live_db_version,
            flat_db
                .latest_version_and_root_hash_live_db()
                .unwrap()
                .unwrap()
                .0
        );
        assert_eq!(
            archival_db_version,
            flat_db
                .latest_version_and_root_hash_archival_db()
                .unwrap()
                .unwrap()
                .0
        );
    }

    struct TestData {
        data: HashMap<u64, (Changes, Changes)>,
    }

    impl TestData {
        fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }

        fn insert(
            &mut self,
            version: u64,
            op_user: Vec<OperationType>,
            op_kernel: Vec<OperationType>,
        ) {
            let user = make_data(op_user, version);
            let kernel = make_data(op_kernel, version);
            self.data.insert(version, (user, kernel));
        }

        fn get_user_data_for_version(&self, version: u64) -> Changes {
            self.data.get(&version).unwrap().0.clone()
        }

        fn get_kernel_data_for_version(&self, version: u64) -> Changes {
            self.data.get(&version).unwrap().1.clone()
        }

        fn change_set(&self, version: u64) -> StateChanges {
            make_change_set(
                self.get_user_data_for_version(version),
                self.get_kernel_data_for_version(version),
                version,
            )
        }
    }

    fn data_to_insert_per_verson() -> TestData {
        let mut data = TestData::new();
        data.insert(
            0,
            vec![
                OperationType::Insert("user_key1"),
                OperationType::Insert("user_key2"),
                OperationType::Insert("user_key3"),
            ],
            vec![
                OperationType::Insert("kernel_key1"),
                OperationType::Insert("kernel_key2"),
                OperationType::Insert("kernel_key3"),
            ],
        );

        data.insert(
            1,
            vec![
                OperationType::Insert("user_key1"),
                OperationType::Insert("user_key2"),
            ],
            vec![OperationType::Insert("kernel_key4")],
        );

        data.insert(
            2,
            vec![
                OperationType::Delete("user_key1"),
                OperationType::Insert("user_key2"),
            ],
            vec![
                OperationType::Delete("kernel_key3"),
                OperationType::Insert("kernel_key11"),
            ],
        );

        data
    }

    enum OperationType {
        Insert(&'static str),
        Delete(&'static str),
    }

    fn make_key(key: &str) -> SlotKey {
        SlotKey::from_slice(key.as_bytes())
    }

    fn make_value(key_str: &str, version: u64) -> SlotValue {
        SlotValue::from(format!("{key_str}_value_{version}").as_str())
    }

    fn make_data(ops: Vec<OperationType>, version: u64) -> Changes {
        let mut data = Vec::new();

        for op in ops {
            match op {
                OperationType::Insert(key_str) => {
                    let key = make_key(key_str);
                    let value = make_value(key_str, version);
                    data.push((key, Some(value)));
                }
                OperationType::Delete(key) => {
                    let key = make_key(key);
                    data.push((key, None));
                }
            }
        }

        data
    }

    fn make_change_set(
        user_changes: Changes,
        kernel_changes: Changes,
        version: u64,
    ) -> StateChanges {
        let root_hash = [version as u8; 64].to_vec();
        let version = SlotNumber::new(version);
        HistoricalStateReader::materialize_values(user_changes, kernel_changes, root_hash, version)
            .unwrap()
    }

    fn unlock_dbs(temp_dir: &TempDir) {
        let lock_files = ["LOCK", "LOG", "LOG.old"];
        let dbs = ["state-db", "archival-state-db", "accessory", "blob_sender"];
        for lock_file in &lock_files {
            for db in &dbs {
                let _ = std::fs::remove_file(temp_dir.path().join(db).join(lock_file));
            }
        }
    }
}
