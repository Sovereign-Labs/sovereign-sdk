use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use alloy::primitives::B256;
use alloy::providers::Provider;
use alloy::signers::local::PrivateKeySigner;
use sov_evm_test_utils::SimpleStorage;

/// Verifies that contract deployments works when `to` and `gas` are absent.
/// The handler should auto-estimate gas and convert `to: None` to `to: Some(TxKind::Create)`.
#[tokio::test(flavor = "multi_thread")]
async fn contract_deploy_via_eth_send_transaction_with_no_gas_limit() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    let from = signer.address();
    let client = alloy_client(rollup.http_addr);

    let deploy = SimpleStorage::deploy_builder(client.clone());
    let deploy_request = deploy.into_transaction_request();
    let bytecode = deploy_request.input.into_input();

    // Make a raw RPC call to eth_sendTransaction with contract deployment
    // (to=None, no gas limit, only gas price)
    let params = serde_json::json!([{
        "from": from,
        "maxFeePerGas": "0x100",  // Sufficient to cover base fee
        "maxPriorityFeePerGas": "0x1",
        "data": bytecode,
    }]);

    // Should auto-estimate gas and deploy the contract
    let tx_hash: B256 = client
        .client()
        .request("eth_sendTransaction", &params)
        .await?;
    let receipt = client
        .get_transaction_receipt(tx_hash)
        .await?
        .expect("Receipt should exist");

    assert!(receipt.contract_address.is_some());
    assert!(receipt.status());

    Ok(())
}
