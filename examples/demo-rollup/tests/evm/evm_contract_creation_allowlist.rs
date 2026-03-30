use crate::evm::evm_test_helper::alloy_client_with_signer;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SECONDARY_SENDER_PRIV_KEY;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use alloy_rpc_types_eth::error::EthRpcErrorCode;
use sov_evm_test_utils::SimpleStorage;

#[tokio::test(flavor = "multi_thread")]
async fn allowed() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client_with_signer(rollup.http_addr, SENDER_PRIV_KEY);

    let _ = SimpleStorage::deploy(client).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn denied() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client_with_signer(rollup.http_addr, SECONDARY_SENDER_PRIV_KEY);

    let err = SimpleStorage::deploy(client).await.unwrap_err();
    let rejected_code = EthRpcErrorCode::TransactionRejected.code();
    assert_eq!(
        err.to_string(),
        format!("server returned an error response: error code {rejected_code}: Contract creation is only allowed from allowed addresses. 0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71 is not on the list"));
    Ok(())
}
