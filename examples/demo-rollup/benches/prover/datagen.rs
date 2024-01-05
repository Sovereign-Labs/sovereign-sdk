use sov_data_generators::{bank_data::BankMessageGenerator, MessageGenerator};
use sov_demo_rollup::MockDemoRollup;
use sov_mock_da::{MockAddress, MockBlock, MockDaService};
use sov_rollup_interface::services::da::DaService;

const DEFAULT_TRANSFER_NUM: u64 = 100;

pub async fn get_bench_blocks() -> Vec<MockBlock> {
    let da_service = MockDaService::new(MockAddress::default());
    let mut blocks = vec![];

    let create_token_message_gen = BankMessageGenerator::default_generate_create_token();
    let blob = create_token_message_gen.create_blobs::<<MockDemoRollup as sov_modules_rollup_blueprint::RollupBlueprint>::NativeRuntime>();
    da_service.send_transaction(&blob).await.unwrap();
    let block1 = da_service.get_block_at(1).await.unwrap();
    blocks.push(block1);

    let create_transfer_message_gen =
        BankMessageGenerator::default_generate_random_transfers(DEFAULT_TRANSFER_NUM);
    for i in 0..10 {
        let blob = create_transfer_message_gen.create_blobs::<<MockDemoRollup as sov_modules_rollup_blueprint::RollupBlueprint>::NativeRuntime>();
        da_service.send_transaction(&blob).await.unwrap();
        let blocki = da_service.get_block_at(2 + i).await.unwrap();
        blocks.push(blocki);
    }

    blocks
}
