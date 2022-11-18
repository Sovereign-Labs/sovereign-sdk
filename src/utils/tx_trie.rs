use jmt::mock::MockTreeStore;
use jmt::proof::SparseMerkleRangeProof;
use jmt::storage::TreeUpdateBatch;
use jmt::RootHash;

use crate::core::traits::Transaction;

/// Creates a jellyfish merkle tree out of the hashes of the provided. The key is the transaction index,
/// and the value is the hash. The zero hash is inserted into the tree with the key `txs.len()` to allow for
/// easy verification of completeness.
pub fn build_tx_trie<T: Transaction>(txs: &Vec<T>) -> (RootHash, TreeUpdateBatch, MockTreeStore) {
    let mut store = jmt::mock::MockTreeStore::new(true);
    let jelly = jmt::JellyfishMerkleTree::new(&mut store);
    let (root, update) = jelly
        .put_value_set(
            txs.iter()
                .enumerate()
                .map(|(idx, tx)| {
                    (
                        (idx as u32).to_le_bytes().into(),
                        tx.hash().as_ref().to_vec(),
                    )
                })
                .chain(std::iter::once((
                    txs.len().to_le_bytes().into(),
                    [0u8; 32].to_vec(),
                )))
                .collect(),
            0,
        )
        .expect("Trie update is valid");
    (root, update, store)
}

pub fn get_tx_proof<T: Transaction>(txs: &Vec<T>) -> SparseMerkleRangeProof {
    let (_, batch, mut store) = build_tx_trie(txs);
    store
        .write_tree_update_batch(batch)
        .expect("batch is valid");
    jmt::JellyfishMerkleTree::new(&mut store)
        .get_range_proof(txs.len().to_le_bytes().into(), 0)
        .expect("root must exist")
}
