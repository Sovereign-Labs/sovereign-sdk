use std::collections::VecDeque;

#[derive(Default)]
pub struct EthBatchBuilder {
    mempool: VecDeque<Vec<u8>>,
}

impl EthBatchBuilder {
    fn make_blob(&mut self) -> Vec<Vec<u8>> {
        let mut txs = Vec::new();

        while let Some(raw_tx) = self.mempool.pop_front() {
            txs.push(raw_tx);
        }
        txs
    }

    /// Adds `txs` to the mempool and attempts to create a blob with a minimum size of `min_blob_size`.
    pub fn add_transactions_and_get_next_blob(
        &mut self,
        min_blob_size: Option<usize>,
        txs: Vec<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        for tx in txs {
            self.mempool.push_back(tx);
        }
        if let Some(min_blob_size) = min_blob_size {
            if self.mempool.len() >= min_blob_size {
                return self.make_blob();
            }
        }
        Vec::default()
    }
}
