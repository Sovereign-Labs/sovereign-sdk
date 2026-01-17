use crate::evm::evm_test_helper::create_simple_storage_client;
use crate::evm::evm_test_helper::start_node;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter};
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockDemoRollup;
use sov_demo_rollup::MockRollupSpec;
use sov_eth_client::SimpleStorageClient;
use sov_rollup_interface::execution_mode::Native;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use sov_test_utils::test_rollup::TestRollup;

async fn setup_test_rollup() -> (
    TestRollup<MockDemoRollup<Native>>,
    SimpleStorageClient,
    alloy_primitives::Address,
) {
    let host_args = mock_da_risc0_host_args();
    let config = get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(host_args);
    let test_rollup = start_node(config, 0, Some(EVM_EXTENSION), None).await;
    test_rollup.wait_for_next_blocks(10).await;
    let evm_client = create_simple_storage_client(test_rollup.http_addr, SENDER_PRIV_KEY).await;

    let contract_address = evm_client.alloy_deploy_contract().await;

    (test_rollup, evm_client, contract_address)
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_oracle_timestamp() {
    let (test_rollup, evm_client, contract_addr) = setup_test_rollup().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let start_block = evm_client
        .eth_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    evm_client.alloy_emit_logs(contract_addr, 0, 6).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    evm_client.alloy_emit_logs(contract_addr, 0, 2).await;

    test_rollup.wait_for_next_blocks(1).await;

    // Check logs from evm txs.
    {
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest);

        // There are four EVM events across two transactions:
        //
        // - Events 0 and 1 occur in the same transaction, so they should share the same timestamp.
        // - Events 2 and 3 occur in another transaction, so they should also share a timestamp.

        // Timestamps should be consistent within a tx and non-decreasing across txs.
        let logs = evm_client.get_logs_with_timestamp(&filter).await;
        assert_eq!(logs[0].time_executed_ms, logs[1].time_executed_ms);
        assert_eq!(logs[2].time_executed_ms, logs[3].time_executed_ms);
        assert!(logs[2].time_executed_ms >= logs[0].time_executed_ms);
    }
}
