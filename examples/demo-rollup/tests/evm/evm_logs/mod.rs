use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::U256;
use alloy_provider::DynProvider;
use alloy_rpc_types_eth::TransactionReceipt;
use sov_demo_rollup::MockDemoRollup;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::SimpleStorage;
use sov_test_utils::SimpleStorage::SimpleStorageInstance;

mod basic;
mod cursor;
mod filtering;
mod limits;
mod old;
mod subscription;

async fn setup() -> anyhow::Result<(
    DynProvider,
    SimpleStorageInstance<DynProvider>,
    TestRollup<MockDemoRollup<Native>>,
)> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client(rollup.http_addr);
    let contract = SimpleStorage::deploy(client.clone()).await?;
    Ok((client, contract, rollup))
}

async fn emit(
    contract: &SimpleStorageInstance<DynProvider>,
    count: usize,
    topic: usize,
) -> anyhow::Result<TransactionReceipt> {
    let tx = contract
        .emitLogs(U256::from(topic), U256::from(count))
        .send()
        .await?;
    let receipt = tx.get_receipt().await?;
    Ok(receipt)
}
