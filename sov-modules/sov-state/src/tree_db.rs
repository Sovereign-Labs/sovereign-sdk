use jmt::{storage::TreeReader, OwnedValue};
use sovereign_db::state_db::StateDB;
use sovereign_sdk::core::traits::Witness;

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

    // pub fn put_preimage(&self, key_hash: KeyHash, key: &Vec<u8>) -> Result<(), anyhow::Error> {
    //     self.state_db.put_preimage(key_hash, key)
    // }

    // pub fn get_next_version(&self) -> Version {
    //     self.state_db.get_next_version()
    // }

    // pub fn inc_next_version(&self) {
    //     self.state_db.inc_next_version()
    // }
}

impl<'a, W: Witness> TreeReader for TreeReadLogger<'a, W> {
    fn get_node_option(
        &self,
        node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        let node_opt = self.state_db.get_node_option(node_key)?;
        self.witness
            .add_hint(node_opt.as_ref().map(|node| node.encode().unwrap()));
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
