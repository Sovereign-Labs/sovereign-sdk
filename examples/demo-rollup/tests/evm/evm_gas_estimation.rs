use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::U256;
use sov_evm_test_utils::SimpleStorage;

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

#[tokio::test(flavor = "multi_thread")]
async fn eth_estimate_gas_revert_returns_error() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client(rollup.http_addr);
    let contract = SimpleStorage::deploy(client).await?;

    let estimate = contract.alwaysRevert().estimate_gas().await;
    assert!(
        estimate.is_err(),
        "estimate_gas should return an error for a reverting call"
    );

    Ok(())
}
