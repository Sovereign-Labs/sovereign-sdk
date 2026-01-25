use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use alloy::consensus::{TxEip1559, TypedTransaction};
use alloy::eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy::eips::eip2718::Encodable2718;
use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use anyhow::{bail, Context};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use demo_stf::runtime::{Runtime, RuntimeCall};
use futures::stream::BoxStream;
use futures::StreamExt;
use secp256k1::{PublicKey, SecretKey};
use sov_api_spec::types as api_types;
use sov_bank::config_gas_token_id;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_eth_dev_signer::Signer;
use sov_evm::{EthereumAuthenticator, RlpEvmTransaction};
use sov_evm_test_utils::LegacySimpleStorage;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaSpec};
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey as _;
use sov_modules_api::{Amount, CryptoSpec, OperatingMode, RawTx, Runtime as RuntimeT, Spec};
use sov_modules_macros::config_value;
use sov_risc0_adapter::crypto::private_key::Risc0PrivateKey;
use sov_rollup_interface::node::da::DaService;
use sov_sequencer::ForcedTxBatchNotification;
use sov_synthetic_load::CallMessage as SyntheticLoadCall;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::test_rollup::{read_private_key, RollupBuilder};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;
use tokio::time::{sleep, timeout};

use crate::bank::helpers::{assert_balance, create_keys_and_addresses};
use crate::evm::evm_test_helper::{alloy_client, SENDER_PRIV_KEY};
use crate::test_helpers::{
    build_transfer_token_tx, test_genesis_source, DemoRollupSpec, CHAIN_HASH,
};

type TestSpec = DemoRollupSpec;
type TestPrivateKey = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;

const MAX_TX_FEE: Amount = Amount::new(100_000_000);
const UNREGISTERED_SENDER: MockAddress = MockAddress::new([121; 32]);
const MINIMUM_BOND: Amount = Amount::new(100_000_000);
const FINALIZATION_BLOCKS: u32 = 1;
const FORCED_TX_BATCH_TIMEOUT: Duration = Duration::from_secs(60);
const FORCED_TX_BATCH_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Verifies that a rollup with a preferred sequencer can handle forced registration from a different DA address.
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_forced_sequencer_registration() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");
    let (rollup, da_service) = setup().await;
    run_forced_tx_test(rollup, da_service, forced_sequencer_registration_test_case).await
}

async fn forced_sequencer_registration_test_case(
    da_service: Arc<impl DaService>,
    client: NodeClient,
    _http_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let key_and_address = read_private_key::<TestSpec>("tx_signer_private_key.json");
    let key = key_and_address.private_key;

    let tx = build_register_sequencer_tx(&key, 0);
    let mut forced_tx_batches = subscribe_forced_tx_batches(&client).await?;

    submit_forced_tx(
        da_service.as_ref(),
        ForcedTx::Runtime(tx),
        "Failed to submit forced sequencer registration to DA",
    )
    .await?;
    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;

    let allowed_sequencer = poll_until("sequencer registration to be visible", || async {
        client
            .sequencer_rollup_address::<TestSpec, MockDaSpec>(&UNREGISTERED_SENDER)
            .await
    })
    .await?;
    assert_eq!(allowed_sequencer.balance, MINIMUM_BOND);
    assert_eq!(allowed_sequencer.address, key_and_address.address);
    Ok(())
}

fn build_register_sequencer_tx(
    key: &TestPrivateKey,
    nonce: u64,
) -> Transaction<Runtime<TestSpec>, TestSpec> {
    let msg =
        RuntimeCall::<TestSpec>::SequencerRegistry(sov_sequencer_registry::CallMessage::Register {
            da_address: UNREGISTERED_SENDER,
            amount: MINIMUM_BOND,
        });
    let chain_id = config_value!("CHAIN_ID");
    let max_priority_fee_bips = PriorityFeeBips::ZERO;
    let max_fee = MAX_TX_FEE;
    let gas_limit = None;
    Transaction::<Runtime<TestSpec>, TestSpec>::new_signed_tx(
        key,
        &CHAIN_HASH,
        UnsignedTransaction::new(
            msg,
            chain_id,
            max_priority_fee_bips,
            max_fee,
            UniquenessData::Nonce(nonce),
            gas_limit,
        ),
    )
}

fn transaction_into_blob(transaction: Transaction<Runtime<TestSpec>, TestSpec>) -> Vec<u8> {
    borsh::to_vec(
        &<Runtime<TestSpec> as RuntimeT<TestSpec>>::Auth::encode_with_standard_auth(RawTx {
            data: borsh::to_vec(&transaction).unwrap(),
        }),
    )
    .unwrap()
}

async fn submit_preferred_tx(
    client: &NodeClient,
    tx: &Transaction<Runtime<TestSpec>, TestSpec>,
) -> anyhow::Result<()> {
    client
        .client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&borsh::to_vec(tx).unwrap()),
        })
        .await?;
    Ok(())
}

enum ForcedTx<'a> {
    Runtime(Transaction<Runtime<TestSpec>, TestSpec>),
    Evm {
        account: &'a EvmAccount,
        tx: TxEip1559,
    },
}

async fn submit_forced_tx(
    da_service: &impl DaService,
    tx: ForcedTx<'_>,
    description: &str,
) -> anyhow::Result<()> {
    let blob = match tx {
        ForcedTx::Runtime(tx) => transaction_into_blob(tx),
        ForcedTx::Evm { account, tx } => evm_transaction_into_blob(account, tx),
    };
    let _receipt = da_service
        .send_transaction(&blob)
        .await
        .await?
        .expect(description);
    Ok(())
}

async fn produce_block_and_wait_slot(
    rollup: &TestRollup<MockDemoRollup<Native>>,
    slot_subscription: &mut BoxStream<'static, anyhow::Result<api_types::Slot>>,
) -> anyhow::Result<api_types::Slot> {
    rollup.da_service.produce_block_now().await?;
    slot_subscription
        .next()
        .await
        .context("Slot subscription ended")?
}

async fn subscribe_forced_tx_batches(
    client: &NodeClient,
) -> anyhow::Result<BoxStream<'static, anyhow::Result<ForcedTxBatchNotification>>> {
    let subscription = client
        .client
        .subscribe_to_ws::<ForcedTxBatchNotification>(
            "/sequencer/test-utils/forced-tx-batch-notifier/ws",
        )
        .await?;
    Ok(subscription)
}

/// A handy helper that waits until the *sequencer* has processed a forced tx batch.
/// This ensures that the test will fail if the sequencer misbehaves even if the node later picks up the batch.
async fn wait_for_forced_tx_batch(
    subscription: &mut BoxStream<'static, anyhow::Result<ForcedTxBatchNotification>>,
) -> anyhow::Result<ForcedTxBatchNotification> {
    timeout(FORCED_TX_BATCH_TIMEOUT, subscription.next())
        .await
        .context("Timed out waiting for forced tx batch notification")?
        .transpose()?
        .context("Forced tx batch notification stream ended")
}

async fn poll_until<T, F, Fut>(description: &str, mut check: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<Option<T>>>,
{
    let deadline = tokio::time::Instant::now() + FORCED_TX_BATCH_TIMEOUT;
    loop {
        if let Some(value) = check().await? {
            return Ok(value);
        }

        if tokio::time::Instant::now() >= deadline {
            bail!("Timed out waiting for {description}");
        }

        sleep(FORCED_TX_BATCH_POLL_INTERVAL).await;
    }
}

async fn wait_for_bank_balance(
    client: &NodeClient,
    expected_amount: u128,
    token_id: sov_bank::TokenId,
    user_address: <TestSpec as Spec>::Address,
) -> anyhow::Result<()> {
    poll_until(
        &format!("bank balance {} for {}", expected_amount, user_address),
        || async {
            let success = assert_balance(client, expected_amount, token_id, user_address, None)
                .await
                .is_ok();
            Ok(success.then_some(()))
        },
    )
    .await
}

// ============================================================================
// EVM Transaction Tests for Unregistered Sequencers
// ============================================================================

/// Runs a forced transaction test with standard setup.
/// Handles the tokio::select! boilerplate for running the test case
/// concurrently with the rollup task.
async fn run_forced_tx_test<Da, F, Fut>(
    rollup: TestRollup<MockDemoRollup<Native>>,
    da_service: Arc<Da>,
    test_fn: F,
) -> anyhow::Result<()>
where
    Da: DaService,
    F: FnOnce(Arc<Da>, NodeClient, std::net::SocketAddr) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let http_addr = rollup.http_addr;
    let client = rollup.client.clone();

    tokio::select! {
        err = rollup.rollup_task => err??,
        res = test_fn(da_service, client, http_addr) => res?,
    };
    Ok(())
}

/// Helper struct for EVM account management in tests.
struct EvmAccount(SecretKey);

impl EvmAccount {
    fn generate() -> Self {
        use rand::RngCore;
        let mut key_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key_bytes);
        let secret_key = SecretKey::from_slice(&key_bytes).unwrap();
        Self(secret_key)
    }

    fn from_private_key_hex(hex_key: &str) -> Self {
        let key_bytes = hex::decode(hex_key.trim_start_matches("0x")).unwrap();
        let secret_key = SecretKey::from_slice(&key_bytes).unwrap();
        Self(secret_key)
    }

    fn public_key(&self) -> PublicKey {
        PublicKey::from_secret_key(secp256k1::SECP256K1, &self.0)
    }

    fn address(&self) -> Address {
        reth_primitives::public_key_to_address(self.public_key())
    }

    fn sign(&self, tx: TypedTransaction) -> RlpEvmTransaction {
        let signer = Signer::new(self.0);
        let signed_tx = signer.sign_transaction(tx).unwrap();
        let rlp = signed_tx.encoded_2718();
        RlpEvmTransaction { rlp }
    }
}

/// Builds an EVM transfer transaction.
fn build_evm_transfer_tx(to: Address, value: U256, nonce: u64) -> TxEip1559 {
    TxEip1559 {
        to: TxKind::Call(to),
        value,
        nonce,
        gas_limit: 1_000_000, // Use larger gas limit like integration tests
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    }
}

/// Builds an EVM contract call transaction.
fn build_evm_contract_call_tx(contract_address: Address, calldata: Bytes, nonce: u64) -> TxEip1559 {
    TxEip1559 {
        to: TxKind::Call(contract_address),
        value: U256::ZERO,
        input: calldata,
        nonce,
        gas_limit: 1_000_000, // Use larger gas limit like other tests
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    }
}

/// Encodes an EVM transaction into a blob suitable for unregistered batch submission.
fn evm_transaction_into_blob(account: &EvmAccount, tx: TxEip1559) -> Vec<u8> {
    let signed_eth_tx = account.sign(TypedTransaction::Eip1559(tx));
    let data = borsh::to_vec(&signed_eth_tx).unwrap();
    let raw_tx = RawTx { data };

    // Use encode_with_ethereum_auth to create EvmAuthenticatorInput::Evm variant
    borsh::to_vec(
        &<Runtime<TestSpec> as EthereumAuthenticator<TestSpec>>::encode_with_ethereum_auth(raw_tx),
    )
    .unwrap()
}

async fn setup() -> (TestRollup<MockDemoRollup<Native>>, Arc<impl DaService>) {
    setup_with_block_producing(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await
}

async fn setup_with_block_producing(
    block_producing: BlockProducingConfig,
) -> (TestRollup<MockDemoRollup<Native>>, Arc<impl DaService>) {
    let is_manual = matches!(&block_producing, BlockProducingConfig::Manual);
    let rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Zk),
        block_producing,
        FINALIZATION_BLOCKS,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.automatic_batch_production = true;
        c.rollup_prover_config = None;
        c.max_channel_size = 1;
        c.max_infos_in_db = 1;
    })
    .start()
    .await
    .unwrap();

    if is_manual {
        rollup.tenderly_produce_blocks(10).await.unwrap();
    } else {
        rollup.wait_for_next_blocks(10).await;
    }
    rollup.wait_for_sequencer_ready().await.unwrap();

    let da_service = Arc::new(
        rollup
            .da_service
            .another_on_the_same_layer(UNREGISTERED_SENDER)
            .await,
    );

    (rollup, da_service)
}

/// Verifies that EVM transactions can be processed from unregistered sequencer batches.
/// This is the core test for the new functionality that allows EVM txs in unregistered batches.
#[tokio::test(flavor = "multi_thread")]
async fn test_evm_transfer_in_unregistered_batch() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");
    let (rollup, da_service) = setup().await;
    run_forced_tx_test(rollup, da_service, evm_tx_unregistered_test_case).await
}

async fn evm_tx_unregistered_test_case(
    da_service: Arc<impl DaService>,
    client: NodeClient,
    http_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    // Use the funded sender from genesis (same as used in EVM tests)
    let sender = EvmAccount::from_private_key_hex(SENDER_PRIV_KEY);

    // Generate a random receiver
    let receiver_address = EvmAccount::generate().address();

    // Get initial balance via RPC
    let provider = alloy_client(http_addr);
    let initial_balance = provider.get_balance(receiver_address).await?;
    assert_eq!(
        initial_balance,
        U256::ZERO,
        "Receiver should start with 0 balance"
    );

    let mut forced_tx_batches = subscribe_forced_tx_batches(&client).await?;

    // Submit EVM transfer via unregistered DA service (forced tx).
    let transfer_amount = U256::from(1u64);
    let nonce = provider.get_transaction_count(sender.address()).await?;
    let forced_tx = build_evm_transfer_tx(receiver_address, transfer_amount, nonce);
    submit_forced_tx(
        da_service.as_ref(),
        ForcedTx::Evm {
            account: &sender,
            tx: forced_tx,
        },
        "Failed to submit EVM tx blob to DA",
    )
    .await?;

    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;

    poll_until(
        &format!("EVM balance {} for {}", transfer_amount, receiver_address),
        || async {
            let balance = provider.get_balance(receiver_address).await?;
            Ok((balance == transfer_amount).then_some(()))
        },
    )
    .await?;
    Ok(())
}

/// Verifies that standard (bank) transactions can be processed from unregistered sequencer batches.
#[tokio::test(flavor = "multi_thread")]
async fn test_bank_transfer_in_unregistered_batch() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");
    let (rollup, da_service) = setup().await;
    run_forced_tx_test(rollup, da_service, bank_transfer_unregistered_test_case).await
}

async fn bank_transfer_unregistered_test_case(
    da_service: Arc<impl DaService>,
    client: NodeClient,
    _http_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let (key, _, _, recipient_address) = create_keys_and_addresses();
    let token_id = config_gas_token_id();
    let transfer_amount = 250u128;

    let transfer_tx =
        build_transfer_token_tx(&key, token_id, recipient_address, transfer_amount, 0);
    let mut forced_tx_batches = subscribe_forced_tx_batches(&client).await?;

    submit_forced_tx(
        da_service.as_ref(),
        ForcedTx::Runtime(transfer_tx),
        "Failed to submit bank transfer blob to DA",
    )
    .await?;
    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;

    wait_for_bank_balance(&client, transfer_amount, token_id, recipient_address).await?;

    Ok(())
}

/// Builds a transaction that writes a large state value, consuming metered gas.
/// Gas is charged based on the size of data written.
fn build_state_heavy_tx(
    key: &TestPrivateKey,
    data_size: u64,
    nonce: u64,
) -> Transaction<Runtime<TestSpec>, TestSpec> {
    let msg = RuntimeCall::<TestSpec>::SyntheticLoad(SyntheticLoadCall::ReadAndSetHeavyState {
        number_of_new_values: data_size,
        max_heavy_state_size: data_size,
        salt: 42,
    });
    let chain_id = config_value!("CHAIN_ID");
    let max_priority_fee_bips = PriorityFeeBips::ZERO;
    let max_fee = MAX_TX_FEE;
    Transaction::<Runtime<TestSpec>, TestSpec>::new_signed_tx(
        key,
        &CHAIN_HASH,
        UnsignedTransaction::new(
            msg,
            chain_id,
            max_priority_fee_bips,
            max_fee,
            UniquenessData::Nonce(nonce),
            None,
        ),
    )
}

/// Verifies that forced transactions exceeding the gas limit are rejected.
/// Uses state-heavy operations (writes large data) which are metered.
#[tokio::test(flavor = "multi_thread")]
async fn test_forced_tx_exceeding_gas_limit_rejected() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");
    // Set explicit gas parameters so production changes don't break this test.
    // Gas limit for forced txs: 50k per dimension.
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_UNREGISTERED_SEQUENCER_EXEC_GAS_PER_TX",
        "[50000,50000]",
    );
    // Per-byte storage update cost: 100 gas/byte (default is 1).
    // Writing 1000 u64s = 8000 bytes * 100 gas/byte = 800k gas > 50k limit.
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_GAS_TO_CHARGE_PER_BYTE_STORAGE_UPDATE",
        "[100,100]",
    );
    let (rollup, da_service) = setup().await;
    run_forced_tx_test(rollup, da_service, forced_tx_gas_limit_test_case).await
}

async fn forced_tx_gas_limit_test_case(
    da_service: Arc<impl DaService>,
    client: NodeClient,
    _http_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let (key, _, _, recipient_address) = create_keys_and_addresses();
    let token_id = config_gas_token_id();
    let mut forced_tx_batches = subscribe_forced_tx_batches(&client).await?;

    // Submit a forced tx that exceeds the gas limit by writing large state data.
    // 1000 u64s = 8000 bytes * 100 gas/byte = 800k gas > 50k limit.
    let excessive_data_size = 1000;
    let gas_heavy_tx = build_state_heavy_tx(&key, excessive_data_size, 0);
    submit_forced_tx(
        da_service.as_ref(),
        ForcedTx::Runtime(gas_heavy_tx),
        "Failed to submit gas-heavy tx",
    )
    .await?;
    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;

    // The gas-heavy tx should have been rejected. Submit a preferred tx with the same nonce (0).
    // If the gas-heavy tx was processed, this would fail due to nonce reuse.
    let transfer_amount = 100u128;
    let valid_tx = build_transfer_token_tx(&key, token_id, recipient_address, transfer_amount, 0);
    submit_preferred_tx(&client, &valid_tx).await?;

    // Verify the transfer succeeded, confirming the gas-heavy tx was rejected.
    wait_for_bank_balance(&client, transfer_amount, token_id, recipient_address).await?;

    Ok(())
}

/// Verifies that only UNREGISTERED_BLOBS_PER_SLOT (5) forced blobs are processed per slot.
/// Blobs beyond this limit should be ignored.
#[tokio::test(flavor = "multi_thread")]
async fn test_max_forced_blobs_per_slot() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");
    // Use manual block production to ensure all forced txs land in one DA block
    let (rollup, da_service) = setup_with_block_producing(BlockProducingConfig::Manual).await;

    let res = max_blobs_per_slot_test_case(&rollup, da_service).await;
    let shutdown_res = rollup.shutdown().await;
    shutdown_res?;
    res
}

async fn max_blobs_per_slot_test_case(
    rollup: &TestRollup<MockDemoRollup<Native>>,
    da_service: Arc<impl DaService>,
) -> anyhow::Result<()> {
    const MAX_BLOBS_PER_SLOT: usize = 5; // From constants.toml: UNREGISTERED_BLOBS_PER_SLOT
    const BLOBS_TO_SEND: usize = 7; // Send more than the limit

    let client = &rollup.client;
    let (key, _, _, recipient_address) = create_keys_and_addresses();
    let token_id = config_gas_token_id();
    let transfer_amount = 10u128;

    let mut slot_subscription = rollup.api_client().subscribe_slots().await?;

    // Submit 7 forced txs with nonces 0-6 without producing a DA block.
    // They will all queue up to be included in the next DA block.
    for i in 0..BLOBS_TO_SEND {
        let tx =
            build_transfer_token_tx(&key, token_id, recipient_address, transfer_amount, i as u64);
        submit_forced_tx(
            da_service.as_ref(),
            ForcedTx::Runtime(tx),
            "Failed to submit forced tx",
        )
        .await?;
    }
    let mut state_update_subscription = rollup.subscribe_state_updates().await?;
    let mut blobs_subscription = rollup.subscribe_to_blobs_from_blob_sender().await?;

    // Wait for all of the blobs to processed. We need to keep producing blocks so the hte visible slot number is incremented.
    for i in 0..20 {
        produce_block_and_wait_slot(rollup, &mut slot_subscription).await?;
        let next = state_update_subscription.next().await.unwrap()?;
        while let Ok(Some(Ok(next))) =
            tokio::time::timeout(Duration::from_secs(10), blobs_subscription.next()).await
        {
            let next_str = serde_json::to_string(&next).unwrap();
            if next_str.contains("Published") {
                break;
            }
        }
    }

    // Verify that exactly MAX_BLOBS_PER_SLOT (5) transactions were processed.
    // Balance should be 5 * 10 = 50, not 7 * 10 = 70.
    let expected_balance = transfer_amount * MAX_BLOBS_PER_SLOT as u128;
    wait_for_bank_balance(client, expected_balance, token_id, recipient_address).await?;

    Ok(())
}

/// Verifies forced transactions show up in sequencer state after a resync
#[tokio::test(flavor = "multi_thread")]
async fn test_forced_txs_survive_resync() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "500");

    let (rollup, da_service) = setup_with_block_producing(BlockProducingConfig::Manual).await;
    let res = forced_txs_resync_test_case(&rollup, da_service).await;
    let shutdown_res = rollup.shutdown().await;
    shutdown_res?;
    res?;
    Ok(())
}

/// Verifies forced transactions show up in sequencer state after a resync
///
/// Steps:
/// 1. Fund a key for post-resync valid forced txs.
/// 2. Pause the sequencer and send forced txs to unsync the node.
/// 3. Resume the sequencer and wait for it to become unready then ready again.
/// 4. Send more forced txs (both valid and invalid) to verify they don't break the sequencer post-resync.
/// 5. Verify all pre-resync forced txs were processed with exact balance checks.
/// 6. Send one more preferred tx to make sure everything is working correctly after the resync.
async fn forced_txs_resync_test_case(
    rollup: &TestRollup<MockDemoRollup<Native>>,
    forced_da_service: Arc<impl DaService>,
) -> anyhow::Result<()> {
    const RESYNC_FORCED_BLOCKS: usize = 15;
    const RESYNC_EXTRA_BLOCKS: usize = 15;
    const GAS_FUNDING_AMOUNT: u128 = 1_000_000_000_000;

    let client = &rollup.client;
    let (key, _, _, recipient_address) = create_keys_and_addresses();
    let token_id = config_gas_token_id(); // Use gas token - no need to create tokens

    // For post-resync forced tx coverage:
    // - funded_key: has gas tokens, so its txs will succeed
    // - unfunded_key: no gas tokens, so its txs will fail (tests error handling)
    // - post_resync_recipient: separate recipient so post-resync txs don't affect main balance checks
    let funded_key = Risc0PrivateKey::generate();
    let funded_address = funded_key.pub_key().credential_id().into();
    let unfunded_key = Risc0PrivateKey::generate();
    let post_resync_recipient: <TestSpec as Spec>::Address =
        Risc0PrivateKey::generate().pub_key().credential_id().into();

    let preferred_transfer = 50u128;
    let forced_transfer = 5u128;
    let total_forced_tx_amount = forced_transfer * RESYNC_FORCED_BLOCKS as u128;
    let mut slot_subscription = rollup.api_client().subscribe_slots().await?;

    // Fund the key for post-resync valid forced txs
    {
        let fund_tx = build_transfer_token_tx::<TestSpec>(
            &key,
            token_id,
            funded_address,
            GAS_FUNDING_AMOUNT,
            0,
        );
        submit_preferred_tx(client, &fund_tx).await?;
        produce_block_and_wait_slot(rollup, &mut slot_subscription).await?;
    }

    let forced_start_nonce = 1u64;
    // Pause the sequencer and send forced txs enough times to unsync the sequencer. We want to check that the sequencer handles this case correctly
    rollup.pause_preferred_batches().await;
    for i in 0..RESYNC_FORCED_BLOCKS {
        let tx = build_transfer_token_tx(
            &key,
            token_id,
            recipient_address,
            forced_transfer,
            forced_start_nonce + i as u64,
        );
        submit_forced_tx(
            forced_da_service.as_ref(),
            ForcedTx::Runtime(tx),
            "Failed to submit forced bank transfer blob to DA",
        )
        .await?;
        rollup.da_service.produce_block_now().await?;
    }
    rollup.resume_preferred_batches().await;

    // Make sure the sequencer becomes unready. Then, let it get back to ready.
    rollup.wait_for_sequencer_not_ready().await?;
    rollup.wait_for_sequencer_ready().await?;

    // Wait for all forced blocks to be processed
    let mut state_update_subscription = rollup.subscribe_state_updates().await?;
    let mut slot_num_of_last_forced_tx = 0;
    for _ in 0..RESYNC_FORCED_BLOCKS {
        let slot = slot_subscription.next().await.unwrap()?;
        slot_num_of_last_forced_tx = slot.number;
    }
    // This part is sensitive to numbers; we need to produce more blocks than we had forced blocks *plus* in flight batches. This gives time for the blob sender
    // to actually send all of the original blobs on chain and then the preferred sequencer to create new batches that increment the visible slot number.
    // This is necessary because the preferred sequencer tries not to produce batches when there are more than a few blobs in flight, and the blob sender
    // only recognizes that a blob is no longer in flight when it appears in the ledger DB - but that only happens after the visible slot number has been incremented.
    // So (because of all the forced txs) we have a mild chicken-and-egg problem where the preferred sequencer only produces batches very infrequently.
    //
    // Originally, these da blocks were empty - and having them empty works fine. But we more recently added some forced txs from other addresses (both valid and invalid)
    // during these blocks to make sure that forced txs  don't cause any breakage in the sequencer after resync.
    let nonce_of_first_possibly_skipped_tx = 0;
    for i in 0..RESYNC_FORCED_BLOCKS + RESYNC_EXTRA_BLOCKS {
        // Send some txs from other addresses (both valid and invalid). We'll sanity check that at least one of these went through,
        // but (unlike the primary transfers above), we're not going to wait for the sequencer to increment the visible slot number enough to process all of them.
        {
            let tx = build_transfer_token_tx(
                if i % 2 == 0 {
                    &funded_key
                } else {
                    &unfunded_key
                },
                token_id,
                post_resync_recipient,
                forced_transfer,
                nonce_of_first_possibly_skipped_tx + (i / 2) as u64,
            );
            submit_forced_tx(
                forced_da_service.as_ref(),
                ForcedTx::Runtime(tx),
                "Failed to submit forced bank transfer blob to DA",
            )
            .await?;
        }

        // The important part - produce a block to give the sequencer a chance to increment the visible slot number.
        // Wait for the slot update notification and the state update notification to be sure we're not racing.
        let _slot = produce_block_and_wait_slot(rollup, &mut slot_subscription).await?;
    }

    // Now that we've produced batches to increment the visible slot number, wait for the state update notification that we've processed all of the forced txs.
    for _ in 0..RESYNC_FORCED_BLOCKS + RESYNC_EXTRA_BLOCKS {
        let update = state_update_subscription
            .next()
            .await
            .context("State update subscription ended")??;
        if update.slot_number.get() >= slot_num_of_last_forced_tx {
            break;
        }
    }

    // Verify all pre-resync forced txs went through by checking recipient balance
    assert_balance(
        client,
        total_forced_tx_amount,
        token_id,
        recipient_address,
        None,
    )
    .await?;

    // Send one more preferred tx to make sure everything is working correctly after the resync.
    let preferred_tx = build_transfer_token_tx::<TestSpec>(
        &key,
        token_id,
        recipient_address,
        preferred_transfer,
        forced_start_nonce + RESYNC_FORCED_BLOCKS as u64,
    );
    submit_preferred_tx(client, &preferred_tx).await?;
    wait_for_bank_balance(
        client,
        total_forced_tx_amount + preferred_transfer,
        token_id,
        recipient_address,
    )
    .await?;

    // As an extra sanity check, make sure that at least one of our forced txs went through *after* the resync.
    // This uses a separate recipient so post-resync txs don't affect the main balance checks above.
    let post_resync_balance = client
        .get_balance::<TestSpec>(&post_resync_recipient, &token_id, None)
        .await
        .with_context(|| {
            format!("Failed to get post-resync balance for user {post_resync_recipient})")
        })?;

    assert_ne!(
        post_resync_balance,
        Amount::ZERO,
        "Post-resync recipient balance should be non-zero"
    );

    Ok(())
}

/// Verifies that EVM contract calls can be processed from unregistered sequencer batches.
/// Uses the forced batch notifier to avoid timing races when waiting for execution.
#[tokio::test(flavor = "multi_thread")]
async fn test_evm_contract_call_in_unregistered_batch() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");
    let (rollup, da_service) = setup().await;
    run_forced_tx_test(rollup, da_service, evm_contract_call_unregistered_test_case).await
}

async fn evm_contract_call_unregistered_test_case(
    da_service: Arc<impl DaService>,
    client: NodeClient,
    http_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    sov_test_utils::initialize_logging();
    let sender = EvmAccount::from_private_key_hex(SENDER_PRIV_KEY);
    let mut forced_tx_batches = subscribe_forced_tx_batches(&client).await?;

    // First, deploy the SimpleStorage contract via SimpleStorageClient
    let contract = LegacySimpleStorage::default();
    let simple_storage =
        sov_eth_client::SimpleStorageClient::new(SENDER_PRIV_KEY, contract, http_addr).await;

    // Deploy the contract
    let tx_hash = simple_storage
        .deploy_contract()
        .await
        .expect("Failed to deploy contract");
    let receipt = simple_storage.wait_for_receipt(tx_hash).await;
    let contract_address = receipt
        .contract_address
        .expect("Contract should have been deployed");

    // Wait a bit for state to settle

    // Now call setValue via unregistered batch
    let set_value: u32 = 42;
    // Create a new contract instance for generating calldata
    let contract_for_calldata = LegacySimpleStorage::default();
    let calldata = Bytes::from(contract_for_calldata.set(set_value).to_vec());

    let call_tx = build_evm_contract_call_tx(contract_address, calldata, 1);

    // Submit via unregistered DA service
    submit_forced_tx(
        da_service.as_ref(),
        ForcedTx::Evm {
            account: &sender,
            tx: call_tx,
        },
        "Failed to submit contract call blob to DA",
    )
    .await?;
    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;
    let provider = alloy_client(http_addr);

    // Verify the contract state was updated by calling getValue
    let get_calldata = Bytes::from(contract_for_calldata.get().to_vec());
    let call_request = TransactionRequest::default()
        .with_to(contract_address)
        .with_input(get_calldata);

    let result = provider.call(call_request).await?;
    let resp_array: [u8; 32] = result.to_vec().try_into().unwrap();
    let value = U256::from_be_bytes(resp_array);
    assert_eq!(
        value,
        U256::from(set_value),
        "Contract value should match set value"
    );

    Ok(())
}
