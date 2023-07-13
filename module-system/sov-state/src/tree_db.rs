use borsh::BorshSerialize;
use jmt::storage::TreeReader;
use jmt::OwnedValue;
use sov_db::state_db::StateDB;

use crate::witness::Witness;

pub struct TreeReadLogger<'a, W> {
    state_db: StateDB,
    witness: &'a W,
}

impl<'a, W: Witness> TreeReadLogger<'a, W> {
    /// Creates a tree read logger wrapping the provided StateDB.
    /// The logger is recording by default
    pub fn with_db_and_witness(db: StateDB, witness: &'a W) -> Self {
        Self {
            state_db: db,
            witness,
        }
    }
}

impl<'a, W: Witness> TreeReader for TreeReadLogger<'a, W> {
    fn get_node_option(
        &self,
        node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        let node_opt = self.state_db.get_node_option(node_key)?;
        self.witness
            .add_hint(node_opt.as_ref().map(|node| node.try_to_vec().unwrap()));
        Ok(node_opt)
    }

    fn get_value_option(
        &self,
        max_version: jmt::Version,
        key_hash: jmt::KeyHash,
    ) -> anyhow::Result<Option<OwnedValue>> {
        let value_opt = self.state_db.get_value_option(max_version, key_hash)?;
        self.witness.add_hint(value_opt.clone());
        Ok(value_opt)
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        unimplemented!()
    }
}
