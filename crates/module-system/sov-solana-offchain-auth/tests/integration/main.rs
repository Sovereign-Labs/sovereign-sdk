#![allow(unused_imports)]
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_node_client::NodeClient;
use std::str::FromStr;
use std::sync::Arc;
use tokio_stream::StreamExt;

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_bank::BalanceResponse;
use sov_bank::{Amount, CallMessage as BankCallMessage, Coins, TokenId};
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaService};
use sov_mock_zkvm::crypto::Ed25519Signature;
use sov_modules_api::capabilities::{TransactionAuthenticator, UniquenessData};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::transaction::{PubKeyAndSignature, Transaction, UnsignedTransaction};
use sov_modules_api::{prelude::*, Base58Address, PrivateKey, SafeVec};
use sov_modules_api::{CryptoSpec, FullyBakedTx, RawTx, Runtime, Spec};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::{
    AuthorizedSequencers, PayeePolicy, PayerGenesisConfig, PaymasterConfig,
    PaymasterPolicyInitializer,
};
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::rest_api::AcceptTx;
use sov_solana_offchain_auth::authentication::{
    SolanaOffchainSimpleMessage, SolanaOffchainSimpleMultisigMessage,
    SolanaOffchainSpecCompliantMessage, SolanaOffchainSpecCompliantMultisigMessage,
    SolanaOffchainUnsignedTransactionV0, SolanaOffchainUnsignedTransactionV1,
    MULTISIG_SIMPLE_DISCRIMINATOR,
};
use sov_solana_offchain_auth::utils::{
    make_multisig_preamble_for_message, make_preamble_for_message,
};
use sov_solana_offchain_auth::{
    SolanaOffchainAuthenticator, SolanaOffchainAuthenticatorInput, SolanaOffchainAuthenticatorTrait,
};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{BankConfig, Runtime as _};
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    generate_runtime, RtAgnosticBlueprint, TestStorage, TestUser, TEST_DEFAULT_GAS_LIMIT,
    TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE,
};
use sov_test_utils::{MockDaSpec, MockZkvm, MockZkvmCryptoSpec};
use sov_value_setter::ValueSetterConfig;
use tempfile::tempdir;

mod blueprint;
use blueprint::SolanaOffchainAuthBlueprint;

// Define a test spec that uses Base58Address instead of the default Address type
pub type SolanaTestSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    Base58Address, // Use Base58Address instead of the default Address
    Native,
    MockZkvmCryptoSpec,
    TestStorage,
>;

/// An arbitrary base58 address.
const RECIPIENT_ADDRESS: &str = "4zdwHNaEa5npHtRtaZ3RL1m6rptuQZ6RBLHG6cAyVHjL";

// Generate the test runtime with Solana offchain authenticator
generate_runtime! {
    name: TestRuntime,
    modules: [
        value_setter: sov_value_setter::ValueSetter<S>,
        paymaster: sov_paymaster::Paymaster<S>,
    ],
    operating_mode: sov_modules_api::runtime::OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::config::MinimalOptimisticGenesisConfig<S>,
    gas_enforcer: paymaster: sov_paymaster::Paymaster<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: SolanaOffchainAuthenticator<S, Self>,
    auth_call_wrapper: |call| call,
}

impl<S: Spec> SolanaOffchainAuthenticatorTrait<S> for TestRuntime<S> {
    fn add_solana_offchain_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input {
        SolanaOffchainAuthenticatorInput::SolanaOffchain(tx)
    }
}

type RT = TestRuntime<SolanaTestSpec>;
type S = SolanaTestSpec;
type TestHasher = <<S as Spec>::CryptoSpec as CryptoSpec>::Hasher;

async fn create_test_rollup() -> anyhow::Result<(
    TestRollup<SolanaOffchainAuthBlueprint<SolanaTestSpec, RT>>,
    TestUser<SolanaTestSpec>,
)> {
    // Create genesis config
    let genesis_config = HighLevelOptimisticGenesisConfig::<SolanaTestSpec>::generate()
        .add_accounts_with_default_balance(1);
    let sequencer = genesis_config.initial_sequencer.clone();
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config = <RT as Runtime<SolanaTestSpec>>::GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
        PaymasterConfig {
            payers: [PayerGenesisConfig {
                payer_address: admin.address(),
                policy: PaymasterPolicyInitializer {
                    default_payee_policy: PayeePolicy::Allow {
                        max_fee: None,
                        gas_limit: None,
                        max_gas_price: None,
                        transaction_limit: None,
                    },
                    payees: SafeVec::new(),
                    authorized_sequencers: AuthorizedSequencers::All,
                    authorized_updaters: [admin.address()].as_ref().try_into().unwrap(),
                },
                sequencers_to_register: [sequencer.da_address].as_ref().try_into().unwrap(),
            }]
            .as_ref()
            .try_into()
            .unwrap(),
        },
    );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config,
    };

    let dir = Arc::new(tempdir()?);
    let seq_da_address = genesis_params
        .runtime
        .sequencer_registry
        .sequencer_config
        .seq_da_address;

    // The genesis config uses these bytes [172; 32] to generate the default prover and sequencer
    // addresses.
    // The RollupBuilder normally defaults to using the bech32 encoding of these bytes as defined
    // in the constants TEST_DEFAULT_PROVER_ADDRESS and TEST_DEFAULT_SEQUENCER_ADDRESS. We need to
    // override them with the base58 encoding of the same bytes, since our spec uses Base58Address.
    let prover_sequencer_bytes = [172; 32];
    let prover_sequencer_base58 = Base58Address::from(prover_sequencer_bytes);

    // Build the test rollup
    let rollup = RollupBuilder::<SolanaOffchainAuthBlueprint<SolanaTestSpec, RT>>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        3, // finalization blocks
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir.clone());
        c.automatic_batch_production = false;
        c.max_batch_size_bytes = 1024 * 1024; // 1MB
        c.blob_processing_timeout_secs = 60;
        // Override the hardcoded bech32 addresses with base58 equivalents
        c.prover_address = prover_sequencer_base58.to_string();
        c.sequencer_address = prover_sequencer_base58.to_string();
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .set_persistent_da()
    .start()
    .await?;

    rollup.produce_enough_finalized_slots().await;
    rollup.wait_for_sequencer_ready().await?;

    Ok((rollup, admin))
}

fn create_transfer_tx_json(amount: Amount, recipient: &str) -> String {
    let msg: TestRuntimeCall<S> = TestRuntimeCall::Bank(BankCallMessage::Transfer {
        to: <S as Spec>::Address::from_str(recipient).unwrap(),
        coins: Coins {
            amount,
            token_id: config_value!("GAS_TOKEN_ID"),
        },
    });
    let unsigned_tx = UnsignedTransaction::<RT, S>::new(
        msg,
        config_value!("CHAIN_ID"),
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        UniquenessData::Generation(0),
        Some(TEST_DEFAULT_GAS_LIMIT.into()),
    );
    let solana_unsigned_tx = SolanaOffchainUnsignedTransactionV0::<RT, S> {
        runtime_call: unsigned_tx.runtime_call,
        uniqueness: unsigned_tx.uniqueness,
        details: unsigned_tx.details,
        chain_name: config_value!("CHAIN_NAME").to_string().try_into().unwrap(),
    };

    serde_json::to_string(&solana_unsigned_tx).unwrap()
}

async fn submit_tx(
    client: &sov_api_spec::client::Client,
    raw_tx_bytes: Vec<u8>,
) -> reqwest::Response {
    let request = AcceptTx {
        body: sov_sequencer::rest_api::Base64Blob { blob: raw_tx_bytes },
    };

    let response = client
        .client()
        .post(format!(
            "{}/sequencer/accept_solana_offchain_tx",
            client.baseurl()
        ))
        .json(&request)
        .send()
        .await
        .expect("Failed to send request");

    response
}

async fn query_balance(client: &NodeClient, address: &str) -> Option<Amount> {
    let gas_token_id: TokenId = config_value!("GAS_TOKEN_ID");

    let Ok(response) = client
        .query_rest_endpoint::<BalanceResponse>(&format!(
            "/modules/bank/tokens/{gas_token_id}/balances/{address}"
        ))
        .await
    else {
        return None;
    };

    response.amount
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rollup_initialization() {
    // Just test that we can create a rollup with the Solana authenticator
    let rollup = create_test_rollup().await;
    assert!(
        rollup.is_ok(),
        "Failed to create test rollup: {:?}",
        rollup.err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_ledger_signed_transaction() {
    // From the test Ledger device used to generate this
    const LEDGER_ADDRESS: &str = "8YkzDTyLd3buhMw9CMfYYt3FLmcu1BeFr5nMeierYM1v";

    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    // First we must fund the Ledger account, using the basic/raw signature type
    {
        let funding_json_str = create_transfer_tx_json(Amount(13_000), LEDGER_ADDRESS);
        let encoded_tx = funding_json_str.as_bytes().to_vec();
        let signer = admin.private_key();
        let pubkey = signer.pub_key();
        let signature = signer.sign(&encoded_tx);

        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx,
            chain_hash: RT::CHAIN_HASH,
            pubkey,
            signature,
        };
        let raw_tx_bytes = borsh::to_vec(&message).unwrap();

        let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
        assert!(
            response.status().is_success(),
            "Expected funding transaction to succeed"
        );
    }

    // Now we can have the Ledger account transfer part of its balance
    let transfer_json_tx = create_transfer_tx_json(Amount(5_000), RECIPIENT_ADDRESS);
    // Sanity check - if this changes, the test will need to be re-signed with a Ledger device.
    // (If a different Ledger device or account is used, the public key above would also need to be
    // updated.)
    assert_eq!(
        transfer_json_tx,
        r#"{"runtime_call":{"bank":{"transfer":{"to":"4zdwHNaEa5npHtRtaZ3RL1m6rptuQZ6RBLHG6cAyVHjL","coins":{"amount":"5000","token_id":"token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7"}}}},"uniqueness":{"generation":0},"details":{"max_priority_fee_bips":0,"max_fee":"100000000000","gas_limit":[1000000000,1000000000],"chain_id":4321},"chain_name":"TestChain"}"#,
        "JSON changed - re-sign on Ledger and update the hardcoded signature"
    );
    let encoded_tx = transfer_json_tx.as_bytes().to_vec();
    let pubkey: [u8; 32] = bs58::decode(LEDGER_ADDRESS)
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();
    let signature: Ed25519Signature = bs58::decode(
        "3GBYQrmcKtUiXAQLz2bUR55Kh7YfgUy2g199ePXYSUHbRHLAsdjcTctSrt98oiA79nZVQU79AbBpiKU23Z2UTstQ",
    )
    .into_vec()
    .unwrap()
    .as_slice()
    .try_into()
    .unwrap();

    let mut signed_message_with_preamble =
        make_preamble_for_message(&pubkey, &RT::CHAIN_HASH, encoded_tx.len() as u16).to_vec();
    signed_message_with_preamble.extend_from_slice(&encoded_tx);

    // Sanity check — if this changes, re-sign on the Ledger and update the signature below.
    let message_str = bs58::encode(&signed_message_with_preamble).into_string();
    assert_eq!(
        message_str,
        "45bxAZgjJHtL6EmowbCZiBduiwEePySEehCCaCVVJouRB7hQRL3qsv4PNmQvE9NVDfFKmfmVNaNS5a32X1fpSmjJVk19Dk9VSqLyYXxeVuGCZR4jCx7JTx1qbLHD3amNkHvmCnhkgLbT8HgkPwHZWPMeapAo2cL9N3CRzPMZFM5ikWb8yJXzFpCzBjsL1fkCtkDz2BoZPHAtrh5Zvhdae6W9Qypme1iUys8iu4A4e3mk6Nh2us2iLgPcEJhK7xsNxm66CxogjGnwBW3ioTfjRby9LHVzJwQ2dFLJT8kugres5xGG6PxKFNkhFRV4bPgYJvh4ZUUUZSWKkVeW4Ep3nH4BTn1N5WkouYWjKvhy54FCasbM8AyWYAyCnSpX5e8jFEN12rBzmq4HEEJxJY51rTULCEK8Uq2ZJKAe5yTXUisQRw1hZo5bQGku5DwfGyupyfiE6b78vm7uJvQcipLFBoDByrrwPRGhYXK3m8axmcDUFm2PiCpRfYSMk7kGsNn1LLD7D2EwovWDCVERVEB5zftfj6gx9ZeFeZi1c2PdUYsvhZjHdVEa8JVFmts5Q8hD1Wf53DjQAmCFa8GzTK1cekDN47ENFfPaPQsLRFTEXn",
        "Multisig message bytes changed - re-sign on ledger and update the hardcoded signature"
    );

    let message = SolanaOffchainSpecCompliantMessage::<S> {
        signed_message_with_preamble,
        signature,
    };

    let raw_tx_bytes = borsh::to_vec(&message).unwrap();

    let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
    assert!(
        response.status().is_success(),
        "Expected Ledger transaction to succeed"
    );

    let ledger_balance = query_balance(&test_rollup.client, LEDGER_ADDRESS).await;
    assert_eq!(
        ledger_balance,
        Some(Amount::new(8_000)),
        "Expected ledger account to have received 8,000 tokens remaining"
    );

    let recipient_balance = query_balance(&test_rollup.client, RECIPIENT_ADDRESS).await;
    assert_eq!(
        recipient_balance,
        Some(Amount::new(5_000)),
        "Expected recipient to have received 5,000 tokens"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_raw_signed_message_transaction() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    let initial_amount = query_balance(&test_rollup.client, RECIPIENT_ADDRESS).await;
    assert_eq!(
        initial_amount, None,
        "Expected recipient to have no initial balance"
    );

    let tx_str = create_transfer_tx_json(Amount(10_000), RECIPIENT_ADDRESS);
    let encoded_tx = tx_str.as_bytes().to_vec();
    let signer = admin.private_key();
    let pubkey = signer.pub_key();
    let signature = signer.sign(&encoded_tx);

    let message = SolanaOffchainSimpleMessage::<S> {
        signed_message: encoded_tx,
        chain_hash: RT::CHAIN_HASH,
        pubkey,
        signature,
    };
    let raw_tx_bytes = borsh::to_vec(&message).unwrap();

    let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;

    assert!(
        response.status().is_success(),
        "Expected transaction to succeed. Response: {response:?}"
    );

    let final_balance = query_balance(&test_rollup.client, RECIPIENT_ADDRESS).await;
    assert_eq!(
        final_balance,
        Some(Amount::new(10_000)),
        "Expected recipient to have received 10,000 tokens"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_invalid_raw_signed_message_transaction() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    let tx_str = create_transfer_tx_json(Amount(10_000), RECIPIENT_ADDRESS);
    let encoded_tx = tx_str.as_bytes().to_vec();
    let signer = admin.private_key();
    let pubkey = signer.pub_key();
    let mut signature_bytes = signer.sign(&encoded_tx).msg_sig.to_bytes();
    // mutate a random byte to make the signature invalid
    signature_bytes[5] = signature_bytes[5].wrapping_add(1);
    let signature: Ed25519Signature = signature_bytes.as_slice().try_into().unwrap();

    let message = SolanaOffchainSimpleMessage::<S> {
        signed_message: encoded_tx,
        chain_hash: RT::CHAIN_HASH,
        pubkey,
        signature,
    };
    let raw_tx_bytes = borsh::to_vec(&message).unwrap();

    let client = test_rollup.api_client();
    let response = submit_tx(client, raw_tx_bytes).await;

    assert_eq!(
        response.status(),
        400,
        "Expected 400 status for invalid signature"
    );
    let response_text = response.text().await.expect("Failed to read response body");

    assert!(
        response_text.contains("Signature verification failed")
            || response_text.contains("Verification equation was not satisfied"),
        "Expected signature verification error, got: {response_text}"
    );
}

// Sanity check of the wrapper implementation
#[test]
fn test_auth_wrapper() {
    let raw_tx = RawTx::new(vec![1, 2, 3]);

    // Test standard auth
    let standard_auth = <RT as Runtime<SolanaTestSpec>>::Auth::add_standard_auth(raw_tx.clone());
    assert!(matches!(
        standard_auth,
        SolanaOffchainAuthenticatorInput::Standard(_)
    ));

    // Test Solana offchain auth
    let solana_auth = RT::add_solana_offchain_auth(raw_tx);
    assert!(matches!(
        solana_auth,
        SolanaOffchainAuthenticatorInput::SolanaOffchain(_)
    ));
}

fn create_multisig_transfer_tx_json(
    amount: Amount,
    recipient: &str,
    multisig_id: <S as Spec>::Address,
) -> String {
    let msg: TestRuntimeCall<S> = TestRuntimeCall::Bank(BankCallMessage::Transfer {
        to: <S as Spec>::Address::from_str(recipient).unwrap(),
        coins: Coins {
            amount,
            token_id: config_value!("GAS_TOKEN_ID"),
        },
    });
    let unsigned_tx = UnsignedTransaction::<RT, S>::new(
        msg,
        config_value!("CHAIN_ID"),
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        UniquenessData::Nonce(0),
        Some(TEST_DEFAULT_GAS_LIMIT.into()),
    );
    let solana_unsigned_tx = SolanaOffchainUnsignedTransactionV1::<RT, S> {
        runtime_call: unsigned_tx.runtime_call,
        uniqueness: unsigned_tx.uniqueness,
        details: unsigned_tx.details,
        chain_name: config_value!("CHAIN_NAME").to_string().try_into().unwrap(),
        multisig_id,
        version: 1,
    };
    serde_json::to_string(&solana_unsigned_tx).unwrap()
}

fn create_multisig_simple_wire_bytes(json_str: &str) -> Vec<u8> {
    let mut signed_message = vec![MULTISIG_SIMPLE_DISCRIMINATOR];
    signed_message.extend_from_slice(json_str.as_bytes());
    signed_message
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_multisig_simple_message_transaction() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    // Create 3 signers for a 2-of-3 multisig
    let key1 = Ed25519PrivateKey::generate();
    let key2 = Ed25519PrivateKey::generate();
    let key3 = Ed25519PrivateKey::generate();
    // To update the TypeScript byte-compatibility test vectors in
    // solana-signable-rollup.test.ts, run this test with --nocapture and copy the printed values.
    println!("KEY1_PRIV_HEX: {}", key1.as_hex());
    println!("KEY2_PRIV_HEX: {}", key2.as_hex());
    println!("KEY3_PRIV_HEX: {}", key3.as_hex());
    let pub1 = key1.pub_key();
    let pub2 = key2.pub_key();
    let pub3 = key3.pub_key();
    let min_signers: u8 = 2;

    // Compute the multisig address
    let multisig =
        sov_modules_api::Multisig::new(min_signers, vec![pub1.clone(), pub2.clone(), pub3.clone()]);
    let credential_id = multisig.credential_id::<TestHasher>();
    let multisig_address: <SolanaTestSpec as Spec>::Address = credential_id.into();
    let multisig_address_str = multisig_address.to_string();

    // Fund the multisig address from admin
    {
        let funding_json = create_transfer_tx_json(Amount(20_000), &multisig_address_str);
        let encoded_tx = funding_json.as_bytes().to_vec();
        let signer = admin.private_key();
        let pubkey = signer.pub_key();
        let signature = signer.sign(&encoded_tx);

        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx,
            chain_hash: RT::CHAIN_HASH,
            pubkey,
            signature,
        };
        let raw_tx_bytes = borsh::to_vec(&message).unwrap();
        let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
        assert!(
            response.status().is_success(),
            "Expected funding transaction to succeed"
        );
    }

    let funded_balance = query_balance(&test_rollup.client, &multisig_address_str).await;
    assert_eq!(funded_balance, Some(Amount::new(20_000)));

    // Build a transfer from the multisig to the recipient, using V1 format which commits
    // to the credential_id in the signed message.
    let transfer_json =
        create_multisig_transfer_tx_json(Amount(7_000), RECIPIENT_ADDRESS, multisig_address);
    let json_bytes = transfer_json.as_bytes();
    let wire_bytes = create_multisig_simple_wire_bytes(&transfer_json);

    // Signers 3 and 1 sign the JSON directly (deliberately out of order relative to how
    // the credential was constructed from [pub1, pub2, pub3]) to verify order independence.
    let sig3 = key3.sign(json_bytes);
    let sig1 = key1.sign(json_bytes);

    let multisig_msg = SolanaOffchainSimpleMultisigMessage::<S> {
        wire_bytes,
        chain_hash: RT::CHAIN_HASH,
        signatures: vec![
            PubKeyAndSignature {
                signature: sig3,
                pub_key: pub3.clone(),
            },
            PubKeyAndSignature {
                signature: sig1,
                pub_key: pub1.clone(),
            },
        ]
        .try_into()
        .unwrap(),
        unused_pub_keys: vec![pub2.clone()].try_into().unwrap(),
        min_signers,
    };

    let raw_tx_bytes = borsh::to_vec(&multisig_msg).unwrap();
    {
        let request = AcceptTx {
            body: sov_sequencer::rest_api::Base64Blob {
                blob: raw_tx_bytes.clone(),
            },
        };
        println!(
            "MULTISIG_POST_PAYLOAD: {}",
            serde_json::to_string(&request).unwrap()
        );
    }
    let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
    assert!(
        response.status().is_success(),
        "Expected multisig transaction to succeed. Response: {response:?}"
    );

    let recipient_balance = query_balance(&test_rollup.client, RECIPIENT_ADDRESS).await;
    assert_eq!(
        recipient_balance,
        Some(Amount::new(7_000)),
        "Expected recipient to have received 7,000 tokens"
    );

    let multisig_balance = query_balance(&test_rollup.client, &multisig_address_str).await;
    assert_eq!(
        multisig_balance,
        Some(Amount::new(13_000)),
        "Expected multisig to have 13,000 tokens remaining"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_multisig_insufficient_signatures() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    let key1 = Ed25519PrivateKey::generate();
    let key2 = Ed25519PrivateKey::generate();
    let key3 = Ed25519PrivateKey::generate();
    let pub1 = key1.pub_key();
    let pub2 = key2.pub_key();
    let pub3 = key3.pub_key();
    let min_signers: u8 = 2;

    let multisig =
        sov_modules_api::Multisig::new(min_signers, vec![pub1.clone(), pub2.clone(), pub3.clone()]);
    let credential_id = multisig.credential_id::<TestHasher>();
    let multisig_address: <SolanaTestSpec as Spec>::Address = credential_id.into();
    let multisig_address_str = multisig_address.to_string();

    // Fund the multisig
    {
        let funding_json = create_transfer_tx_json(Amount(20_000), &multisig_address_str);
        let encoded_tx = funding_json.as_bytes().to_vec();
        let signer = admin.private_key();
        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx.clone(),
            chain_hash: RT::CHAIN_HASH,
            pubkey: signer.pub_key(),
            signature: signer.sign(&encoded_tx),
        };
        let response = submit_tx(test_rollup.api_client(), borsh::to_vec(&message).unwrap()).await;
        assert!(response.status().is_success());
    }

    // Only 1 signer for a 2-of-3 multisig — should fail
    let transfer_json =
        create_multisig_transfer_tx_json(Amount(5_000), RECIPIENT_ADDRESS, multisig_address);
    let json_bytes = transfer_json.as_bytes();
    let wire_bytes = create_multisig_simple_wire_bytes(&transfer_json);
    let sig1 = key1.sign(json_bytes);

    let multisig_msg = SolanaOffchainSimpleMultisigMessage::<S> {
        wire_bytes,
        chain_hash: RT::CHAIN_HASH,
        signatures: vec![PubKeyAndSignature {
            signature: sig1,
            pub_key: pub1,
        }]
        .try_into()
        .unwrap(),
        unused_pub_keys: vec![pub2, pub3].try_into().unwrap(),
        min_signers,
    };

    let response = submit_tx(
        test_rollup.api_client(),
        borsh::to_vec(&multisig_msg).unwrap(),
    )
    .await;
    assert_eq!(
        response.status(),
        400,
        "Expected 400 for insufficient signatures"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_multisig_invalid_signature() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    let key1 = Ed25519PrivateKey::generate();
    let key2 = Ed25519PrivateKey::generate();
    let key3 = Ed25519PrivateKey::generate();
    let pub1 = key1.pub_key();
    let pub2 = key2.pub_key();
    let pub3 = key3.pub_key();
    let min_signers: u8 = 2;

    let multisig =
        sov_modules_api::Multisig::new(min_signers, vec![pub1.clone(), pub2.clone(), pub3.clone()]);
    let credential_id = multisig.credential_id::<TestHasher>();
    let multisig_address: <SolanaTestSpec as Spec>::Address = credential_id.into();
    let multisig_address_str = multisig_address.to_string();

    // Fund the multisig
    {
        let funding_json = create_transfer_tx_json(Amount(20_000), &multisig_address_str);
        let encoded_tx = funding_json.as_bytes().to_vec();
        let signer = admin.private_key();
        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx.clone(),
            chain_hash: RT::CHAIN_HASH,
            pubkey: signer.pub_key(),
            signature: signer.sign(&encoded_tx),
        };
        let response = submit_tx(test_rollup.api_client(), borsh::to_vec(&message).unwrap()).await;
        assert!(response.status().is_success());
    }

    // Corrupt the second signature
    let transfer_json =
        create_multisig_transfer_tx_json(Amount(5_000), RECIPIENT_ADDRESS, multisig_address);
    let json_bytes = transfer_json.as_bytes();
    let wire_bytes = create_multisig_simple_wire_bytes(&transfer_json);
    let sig1 = key1.sign(json_bytes);
    let mut sig2_bytes = key2.sign(json_bytes).msg_sig.to_bytes();
    sig2_bytes[10] = sig2_bytes[10].wrapping_add(1);
    let sig2: Ed25519Signature = sig2_bytes.as_slice().try_into().unwrap();

    let multisig_msg = SolanaOffchainSimpleMultisigMessage::<S> {
        wire_bytes,
        chain_hash: RT::CHAIN_HASH,
        signatures: vec![
            PubKeyAndSignature {
                signature: sig1,
                pub_key: pub1,
            },
            PubKeyAndSignature {
                signature: sig2,
                pub_key: pub2,
            },
        ]
        .try_into()
        .unwrap(),
        unused_pub_keys: vec![pub3].try_into().unwrap(),
        min_signers,
    };

    let response = submit_tx(
        test_rollup.api_client(),
        borsh::to_vec(&multisig_msg).unwrap(),
    )
    .await;
    assert_eq!(response.status(), 400, "Expected 400 for invalid signature");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_multisig_spec_compliant_message_transaction() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    // Create 3 signers for a 2-of-3 multisig
    let key1 = Ed25519PrivateKey::generate();
    let key2 = Ed25519PrivateKey::generate();
    let key3 = Ed25519PrivateKey::generate();
    // To update the TypeScript byte-compatibility test vectors in
    // solana-signable-rollup.test.ts, run this test with --nocapture and copy the printed values.
    println!("SPEC_KEY1_PRIV_HEX: {}", key1.as_hex());
    println!("SPEC_KEY2_PRIV_HEX: {}", key2.as_hex());
    println!("SPEC_KEY3_PRIV_HEX: {}", key3.as_hex());
    let pub1 = key1.pub_key();
    let pub2 = key2.pub_key();
    let pub3 = key3.pub_key();
    let min_signers: u8 = 2;

    // Compute the multisig address
    let multisig =
        sov_modules_api::Multisig::new(min_signers, vec![pub1.clone(), pub2.clone(), pub3.clone()]);
    let credential_id = multisig.credential_id::<TestHasher>();
    let multisig_address: <SolanaTestSpec as Spec>::Address = credential_id.into();
    let multisig_address_str = multisig_address.to_string();

    // Fund the multisig address from admin
    {
        let funding_json = create_transfer_tx_json(Amount(20_000), &multisig_address_str);
        let encoded_tx = funding_json.as_bytes().to_vec();
        let signer = admin.private_key();
        let pubkey = signer.pub_key();
        let signature = signer.sign(&encoded_tx);

        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx,
            chain_hash: RT::CHAIN_HASH,
            pubkey,
            signature,
        };
        let raw_tx_bytes = borsh::to_vec(&message).unwrap();
        let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
        assert!(
            response.status().is_success(),
            "Expected funding transaction to succeed"
        );
    }

    let funded_balance = query_balance(&test_rollup.client, &multisig_address_str).await;
    assert_eq!(funded_balance, Some(Amount::new(20_000)));

    // Build a transfer from the multisig using spec-compliant format.
    let transfer_json =
        create_multisig_transfer_tx_json(Amount(7_000), RECIPIENT_ADDRESS, multisig_address);
    let json_bytes = transfer_json.as_bytes();

    // Build the multisig preamble with a canonical signer ordering so the emitted payload
    // matches the TypeScript client, which canonicalizes multisigPubkeys internally.
    let mut preamble_pubkeys = vec![*pub1.bytes(), *pub2.bytes(), *pub3.bytes()];
    preamble_pubkeys.sort();
    let preamble = make_multisig_preamble_for_message(
        &preamble_pubkeys,
        &RT::CHAIN_HASH,
        json_bytes.len() as u16,
    );

    let mut signed_message_with_preamble = preamble;
    signed_message_with_preamble.extend_from_slice(json_bytes);

    // Signers 1 and 3 sign the preamble+JSON. The envelope signatures must be ordered to match
    // the set bits in signer_bitfield from lowest signer index to highest.
    let sig1 = key1.sign(&signed_message_with_preamble);
    let sig3 = key3.sign(&signed_message_with_preamble);
    let mut signatures = Vec::with_capacity(2);
    let mut signer_bitfield = 0u32;
    for (idx, pubkey) in preamble_pubkeys.iter().enumerate() {
        if pubkey == pub1.bytes() {
            signatures.push(sig1.clone());
            signer_bitfield |= 1 << idx;
        } else if pubkey == pub3.bytes() {
            signatures.push(sig3.clone());
            signer_bitfield |= 1 << idx;
        }
    }
    assert_eq!(signatures.len(), 2);

    let multisig_msg = SolanaOffchainSpecCompliantMultisigMessage::<S> {
        signed_message_with_preamble,
        signatures: signatures.try_into().unwrap(),
        signer_bitfield,
        min_signers,
    };

    let raw_tx_bytes = borsh::to_vec(&multisig_msg).unwrap();
    {
        let request = AcceptTx {
            body: sov_sequencer::rest_api::Base64Blob {
                blob: raw_tx_bytes.clone(),
            },
        };
        println!(
            "MULTISIG_SPEC_POST_PAYLOAD: {}",
            serde_json::to_string(&request).unwrap()
        );
    }
    let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
    assert!(
        response.status().is_success(),
        "Expected spec-compliant multisig transaction to succeed. Response: {response:?}"
    );

    let recipient_balance = query_balance(&test_rollup.client, RECIPIENT_ADDRESS).await;
    assert_eq!(
        recipient_balance,
        Some(Amount::new(7_000)),
        "Expected recipient to have received 7,000 tokens"
    );

    let multisig_balance = query_balance(&test_rollup.client, &multisig_address_str).await;
    assert_eq!(
        multisig_balance,
        Some(Amount::new(13_000)),
        "Expected multisig to have 13,000 tokens remaining"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_spec_compliant_multisig_insufficient_signatures() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    let key1 = Ed25519PrivateKey::generate();
    let key2 = Ed25519PrivateKey::generate();
    let key3 = Ed25519PrivateKey::generate();
    let pub1 = key1.pub_key();
    let pub2 = key2.pub_key();
    let pub3 = key3.pub_key();
    let min_signers: u8 = 2;

    let multisig =
        sov_modules_api::Multisig::new(min_signers, vec![pub1.clone(), pub2.clone(), pub3.clone()]);
    let credential_id = multisig.credential_id::<TestHasher>();
    let multisig_address: <SolanaTestSpec as Spec>::Address = credential_id.into();
    let multisig_address_str = multisig_address.to_string();

    // Fund the multisig
    {
        let funding_json = create_transfer_tx_json(Amount(20_000), &multisig_address_str);
        let encoded_tx = funding_json.as_bytes().to_vec();
        let signer = admin.private_key();
        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx.clone(),
            chain_hash: RT::CHAIN_HASH,
            pubkey: signer.pub_key(),
            signature: signer.sign(&encoded_tx),
        };
        let response = submit_tx(test_rollup.api_client(), borsh::to_vec(&message).unwrap()).await;
        assert!(response.status().is_success());
    }

    // Only 1 signer when 2 are required
    let transfer_json =
        create_multisig_transfer_tx_json(Amount(5_000), RECIPIENT_ADDRESS, multisig_address);
    let json_bytes = transfer_json.as_bytes();

    let preamble = make_multisig_preamble_for_message(
        &[*pub1.bytes(), *pub2.bytes(), *pub3.bytes()],
        &RT::CHAIN_HASH,
        json_bytes.len() as u16,
    );

    let mut signed_message_with_preamble = preamble;
    signed_message_with_preamble.extend_from_slice(json_bytes);

    let sig1 = key1.sign(&signed_message_with_preamble);

    // Only signer 0 signed → bitfield 0b001
    let multisig_msg = SolanaOffchainSpecCompliantMultisigMessage::<S> {
        signed_message_with_preamble,
        signatures: vec![sig1].try_into().unwrap(),
        signer_bitfield: 0b001,
        min_signers,
    };

    let response = submit_tx(
        test_rollup.api_client(),
        borsh::to_vec(&multisig_msg).unwrap(),
    )
    .await;
    assert_eq!(
        response.status(),
        400,
        "Expected 400 for insufficient signatures"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_submit_spec_compliant_multisig_invalid_signature() {
    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    let key1 = Ed25519PrivateKey::generate();
    let key2 = Ed25519PrivateKey::generate();
    let key3 = Ed25519PrivateKey::generate();
    let pub1 = key1.pub_key();
    let pub2 = key2.pub_key();
    let pub3 = key3.pub_key();
    let min_signers: u8 = 2;

    let multisig =
        sov_modules_api::Multisig::new(min_signers, vec![pub1.clone(), pub2.clone(), pub3.clone()]);
    let credential_id = multisig.credential_id::<TestHasher>();
    let multisig_address: <SolanaTestSpec as Spec>::Address = credential_id.into();
    let multisig_address_str = multisig_address.to_string();

    // Fund the multisig
    {
        let funding_json = create_transfer_tx_json(Amount(20_000), &multisig_address_str);
        let encoded_tx = funding_json.as_bytes().to_vec();
        let signer = admin.private_key();
        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx.clone(),
            chain_hash: RT::CHAIN_HASH,
            pubkey: signer.pub_key(),
            signature: signer.sign(&encoded_tx),
        };
        let response = submit_tx(test_rollup.api_client(), borsh::to_vec(&message).unwrap()).await;
        assert!(response.status().is_success());
    }

    let transfer_json =
        create_multisig_transfer_tx_json(Amount(5_000), RECIPIENT_ADDRESS, multisig_address);
    let json_bytes = transfer_json.as_bytes();

    let preamble = make_multisig_preamble_for_message(
        &[*pub1.bytes(), *pub2.bytes(), *pub3.bytes()],
        &RT::CHAIN_HASH,
        json_bytes.len() as u16,
    );

    let mut signed_message_with_preamble = preamble;
    signed_message_with_preamble.extend_from_slice(json_bytes);

    let sig1 = key1.sign(&signed_message_with_preamble);
    // Corrupt signer 2's signature
    let mut sig2_bytes = key2.sign(&signed_message_with_preamble).msg_sig.to_bytes();
    sig2_bytes[10] = sig2_bytes[10].wrapping_add(1);
    let sig2: Ed25519Signature = sig2_bytes.as_slice().try_into().unwrap();

    // Signers 0 and 1 → bitfield 0b011
    let multisig_msg = SolanaOffchainSpecCompliantMultisigMessage::<S> {
        signed_message_with_preamble,
        signatures: vec![sig1, sig2].try_into().unwrap(),
        signer_bitfield: 0b011,
        min_signers,
    };

    let response = submit_tx(
        test_rollup.api_client(),
        borsh::to_vec(&multisig_msg).unwrap(),
    )
    .await;
    assert_eq!(response.status(), 400, "Expected 400 for invalid signature");
}

/// 2-of-3 multisig where one signer is a Ledger device (spec-compliant preamble) and the other
/// two are mock Ed25519 keys with hardcoded seeds. The Ledger signature and exact JSON bytes are
/// hardcoded — if the transaction format changes, re-sign on the Ledger and update the constants.
#[tokio::test(flavor = "multi_thread")]
async fn test_submit_ledger_signed_multisig_transaction() {
    // Ledger device pubkey (signer index 0 in the preamble)
    const LEDGER_ADDRESS: &str = "8YkzDTyLd3buhMw9CMfYYt3FLmcu1BeFr5nMeierYM1v";

    let (test_rollup, admin) = create_test_rollup().await.expect("Failed to create rollup");

    let ledger_pubkey: [u8; 32] = bs58::decode(LEDGER_ADDRESS)
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();

    // Fixed mock signers — seeds must not change, otherwise the multisig_id (and thus the JSON
    // the Ledger signs) will change and invalidate the hardcoded Ledger signature.
    let key2: Ed25519PrivateKey = vec![0x02u8; 32].try_into().unwrap();
    let key3: Ed25519PrivateKey = vec![0x03u8; 32].try_into().unwrap();
    let pub2 = key2.pub_key();
    let pub3 = key3.pub_key();
    let min_signers: u8 = 2;

    // The Ledger pubkey is placed at index 0 in the preamble signer list.
    let ledger_pub: <<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey =
        borsh::from_slice(&ledger_pubkey).unwrap();
    let multisig = sov_modules_api::Multisig::new(
        min_signers,
        vec![ledger_pub.clone(), pub2.clone(), pub3.clone()],
    );
    let credential_id = multisig.credential_id::<TestHasher>();
    let multisig_address: <SolanaTestSpec as Spec>::Address = credential_id.into();
    let multisig_address_str = multisig_address.to_string();

    // Fund the multisig address from admin
    {
        let funding_json_str = create_transfer_tx_json(Amount(13_000), &multisig_address_str);
        let encoded_tx = funding_json_str.as_bytes().to_vec();
        let signer = admin.private_key();
        let pubkey = signer.pub_key();
        let signature = signer.sign(&encoded_tx);

        let message = SolanaOffchainSimpleMessage::<S> {
            signed_message: encoded_tx,
            chain_hash: RT::CHAIN_HASH,
            pubkey,
            signature,
        };
        let raw_tx_bytes = borsh::to_vec(&message).unwrap();
        let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
        assert!(
            response.status().is_success(),
            "Expected funding transaction to succeed"
        );
    }

    // Build the V1 multisig transfer transaction
    let transfer_json_tx =
        create_multisig_transfer_tx_json(Amount(5_000), RECIPIENT_ADDRESS, multisig_address);
    let encoded_tx = transfer_json_tx.as_bytes();

    // Build the multisig preamble with all 3 pubkeys (Ledger at index 0)
    let preamble = make_multisig_preamble_for_message(
        &[ledger_pubkey, *pub2.bytes(), *pub3.bytes()],
        &RT::CHAIN_HASH,
        encoded_tx.len() as u16,
    );

    let mut signed_message_with_preamble = preamble;
    signed_message_with_preamble.extend_from_slice(encoded_tx);

    // Sanity check — if this changes, re-sign on the Ledger and update the signature below.
    let message_str = bs58::encode(&signed_message_with_preamble).into_string();
    assert_eq!(
        message_str,
        "A6sH1HqabEhAGQGYUgtarGjdDxcazsV4yHPicsZfiHDtN6kvfbHrUafGXwrTc52sHHTnd2yqsSvHbFPkBVgeqYZf3dWYeQJPDBL6wpCYdSd8pRseA2kRN9GB4kMHiZSujHzgQDC5dHgEFheGBSjwbCvGS6z6whMQ7b5Vi5vJkccMcojexkC9WRoGudKzbhAETrGgwQj2HAXSy822wrPVqYYELc6kWSLaFukqgJLKfxMrsJoaRs6mZfoAkSFEfSKpqLfzn3mLxoCren5X1V2afeMoYUke61W63WTKvgKMBLhLVJ2Qa3gfiJhCoHiqVcPLfaXQ8cQrpPijC5FPuDhBNJBpMSKcWCKbzAUeX8H4FEJMm5uXfuB72V3EXzTJRixsSQrvr7QY5eRkVeQzW5JXPxTgCtAJyr3zT5mMgtmzR3jGstgTXnwojNcbxvMJHxFVPefZXg3eK32CxcM5pmp43uL8nnXbYjbPAYBSupHbuDy34rAhZAt9gtoEVVqvFZo23HgVX8xFWK4XTv1n32s2DCQ3ZKA7v3hhBUe1DyUr5HeFZDqVjZZ7H9wopubLXLJHEMmVAxfp76NypBDhz5egqY3yEPbVfo4FANHWMqGBjv7fdsmq2HiLJwNXX5F9AdU89CDz3X2BaC9N5Q8YbWcXaVkNMc4nA3kzsDY61M21xRWWmxft1gxE39n6ebLWhwkCyEbHdWsmxXjWzAsxdz4LBeftRWMBWFYnpoHEUUFPwMAZAehEKAZH5tdm6NQUnqNSfCnngogLzTVDGq3mnT3mKspdit772k",
        "Multisig message bytes changed - re-sign on ledger and update the hardcoded signature"
    );

    // Ledger signs the full preamble+JSON (spec-compliant).
    // TODO: replace with actual Ledger signature after manual signing of `signed_message_with_preamble`.
    let ledger_signature: Ed25519Signature = bs58::decode(
        "5oyT3854c58jCoxFhXxK4tNJ8qVHvFHemDNL36tAinQogSmDFFzpTodB5zeTk4jvuCtpgWjoXwCEfUnBnBkZ2zge",
    )
    .into_vec()
    .unwrap()
    .as_slice()
    .try_into()
    .unwrap();

    // Mock signer key2 also signs the preamble+JSON
    let sig2 = key2.sign(&signed_message_with_preamble);

    // Bitfield: signers 0 (Ledger) and 1 (key2) signed → 0b011
    let multisig_msg = SolanaOffchainSpecCompliantMultisigMessage::<S> {
        signed_message_with_preamble,
        signatures: vec![ledger_signature, sig2].try_into().unwrap(),
        signer_bitfield: 0b011,
        min_signers,
    };

    let raw_tx_bytes = borsh::to_vec(&multisig_msg).unwrap();
    let response = submit_tx(test_rollup.api_client(), raw_tx_bytes).await;
    assert!(
        response.status().is_success(),
        "Expected Ledger multisig transaction to succeed"
    );

    let multisig_balance = query_balance(&test_rollup.client, &multisig_address_str).await;
    assert_eq!(
        multisig_balance,
        Some(Amount::new(8_000)),
        "Expected multisig to have 8,000 tokens remaining"
    );

    let recipient_balance = query_balance(&test_rollup.client, RECIPIENT_ADDRESS).await;
    assert_eq!(
        recipient_balance,
        Some(Amount::new(5_000)),
        "Expected recipient to have received 5,000 tokens"
    );
}
