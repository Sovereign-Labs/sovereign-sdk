use std::collections::BTreeMap;
use std::fmt::Debug;

use anyhow::{ensure, Context};
use jmt::storage::{HasPreimage, NodeBatch, TreeReader};
use jmt::{KeyHash, Version};
use rockbound::cache::delta_reader::DeltaReader;
use rockbound::{SchemaBatch, SchemaKey};
use sov_rollup_interface::common::SlotNumber;

use crate::namespaces::{KernelNamespace, Namespace, UserNamespace};
use crate::schema::namespace::{JmtNodes, KeyHashToKey, StateValues};
use crate::{ensure_version_is_correct, DbOptions};

/// Data that will be saved in the DB.
#[derive(Default)]
pub struct StateTreeChanges {
    /// Node batch
    pub node_batch: NodeBatch,
    /// The original writes that will be stored in the DB.
    pub original_write_values: BTreeMap<(u64, KeyHash), Option<jmt::OwnedValue>>,
}

/// A typed wrapper around the [`DeltaReader`] for materializing rollup state.
#[derive(Debug, Clone)]
pub struct StateDb {
    /// The underlying [`DeltaReader`] correctly routes requests to previous snapshots and/or [`rockbound::DB`]
    db: DeltaReader,
    /// The [`Version`] that will be used for the next batch of writes to the DB
    /// This [`Version`] is also used for querying data,
    /// so if this instance of StateDb is used as read-only, it won't see newer data.
    next_version: SlotNumber,
}

impl StateDb {
    const DB_PATH_SUFFIX: &'static str = "state";
    const DB_NAME: &'static str = "state-db";

    /// Create a new instance of [`StateDb`] from a given [`DeltaReader`].
    pub fn with_delta_reader(reader: DeltaReader) -> anyhow::Result<Self> {
        let next_version = Self::next_version_from(&reader)?;
        Ok(Self {
            db: reader,
            next_version,
        })
    }

    /// Returns the associated JMT handler for a given namespace
    pub fn get_jmt_handler<N: Namespace>(&self) -> JmtHandler<N> {
        JmtHandler {
            state_db: self,
            phantom: Default::default(),
        }
    }

    /// Get the next version from the database snapshot
    fn next_version_from(reader: &DeltaReader) -> anyhow::Result<SlotNumber> {
        let kernel_last_key_value = reader.get_largest::<JmtNodes<KernelNamespace>>()?;
        let kernel_largest_version = kernel_last_key_value.map(|(k, _)| k.version());

        let user_last_key_value = reader.get_largest::<JmtNodes<UserNamespace>>()?;
        let user_largest_version = user_last_key_value.map(|(k, _)| k.version());

        ensure!(
            kernel_largest_version == user_largest_version,
            "Kernel and User namespaces have different largest versions: kernel={:?}, user={:?}",
            kernel_largest_version,
            user_largest_version
        );

        Ok(match user_largest_version {
            None => SlotNumber::GENESIS,
            Some(existing_version) => SlotNumber::new_dangerous(existing_version)
                .checked_add(1)
                .expect("JMT Version overflow. Is is over"),
        })
    }

    /// [`DbOptions`] for [`StateDb`].
    pub fn get_rockbound_options() -> DbOptions {
        DbOptions {
            name: Self::DB_NAME,
            path_suffix: Self::DB_PATH_SUFFIX,
            columns: UserNamespace::get_table_names()
                .into_iter()
                .chain(KernelNamespace::get_table_names())
                .collect(),
        }
    }

    fn materialize_preimages_namespace<'a, N: Namespace>(
        items: impl IntoIterator<Item = (KeyHash, &'a SchemaKey)>,
    ) -> anyhow::Result<SchemaBatch> {
        let mut batch = SchemaBatch::new();
        for (key_hash, key) in items.into_iter() {
            batch.put::<KeyHashToKey<N>>(&key_hash.0, key)?;
        }
        Ok(batch)
    }

    /// Materializes the preimage of a hashed key into the returned [`SchemaBatch`].
    /// Note that the preimage is not checked for correctness,
    /// since the [`StateDb`] is unaware of the hash function used by the JMT.
    pub fn materialize_preimages<'a>(
        kernel_items: impl IntoIterator<Item = (KeyHash, &'a SchemaKey)>,
        user_items: impl IntoIterator<Item = (KeyHash, &'a SchemaKey)>,
    ) -> anyhow::Result<SchemaBatch> {
        let mut kernel_batch =
            Self::materialize_preimages_namespace::<KernelNamespace>(kernel_items)?;
        let user_batch = Self::materialize_preimages_namespace::<UserNamespace>(user_items)?;
        kernel_batch.merge(user_batch);

        Ok(kernel_batch)
    }

    /// Get the current value of the `next_version` counter
    pub fn get_next_version(&self) -> SlotNumber {
        self.next_version
    }

    /// The last version used for writes.
    pub fn last_version(&self) -> Option<SlotNumber> {
        self.next_version.checked_sub(1)
    }

    /// Get an optional value from the database, given a version and a key hash.
    pub fn get_value_option_by_key<N: Namespace>(
        &self,
        version: SlotNumber,
        key: &SchemaKey,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        // Defense programming
        if version >= self.next_version {
            // The future is not set.
            return Ok(None);
        }
        ensure_version_is_correct(
            key,
            version,
            self.db.get_prev::<StateValues<N>>(&(key, version))?,
        )
    }

    fn build_schema_batch_from_changes<N: Namespace>(
        &self,
        data_to_materialize: &StateTreeChanges,
        latest_preimages: Option<&SchemaBatch>,
    ) -> anyhow::Result<SchemaBatch> {
        if data_to_materialize.node_batch.nodes().is_empty() {
            anyhow::bail!(
                "NodeBatch {} should have at least one Node",
                std::any::type_name::<N>()
            );
            // Otherwise next_version won't be properly upgraded.
        }
        // We always .put and not .delete to keep archival data.

        let mut batch = SchemaBatch::new();
        for (node_key, node) in data_to_materialize.node_batch.nodes() {
            batch.put::<JmtNodes<N>>(node_key, node)?;
        }

        for ((version, key_hash), value) in &data_to_materialize.original_write_values {
            let key_preimage = if let Some(latest_preimages) = latest_preimages {
                latest_preimages.get_value::<KeyHashToKey<N>>(&key_hash.0)?
            } else {
                None
            };
            let key_preimage = match key_preimage {
                Some(v) => v,
                None => self
                    .db
                    .get::<KeyHashToKey<N>>(&key_hash.0)?
                    .ok_or(anyhow::format_err!(
                        "Could not find preimage for key hash {key_hash:?}. Has `StateDb::put_preimage` been called for this key?"
                    ))?
            };
            let key = (key_preimage, SlotNumber::new_dangerous(*version));
            batch.put::<StateValues<N>>(&key, value)?;
        }

        Ok(batch)
    }

    /// Converts [`jmt::storage::NodeBatch`]es into serialized [`SchemaBatch`].
    /// Optional `latest_preimages` is for preimages from the current slot,
    /// which might not be available in the [`StateDb`] yet.
    /// Preimages should contain values for both namespaces.
    /// Preimages batch is included in the output, so no need to write it separately.
    pub fn materialize(
        &self,
        kernel_data_to_materialize: &StateTreeChanges,
        user_data_to_materialize: &StateTreeChanges,
        latest_preimages: Option<SchemaBatch>,
    ) -> anyhow::Result<SchemaBatch> {
        let mut kernel_materialized = self.build_schema_batch_from_changes::<KernelNamespace>(
            kernel_data_to_materialize,
            latest_preimages.as_ref(),
        )?;
        let user_materialized = self.build_schema_batch_from_changes::<UserNamespace>(
            user_data_to_materialize,
            latest_preimages.as_ref(),
        )?;

        kernel_materialized.merge(user_materialized);
        if let Some(latest_preimages) = latest_preimages {
            kernel_materialized.merge(latest_preimages);
        }

        Ok(kernel_materialized)
    }
}

/// A simple wrapper around [`StateDb`] that implements [`TreeReader`] for a given namespace.
#[derive(Debug)]
pub struct JmtHandler<'a, N: Namespace> {
    state_db: &'a StateDb,
    phantom: std::marker::PhantomData<N>,
}

/// Default implementations of [`TreeReader`] for [`StateDb`]
impl<N: Namespace> TreeReader for JmtHandler<'_, N> {
    fn get_node_option(
        &self,
        node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        self.state_db.db.get::<JmtNodes<N>>(node_key)
    }

    fn get_value_option(
        &self,
        version: Version,
        key_hash: KeyHash,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        let key_opt = self
            .state_db
            .db
            .get::<KeyHashToKey<N>>(&key_hash.0)
            .context("Preimage for key is not found")?;

        if let Some(key) = key_opt {
            self.state_db
                .get_value_option_by_key::<N>(SlotNumber::new_dangerous(version), &key)
        } else {
            Ok(None)
        }
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        todo!("StateDb does not support [`TreeReader::get_rightmost_leaf`] yet")
    }
}

impl<N: Namespace> HasPreimage for JmtHandler<'_, N> {
    fn preimage(&self, key_hash: KeyHash) -> anyhow::Result<Option<Vec<u8>>> {
        self.state_db.db.get::<KeyHashToKey<N>>(&key_hash.0)
    }
}
