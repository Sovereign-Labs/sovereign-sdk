use sov_rollup_interface::services::batch_builder::BatchBuilder;

pub struct EthBatchBuilder {}

impl BatchBuilder for EthBatchBuilder {
    fn accept_tx(&mut self, tx: Vec<u8>) -> anyhow::Result<()> {
        todo!()
    }

    fn get_next_blob(&mut self) -> anyhow::Result<Vec<Vec<u8>>> {
        todo!()
    }
}
