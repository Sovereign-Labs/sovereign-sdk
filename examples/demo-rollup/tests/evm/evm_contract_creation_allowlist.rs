use crate::evm::evm_test_helper::alloy_client_with_signer;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SECONDARY_SENDER_PRIV_KEY;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use sov_test_utils::SimpleStorage;

#[tokio::test(flavor = "multi_thread")]
async fn allowed() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client_with_signer(rollup.http_addr, SENDER_PRIV_KEY);

    let _ = SimpleStorage::deploy(client.clone()).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn denied() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client_with_signer(rollup.http_addr, SECONDARY_SENDER_PRIV_KEY);

    let err = SimpleStorage::deploy(client.clone()).await.unwrap_err();
    assert_eq!(err.to_string(), "server returned an error response: error code -32603: Revm error: Database error: Contract creation is only allowed from allowed addresses. 0x8BaF92120e06aA0791F1C0515Afe11423f56dEf2 is not on the list");
    Ok(())
}
