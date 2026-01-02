use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::setup_with_simple_storage;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::{Address, U256};
use sov_evm_test_utils::SimpleStorage;

#[tokio::test(flavor = "multi_thread")]
async fn simple_transfer() -> anyhow::Result<()> {
    let (_, test_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;

    let simple_transfer = test_client.make_tx(Some(Address::ZERO), None);
    let gas_estimation = test_client.eth_estimate_gas(simple_transfer).await;
    assert_eq!(gas_estimation, (14_810 / 2) * 3 + 100_000);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_deploy() -> anyhow::Result<()> {
    let (_, test_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;

    let deploy_tx = test_client.make_tx(None, Some(test_client.contract.byte_code()));
    let gas_estimation = test_client.eth_estimate_gas(deploy_tx).await;
    assert_eq!(gas_estimation, (248_136 / 2) * 3 + 100_000);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn big_accessory_state_writes() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client(rollup.http_addr);
    let contract = SimpleStorage::deploy(client).await?;

    let logs_count = U256::from(1_000);
    let call = contract.emitLogs(U256::ZERO, logs_count);
    let gas_estimation = call.estimate_gas().await?;
    let tx = call.send().await?;
    let receipt = tx.get_receipt().await?;
    assert!(receipt.gas_used <= gas_estimation);

    Ok(())
}
