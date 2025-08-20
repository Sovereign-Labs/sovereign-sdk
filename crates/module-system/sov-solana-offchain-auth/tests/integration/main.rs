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
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_api::{prelude::*, Base58Address, PrivateKey, SafeVec};
use sov_modules_api::{FullyBakedTx, RawTx, Runtime, Spec};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::{
    AuthorizedSequencers, PayeePolicy, PayerGenesisConfig, PaymasterConfig,
    PaymasterPolicyInitializer,
};
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::rest_api::AcceptTx;
use sov_solana_offchain_auth::authentication::{
    SolanaOffchainSimpleMessage, SolanaOffchainSpecCompliantMessage,
    SolanaOffchainUnsignedTransaction,
};
use sov_solana_offchain_auth::capabilities::{
    SolanaOffchainAuthenticator, SolanaOffchainAuthenticatorInput, SolanaOffchainAuthenticatorTrait,
};
use sov_solana_offchain_auth::utils::make_preamble_for_message;
use sov_state::{DefaultStorageSpec, ProverStorage};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{BankConfig, Runtime as _};
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    generate_runtime, RtAgnosticBlueprint, TestUser, TEST_DEFAULT_GAS_LIMIT, TEST_DEFAULT_MAX_FEE,
    TEST_DEFAULT_MAX_PRIORITY_FEE,
};
use sov_test_utils::{MockDaSpec, MockZkvm, MockZkvmCryptoSpec, TestHasher, TestStorageSpec};
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
    ProverStorage<DefaultStorageSpec<TestHasher>>,
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
        c.storage = dir.clone();
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

    // Set up the rollup the usual way.
    let mut slot_subscription = rollup.api_client().subscribe_slots().await.unwrap();
    rollup.da_service.produce_n_blocks_now(5).await.unwrap();
    for _ in 0..5 {
        let _ = slot_subscription.next().await.unwrap().unwrap();
    }

    Ok((rollup, admin))
}

fn create_transfer_tx_json(amount: Amount, recipient: &str) -> String {
    let msg: TestRuntimeCall<S> = TestRuntimeCall::Bank(BankCallMessage::Transfer {
        to: <S as Spec>::Address::from_str(recipient).unwrap(),
        coins: Coins {
            amount,
            // Use the gas token ID from the config (which is the pre-configured token)
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
    let solana_unsigned_tx = SolanaOffchainUnsignedTransaction::<RT, S> {
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

    // Query initial balance of recipient (should be 0)
    let response = client
        .query_rest_endpoint::<BalanceResponse>(&format!(
            "/modules/bank/tokens/{gas_token_id}/balances/{address}"
        ))
        .await
        .expect("Failed to query balance");

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
        r#"{"runtime_call":{"bank":{"transfer":{"to":"4zdwHNaEa5npHtRtaZ3RL1m6rptuQZ6RBLHG6cAyVHjL","coins":{"amount":"5000","token_id":"token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7"}}}},"uniqueness":{"generation":0},"details":{"max_priority_fee_bips":0,"max_fee":"100000000000","gas_limit":[1000000000,1000000000],"chain_id":4321},"chain_name":"TestChain"}"#
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
        "Expected transaction to succeed"
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
