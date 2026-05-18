//! Defines namespaces that are used to partition the state of the rollup.

use rockbound::schema::ColumnFamilyName;

pub use crate::schema::namespace::Namespace;

// The `_jmt_values` suffix on the STATE_VALUES_TABLE_NAME constants is a
// historical artifact: the column families originally held JMT-encoded values
// but now hold NOMT-encoded values. The names are kept as-is to preserve
// backward compatibility with existing on-disk databases — renaming would
// invalidate every deployed rollup's state directory.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The Kernel namespace. Has access to the core state information of the rollup
pub struct KernelNamespace;

impl Namespace for KernelNamespace {
    const STATE_VALUES_TABLE_NAME: ColumnFamilyName = "kernel_jmt_values";

    const PRUNING_COLUMN_FAMILY: ColumnFamilyName = "kernel_pruning";

    const VERSION_METADATA_COLUMN: ColumnFamilyName = "kernel_version_metadata";

    const HISTORICAL_COLUMN_FAMILY: ColumnFamilyName = "kernel_historical_state_values";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The User namespace. Has access to the user space and the public information of the rollup.
pub struct UserNamespace;

impl Namespace for UserNamespace {
    const STATE_VALUES_TABLE_NAME: ColumnFamilyName = "user_jmt_values";

    const PRUNING_COLUMN_FAMILY: ColumnFamilyName = "user_pruning";

    const VERSION_METADATA_COLUMN: ColumnFamilyName = "user_version_metadata";

    const HISTORICAL_COLUMN_FAMILY: ColumnFamilyName = "user_historical_state_values";
}
