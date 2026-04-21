use alloy::signers::local::PrivateKeySigner;
use alloy_rpc_types_eth::error::EthRpcErrorCode;
use sov_evm_test_utils::SimpleStorage;

use crate::common::{
    alloy_client_with_signer, setup_test_rollup_with_allowlist, EVM_EXTENSION,
    SECONDARY_SENDER_PRIV_KEY, SENDER_PRIV_KEY,
};

fn sender_address() -> alloy_primitives::Address {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    signer.address()
}

#[tokio::test(flavor = "multi_thread")]
async fn allowed() -> anyhow::Result<()> {
    let rollup = setup_test_rollup_with_allowlist(0, EVM_EXTENSION, &[sender_address()]).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client_with_signer(rollup.http_addr, SENDER_PRIV_KEY);

    let _ = SimpleStorage::deploy(client).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn denied() -> anyhow::Result<()> {
    let rollup = setup_test_rollup_with_allowlist(0, EVM_EXTENSION, &[sender_address()]).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client_with_signer(rollup.http_addr, SECONDARY_SENDER_PRIV_KEY);

    let err = SimpleStorage::deploy(client).await.unwrap_err();
    let rejected_code = EthRpcErrorCode::TransactionRejected.code();
    assert_eq!(
        err.to_string(),
        format!("server returned an error response: error code {rejected_code}: Contract creation is only allowed from allowed addresses. 0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71 is not on the list"));
    Ok(())
}
