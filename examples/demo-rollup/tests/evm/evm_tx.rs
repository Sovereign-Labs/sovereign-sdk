use super::evm_test_helper;
use crate::evm::evm_test_helper::setup_with_simple_storage;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::{B256, U256};
use futures::StreamExt;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_mock_da::storable::StorableMockDaService;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;

#[tokio::test(flavor = "multi_thread")]
async fn evm_tx_tests_instant_finality() -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(300), evm_tx_test(0)).await?
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_tx_tests_non_instant_finality() -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(300), evm_tx_test(3)).await?
}

async fn evm_tx_test(finalization_blocks: u32) -> anyhow::Result<()> {
    let (test_rollup, test_client, _) =
        setup_with_simple_storage(finalization_blocks, EVM_EXTENSION).await;

    sanity_checks(&test_client).await;
    execute_evm_tests(&test_client, &test_rollup.da_service, &test_rollup)
        .await
        .unwrap();

    test_rollup.shutdown_sender.send(()).unwrap();
    Ok(())
}

async fn sanity_checks(test_client: &SimpleStorageClient) {
    let etc_accounts = test_client.eth_accounts().await;
    assert_eq!(vec![test_client.address()], etc_accounts);

    let earliest_block = test_client
        .eth_get_block_by_number(Some("earliest".to_owned()))
        .await;

    // The preferred sequencer ought to have created at least one block.
    let latest_block = test_client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await;
    let pending_block = test_client
        .eth_get_block_by_number(Some("pending".to_owned()))
        .await;

    assert_eq!(latest_block, pending_block);
    assert_eq!(pending_block.header.base_fee_per_gas, Some(0));
    assert_eq!(pending_block.header.hash, B256::ZERO);
    assert_eq!(earliest_block.header.number, 0);
    assert!(pending_block.header.number > earliest_block.header.number);

    // Nonce should be 0 before any transactions
    let nonce = test_client
        .eth_get_transaction_count(test_client.address())
        .await;
    assert_eq!(0, nonce);

    // Balance should be > 0 in genesis and before any transactions
    let balance = test_client.eth_get_balance(test_client.address()).await;
    assert!(balance > U256::ZERO);
}

async fn execute_evm_tests(
    client: &SimpleStorageClient,
    da_service: &StorableMockDaService,
    rollup: &TestRollup<MockDemoRollup<Native>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut state_update_subscription = rollup.subscribe_state_updates().await?;
    let initial_block_number = client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await
        .header
        .number;

    let contract_address = evm_test_helper::deploy_contract_check(client).await?;

    da_service.produce_n_blocks_now(1).await?;
    state_update_subscription
        .next()
        .await
        .expect("Rollup shutdown unexpectedly")?;

    // Nonce should be 1 after the deployment
    let nonce = client.eth_get_transaction_count(client.address()).await;
    assert_eq!(1, nonce);

    // Send a transaction to ensure that the rollup block is created.
    let set_arg = 923;
    evm_test_helper::set_value_check(client, contract_address, set_arg).await?;

    // Check that a new block was published
    let latest_block = client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await;

    assert!(latest_block.header.number > initial_block_number);

    // This should just pass without an error
    client
        .set_value_call_and_estimate_gas(contract_address, set_arg)
        .await?;

    // This call should fail because function does not exist
    let failing_call = client.failing_call(contract_address).await;
    assert!(failing_call.is_err());

    // Create a blob with multiple transactions.
    let values: Vec<u32> = (150..153).collect();
    // Create a blob with multiple transactions.
    evm_test_helper::set_multiple_values_check(client, contract_address, values).await?;

    // TODO: reenable this check by figuring out a way to get finer grained control over preferred batch production.
    //evm_test_helper::gas_check(client, da_service, contract_address).await?;

    let first_block = client.eth_get_block_by_number(Some("0".to_owned())).await;
    let second_block = client.eth_get_block_by_number(Some("1".to_owned())).await;

    // assert parent hash works correctly
    assert_eq!(
        first_block.header.hash, second_block.header.parent_hash,
        "Parent hash should be the hash of the previous block"
    );

    Ok(())
}
