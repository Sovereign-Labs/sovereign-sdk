use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, B256, U256, U64};
use alloy_rpc_types_eth::{AccessListResult, TransactionRequest};
use demo_stf::MultiAddressEvmSolana;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use sov_address::EthereumAddress;
use sov_bank::config_gas_token_id;
use sov_demo_rollup::{MockDemoRollup, MockRollupSpec};
use sov_eth_client::SimpleStorageClient;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Amount;
use sov_test_utils::test_rollup::TestRollup;

use crate::evm::evm_test_helper::{
    create_simple_storage_client, raw_signed_eip1559, setup_test_rollup_with_paymaster, tx_count,
    EVM_EXTENSION, MAX_FEE_PER_GAS, SENDER_PRIV_KEY,
};

// 1_000_000 rather than the minimal 21_000 because state-access charges push the
// real cost of a simple ETH transfer to ~430k SOV (21_000 * 1 * (9+9) = 378k is
// already insufficient).
const GAS_LIMIT: u64 = 1_000_000;

/// Hardhat #4: 0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65
/// Not in any genesis → starts with zero EVM balance, but covered by the paymaster.
const PAYMASTER_SIGNER_PRIV_KEY: &str =
    "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a";

fn paymaster_address() -> Address {
    let signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse().unwrap();
    signer.address()
}

async fn setup_paymaster_client() -> (
    TestRollup<MockDemoRollup<Native>>,
    SimpleStorageClient,
    Address,
) {
    let rollup = setup_test_rollup_with_paymaster(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;
    let client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;

    let addr = paymaster_address();
    let balance: U256 = client
        .ws
        .request("eth_getBalance", rpc_params![addr, "latest"])
        .await
        .unwrap();
    assert_eq!(
        balance,
        U256::ZERO,
        "test precondition failed: paymaster signer must start with zero EVM balance"
    );

    let rollup_balance = rollup
        .client
        .get_balance::<MockRollupSpec<Native>>(
            &MultiAddressEvmSolana::Evm(EthereumAddress(addr)),
            &config_gas_token_id(),
            None,
        )
        .await
        // The balance endpoint returns 404 when an account has never been seen.
        // Treat that as zero; propagate any other error.
        .unwrap_or_else(|err| {
            let err_string = err.to_string();
            if err_string.contains("404 Not Found") {
                Amount::ZERO
            } else {
                panic!("Error from server: {err_string}");
            }
        });
    assert_eq!(
        rollup_balance,
        Amount::ZERO,
        "test precondition failed: paymaster signer must start with zero rollup gas-token balance"
    );

    (rollup, client, addr)
}

fn base_request(from: Address) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(Address::ZERO.into()),
        gas: Some(GAS_LIMIT),
        value: Some(U256::ZERO),
        ..Default::default()
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct GasValues {
    estimate_gas: u64,
    create_access_list_gas: u64,
    receipt_gas_used: u64,
}

#[derive(Debug)]
#[allow(dead_code)]
struct CombinedErrors {
    estimate_gas_error: String,
    call_error: String,
    create_access_list_error: String,
    send_raw_tx_error: String,
}

/// Calls `eth_estimateGas`, `eth_call`, `eth_createAccessList`, and
/// `eth_sendRawTransaction` with the same request parameters and asserts they
/// all agree (all succeed or all fail).
///
/// On the success path, compares gas values across endpoints and verifies that
/// the receipt gas is within the estimate.
async fn get_consistent_response(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
    signer: &PrivateKeySigner,
) -> Result<GasValues, CombinedErrors> {
    // Step 1: 3 simulation calls
    let eth_estimate_gas_result: Result<U64, _> = client
        .ws
        .request("eth_estimateGas", rpc_params![request, "latest"])
        .await;

    let eth_call_result: Result<String, _> = client
        .ws
        .request("eth_call", rpc_params![request, "latest"])
        .await;

    let access_list_result: Result<AccessListResult, _> = client
        .ws
        .request("eth_createAccessList", rpc_params![request, "latest"])
        .await;

    // Normalize eth_createAccessList: the RPC call itself may succeed but return
    // an error in the `error` field of the response.
    let access_list_result: Result<AccessListResult, String> = match access_list_result {
        Ok(alr) if alr.error.is_some() => Err(alr.error.unwrap()),
        Ok(alr) => Ok(alr),
        Err(e) => Err(e.to_string()),
    };

    // Step 2: Build and send raw tx
    let chain_id: U64 = client
        .ws
        .request("eth_chainId", rpc_params![])
        .await
        .unwrap();
    let nonce = tx_count(client, signer.address(), "latest").await.unwrap();

    let gas = request.gas.unwrap_or(GAS_LIMIT);
    let value = request.value.unwrap_or(U256::ZERO);
    let max_priority_fee = request.max_priority_fee_per_gas.unwrap_or(0);
    let max_fee = request.gas_price.or(request.max_fee_per_gas).unwrap_or(0);
    let to = request.to.unwrap_or(alloy_primitives::TxKind::Create);
    let input = request.input.input.clone().unwrap_or_default();

    let raw_tx = raw_signed_eip1559(
        signer,
        chain_id.to::<u64>(),
        nonce,
        gas,
        to,
        value,
        input,
        max_fee,
        max_priority_fee,
    )
    .await
    .unwrap();

    let send_raw_tx_result: Result<B256, _> = client
        .ws
        .request("eth_sendRawTransaction", rpc_params![&raw_tx])
        .await;

    // Step 3: Return
    match (
        &eth_estimate_gas_result,
        &eth_call_result,
        &access_list_result,
        &send_raw_tx_result,
    ) {
        (Ok(estimate_gas), Ok(_), Ok(access_list), Ok(tx_hash)) => {
            let estimate_gas = estimate_gas.to::<u64>();
            let create_access_list_gas = access_list.gas_used.to::<u64>();

            assert!(
                estimate_gas >= create_access_list_gas,
                "eth_estimateGas ({estimate_gas}) should be >= eth_createAccessList gas ({create_access_list_gas})"
            );

            let receipt = client.wait_for_receipt(*tx_hash).await;
            assert!(
                receipt.status(),
                "transaction should succeed when paymaster covers gas"
            );

            let receipt_gas_used = receipt.gas_used;
            assert!(
                estimate_gas >= receipt_gas_used,
                "eth_estimateGas ({estimate_gas}) should be >= receipt gas_used ({receipt_gas_used})"
            );

            Ok(GasValues {
                estimate_gas,
                create_access_list_gas,
                receipt_gas_used,
            })
        }
        (Err(estimate_err), Err(call_err), Err(access_list_err), Err(send_err)) => {
            Err(CombinedErrors {
                estimate_gas_error: estimate_err.to_string(),
                call_error: call_err.to_string(),
                create_access_list_error: access_list_err.to_string(),
                send_raw_tx_error: send_err.to_string(),
            })
        }
        // The consistency check above already panics on mismatch, so this is unreachable.
        _ => {
            panic!(
                "Endpoints disagree!\n  \
             eth_estimateGas: {eth_estimate_gas_result:?}\n  \
             eth_call: {eth_call_result:?}\n  \
             eth_createAccessList: {access_list_result:?}\n  \
             eth_sendRawTransaction: {send_raw_tx_result:?}"
            );
        }
    }
}

async fn assert_simulation_succeeds(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
    signer: &PrivateKeySigner,
) {
    get_consistent_response(client, request, signer)
        .await
        .expect("Requests should be accepted");
}

// No fee fields → balance check skipped (simulation only — signed tx with max_fee=0 would fail)
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_without_fee_fields() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = base_request(addr);

    let _: U64 = client
        .ws
        .request("eth_estimateGas", rpc_params![&request, "latest"])
        .await?;
    let _: String = client
        .ws
        .request("eth_call", rpc_params![&request, "latest"])
        .await?;
    let access_list: AccessListResult = client
        .ws
        .request("eth_createAccessList", rpc_params![&request, "latest"])
        .await?;
    assert!(
        access_list.error.is_none(),
        "eth_createAccessList returned error: {:?}",
        access_list.error
    );

    Ok(())
}

// gasPrice set → paymaster covers sender, all 4 endpoints succeed
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_gas_price() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };
    let signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse()?;
    assert_simulation_succeeds(&client, &request, &signer).await;

    Ok(())
}

// maxFeePerGas set → paymaster covers sender, all 4 endpoints succeed
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_max_fee_per_gas() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    let signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse()?;
    assert_simulation_succeeds(&client, &request, &signer).await;

    Ok(())
}

// Both gasPrice and maxFeePerGas → conflicting fields error
// (simulation only — signed EIP-1559 can't express conflicting fields)
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_rejects_with_conflicting_fee_fields() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas_price: Some(MAX_FEE_PER_GAS),
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };

    let expected = "both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified";

    let estimate_err = client
        .ws
        .request::<U64, _>("eth_estimateGas", rpc_params![&request, "latest"])
        .await
        .unwrap_err();
    assert!(
        estimate_err.to_string().contains(expected),
        "eth_estimateGas: expected error containing {expected:?}, got: {estimate_err}"
    );

    let call_err = client
        .ws
        .request::<String, _>("eth_call", rpc_params![&request, "latest"])
        .await
        .unwrap_err();
    assert!(
        call_err.to_string().contains(expected),
        "eth_call: expected error containing {expected:?}, got: {call_err}"
    );

    let access_list_result: Result<AccessListResult, _> = client
        .ws
        .request("eth_createAccessList", rpc_params![&request, "latest"])
        .await;
    let access_list_err = match access_list_result {
        Ok(alr) if alr.error.is_some() => alr.error.unwrap(),
        Ok(_) => panic!("eth_createAccessList should fail for conflicting fee fields"),
        Err(e) => e.to_string(),
    };
    assert!(
        access_list_err.contains(expected),
        "eth_createAccessList: expected error containing {expected:?}, got: {access_list_err}"
    );

    Ok(())
}

// gasPrice set, gas omitted → paymaster covers sender, all 4 endpoints succeed
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_gas_price_gas_omitted() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas: None,
        gas_price: Some(MAX_FEE_PER_GAS),
        ..base_request(addr)
    };
    let signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse()?;
    assert_simulation_succeeds(&client, &request, &signer).await;

    Ok(())
}

// maxFeePerGas set, gas omitted → paymaster covers sender, all 4 endpoints succeed
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_simulation_succeeds_with_max_fee_per_gas_gas_omitted() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    let request = TransactionRequest {
        gas: None,
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };
    let signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse()?;
    assert_simulation_succeeds(&client, &request, &signer).await;

    Ok(())
}

// Simulation + sendRawTransaction both succeed for paymaster-covered sender
#[tokio::test(flavor = "multi_thread")]
async fn paymaster_send_raw_tx_succeeds() -> anyhow::Result<()> {
    let (_rollup, client, addr) = setup_paymaster_client().await;

    // Use base_fee * 2 as a reasonable fee the paymaster's SOV bank can afford.
    let base_fee: U256 = client.ws.request("eth_gasPrice", rpc_params![]).await?;
    let base_fee_u128: u128 = base_fee.to::<u128>();
    assert!(base_fee_u128 > 0, "base fee should be non-zero");
    let reasonable_max_fee = base_fee_u128 * 2;

    let request = TransactionRequest {
        max_fee_per_gas: Some(reasonable_max_fee),
        max_priority_fee_per_gas: Some(0),
        ..base_request(addr)
    };

    let signer: PrivateKeySigner = PAYMASTER_SIGNER_PRIV_KEY.parse()?;
    get_consistent_response(&client, &request, &signer)
        .await
        .expect("Simulation + sendRawTransaction should both succeed for paymaster-covered sender");

    Ok(())
}
