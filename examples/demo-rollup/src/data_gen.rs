use std::sync::Arc;
use jupiter::verifier::address::CelestiaAddress;
use crate::rng_xfers::RngDaService;
use sov_rollup_interface::mocks::{TestBlob, TestBlock, TestBlockHeader, TestHash};
use sov_rollup_interface::services::da::DaService;

pub fn generate_blocks(start_height: u64, end_height: u64) -> (Vec<TestBlock>, Vec<Vec<TestBlob<CelestiaAddress>>>) {
    let da_service = Arc::new(RngDaService::new());

    // data generation
    let mut blobs = vec![];
    let mut blocks = vec![];

    for height in start_height..end_height {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        let filtered_block = TestBlock {
            curr_hash: barray,
            header: TestBlockHeader {
                prev_hash: TestHash([0u8; 32]),
            },
            height,
        };
        blocks.push(filtered_block.clone());

        let blob_txs = da_service.extract_relevant_txs(&filtered_block);
        blobs.push(blob_txs.clone());
    }
    (blocks, blobs)
}