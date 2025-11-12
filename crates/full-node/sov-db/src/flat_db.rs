//! A database to store the flat state of the rollup (i.e. the raw key-value pairs)
//! used by NOMT.

use std::sync::Arc;

use rockbound::Schema;
use rockbound::{
    default_cf_descriptor, rocksdb::ColumnFamilyDescriptor, versioned_db::VersionedDB, SchemaBatch,
};

use crate::metrics::nomt::FlatStateCommitMetric;
use crate::schema::namespace::{NomtCommittedVersion, NomtHistoricalState, NomtPruningState};
use crate::{
    historical_state::StateChanges,
    namespaces::{KernelNamespace, UserNamespace},
    schema::{namespace::NomtStateValues, tables::StateRootHashes},
    DbOptions,
};

/// A database to store the flat state of the rollup (i.e. the raw key-value pairs)
pub struct FlatStateDb {
    pub(crate) user: VersionedDB<NomtStateValues<UserNamespace>>,
    pub(crate) kernel: VersionedDB<NomtStateValues<KernelNamespace>>,
    pub(crate) other: Arc<rockbound::DB>,
    #[allow(dead_code)]
    // We don't technically need to store the archival db here - it's only accessed through the user/kernel versioned DB wrappers.
    // We keep it here so that the internal structure is more legible by glancing at this struct. Note that unless the user has set the config to separate out the archival db,
    // this will point to the same rockbound::DB as the `other` field.
    pub(crate) archival: Arc<rockbound::DB>,
}

pub struct FlatDbCommitData {
    pub archival: SchemaBatch,
    pub flat: SchemaBatch,
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
        VersionedDB::<NomtStateValues<UserNamespace>>::add_column_families(
            &mut columns,
            separate_archival,
        )?;
        VersionedDB::<NomtStateValues<KernelNamespace>>::add_column_families(
            &mut columns,
            separate_archival,
        )?;
        let other = Self::get_rockbound_options(columns)
            .setup_db_in_path_with_column_descriptors(path.clone(), cache_size)?;
        let other = Arc::new(other);
        let archival = if separate_archival {
            let archival_path = path.join(Self::ARCHIVAL_DB_PATH_SUFFIX);
            let archival_columns = vec![
                default_cf_descriptor(NomtHistoricalState::<UserNamespace>::COLUMN_FAMILY_NAME),
                default_cf_descriptor(NomtHistoricalState::<KernelNamespace>::COLUMN_FAMILY_NAME),
                default_cf_descriptor(NomtPruningState::<UserNamespace>::COLUMN_FAMILY_NAME),
                default_cf_descriptor(NomtPruningState::<KernelNamespace>::COLUMN_FAMILY_NAME),
                default_cf_descriptor(NomtCommittedVersion::<UserNamespace>::COLUMN_FAMILY_NAME),
                default_cf_descriptor(NomtCommittedVersion::<KernelNamespace>::COLUMN_FAMILY_NAME),
            ];
            let archival = Self::get_rockbound_options(archival_columns);
            Arc::new(archival.setup_db_in_path_with_column_descriptors(archival_path, 0)?)
        } else {
            other.clone()
        };
        let user = VersionedDB::<NomtStateValues<UserNamespace>>::from_dbs(
            other.clone(),
            archival.clone(),
        )?;
        let kernel = VersionedDB::<NomtStateValues<KernelNamespace>>::from_dbs(
            other.clone(),
            archival.clone(),
        )?;
        Ok(Self {
            user,
            kernel,
            other,
            archival,
        })
    }

    /// Get the underlying [`rockbound::DB`] for the historical state. Used for testing only.
    pub fn get_db(&self) -> Arc<rockbound::DB> {
        self.other.clone()
    }

    /// Get the underlying [`VersionedDB`] for the user state.
    pub fn get_user_db(&self) -> &VersionedDB<NomtStateValues<UserNamespace>> {
        &self.user
    }

    /// Get the underlying [`VersionedDB`] for the kernel state.
    pub fn get_kernel_db(&self) -> &VersionedDB<NomtStateValues<KernelNamespace>> {
        &self.kernel
    }

    /// [`DbOptions`] for [`FlatStateDb`].
    pub fn get_rockbound_options(
        columns: Vec<ColumnFamilyDescriptor>,
    ) -> DbOptions<ColumnFamilyDescriptor> {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns,
        }
    }

    /// Coalesce all the changes into a single schema batch.
    /// Assumption: only a single thread is committing at a time. Calling prepare_commit multiple times
    /// will result in a version mismatch.
    fn prepare_commit(&self, state: StateChanges) -> anyhow::Result<FlatDbCommitData> {
        let StateChanges {
            user,
            kernel,
            other,
        } = state;

        let mut other_changes = Arc::try_unwrap(other).unwrap_or_else(|arc| (*arc).clone());
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
        let mut archival_data = SchemaBatch::default();
        self.user
            .materialize(&user, &mut other_changes, &mut archival_data, version)?;
        self.kernel
            .materialize(&kernel, &mut other_changes, &mut archival_data, version)?;

        Ok(FlatDbCommitData {
            archival: archival_data,
            flat: other_changes,
        })
    }

    /// Coalesce all the changes into a single schema batch and write it atomically.
    pub fn commit(&self, state: StateChanges) -> anyhow::Result<FlatStateCommitMetric> {
        let start_prepare = std::time::Instant::now();
        let commit = self.prepare_commit(state)?;
        let prepare = start_prepare.elapsed();
        let start_write = std::time::Instant::now();
        self.archival.write_schemas(commit.archival)?;
        #[cfg(feature = "test-utils")]
        if cfg!(debug_assertions) && std::env::var("SOV_CRASH_ON_COMMIT").is_ok() {
            panic!("SOV_CRASH_ON_COMMIT is set, crashing the node");
        }
        self.other.write_schemas(commit.flat)?;
        let write = start_write.elapsed();
        Ok(FlatStateCommitMetric { prepare, write })
    }
}
