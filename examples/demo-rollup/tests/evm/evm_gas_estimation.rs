use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use alloy::providers::Provider;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Bytes, U256, U64};
use serde_json::json;
use sov_evm_test_utils::SimpleStorage;

const SIM_MAX_FEE_PER_GAS: &str = "0x100";
const SIM_MAX_PRIORITY_FEE_PER_GAS: &str = "0x1";

fn sender_address() -> anyhow::Result<Address> {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    Ok(signer.address())
}

fn create_simulation_create_params(
    from: Address,
    bytecode: Bytes,
    nonce: Option<u64>,
) -> serde_json::Value {
    let mut request = json!({
        "from": from,
        "maxFeePerGas": SIM_MAX_FEE_PER_GAS,
        "maxPriorityFeePerGas": SIM_MAX_PRIORITY_FEE_PER_GAS,
        "data": bytecode,
    });
    if let Some(nonce) = nonce {
        request["nonce"] = json!(format!("0x{nonce:x}"));
    }
    json!([request, "latest"])
}

#[tokio::test(flavor = "multi_thread")]
async fn big_accessory_state_writes() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
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
    rollup.wait_for_rollup_height_advance_by(1).await;
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
async fn eth_estimate_gas_rejects_stale_explicit_nonce_after_nonce_advance() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client(rollup.http_addr);
    let from = sender_address()?;

    // Consume nonce 0 in real execution before simulating a second CREATE.
    let _ = SimpleStorage::deploy(client.clone()).await?;
    let next_nonce: u64 = client.get_transaction_count(from).await?;
    assert!(next_nonce > 0, "nonce should advance after deployment");

    let bytecode = SimpleStorage::deploy_builder(client.clone())
        .into_transaction_request()
        .input
        .into_input()
        .expect("contract deployment request should include bytecode");

    let omitted_nonce_params = create_simulation_create_params(from, bytecode.clone(), None);
    let explicit_zero_nonce_params =
        create_simulation_create_params(from, bytecode.clone(), Some(0));
    let explicit_current_nonce_params =
        create_simulation_create_params(from, bytecode, Some(next_nonce));

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
    let explicit_current_value = explicit_current_nonce_estimate
        .as_ref()
        .expect("explicit current nonce estimate should succeed");
    let explicit_zero_error = explicit_zero_nonce_estimate
        .as_ref()
        .expect_err("explicit stale nonce=0 should be rejected after nonce advance")
        .to_string()
        .to_lowercase();
    assert!(
        explicit_zero_error.contains("nonce too low")
            || explicit_zero_error.contains("already used")
            || explicit_zero_error.contains("nonce"),
        "unexpected stale nonce error: {explicit_zero_error}"
    );
    assert_eq!(
        omitted_nonce_estimate
            .as_ref()
            .expect("omitted nonce should succeed"),
        explicit_current_value,
        "omitted nonce should resolve to current account nonce in estimate path"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn eth_call_omitted_nonce_matches_explicit_nonce_for_create() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client(rollup.http_addr);
    let from = sender_address()?;

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

    let omitted_nonce_params = create_simulation_create_params(from, bytecode.clone(), None);
    let explicit_nonce_params = create_simulation_create_params(from, bytecode, Some(next_nonce));

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
