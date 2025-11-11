use base64::Engine;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::PrivateKey;
use sovereign_web3::schema::{json, Serializer, TransactionBuilder};

const CHAIN_ID: u64 = 4321;

// Run with: cargo test -- --ignored
// This is intended as a simple manual smoke test against a pre-running local demo rollup
// to ensure things are still working end-to-end. Will add more comprehensive tests later.
#[test]
#[ignore]
fn test_basic_schema_transaction_submission() {
    let serializer = Serializer::from_url("http://0.0.0.0:12346/rollup/schema").unwrap();
    let call = json!({
        "bank": {
            "create_token": {
                "token_name": "smoke test",
                "initial_balance": "20000",
                "token_decimals": 12,
                "supply_cap": "100000000000",
                "mint_to_address": {
                    "Standard": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
                },
                "admins": [
                    {
                        "Standard": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
                    }
                ],

            }
        }
    });
    let unsigned_tx = TransactionBuilder::new(call)
        .chain_id(CHAIN_ID)
        .build()
        .unwrap();
    let tx_bytes = unsigned_tx.bytes_for_signing(&serializer).unwrap();

    // demo rollup uses paymaster so tx will succeed without funds
    let private_key = Ed25519PrivateKey::generate();
    let signature = private_key.sign(&tx_bytes).as_ref().to_vec();
    let pub_key = private_key.pub_key().bytes().to_vec();
    let signed_tx = unsigned_tx.to_signed(pub_key, signature);

    let tx_bytes = serializer.serialize_tx(&signed_tx).unwrap();
    let encoded_tx = base64::prelude::BASE64_STANDARD.encode(tx_bytes);
    let client = reqwest::blocking::Client::new();
    let response = client
        .post("http://0.0.0.0:12346/sequencer/txs")
        .json(&json!({
            "body": encoded_tx,
        }))
        .send()
        .unwrap();

    assert!(response.status().is_success(), "Request failed");
}
