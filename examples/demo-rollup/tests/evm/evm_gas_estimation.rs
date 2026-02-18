use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use alloy::providers::Provider;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Bytes, U256, U64};
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

#[tokio::test(flavor = "multi_thread")]
async fn eth_estimate_gas_omitted_nonce_differs_from_explicit_zero_after_nonce_advance(
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client(rollup.http_addr);
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let from = signer.address();

    // Consume nonce 0 in real execution before simulating a second CREATE.
    let _ = SimpleStorage::deploy(client.clone()).await?;
    let next_nonce: u64 = client.get_transaction_count(from).await?;
    assert!(next_nonce > 0, "nonce should advance after deployment");

    let bytecode = SimpleStorage::deploy_builder(client.clone())
        .into_transaction_request()
        .input
        .into_input()
        .expect("contract deployment request should include bytecode");

    let omitted_nonce_params = serde_json::json!([
        {
            "from": from,
            "maxFeePerGas": "0x100",
            "maxPriorityFeePerGas": "0x1",
            "data": bytecode.clone(),
        },
        "latest",
    ]);
    let explicit_zero_nonce_params = serde_json::json!([
        {
            "from": from,
            "maxFeePerGas": "0x100",
            "maxPriorityFeePerGas": "0x1",
            "data": bytecode,
            "nonce": "0x0",
        },
        "latest",
    ]);
    let explicit_current_nonce_params = serde_json::json!([
        {
            "from": from,
            "maxFeePerGas": "0x100",
            "maxPriorityFeePerGas": "0x1",
            "data": bytecode,
            "nonce": format!("0x{next_nonce:x}"),
        },
        "latest",
    ]);

    let omitted_nonce_estimate: Result<U64, _> = client
        .client()
        .request("eth_estimateGas", &omitted_nonce_params)
        .await;
    let explicit_zero_nonce_estimate: Result<U64, _> = client
        .client()
        .request("eth_estimateGas", &explicit_zero_nonce_params)
        .await;
    let explicit_current_nonce_estimate: Result<U64, _> = client
        .client()
        .request("eth_estimateGas", &explicit_current_nonce_params)
        .await;

    assert!(
        omitted_nonce_estimate.is_ok(),
        "omitted nonce estimate should succeed after nonce has advanced"
    );
    let explicit_zero_value = explicit_zero_nonce_estimate
        .as_ref()
        .expect("explicit nonce=0 estimate should succeed");
    let explicit_current_value = explicit_current_nonce_estimate
        .as_ref()
        .expect("explicit current nonce estimate should succeed");
    assert_eq!(
        explicit_zero_value, explicit_current_value,
        "explicit nonce value should not change estimate in current SovHandler execution path"
    );

    let differs_from_explicit_zero = match (
        omitted_nonce_estimate.as_ref(),
        explicit_zero_nonce_estimate.as_ref(),
    ) {
        (Ok(omitted), Ok(explicit_zero)) => omitted != explicit_zero,
        (Ok(_), Err(_)) | (Err(_), Ok(_)) => true,
        (Err(_), Err(_)) => false,
    };
    assert!(
        differs_from_explicit_zero,
        "omitted nonce estimate should differ from explicit nonce=0 after nonce has advanced; this captures omitted-nonce lookup side effects in estimate metering; omitted={omitted_nonce_estimate:?}, explicit_zero={explicit_zero_nonce_estimate:?}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_call_omitted_nonce_matches_explicit_nonce_for_create() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client(rollup.http_addr);
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let from = signer.address();

    // Create one contract and then execute one CALL tx to ensure the EVM-side
    // account nonce has advanced beyond zero before CREATE simulation.
    let contract = SimpleStorage::deploy(client.clone()).await?;
    let pending = contract.set(U256::from(1u64)).send().await?;
    let receipt = pending.get_receipt().await?;
    assert!(
        receipt.status(),
        "precondition CALL transaction should succeed"
    );
    let next_nonce: u64 = client.get_transaction_count(from).await?;
    assert!(next_nonce > 0, "nonce should advance after deployment");

    let bytecode = SimpleStorage::deploy_builder(client.clone())
        .into_transaction_request()
        .input
        .into_input()
        .expect("contract deployment request should include bytecode");

    let omitted_nonce_params = serde_json::json!([
        {
            "from": from,
            "maxFeePerGas": "0x100",
            "maxPriorityFeePerGas": "0x1",
            "data": bytecode.clone(),
        },
        "latest",
    ]);
    let explicit_nonce_params = serde_json::json!([
        {
            "from": from,
            "maxFeePerGas": "0x100",
            "maxPriorityFeePerGas": "0x1",
            "data": bytecode,
            "nonce": format!("0x{next_nonce:x}"),
        },
        "latest",
    ]);

    let omitted_nonce_result: Bytes = client
        .client()
        .request("eth_call", &omitted_nonce_params)
        .await?;
    let explicit_nonce_result: Bytes = client
        .client()
        .request("eth_call", &explicit_nonce_params)
        .await?;

    assert_eq!(
        omitted_nonce_result, explicit_nonce_result,
        "omitted nonce should match explicit nonce behavior for eth_call CREATE simulation"
    );

    Ok(())
}
