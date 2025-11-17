use crate::evm::evm_test_helper::start_node;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::test_helpers::DemoRollupSpec;
use futures::StreamExt;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockDemoRollup;
use sov_demo_rollup::MockRollupSpec;
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::preferred::TimingOracleConfig;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::TestRollup;

async fn setup_test_rollup() -> TestRollup<MockDemoRollup<Native>> {
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

    start_node(config, 0, Some(EVM_EXTENSION), Some(time_stamp_config)).await
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_oracle_timestamp() {
    let test_rollup = setup_test_rollup().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut event_subscription = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("ChainState/OracleTimeUpdated")
        .await
        .unwrap();

    for _ in 0..3 {
        // Wait for all the time oracle events or fail with timeout.
        tokio::time::timeout(std::time::Duration::from_secs(1), event_subscription.next())
            .await
            .unwrap();
    }
}
