//! Defines namespaces that are used to partition the state of the rollup.

use rockbound::schema::ColumnFamilyName;

pub use crate::schema::namespace::Namespace;

#[derive(Clone, Copy, Debug)]
/// The Kernel namespace. Has access to the core state information of the rollup
pub struct KernelNamespace;

impl Namespace for KernelNamespace {
    const KEY_HASH_TO_KEY_TABLE_NAME: ColumnFamilyName = "kernel_key_hash_to_key";

    const JMT_NODES_TABLE_NAME: ColumnFamilyName = "kernel_jmt_nodes";

    const STATE_VALUES_TABLE_NAME: ColumnFamilyName = "kernel_jmt_values";
}

#[derive(Clone, Copy, Debug)]
/// The User namespace. Has access to the user space and the public information of the rollup.
pub struct UserNamespace;

impl Namespace for UserNamespace {
    const KEY_HASH_TO_KEY_TABLE_NAME: ColumnFamilyName = "user_key_hash_to_key";

    const JMT_NODES_TABLE_NAME: ColumnFamilyName = "user_jmt_nodes";

    const STATE_VALUES_TABLE_NAME: ColumnFamilyName = "user_jmt_values";
}
