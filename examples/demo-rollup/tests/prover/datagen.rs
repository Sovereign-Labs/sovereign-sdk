use std::env;

use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_demo_rollup::MockDemoRollup;
use sov_mock_da::{MockAddress, MockBlock, MockDaService};
use sov_modules_api::execution_mode::{Native, WitnessGeneration};
use sov_rollup_interface::node::da::DaService;
use sov_test_utils::generators::bank::BankMessageGenerator;
use sov_test_utils::generators::BlobBuildingCtx;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::MessageGenerator;

type S = sov_modules_api::configurable_spec::ConfigurableSpec<
    sov_mock_da::MockDaSpec,
    sov_risc0_adapter::Risc0,
    sov_mock_zkvm::MockZkvm,
    sov_address::MultiAddressEvm,
    WitnessGeneration,
>;

const DEFAULT_BLOCKS: u64 = 10;
const DEFAULT_TXNS_PER_BLOCK: u64 = 100;

pub async fn get_blocks_from_da(mode: BlobBuildingCtx) -> anyhow::Result<Vec<MockBlock>> {
    let txns_per_block = match env::var("SOV_BENCH_TXNS_PER_BLOCK") {
        Ok(txns_per_block) => txns_per_block.parse::<u64>()?,
        Err(_) => {
            println!("SOV_BENCH_TXNS_PER_BLOCK not set, using default");
            DEFAULT_TXNS_PER_BLOCK
        }
    };

    let block_cnt = match env::var("SOV_BENCH_BLOCKS") {
        Ok(block_cnt_str) => block_cnt_str.parse::<u64>()?,
        Err(_) => {
            println!("SOV_BENCH_BLOCKS not set, using default");
            DEFAULT_BLOCKS
        }
    };

    let da_service = MockDaService::new(MockAddress::default());
    let mut blocks = vec![];

    let private_key_and_address: PrivateKeyAndAddress<S> =
        read_private_key::<S>("minter_private_key.json");

    let (create_token_message_gen, transfer_message_gen) =
        BankMessageGenerator::generate_token_and_random_transfers(
            txns_per_block,
            private_key_and_address.private_key,
        );

    let blob = create_token_message_gen.create_blobs::<<MockDemoRollup<Native> as sov_modules_rollup_blueprint::RollupBlueprint<Native>>::Runtime>(&mode);
    da_service.send_transaction(&blob).await.await??;
    let block1 = da_service.get_block_at(1).await?;
    blocks.push(block1);

    for i in 0..block_cnt {
        let blob = transfer_message_gen.create_blobs::<<MockDemoRollup<Native> as sov_modules_rollup_blueprint::RollupBlueprint<Native>>::Runtime>(&mode);
        da_service.send_transaction(&blob).await.await??;
        let blocki = da_service.get_block_at(2 + i).await?;
        blocks.push(blocki);
    }

    Ok(blocks)
}
