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
use sov_modules_api::{Amount, CryptoSpec, OperatingMode, RawTx, Runtime as RuntimeT, Spec};
use sov_modules_macros::config_value;
use sov_rollup_interface::node::da::DaService;
use sov_sequencer::ForcedTxBatchNotification;
use sov_sequencer_registry::KnownSequencer;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::test_rollup::{read_private_key, RollupBuilder};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;
use tokio::time::{sleep, timeout};

use crate::bank::helpers::{
    assert_balance, build_create_token_tx, create_keys_and_addresses, send_tx_and_wait_for_status,
};
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
const RESYNC_FORCED_BLOCKS: usize = 15;

// Verifies that a rollup with a preferred sequencer can handle forced registration from a different DA address.
// Steps:
// 1. Start the rollup with the preferred sequencer and demo-stf.
// 2. Submit a forced registration from another DA address.
// 3. Wait until the forced registration is processed.
// 4. Use the REST API to confirm that the new sequencer is registered.
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_forced_sequencer_registration() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");

    let rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Zk),
        // We need to set the block producing mode to periodic to ensure that the forced registration
        // eventually succeeds because the registration batch is deferred.
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
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
    .await?;

    let da_service = Arc::new(
        rollup
            .da_service
            .another_on_the_same_layer(UNREGISTERED_SENDER)
            .await,
    );

    tokio::select! {
        err = rollup.rollup_task => err??,
        res = forced_sequencer_registration_test_case(da_service, &rollup.client) => res?,
    };
    Ok(())
}

async fn forced_sequencer_registration_test_case(
    da_service: Arc<impl DaService>,
    client: &NodeClient,
) -> anyhow::Result<()> {
    let key_and_address = read_private_key::<TestSpec>("tx_signer_private_key.json");
    let key = key_and_address.private_key;

    let tx = build_register_sequencer_tx(&key, 0);
    let blob = transaction_into_blob(tx);
    let mut forced_tx_batches = subscribe_forced_tx_batches(client).await?;

    let _receipt = da_service
        .send_transaction(&blob)
        .await
        .await?
        .expect("Failed to submit forced sequencer registration to DA");
    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;

    let allowed_sequencer = wait_for_registered_sequencer(client, &UNREGISTERED_SENDER).await?;
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

async fn wait_for_forced_tx_batch(
    subscription: &mut BoxStream<'static, anyhow::Result<ForcedTxBatchNotification>>,
) -> anyhow::Result<ForcedTxBatchNotification> {
    timeout(FORCED_TX_BATCH_TIMEOUT, subscription.next())
        .await
        .context("Timed out waiting for forced tx batch notification")?
        .transpose()?
        .context("Forced tx batch notification stream ended")
}

async fn wait_for_registered_sequencer(
    client: &NodeClient,
    da_address: &MockAddress,
) -> anyhow::Result<KnownSequencer<TestSpec>> {
    let deadline = tokio::time::Instant::now() + FORCED_TX_BATCH_TIMEOUT;
    loop {
        if let Some(entry) = client
            .sequencer_rollup_address::<TestSpec, MockDaSpec>(da_address)
            .await?
        {
            return Ok(entry);
        }

        if tokio::time::Instant::now() >= deadline {
            bail!("Timed out waiting for sequencer registration to be visible");
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
    let deadline = tokio::time::Instant::now() + FORCED_TX_BATCH_TIMEOUT;
    loop {
        if assert_balance(client, expected_amount, token_id, user_address, None)
            .await
            .is_ok()
        {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            bail!(
                "Timed out waiting for bank balance {} for {}",
                expected_amount,
                user_address
            );
        }

        sleep(FORCED_TX_BATCH_POLL_INTERVAL).await;
    }
}

async fn wait_for_evm_balance(
    provider: &impl Provider,
    address: Address,
    expected_balance: U256,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + FORCED_TX_BATCH_TIMEOUT;
    loop {
        if provider.get_balance(address).await? == expected_balance {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            bail!(
                "Timed out waiting for EVM balance {} for {}",
                expected_balance,
                address
            );
        }

        sleep(FORCED_TX_BATCH_POLL_INTERVAL).await;
    }
}

// ============================================================================
// EVM Transaction Tests for Unregistered Sequencers
// ============================================================================

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
fn build_evm_transfer_tx(_account: &EvmAccount, to: Address, value: U256, nonce: u64) -> TxEip1559 {
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
fn build_evm_contract_call_tx(
    _account: &EvmAccount,
    contract_address: Address,
    calldata: Bytes,
    nonce: u64,
) -> TxEip1559 {
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

/// Address for second unregistered sender (different from UNREGISTERED_SENDER used in registration test)
const UNREGISTERED_EVM_SENDER: MockAddress = MockAddress::new([122; 32]);

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
            .another_on_the_same_layer(UNREGISTERED_EVM_SENDER)
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

    let http_addr = rollup.http_addr;
    let client = rollup.client.clone();

    tokio::select! {
        err = rollup.rollup_task => err??,
        res = evm_tx_unregistered_test_case(da_service, &client, http_addr) => res?,
    };
    Ok(())
}

async fn evm_tx_unregistered_test_case(
    da_service: Arc<impl DaService>,
    client: &NodeClient,
    http_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    // Use the funded sender from genesis (same as used in EVM tests)
    let sender = EvmAccount::from_private_key_hex(SENDER_PRIV_KEY);
    // let sender_address = sender.address();

    // Generate a random receiver
    let receiver = EvmAccount::generate();
    let receiver_address = receiver.address();

    // Get initial balance via RPC
    let provider = alloy_client(http_addr);
    let initial_balance = provider.get_balance(receiver_address).await?;
    assert_eq!(
        initial_balance,
        U256::ZERO,
        "Receiver should start with 0 balance"
    );

    // Build EVM transfer tx
    let transfer_amount = U256::from(1u64);
    let mut forced_tx_batches = subscribe_forced_tx_batches(client).await?;

    // Send via preferred sequencer first to establish nonce ordering.
    let preferred_nonce = provider.get_transaction_count(sender.address()).await?;
    let tx = build_evm_transfer_tx(&sender, receiver_address, transfer_amount, preferred_nonce);
    provider
        .send_transaction(tx.into())
        .await?
        .get_receipt()
        .await?;

    // Submit via unregistered DA service after preferred tx is confirmed.
    let forced_nonce = provider.get_transaction_count(sender.address()).await?;
    let forced_tx = build_evm_transfer_tx(&sender, receiver_address, transfer_amount, forced_nonce);
    let blob = evm_transaction_into_blob(&sender, forced_tx);
    let _receipt = da_service
        .send_transaction(&blob)
        .await
        .await?
        .expect("Failed to submit EVM tx blob to DA");

    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;

    wait_for_evm_balance(
        &provider,
        receiver_address,
        transfer_amount + transfer_amount,
    )
    .await?;

    // Note: the sequencer sends the batch notification before udpating the API state, so we still have to wait for the balance to be updated.
    let tx = build_evm_transfer_tx(&sender, receiver_address, transfer_amount, forced_nonce + 1);
    provider
        .send_transaction(tx.into())
        .await?
        .get_receipt()
        .await?;

    assert_eq!(
        provider.get_balance(receiver_address).await?,
        (transfer_amount + transfer_amount + transfer_amount)
    );
    Ok(())
}

/// Verifies that standard (bank) transactions can be processed from unregistered sequencer batches.
#[tokio::test(flavor = "multi_thread")]
async fn test_bank_transfer_in_unregistered_batch() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");

    let (rollup, da_service) = setup().await;
    let client = rollup.client.clone();

    tokio::select! {
        err = rollup.rollup_task => err??,
        res = bank_transfer_unregistered_test_case(da_service, &client) => res?,
    };
    Ok(())
}

async fn bank_transfer_unregistered_test_case(
    da_service: Arc<impl DaService>,
    client: &NodeClient,
) -> anyhow::Result<()> {
    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();
    let initial_balance = 1_000u128;
    let transfer_amount = 250u128;

    let create_token_tx = build_create_token_tx(&key, 0, initial_balance);
    send_tx_and_wait_for_status(&[create_token_tx], client).await?;
    wait_for_bank_balance(client, initial_balance, token_id, user_address).await?;

    let transfer_tx =
        build_transfer_token_tx(&key, token_id, recipient_address, transfer_amount, 1);
    let blob = transaction_into_blob(transfer_tx);
    let mut forced_tx_batches = subscribe_forced_tx_batches(client).await?;

    let _receipt = da_service
        .send_transaction(&blob)
        .await
        .await?
        .expect("Failed to submit bank transfer blob to DA");
    wait_for_forced_tx_batch(&mut forced_tx_batches).await?;

    wait_for_bank_balance(
        client,
        initial_balance - transfer_amount,
        token_id,
        user_address,
    )
    .await?;
    wait_for_bank_balance(client, transfer_amount, token_id, recipient_address).await?;

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
/// 1. Create a token and do one transfer through the prefererd sequencer.
/// 2. Pause the sequencer and create enough batches to unsync the node. Each da block should include some forced txs to give good coverage
/// 4. Let it get back to ready.
/// 5. Wait for the visible slot number to increment enough times to cover all the forced txs.
/// 6. Send one more preferred tx to make sure everything is working correctly after the resync.
async fn forced_txs_resync_test_case(
    rollup: &TestRollup<MockDemoRollup<Native>>,
    forced_da_service: Arc<impl DaService>,
) -> anyhow::Result<()> {
    let client = &rollup.client;
    let (key, user_address, token_id, recipient_address) = create_keys_and_addresses();
    let initial_balance = 1_000u128;
    let preferred_transfer = 50u128;
    let forced_transfer = 5u128;
    let total_forced = forced_transfer * RESYNC_FORCED_BLOCKS as u128;
    let mut slot_subscription = rollup.api_client().subscribe_slots().await?;

    // Setup - create a token and do one transfer through the prefererd sequencer.
    {
        let create_token_tx = build_create_token_tx(&key, 0, initial_balance);
        client
            .client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&borsh::to_vec(&create_token_tx).unwrap()),
            })
            .await?;
        rollup.da_service.produce_block_now().await?;
        let _ = slot_subscription.next().await.unwrap()?;
        assert_balance(client, initial_balance, token_id, user_address, None).await?;

        let preferred_tx = build_transfer_token_tx::<TestSpec>(
            &key,
            token_id,
            recipient_address,
            preferred_transfer,
            1,
        );
        client
            .client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&borsh::to_vec(&preferred_tx).unwrap()),
            })
            .await?;
        assert_balance(
            client,
            initial_balance - preferred_transfer,
            token_id,
            user_address,
            None,
        )
        .await?;
        assert_balance(
            client,
            preferred_transfer,
            token_id,
            recipient_address,
            None,
        )
        .await?;

        rollup.da_service.produce_block_now().await?;
        let _ = slot_subscription.next().await.unwrap()?;
    }

    let forced_start_nonce = 2u64;
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
        let blob = transaction_into_blob(tx);
        let _receipt = forced_da_service
            .send_transaction(&blob)
            .await
            .await?
            .expect("Failed to submit forced bank transfer blob to DA");
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
    for _ in 0..RESYNC_FORCED_BLOCKS + 10 {
        rollup.da_service.produce_block_now().await?;
        let slot = slot_subscription.next().await.unwrap()?;
        let _ = state_update_subscription.next().await.unwrap()?;
    }
    // Now, that we're sure we've produced batches to increment the visible slot number, just wait for the state update notification that we've processed all of the forced txs.
    for _ in 0..RESYNC_FORCED_BLOCKS + 10 {
        let update = state_update_subscription.next().await.unwrap()?;
        if update.slot_number.get() >= slot_num_of_last_forced_tx {
            break;
        }
    }

    let expected_sender = initial_balance - preferred_transfer - total_forced;
    let expected_recipient = preferred_transfer + total_forced;
    assert_balance(client, expected_sender, token_id, user_address, None).await?;
    assert_balance(
        client,
        expected_recipient,
        token_id,
        recipient_address,
        None,
    )
    .await?;

    // Send one more preferred tx to make sure everything is working correctly after the resync
    let preferred_tx = build_transfer_token_tx::<TestSpec>(
        &key,
        token_id,
        recipient_address,
        preferred_transfer,
        forced_start_nonce + RESYNC_FORCED_BLOCKS as u64,
    );
    client
        .client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&borsh::to_vec(&preferred_tx).unwrap()),
        })
        .await?;

    assert_balance(
        client,
        expected_sender - preferred_transfer,
        token_id,
        user_address,
        None,
    )
    .await?;
    assert_balance(
        client,
        expected_recipient + preferred_transfer,
        token_id,
        recipient_address,
        None,
    )
    .await?;

    Ok(())
}

/// Verifies that EVM contract calls can be processed from unregistered sequencer batches.
/// Uses the forced batch notifier to avoid timing races when waiting for execution.
#[tokio::test(flavor = "multi_thread")]
async fn test_evm_contract_call_in_unregistered_batch() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");

    let (rollup, da_service) = setup().await;

    let http_addr = rollup.http_addr;
    let client = rollup.client.clone();

    tokio::select! {
        err = rollup.rollup_task => err??,
        res = evm_contract_call_unregistered_test_case(da_service, &client, http_addr) => res?,
    };
    Ok(())
}

async fn evm_contract_call_unregistered_test_case(
    da_service: Arc<impl DaService>,
    client: &NodeClient,
    http_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    sov_test_utils::initialize_logging();
    let sender = EvmAccount::from_private_key_hex(SENDER_PRIV_KEY);
    let mut forced_tx_batches = subscribe_forced_tx_batches(client).await?;

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

    let call_tx = build_evm_contract_call_tx(&sender, contract_address, calldata, 1);
    let blob = evm_transaction_into_blob(&sender, call_tx);

    // Submit via unregistered DA service
    let _receipt = da_service
        .send_transaction(&blob)
        .await
        .await?
        .expect("Failed to submit contract call blob to DA");
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
