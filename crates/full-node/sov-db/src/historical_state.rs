use std::fmt::Debug;
use std::sync::Arc;

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::versioned_db::{VersionedDeltaReader, VersionedSchemaBatch};
use rockbound::{SchemaBatch, SchemaKey, SchemaValue};
use sov_rollup_interface::common::SlotNumber;

use crate::metrics::StateMaterializationMetrics;
use crate::namespaces::{KernelNamespace, Namespace, UserNamespace};
use crate::schema::namespace::NomtStateValues;
use crate::schema::tables::StateRootHashes;
use crate::schema::types::StateRootHashId;
use crate::DbOptions;

const STATE_ROOT_HASH_SINGLETON: StateRootHashId = StateRootHashId(0);

/// A typed wrapper around the [`DeltaReader`] for reading materializing historical rollup state.
#[derive(Debug, Clone)]
pub struct HistoricalStateReader {
    /// The underlying [`DeltaReader`] correctly routes requests to previous snapshots and/or [`rockbound::DB`]
    user: VersionedDeltaReader<NomtStateValues<UserNamespace>>,
    /// The underlying [`DeltaReader`] correctly routes requests to previous snapshots and/or [`rockbound::DB`]
    kernel: VersionedDeltaReader<NomtStateValues<KernelNamespace>>,
    other: DeltaReader,

    /// The [`SlotNumber`] that will be used for the next batch of writes to the DB
    /// This [`SlotNumber`] is also used for querying data,
    /// so if this instance of [`HistoricalStateReader`] is used as read-only, it won't see newer data.
    next_version: SlotNumber,
}

/// A collection of changes to the state db. Includes versioned changes to user/kernel state, and a plain schema batch of changes to any other columns.
#[derive(Debug, Clone, Default)]
pub struct StateChanges {
    pub(crate) user: Arc<VersionedSchemaBatch<NomtStateValues<UserNamespace>>>,
    pub(crate) kernel: Arc<VersionedSchemaBatch<NomtStateValues<KernelNamespace>>>,
    pub(crate) other: Arc<SchemaBatch>,
}

impl HistoricalStateReader {
    const DB_PATH_SUFFIX: &'static str = "historical_state";
    const DB_NAME: &'static str = "historical-state-db";

    // Used for testing only.
    #[cfg(test)]
    fn new_empty(flat_state: &crate::storage_manager::FlatStateDb) -> Self {
        let kernel_version = flat_state
            .get_kernel_db()
            .load_latest_committed_version()
            .unwrap();
        let user_version = flat_state
            .get_user_db()
            .load_latest_committed_version()
            .unwrap();
        assert_eq!(
            kernel_version, user_version,
            "Kernel and user should always have the same latest version"
        );
        let kernel =
            VersionedDeltaReader::new(flat_state.get_kernel_db().clone(), kernel_version, vec![]);
        let user =
            VersionedDeltaReader::new(flat_state.get_user_db().clone(), user_version, vec![]);
        let other = DeltaReader::new(flat_state.get_db(), vec![]);
        let next_version = match user.latest_version() {
            Some(latest_version) => SlotNumber::new(
                latest_version
                    .checked_add(1)
                    .expect("State version overflow. It's all over."),
            ),
            None => SlotNumber::GENESIS,
        };
        Self {
            user,
            kernel,
            other,
            next_version,
        }
    }

    /// Create a new instance of [`HistoricalStateReader`].
    pub fn new(
        user: VersionedDeltaReader<NomtStateValues<UserNamespace>>,
        kernel: VersionedDeltaReader<NomtStateValues<KernelNamespace>>,
        other: DeltaReader,
    ) -> Self {
        // Cross check the versions across all three dbs
        assert_eq!(
            user.latest_version(),
            kernel.latest_version(),
            "User and kernel must have the same latest version"
        );
        assert_eq!(
            user.latest_version(),
            Self::last_version_from_reader(&other)
                .expect("Failed to get last version from db")
                .map(|v| v.get()),
            "Other must have the same last version as user"
        );
        let next_version = match user.latest_version() {
            Some(latest_version) => SlotNumber::new(
                latest_version
                    .checked_add(1)
                    .expect("State version overflow. It's all over."),
            ),
            None => SlotNumber::GENESIS,
        };
        Self {
            user,
            kernel,
            other,
            next_version,
        }
    }

    /// Get the latest root hash entry from [`DeltaReader`].
    pub fn last_version_from_reader(reader: &DeltaReader) -> anyhow::Result<Option<SlotNumber>> {
        let last_root_hash = reader.get_largest::<StateRootHashes>()?;
        let last_root_hash_version = last_root_hash.map(|((version, _key), _)| version);

        Ok(last_root_hash_version)
    }

    /// [`DbOptions`] for [`HistoricalStateReader`].
    pub fn get_rockbound_options() -> DbOptions {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns: UserNamespace::get_jmt_table_names()
                .into_iter()
                .chain(KernelNamespace::get_jmt_table_names())
                .chain(vec![StateRootHashes::table_name()])
                .collect(),
        }
    }

    /// Get the current value of the `next_version` counter
    pub fn get_next_version(&self) -> SlotNumber {
        self.next_version
    }

    /// The last version used for writes.
    pub fn last_version(&self) -> Option<SlotNumber> {
        self.next_version.checked_sub(1)
    }

    /// The last version committed to the database.
    /// Can differ from [`Self::last_version`] in case if a newer version has been written to the underlying database.
    pub fn last_version_unbound(&self) -> anyhow::Result<SlotNumber> {
        Self::last_version_from_reader(&self.other).map(|v| v.unwrap_or(SlotNumber::GENESIS))
    }

    /// Get an optional value from the database, given a version and a key hash.
    pub fn get_user_value_option_by_key(
        &self,
        key: &SchemaKey,
    ) -> anyhow::Result<Option<SchemaValue>> {
        Ok(self.user.get_latest_borrowed(key)?.flatten())
    }

    /// Get a value from the historical state, given a version and a key hash.
    pub fn get_user_value_option_by_key_historical(
        &self,
        key: &SchemaKey,
        version: SlotNumber,
    ) -> anyhow::Result<Option<SchemaValue>> {
        Ok(self
            .user
            .get_historical_borrowed(key, version.get())?
            .flatten())
    }

    /// Get an optional value from the database, given a version and a key hash.
    pub fn get_kernel_value_option_by_key(
        &self,
        key: &SchemaKey,
    ) -> anyhow::Result<Option<SchemaValue>> {
        Ok(self.kernel.get_latest_borrowed(key)?.flatten())
    }

    /// Get a value from the historical state, given a version and a key hash.
    pub fn get_kernel_value_option_by_key_historical(
        &self,
        key: &SchemaKey,
        version: SlotNumber,
    ) -> anyhow::Result<Option<SchemaValue>> {
        Ok(self
            .kernel
            .get_historical_borrowed(key, version.get())?
            .flatten())
    }

    /// Get the serialized root hash for a given version.
    pub fn get_serialized_root_hash_from_reader(
        reader: &DeltaReader,
        version: SlotNumber,
    ) -> anyhow::Result<Option<SchemaValue>> {
        let key = (version, STATE_ROOT_HASH_SINGLETON);
        let value = reader.get_prev::<StateRootHashes>(&key)?;
        Ok(value.map(|(_, value)| value))
    }

    /// Get the serialized root hash for a given version.
    pub fn get_serialized_root_hash(
        &self,
        version: SlotNumber,
    ) -> anyhow::Result<Option<SchemaValue>> {
        Self::get_serialized_root_hash_from_reader(&self.other, version)
    }

    /// Collects a sequence of key-value pairs into [`SchemaBatch`].
    pub fn materialize_values(
        user_changes: impl IntoIterator<Item = (SchemaKey, Option<SchemaValue>)>,
        kernel_changes: impl IntoIterator<Item = (SchemaKey, Option<SchemaValue>)>,
        root_hash: SchemaValue,
        version: SlotNumber,
    ) -> anyhow::Result<StateChanges> {
        let mut batch = SchemaBatch::default();
        let mut has_kernel_been_updated = false;
        let mut has_user_been_updated = false;
        let mut metric = StateMaterializationMetrics::new(version.get());
        let mut kernel_batch = VersionedSchemaBatch::default();
        let mut user_batch = VersionedSchemaBatch::default();

        // We always .put and not .delete to keep archival data.
        for (key, value) in kernel_changes {
            metric.inc_kernel_items();
            metric.track_key_value_size(&key, &value);
            kernel_batch.put_versioned(Arc::new(key), value);
            has_kernel_been_updated = true;
        }
        for (key, value) in user_changes {
            metric.inc_user_items();
            metric.track_key_value_size(&key, &value);
            user_batch.put_versioned(Arc::new(key), value);
            has_user_been_updated = true;
        }
        if has_user_been_updated && !has_kernel_been_updated {
            anyhow::bail!(
                "User namespace got updated without kernel namespace, but always should be."
            );
        }

        tracing::trace!(
            %version,
            root_hash = %hex::encode(&root_hash),
            "Materialized root hash"
        );
        batch.put::<StateRootHashes>(&(version, STATE_ROOT_HASH_SINGLETON), &root_hash)?;

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(metric);
        });

        Ok(StateChanges {
            user: Arc::new(user_batch),
            kernel: Arc::new(kernel_batch),
            other: Arc::new(batch),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_manager::FlatStateDb;

    #[test]
    fn verify_last_version_bumped_properly() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = FlatStateDb::new(tempdir.path().to_path_buf()).unwrap();

        let key1 = b"AAA";
        let key2 = b"BBB";

        let writes = vec![
            vec![(key2.to_vec(), Some(vec![1, 1, 1]))],
            vec![(key1.to_vec(), Some(vec![2, 2, 2]))],
            vec![(key1.to_vec(), Some(vec![3, 3, 3]))],
            vec![(key1.to_vec(), Some(vec![4, 4, 4]))],
        ];
        for (idx, kernel_writes) in writes.into_iter().enumerate() {
            let historical_state = HistoricalStateReader::new_empty(&rocksdb);
            let slot_number = SlotNumber::new(idx as u64);
            assert_eq!(slot_number.checked_sub(1), historical_state.last_version());
            assert_eq!(slot_number, historical_state.get_next_version());

            let root_hash = idx.to_be_bytes().to_vec();

            let changes = HistoricalStateReader::materialize_values(
                vec![],
                kernel_writes,
                root_hash,
                slot_number,
            )
            .unwrap();
            rocksdb.commit(changes).unwrap();
        }
    }

    #[test]
    fn test_no_bound_on_passed_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path();
        let rocksdb = FlatStateDb::new(db_path.to_path_buf()).unwrap();

        // Create two independent readers on the same database.
        let reader1 = HistoricalStateReader::new_empty(&rocksdb);
        let reader2 = HistoricalStateReader::new_empty(&rocksdb);

        // --- First set of changes (version 0) ---
        let version0 = SlotNumber::new(0);
        assert_eq!(reader1.get_next_version(), version0);
        assert_eq!(reader2.get_next_version(), version0);

        let root_hash0 = vec![1; 32];
        let changes0 = HistoricalStateReader::materialize_values(
            vec![],
            vec![(b"key1".to_vec(), Some(b"value1".to_vec()))],
            root_hash0.clone(),
            version0,
        )
        .unwrap();
        rocksdb.commit(changes0).unwrap();
        assert_eq!(reader1.get_next_version(), version0);
        assert_eq!(reader2.get_next_version(), version0);

        // Both readers should see the new latest version and be able to query the root hash.
        assert_eq!(
            reader1.get_serialized_root_hash(version0).unwrap(),
            Some(root_hash0.clone())
        );
        assert_eq!(
            reader2.get_serialized_root_hash(version0).unwrap(),
            Some(root_hash0.clone())
        );

        // --- Second set of changes (version 1) ---
        let version1 = SlotNumber::new(1);
        let root_hash1 = vec![2; 32];
        let changes1 = HistoricalStateReader::materialize_values(
            vec![],
            vec![(b"key2".to_vec(), Some(b"value2".to_vec()))],
            root_hash1.clone(),
            version1,
        )
        .unwrap();
        rocksdb.commit(changes1).unwrap();
        assert_eq!(reader1.get_next_version(), version0);
        assert_eq!(reader2.get_next_version(), version0);

        // Both readers should again see the and be able to query the new root hash.
        assert_eq!(
            reader1.get_serialized_root_hash(version1).unwrap(),
            Some(root_hash1.clone())
        );
        assert_eq!(
            reader2.get_serialized_root_hash(version1).unwrap(),
            Some(root_hash1.clone())
        );

        // They should also still be able to query the old root hash.
        assert_eq!(
            reader1.get_serialized_root_hash(version0).unwrap(),
            Some(root_hash0.clone())
        );
        assert_eq!(
            reader2.get_serialized_root_hash(version0).unwrap(),
            Some(root_hash0)
        );
    }

    #[test]
    fn test_unbound_last_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = FlatStateDb::new(tempdir.path().to_path_buf()).unwrap();

        let reader1 = HistoricalStateReader::new_empty(&rocksdb);
        let reader2 = HistoricalStateReader::new_empty(&rocksdb);

        // Initially, both should see no last version
        assert_eq!(reader1.last_version(), None);
        assert_eq!(reader2.last_version(), None);
        assert_eq!(reader1.last_version_unbound().unwrap(), SlotNumber::GENESIS);
        assert_eq!(reader2.last_version_unbound().unwrap(), SlotNumber::GENESIS);

        // Reader1 materializes data at version 0
        let version0 = SlotNumber::new(0);
        let changes0 = HistoricalStateReader::materialize_values(
            vec![],
            vec![(b"key1".to_vec(), Some(b"value1".to_vec()))],
            vec![1; 32],
            version0,
        )
        .unwrap();
        rocksdb.commit(changes0).unwrap();

        // Reader1's bound version stays the same, but unbound sees the update
        assert_eq!(reader1.last_version(), None);
        assert_eq!(reader1.last_version_unbound().unwrap(), SlotNumber::GENESIS);

        // Reader2 also sees the update through unbound, but not through bound
        assert_eq!(reader2.last_version(), None);
        assert_eq!(reader2.last_version_unbound().unwrap(), SlotNumber::GENESIS);

        // Reader2 materializes data at version 1
        let version1 = SlotNumber::new(1);
        let changes1 = HistoricalStateReader::materialize_values(
            vec![],
            vec![(b"key2".to_vec(), Some(b"value2".to_vec()))],
            vec![2; 32],
            version1,
        )
        .unwrap();
        rocksdb.commit(changes1).unwrap();

        // Both readers see the latest version through unbound
        assert_eq!(reader1.last_version_unbound().unwrap(), version1);
        assert_eq!(reader2.last_version_unbound().unwrap(), version1);

        // But their bound versions remain unchanged
        assert_eq!(reader1.last_version(), None);
        assert_eq!(reader2.last_version(), None);
    }
}
