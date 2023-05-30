/// BlockBuilder trait is responsible for managing mempool and building batches.
pub trait BatchBuilder {
    /// Accept a new transaction.
    /// Can return error if transaction is invalid or mempool is full.
    fn accept_tx(&self, tx: Vec<u8>) -> anyhow::Result<()>;

    /// Builds a new batch out of transactions in mempool.
    /// Working set is consumed, to emphasize that it is not is not going to be used after this call.
    /// TODO: Do we want to return next_root_hash for the batch?
    fn get_next_blob(&self) -> anyhow::Result<Vec<Vec<u8>>>;
}
