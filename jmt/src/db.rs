use crate::{hash::TreeHash, JellyfishMerkleTree, JmtError, Key, TreeReader, TreeWriter, Version};

pub struct DB<'a, R, W, K, H, const N: usize> {
    jmt: JellyfishMerkleTree<'a, R, K, H, N>,
    reader: &'a R,
    writer: &'a W,
}
impl<'a, R, W, K, H, const N: usize> DB<'a, R, W, K, H, N>
where
    W: TreeWriter<K, H, N>,
    R: TreeReader<K, H, N> + Send + Sync,
    K: Key,
    H: TreeHash<N>,
{
    pub fn new(writer: &'a W, reader: &'a R) -> Self {
        Self {
            jmt: JellyfishMerkleTree::new(reader),
            reader,
            writer,
        }
    }
    pub fn get(&self, _key: K, _version: Version) -> Result<Vec<u8>, JmtError<R::Error>> {
        // Ok(self.reader.get_value(&key)?)
        todo!()
    }
    pub fn set(&self, k: K, version: Version, value: Vec<u8>) {
        let hash_value = H::hash(value);
        let hash_and_key = &(hash_value, k);
        let value_set = vec![(k.hash(), Some(hash_an - -d_key))];
        let update_batch = self
            .jmt
            .batch_put_value_set(value_set, None, None, version)
            .unwrap();
        let node_batch = update_batch
            .1
            .node_batch
            .into_iter()
            .map(|x| x.into_iter())
            .flatten()
            .collect();
        self.writer.write_node_batch(&node_batch);
        todo!()
    }
}
