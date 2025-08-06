//! Defines namespaces that are used to partition the state of the rollup.

use rockbound::schema::ColumnFamilyName;

pub use crate::schema::namespace::Namespace;

#[derive(Clone, Copy, Debug, Default)]
/// The Kernel namespace. Has access to the core state information of the rollup
pub struct KernelNamespace;

impl Namespace for KernelNamespace {
    const KEY_HASH_TO_KEY_TABLE_NAME: ColumnFamilyName = "kernel_key_hash_to_key";

    const JMT_NODES_TABLE_NAME: ColumnFamilyName = "kernel_jmt_nodes";

    const STATE_VALUES_TABLE_NAME: ColumnFamilyName = "kernel_jmt_values";

    const PRUNING_COLUMN_FAMILY: ColumnFamilyName = "kernel_pruning";

    const VERSION_METADATA_COLUMN: ColumnFamilyName = "kernel_version_metadata";

    const HISTORICAL_COLUMN_FAMILY: ColumnFamilyName = "kernel_historical_state_values";
}

#[derive(Clone, Copy, Debug, Default)]
/// The User namespace. Has access to the user space and the public information of the rollup.
pub struct UserNamespace;

impl Namespace for UserNamespace {
    const KEY_HASH_TO_KEY_TABLE_NAME: ColumnFamilyName = "user_key_hash_to_key";

    const JMT_NODES_TABLE_NAME: ColumnFamilyName = "user_jmt_nodes";

    const STATE_VALUES_TABLE_NAME: ColumnFamilyName = "user_jmt_values";

    const PRUNING_COLUMN_FAMILY: ColumnFamilyName = "user_pruning";

    const VERSION_METADATA_COLUMN: ColumnFamilyName = "user_version_metadata";

    const HISTORICAL_COLUMN_FAMILY: ColumnFamilyName = "user_historical_state_values";
}
