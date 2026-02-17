use alloy::consensus::TxLegacy;
use alloy::eips::eip2718::Encodable2718;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use reqwest::Client;
use secp256k1::SecretKey;
use serde_json::{json, Value};
use sov_eth_dev_signer::Signer;

use crate::evm::evm_test_helper::{setup_test_rollup, EVM_EXTENSION, SENDER_PRIV_KEY};

const TX_REJECTED_CODE: i32 = -32003;

#[tokio::test(flavor = "multi_thread")]
async fn test_legacy_transaction_returns_tx_type_not_supported() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_sequencer_ready().await?;

    // Parse the private key and create signer
    let private_key_bytes = hex::decode(&SENDER_PRIV_KEY[2..])?; // Remove 0x prefix
    let secret_key = SecretKey::from_slice(&private_key_bytes)?;
    let signer = Signer::new(secret_key);

    // Create a legacy transaction (Type 0)
    let legacy_tx = TxLegacy {
        chain_id: Some(5655),
        nonce: 0,
        gas_price: 1_000_000_000,
        gas_limit: 21000,
        to: TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
    };

    // Sign and encode the legacy transaction
    let signed_tx = signer
        .sign_transaction(alloy::consensus::TypedTransaction::Legacy(legacy_tx))
        .expect("Failed to sign transaction");
    let raw_tx = hex::encode(signed_tx.encoded_2718());

    // Send via raw JSON-RPC
    let client = Client::new();
    let response = client
        .post(format!("http://{}/rpc", rollup.http_addr))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "eth_sendRawTransaction",
            "params": [format!("0x{}", raw_tx)],
            "id": 1
        }))
        .send()
        .await?
        .json::<Value>()
        .await?;

    // Verify error response
    let error = response.get("error").expect("Expected error response");
    let code = error.get("code").and_then(|c| c.as_i64()).unwrap() as i32;
    let message = error.get("message").and_then(|m| m.as_str()).unwrap();

    assert_eq!(
        code, TX_REJECTED_CODE,
        "Expected TransactionRejected error code (-32003), got {code}"
    );
    assert!(
        message.contains("transaction type not supported"),
        "Error message should mention 'transaction type not supported', got: {message}"
    );

    Ok(())
}
