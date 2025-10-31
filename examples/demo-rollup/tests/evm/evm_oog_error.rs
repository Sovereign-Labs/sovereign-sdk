use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy::contract::Error;
use alloy::transports::RpcError;
use sov_test_utils::SimpleStorage;

#[tokio::test(flavor = "multi_thread")]
async fn returns_readable_oog_error() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    let client = alloy_client(rollup.http_addr);
    rollup.wait_for_next_blocks(1).await;

    let err = SimpleStorage::deploy_builder(client)
        .gas(50_000)
        .deploy()
        .await
        .unwrap_err();
    let Error::TransportError(RpcError::ErrorResp(payload)) = err else {
        panic!("Expected transport error")
    };
    let data = payload.data.unwrap();
    assert_eq!(data.get(), "\"400 Bad Request - 'Transaction execution unsuccessful' ({\\\"message\\\": String(\\\"EVM execution error: Halt { reason: OutOfGas(Basic), gas_used: 38614 }\\\")})\"");

    Ok(())
}
