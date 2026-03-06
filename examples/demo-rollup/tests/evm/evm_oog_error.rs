use std::net::SocketAddr;

use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy::contract::Error;
use alloy::transports::RpcError;
use sov_evm_test_utils::SimpleStorage;

async fn deploy_with_gas(addr: SocketAddr, gas: u64) -> String {
    let client = alloy_client(addr);
    let err = SimpleStorage::deploy_builder(client)
        .gas(gas)
        .deploy()
        .await
        .unwrap_err();
    let Error::TransportError(RpcError::ErrorResp(payload)) = err else {
        panic!("Expected transport error")
    };
    let data = payload.data.unwrap();
    data.get().into()
}

/// Currently those OOG errors are far from readable and this test is here for demonstration and regression detection
#[tokio::test(flavor = "multi_thread")]
#[ignore = "see https://github.com/Sovereign-Labs/sovereign-sdk/pull/2100"]
async fn returns_readable_oog_error() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    assert_eq!(deploy_with_gas(rollup.http_addr, 0).await, "\"400 Bad Request - 'Transaction execution unsuccessful' ({\\\"error\\\": String(\\\"TransactionReceipt { tx_hash: 0x79ac94a6435e0941eca945e2270e649de138f2f3aa973688cc67ded90d817358, body_to_save: \\\\\\\"<removed>\\\\\\\", events: [], receipt: Skipped(SkippedTxContents { gas_used: GasUnit[3426, 3426], error: OutOfGas(\\\\\\\"The amount to charge is greater than the funds available in the meter. Amount to charge 61668, remaining_funds  0, price GasPrice[9, 9]\\\\\\\") }) }\\\")})\"");
    assert_eq!(deploy_with_gas(rollup.http_addr, 20_000).await, "\"400 Bad Request - 'Transaction execution unsuccessful' ({\\\"CoreModuleError\\\": Object {\\\"error_code\\\": String(\\\"generic\\\"), \\\"message\\\": String(\\\"EVM execution error: Halt { reason: OutOfGas(Basic), gas_used: 8614 }\\\")}})\"");

    Ok(())
}
