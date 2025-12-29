//! A database to store the flat state of the rollup (i.e. the raw key-value pairs)
//! used by NOMT.

use crate::metrics::nomt::FlatStateCommitMetric;
use crate::{
    historical_state::StateChanges,
    namespaces::{KernelNamespace, UserNamespace},
    schema::{namespace::NomtStateValues, tables::StateRootHashes},
    DbOptions,
};
use parking_lot::{MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock};
use parking_lot::{RwLockReadGuard, RwLockWriteGuard};
use rockbound::versioned_db::CacheForVersionedDB;
use rockbound::versioned_db::SchemaWithVersion;
use rockbound::{
    default_cf_descriptor, rocksdb::ColumnFamilyDescriptor, versioned_db::VersionedDB,
};
use rockbound::{rocksdb, DB};
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
    pub fn new(
        path: std::path::PathBuf,
        cache_size: usize,
        separate_archival: bool,
    ) -> anyhow::Result<Self> {
        let mut columns: Vec<ColumnFamilyDescriptor> =
            vec![default_cf_descriptor(StateRootHashes::table_name())];
        VersionedDB::<NomtStateValues<UserNamespace>, DbCache>::add_column_families(
            &mut columns,
            separate_archival,
        )?;
        VersionedDB::<NomtStateValues<KernelNamespace>, DbCache>::add_column_families(
            &mut columns,
            separate_archival,
        )?;
        let live_db = Self::get_rockbound_options(columns)
            .setup_db_in_path_with_column_descriptors(path.clone())?;
        let live_db = Arc::new(live_db);
        let archival_db = if separate_archival {
            let archival_path = path.join(Self::ARCHIVAL_DB_PATH_SUFFIX);
            let archival_columns = vec![
                default_cf_descriptor(
                    NomtStateValues::<UserNamespace>::HISTORICAL_COLUMN_FAMILY_NAME,
                ),
                default_cf_descriptor(
                    NomtStateValues::<KernelNamespace>::HISTORICAL_COLUMN_FAMILY_NAME,
                ),
                default_cf_descriptor(NomtStateValues::<UserNamespace>::PRUNING_COLUMN_FAMILY_NAME),
                default_cf_descriptor(
                    NomtStateValues::<KernelNamespace>::PRUNING_COLUMN_FAMILY_NAME,
                ),
                default_cf_descriptor(
                    NomtStateValues::<UserNamespace>::VERSION_METADATA_COLUMN_FAMILY_NAME,
                ),
                default_cf_descriptor(
                    NomtStateValues::<KernelNamespace>::VERSION_METADATA_COLUMN_FAMILY_NAME,
                ),
            ];
            let archival = Self::get_rockbound_options(archival_columns);
            Arc::new(archival.setup_db_in_path_with_column_descriptors(archival_path)?)
        } else {
            live_db.clone()
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

    /// Coalesce all the changes into a single schema batch and write it atomically.
    pub fn commit(&self, state: StateChanges) -> anyhow::Result<FlatStateCommitMetric> {
        let start_prepare = std::time::Instant::now();
        let prepare = start_prepare.elapsed();
        let start_write = std::time::Instant::now();
        let version = self
            .kernel
            .get_committed_version()?
            .and_then(|v| v.checked_add(1))
            .unwrap_or(0);
        if cfg!(debug_assertions) {
            let user_version = self
                .user
                .get_committed_version()?
                .and_then(|v| v.checked_add(1))
                .unwrap_or(0);
            assert_eq!(user_version, version);
        }

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
            &state.kernel,
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
            &state.user,
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
            &mut live_db_batch,
            &state.root_hash_batch,
            &self.live_db,
        )?;

        // Write batches.
        self.archival_db.write_db_batch(archival_db_batch)?;
        self.kernel.store_committed_archival_version(version);
        self.user.store_committed_archival_version(version);
        self.live_db.write_db_batch(live_db_batch)?;

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
