use std::time::Duration;

use crate::evm::evm_test_helper::setup;
use sov_eth_client::TestClient;
use sov_mock_da::storable::service::StorableMockDaService;
use tokio::time::sleep;

use super::evm_test_helper;

#[tokio::test(flavor = "multi_thread")]
async fn evm_tx_tests_instant_finality() -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(300), evm_tx_test(0)).await?
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_tx_tests_non_instant_finality() -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(300), evm_tx_test(3)).await?
}

async fn evm_tx_test(finalization_blocks: u32) -> anyhow::Result<()> {
    let (test_rollup, test_client, _) = setup(finalization_blocks).await;

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
