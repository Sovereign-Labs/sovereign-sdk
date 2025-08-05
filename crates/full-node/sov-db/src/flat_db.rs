//! A database to store the flat state of the rollup (i.e. the raw key-value pairs)
//! used by NOMT.

use std::sync::Arc;

use rockbound::{
    default_cf_descriptor, rocksdb::ColumnFamilyDescriptor, versioned_db::VersionedDB, SchemaBatch,
};

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
}

impl FlatStateDb {
    const DB_NAME: &'static str = "state";
    const DB_PATH_SUFFIX: &'static str = "state-db";

    /// Create a new [`FlatStateDb`] from a path.
    pub fn new(path: std::path::PathBuf) -> anyhow::Result<Self> {
        let mut columns = vec![default_cf_descriptor(StateRootHashes::table_name())];
        VersionedDB::<NomtStateValues<UserNamespace>>::add_column_families(&mut columns)?;
        VersionedDB::<NomtStateValues<KernelNamespace>>::add_column_families(&mut columns)?;
        let other =
            Self::get_rockbound_options(columns).setup_db_in_path_with_column_descriptors(path)?;
        let other = Arc::new(other);
        let user = VersionedDB::<NomtStateValues<UserNamespace>>::from_db(other.clone())?;
        let kernel = VersionedDB::<NomtStateValues<KernelNamespace>>::from_db(other.clone())?;
        Ok(Self {
            user,
            kernel,
            other,
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

    /// Coalesce all the changes into a single schema batch.
    /// Assumption: only a single thread is committing at a time. Calling prepare_commit multiple times
    /// will result in a version mismatch.
    fn prepare_commit(&self, state: StateChanges) -> anyhow::Result<SchemaBatch> {
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
        self.user.materialize(&user, &mut other_changes, version)?;
        self.kernel
            .materialize(&kernel, &mut other_changes, version)?;

        Ok(other_changes)
    }

    /// Coalesce all the changes into a single schema batch and write it atomically.
    pub fn commit(&self, state: StateChanges) -> anyhow::Result<()> {
        let commit = self.prepare_commit(state)?;
        self.other.write_schemas(commit)?;
        Ok(())
    }
}
