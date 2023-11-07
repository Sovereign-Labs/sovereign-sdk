use sov_mock_da::{MockAddress, MockBlock, MockDaService};
use sov_rng_da_service::{generate_create, generate_transfers};
use sov_rollup_interface::services::da::DaService;

pub async fn get_bench_blocks() -> Vec<MockBlock> {
    let da_service = MockDaService::new(MockAddress::default());

    let mut blocks = vec![];
    let blob = generate_create(0);
    da_service.send_transaction(&blob).await.unwrap();
    let block1 = da_service.get_finalized_at(0).await.unwrap();
    blocks.push(block1);

    let blob = generate_transfers(3, 1);
    da_service.send_transaction(&blob).await.unwrap();
    let block2 = da_service.get_finalized_at(0).await.unwrap();
    blocks.push(block2);

    let blob = generate_transfers(10, 4);
    da_service.send_transaction(&blob).await.unwrap();
    let block2 = da_service.get_finalized_at(0).await.unwrap();
    blocks.push(block2);

    blocks
}
