use std::fmt::Debug;

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::{SchemaBatch, SchemaKey, SchemaValue};
use sov_rollup_interface::common::SlotNumber;

use crate::metrics::StateMaterializationMetrics;
use crate::namespaces::{KernelNamespace, Namespace, UserNamespace};
use crate::schema::namespace::StateValues;
use crate::schema::tables::StateRootHashes;
use crate::schema::types::StateRootHashId;
use crate::{ensure_version_is_correct, DbOptions};

const STATE_ROOT_HASH_SINGLETON: StateRootHashId = StateRootHashId(0);

/// A typed wrapper around the [`DeltaReader`] for reading materializing historical rollup state.
#[derive(Debug, Clone)]
pub struct HistoricalStateReader {
    /// The underlying [`DeltaReader`] correctly routes requests to previous snapshots and/or [`rockbound::DB`]
    db: DeltaReader,
    /// The [`SlotNumber`] that will be used for the next batch of writes to the DB
    /// This [`SlotNumber`] is also used for querying data,
    /// so if this instance of [`HistoricalStateReader`] is used as read-only, it won't see newer data.
    next_version: SlotNumber,
}

impl HistoricalStateReader {
    const DB_PATH_SUFFIX: &'static str = "historical_state";
    const DB_NAME: &'static str = "historical-state-db";

    /// Create a new instance of [`HistoricalStateReader`] from a given [`DeltaReader`].
    pub fn with_delta_reader(reader: DeltaReader) -> anyhow::Result<Self> {
        let next_version = Self::next_version_from(&reader)?;
        tracing::trace!(?next_version, "Initialized historical state reader");
        Ok(Self {
            db: reader,
            next_version,
        })
    }

    fn last_version_from_reader(reader: &DeltaReader) -> anyhow::Result<Option<SlotNumber>> {
        let last_root_hash = reader.get_largest::<StateRootHashes>()?;
        let last_root_hash_version = last_root_hash.map(|((version, _key), _)| version);

        Ok(last_root_hash_version)
    }

    /// Get the next version from the database snapshot
    fn next_version_from(reader: &DeltaReader) -> anyhow::Result<SlotNumber> {
        let last_root_hash_version = Self::last_version_from_reader(reader)?;

        Ok(match last_root_hash_version {
            None => SlotNumber::GENESIS,
            Some(existing_version) => existing_version
                .checked_add(1)
                .expect("State version overflow. Is is over"),
        })
    }

    /// [`DbOptions`] for [`HistoricalStateReader`].
    pub fn get_rockbound_options() -> DbOptions {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns: UserNamespace::get_table_names()
                .into_iter()
                .chain(KernelNamespace::get_table_names())
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
        Self::last_version_from_reader(&self.db).map(|v| v.unwrap_or(SlotNumber::GENESIS))
    }

    /// Get an optional value from the database, given a version and a key hash.
    pub fn get_value_option_by_key<N: Namespace>(
        &self,
        version: SlotNumber,
        key: &SchemaKey,
    ) -> anyhow::Result<Option<SchemaValue>> {
        // Defense programming
        if version >= self.next_version {
            // The future is not set.
            return Ok(None);
        }
        ensure_version_is_correct(
            key,
            version,
            self.db
                .get_prev::<StateValues<N>>(&(key.to_vec(), version))?,
        )
    }

    /// Get the serialized root hash for a given version.
    pub fn get_serialized_root_hash(
        &self,
        version: SlotNumber,
    ) -> anyhow::Result<Option<SchemaValue>> {
        let key = (version, STATE_ROOT_HASH_SINGLETON);
        let value = self.db.get_prev::<StateRootHashes>(&key)?;
        Ok(value.map(|(_, value)| value))
    }

    /// Collects a sequence of key-value pairs into [`SchemaBatch`].
    pub fn materialize_values(
        user_changes: impl IntoIterator<Item = (SchemaKey, Option<SchemaValue>)>,
        kernel_changes: impl IntoIterator<Item = (SchemaKey, Option<SchemaValue>)>,
        root_hash: SchemaValue,
        version: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::default();
        let mut has_kernel_been_updated = false;
        let mut has_user_been_updated = false;
        let mut metric = StateMaterializationMetrics::new(version.get());

        // We always .put and not .delete to keep archival data.
        for (key, value) in kernel_changes {
            metric.inc_kernel_items();
            metric.track_key_value_size(&key, &value);
            batch.put::<StateValues<KernelNamespace>>(&(key, version), &value)?;
            has_kernel_been_updated = true;
        }
        for (key, value) in user_changes {
            metric.inc_user_items();
            metric.track_key_value_size(&key, &value);
            batch.put::<StateValues<UserNamespace>>(&(key, version), &value)?;
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

        Ok(batch)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn verify_last_version_bumped_properly() {
        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            HistoricalStateReader::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );

        let key1 = b"AAA";
        let key2 = b"BBB";

        let writes = vec![
            vec![(key2.to_vec(), Some(vec![1, 1, 1]))],
            vec![(key1.to_vec(), Some(vec![2, 2, 2]))],
            vec![(key1.to_vec(), Some(vec![3, 3, 3]))],
            vec![(key1.to_vec(), Some(vec![4, 4, 4]))],
        ];
        for (idx, kernel_writes) in writes.into_iter().enumerate() {
            let reader = DeltaReader::new(rocksdb.clone(), Vec::new());
            let historical_state = HistoricalStateReader::with_delta_reader(reader).unwrap();
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
            rocksdb.write_schemas(&changes).unwrap();
        }
    }

    #[test]
    fn test_no_bound_on_passed_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path();
        let rocksdb = Arc::new(
            HistoricalStateReader::get_rockbound_options()
                .default_setup_db_in_path(db_path)
                .unwrap(),
        );

        // Create two independent readers on the same database.
        let reader1 = HistoricalStateReader::with_delta_reader(DeltaReader::new(
            rocksdb.clone(),
            Default::default(),
        ))
        .unwrap();
        let reader2 = HistoricalStateReader::with_delta_reader(DeltaReader::new(
            rocksdb.clone(),
            Default::default(),
        ))
        .unwrap();

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
        rocksdb.write_schemas(&changes0).unwrap();
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
        rocksdb.write_schemas(&changes1).unwrap();
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
        let rocksdb = Arc::new(
            HistoricalStateReader::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );

        let reader1 = HistoricalStateReader::with_delta_reader(DeltaReader::new(
            rocksdb.clone(),
            Default::default(),
        ))
        .unwrap();
        let reader2 = HistoricalStateReader::with_delta_reader(DeltaReader::new(
            rocksdb.clone(),
            Default::default(),
        ))
        .unwrap();

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
        rocksdb.write_schemas(&changes0).unwrap();

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
        rocksdb.write_schemas(&changes1).unwrap();

        // Both readers see the latest version through unbound
        assert_eq!(reader1.last_version_unbound().unwrap(), version1);
        assert_eq!(reader2.last_version_unbound().unwrap(), version1);

        // But their bound versions remain unchanged
        assert_eq!(reader1.last_version(), None);
        assert_eq!(reader2.last_version(), None);
    }
}
