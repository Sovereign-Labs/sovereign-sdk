//! Integration tests for the preferred sequencer's transaction nonce queue functionality.
//!
//! These tests verify that the sequencer can accept transactions with future nonces (out-of-order)
//! and queue them for execution once the missing nonces arrive.

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::{types as api_types, Client};
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::prelude::*;
use sov_modules_api::{DispatchCall, RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::PaymasterConfig;
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_test_modules::hooks_count::HooksCount;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath, TestRollup};
use sov_test_utils::{
    default_test_tx_details, generate_optimistic_runtime_with_kernel, test_signed_transaction,
    RtAgnosticBlueprint, TestSpec, TestUser, TEST_BLOB_PROCESSING_TIMEOUT,
    TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE, TEST_MAX_CONCURRENT_BLOBS,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_stream::StreamExt;

use crate::utils::{
    tempdir_inside_codebase_dir, ModuleWithVersionedStateAccessInSlotHook,
    MAX_BATCH_EXECUTION_TIME_MILLIS,
};

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [value_setter: ValueSetter<S>, hooks_count: HooksCount<S>, paymaster: sov_paymaster::Paymaster<S>, slot_hook_checker: ModuleWithVersionedStateAccessInSlotHook<S>],
);

type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

const DEFAULT_SIZE: u64 = 100;
const DEFAULT_TIMEOUT: u64 = 2000;

/// Creates a test rollup with custom nonce queue configuration.
async fn create_test_rollup(
    maximum_future_nonce_delta: u64,
    future_nonce_transaction_timeout_millis: u64,
) -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
            (),
            PaymasterConfig::default(),
            (),
        );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = tempdir_inside_codebase_dir();

    let builder = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params.clone()),
        BlockProducingConfig::Manual,
        TEST_FINALIZATION_BLOCKS,
    )
    .set_config(|c| {
        c.automatic_batch_production = true;
        c.storage = StoragePath::Tmp(dir);
        c.max_batch_size_bytes = TEST_MAX_BATCH_SIZE;
        c.blob_processing_timeout_secs = TEST_BLOB_PROCESSING_TIMEOUT;
        c.max_concurrent_blobs = TEST_MAX_CONCURRENT_BLOBS;

        let mut preferred_config = match &c.sequencer_config {
            SequencerKindConfig::Preferred(p) => p.clone(),
            SequencerKindConfig::Standard(_) => PreferredSequencerConfig::default(),
        };
        preferred_config.batch_execution_time_limit_millis = MAX_BATCH_EXECUTION_TIME_MILLIS;
        preferred_config.maximum_future_nonce_delta = maximum_future_nonce_delta;
        preferred_config.future_nonce_transaction_timeout_millis =
            future_nonce_transaction_timeout_millis;
        c.sequencer_config = SequencerKindConfig::Preferred(preferred_config);
    })
    .set_da_config(|c| {
        c.sender_address = genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address;
    });

    let test_rollup = builder.start().await.unwrap();

    // Set up the rollup the usual way. We need this in all our tests.
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    for _ in 0..5 {
        let _ = slot_subscription.next().await.unwrap().unwrap();
    }

    (test_rollup, admin)
}

/// Helper to create a ValueSetter transaction with a specific nonce.
fn tx_set_value(key: &Ed25519PrivateKey, nonce: u64, value_to_set: u64) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetValue {
            value: value_to_set as u32,
            gas: None,
        },
    );

    let tx_details = default_test_tx_details::<TestSpec>();
    let tx = test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        &msg,
        UniquenessData::Nonce(nonce),
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
        tx_details,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

async fn submit_tx_set_value(
    client: &Client,
    key: &Ed25519PrivateKey,
    nonce: u64,
    expect_success: bool,
) {
    let tx = tx_set_value(&key, nonce, nonce);
    let res = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await;
    println!("RECEIVED result for nonce {nonce}: {res:?}");
    match expect_success {
        true => {
            res.unwrap();
        }
        false => {
            res.unwrap_err();
        }
    };
}

/// Helper that submits several transactions in parallel, but with a small delay between each to
/// ensure that the API requests land in order, as specified by the `nonces` Vec.
/// Returns the handles for each submission task. Each task asserts either unwrap() or unwrap_err()
/// depending on the `expect_success` parameter.
///
/// The total submission takes `nonces.len() * 10` miliseconds (plus any execution time) to ensure
/// API request ordering - bear this in mind when the queue timeout is short.
async fn submit_parallel_txs(
    client: &Client,
    key: &Ed25519PrivateKey,
    nonces: Vec<u64>,
    expect_success: bool,
) -> Vec<JoinHandle<()>> {
    let mut handles = vec![];
    for nonce in nonces {
        let client = client.clone();
        let key = key.clone();
        let handle = tokio::spawn(async move {
            submit_tx_set_value(&client, &key, nonce, expect_success).await;
        });
        handles.push(handle);

        // Small delay to ensure transactions arrive in the intended order
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handles
}

/// Helper to query the current value from the ValueSetter module.
async fn query_set_value(
    test_rollup: &TestRollup<TestBlueprint>,
    expected: u64,
) -> anyhow::Result<()> {
    let url = "/modules/value-setter/state/value";
    let response = test_rollup
        .client
        .query_rest_endpoint::<serde_json::Value>(url)
        .await?;

    // Quick check if the request returned an error
    if response.get("message").is_some() {
        return Err(anyhow::anyhow!("API request failed: {:?}", response));
    }

    let found_value = response["value"].as_u64().unwrap_or_default();

    anyhow::ensure!(
        found_value == expected,
        "Expected value {}, found {}",
        expected,
        found_value
    );

    Ok(())
}

/// Test that transactions with normal consecutive nonces are accepted and executed in order.
#[tokio::test(flavor = "multi_thread")]
async fn test_consecutive_nonces() {
    let (test_rollup, admin) = create_test_rollup(DEFAULT_SIZE, DEFAULT_TIMEOUT).await;
    let client = test_rollup.api_client().clone();

    // Submit 5 transactions with consecutive nonces (0, 1, 2, 3, 4)
    for nonce in 0..5 {
        submit_tx_set_value(&client, &admin.private_key, nonce, true).await;
    }

    // Sanity check
    query_set_value(&test_rollup, 4).await.unwrap();
}

/// Test that transactions with reordered nonces still all get in.
#[tokio::test(flavor = "multi_thread")]
async fn test_out_of_order_nonces() {
    let (test_rollup, admin) = create_test_rollup(DEFAULT_SIZE, DEFAULT_TIMEOUT).await;
    let client = test_rollup.api_client().clone();

    // Submit 5 transactions with reverse order nonces (4, 3, 2, 1, 0)
    let nonces = (0..5).rev().collect();
    let handles = submit_parallel_txs(&client, &admin.private_key, nonces, true).await;

    // Wait for all submissions to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Sanity check
    query_set_value(&test_rollup, 4).await.unwrap();
}

/// Test transactions timing out from the queue.
#[tokio::test(flavor = "multi_thread")]
async fn test_nonce_queue_timeout() {
    // Long enough to be able to submit ordered transactions with
    // small waits, short enough to not slow down the test
    const SHORT_QUEUE_TIMEOUT: u64 = 200;
    let (test_rollup, admin) = create_test_rollup(DEFAULT_SIZE, SHORT_QUEUE_TIMEOUT).await;
    let client = test_rollup.api_client().clone();
    let key = admin.private_key;

    let first_range = (1..4).rev().collect(); // 3, 2, 1
    let second_range = (5..8).rev().collect(); // 7, 6, 5

    // Submit the first range, missing nonce 0 so they all get queued
    let mut handles = submit_parallel_txs(&client, &key, first_range, true).await;
    // Submit the second range, with another gap at nonce 4. These will fail later.
    handles.extend(submit_parallel_txs(&client, &key, second_range, false).await);

    // Now submit tx 0, so the first range gets executed
    submit_tx_set_value(&client, &key, 0, true).await;

    // Wait past the queue timeout configured at the start of the test. The second range should
    // have all timed out and been rejected by the time this is over.
    tokio::time::sleep(Duration::from_millis(SHORT_QUEUE_TIMEOUT)).await;

    // Now submit tx 4
    submit_tx_set_value(&client, &key, 4, true).await;

    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }

    // Sanity check: transaction with nonce 4 should have been the last to set value
    query_set_value(&test_rollup, 4).await.unwrap();
}

/// Test transaction rejection with a zero-length queue
#[tokio::test(flavor = "multi_thread")]
async fn test_zero_length_queue() {
    let (test_rollup, admin) = create_test_rollup(0, DEFAULT_TIMEOUT).await;
    let client = test_rollup.api_client().clone();
    let key = admin.private_key;

    // Spam some transactions with nonces above 0. Expect them to fail because queue size is 0.
    let handles = submit_parallel_txs(&client, &key, (1..5).collect(), false).await;
    // Send the 0 nonce transaction - if the above had gotten queued (which they shouldn't have),
    // this would have let them succeed
    submit_tx_set_value(&client, &key, 0, true).await;

    for handle in handles {
        handle.await.unwrap();
    }
    // Sanity check
    query_set_value(&test_rollup, 0).await.unwrap();
}
