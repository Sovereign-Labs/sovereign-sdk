use std::time::Duration;

use full_node_configs::sequencer::default_ideal_lag_behind_finalized_slot;
use sov_demo_rollup::{mock_da_risc0_host_args, MockRollupSpec};
use sov_eth_client::TestClient;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Spec;
use sov_modules_macros::config_value;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::initialize_logging;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use tokio::time::sleep;

use super::evm_test_helper;

#[tokio::test(flavor = "multi_thread")]
async fn evm_tx_tests_instant_finality() -> anyhow::Result<()> {
    let rollup_prover_config =
        get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(mock_da_risc0_host_args());
    tokio::time::timeout(
        std::time::Duration::from_secs(300),
        evm_tx_test(0, rollup_prover_config),
    )
    .await?
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_tx_tests_non_instant_finality() -> anyhow::Result<()> {
    initialize_logging();
    tokio::time::timeout(
        std::time::Duration::from_secs(300),
        evm_tx_test(3, RollupProverConfig::Skip),
    )
    .await?
}

async fn evm_tx_test(
    finalization_blocks: u32,
    rollup_prover_config: RollupProverConfig<<MockRollupSpec<Native> as Spec>::InnerZkvm>,
) -> anyhow::Result<()> {
    let chain_id = config_value!("CHAIN_ID");
    // temp_dir is hold here os it is not removed during test run
    let test_rollup = evm_test_helper::start_node(rollup_prover_config, finalization_blocks).await;

    test_rollup
        .da_service
        .produce_n_blocks_now(10 + default_ideal_lag_behind_finalized_slot() as usize)
        .await?;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let (test_client, _) = evm_test_helper::create_test_client(
        test_rollup.http_addr,
        chain_id,
        // This will produce an evm key exist in rollup accounts-genesis.
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    )
    .await;

    sanity_checks(&test_client).await;
    execute_evm_tests(&test_client, &test_rollup.da_service)
        .await
        .unwrap();

    test_rollup.shutdown_sender.send(()).unwrap();
    Ok(())
}

async fn sanity_checks(test_client: &TestClient) {
    let etc_accounts = test_client.eth_accounts().await;
    assert_eq!(vec![test_client.from_addr], etc_accounts);

    let eth_chain_id = test_client.eth_chain_id().await;
    assert_eq!(test_client.chain_id, eth_chain_id);

    // The preferred sequencer ought to have created at least one block.
    let latest_block = test_client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await;
    let earliest_block = test_client
        .eth_get_block_by_number(Some("earliest".to_owned()))
        .await;

    assert!(latest_block.number.unwrap().as_u64() > earliest_block.number.unwrap().as_u64());
    assert!(latest_block.number.unwrap().as_u64() > 0);

    // Nonce should be 0 before any transactions
    let nonce = test_client
        .eth_get_transaction_count(test_client.from_addr)
        .await;
    assert_eq!(0, nonce);

    // Balance should be > 0 in genesis and before any transactions
    let balance = test_client.eth_get_balance(test_client.from_addr).await;
    assert!(balance > ethereum_types::U256::zero());
}

async fn execute_evm_tests(
    client: &TestClient,
    da_service: &StorableMockDaService,
) -> Result<(), Box<dyn std::error::Error>> {
    let initial_block_number = client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await
        .number
        .unwrap()
        .as_u64();

    let contract_address = evm_test_helper::deploy_contract_check(client).await?;

    da_service.produce_n_blocks_now(1).await?;
    sleep(Duration::from_secs(1)).await;

    // Nonce should be 1 after the deployment
    let nonce = client.eth_get_transaction_count(client.from_addr).await;
    assert_eq!(1, nonce);

    // Check that a new block was published
    let latest_block = client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await;

    assert!(latest_block.number.unwrap().as_u64() > initial_block_number);

    let set_arg = 923;
    evm_test_helper::set_value_check(client, contract_address, set_arg).await?;

    // This should just pass without an error
    client.set_value_call(contract_address, set_arg).await?;

    // This call should fail because function does not exist
    let failing_call = client.failing_call(contract_address).await;
    assert!(failing_call.is_err());

    // Create a blob with multiple transactions.
    let values: Vec<u32> = (150..153).collect();
    // Create a blob with multiple transactions.
    evm_test_helper::set_multiple_values_check(client, contract_address, values).await?;

    let value = 103;
    evm_test_helper::set_value_unsigned_check(client, contract_address, value).await?;

    // TODO: reenable this check by figuring out a way to get finer grained control over preferred batch production.
    //evm_test_helper::gas_check(client, da_service, contract_address).await?;

    let first_block = client.eth_get_block_by_number(Some("0".to_owned())).await;
    let second_block = client.eth_get_block_by_number(Some("1".to_owned())).await;

    // assert parent hash works correctly
    assert_eq!(
        first_block.hash.unwrap(),
        second_block.parent_hash,
        "Parent hash should be the hash of the previous block"
    );

    Ok(())
}
