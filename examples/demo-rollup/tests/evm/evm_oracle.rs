use crate::evm::evm_test_helper::create_simple_storage_client;
use crate::evm::evm_test_helper::start_node;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use crate::test_helpers::DemoRollupSpec;
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter};
use futures::StreamExt;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockDemoRollup;
use sov_demo_rollup::MockRollupSpec;
use sov_eth_client::SimpleStorageClient;
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::preferred::TimingOracleConfig;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::TestRollup;

async fn setup_test_rollup() -> (
    TestRollup<MockDemoRollup<Native>>,
    SimpleStorageClient,
    alloy_primitives::Address,
) {
    let host_args = mock_da_risc0_host_args();
    let config = get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(host_args);

    let priv_key = read_private_key::<DemoRollupSpec>("tx_signer_private_key.json").private_key;
    let private_key_hex = priv_key.as_hex();

    let time_stamp_config = TimingOracleConfig {
        priority_fee_percentage: 0,
        max_fee: 1_000_000,
        interval_millis: 50,
        private_key_hex: Some(private_key_hex),
    };

    let test_rollup = start_node(config, 0, Some(EVM_EXTENSION), Some(time_stamp_config)).await;
    test_rollup.wait_for_next_blocks(10).await;
    let evm_client = create_simple_storage_client(test_rollup.http_addr, SENDER_PRIV_KEY).await;

    let contract_address = evm_client.alloy_deploy_contract().await;

    (test_rollup, evm_client, contract_address)
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_oracle_timestamp() {
    let (test_rollup, evm_client, contract_addr) = setup_test_rollup().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    test_rollup.pause_preferred_batches().await;

    let start_block = evm_client
        .alloy_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    evm_client.alloy_emit_logs(contract_addr, 0, 2).await;

    let mut event_subscription = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("ChainState/OracleTimeUpdated")
        .await
        .unwrap();

    for _ in 0..3 {
        // Wait for time oracle events or fail with timeout.
        tokio::time::timeout(std::time::Duration::from_secs(1), event_subscription.next())
            .await
            .unwrap();
    }
    evm_client.alloy_emit_logs(contract_addr, 0, 2).await;

    test_rollup.resume_preferred_batches().await;
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
        // - The transaction containing events 2 and 3 happens after the one containing events 0 and 1,
        //   so events 2 and 3 should have a later timestamp.

        // But timestamps
        let logs = evm_client.get_logs_with_timestamp(&filter).await;
        assert_eq!(logs[0].time_executed_ms, logs[1].time_executed_ms);
        assert_eq!(logs[2].time_executed_ms, logs[3].time_executed_ms);
        assert!(logs[2].time_executed_ms > logs[0].time_executed_ms);
    }
}
