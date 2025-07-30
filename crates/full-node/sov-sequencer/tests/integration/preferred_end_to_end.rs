//! Integration tests for the preferred sequencer that use [`RollupBuilder`] and
//! thus test sequencer + node interactions.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use borsh::{BorshDeserialize, BorshSerialize};
use futures::future;
use sov_api_spec::types::{
    self as api_types, SequencerListEventsPage, SequencerListEventsResponse, TxReceiptResult,
};
use sov_api_spec::{Client, WsSubscription};
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::prelude::*;
use sov_modules_api::{Amount, DispatchCall, Gas, GasArray, GasPrice, GasUnit, RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_node_client::NodeClient;
use sov_paymaster::{Paymaster, PaymasterConfig};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::ledger_api::IncludeChildren;
use sov_sequencer::preferred::default_ideal_lag_behind_finalized_slot;
use sov_sequencer::StateUpdateNotification;
use sov_test_modules::hooks_count::HooksCount;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, RollupProverConfig, TestRollup};
use sov_test_utils::{
    default_test_signed_transaction, generate_optimistic_runtime_with_kernel, RtAgnosticBlueprint,
    TestSpec, TestUser, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE, TEST_MAX_CONCURRENT_BLOBS,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use test_strategy::Arbitrary;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_stream::StreamExt;
use tracing::{debug, info};

use crate::utils::{
    generate_paymaster_tx, generate_txs, new_test_rollup, pause_update_state,
    tempdir_inside_codebase_dir, tx_set_value_with_gas, ModuleWithVersionedStateAccessInSlotHook,
    MAX_BATCH_EXECUTION_TIME_MILLIS,
};
const DELAYED_TX_DELAY_MS: u64 = 500;

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [value_setter: ValueSetter<S>, hooks_count: HooksCount<S>, paymaster: Paymaster<S>, slot_hook_checker: ModuleWithVersionedStateAccessInSlotHook<S>],
    transaction_delay_ms_wrapper: |call: &Self::Decodable| {
        match call {
            Self::Decodable::HooksCount(sov_test_modules::hooks_count::CallMessage::DelayedCallMsg) => DELAYED_TX_DELAY_MS,
            _ => 0,
        }
    }
);

pub(crate) type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

use sov_test_utils::TEST_BLOB_PROCESSING_TIMEOUT;

const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::OnBatchSubmit {
    block_wait_timeout_ms: None,
};

pub struct DaLayerWithSubscription {
    da_layer: Arc<RwLock<StorableMockDaLayer>>,
    state_update_subscription: WsSubscription<StateUpdateNotification>,
    slot_subscription: WsSubscription<api_types::Slot>,
    /// The number of slots that have been produced but not yet had their notifications received.
    back_slot_notifications: u64,
}

impl DaLayerWithSubscription {
    pub async fn new(test_rollup: &TestRollup<TestBlueprint>) -> Self {
        assert!(matches!(test_rollup
            .da_service
            .block_producing(),
            BlockProducingConfig::Manual),
            "Can't currently use DaLayerWithSubscription with a non-manual block producing config because notifications may be produced without our knowledge"
        );
        let da_layer = test_rollup.da_service.da_layer().clone();
        let state_update_subscription = test_rollup.subscribe_state_updates().await;
        let slot_subscription = test_rollup
            .api_client()
            .subscribe_slots_with_children(IncludeChildren::new(true))
            .await;
        Self {
            da_layer,
            state_update_subscription,
            slot_subscription,
            back_slot_notifications: 0,
        }
    }

    pub async fn produce_block(&mut self) -> anyhow::Result<()> {
        let mut lock = self.da_layer.write().await;
        lock.produce_block().await?;
        self.back_slot_notifications += 1;
        Ok(())
    }

    /// Waits for a new slot notification to be produced, Clearing any older ones from the queue first.
    /// This is useful when you've been producing blocks but without waiting for notifications and now you want to wait again.
    pub async fn wait_for_new_slot_notification(&mut self) -> api_types::Slot {
        let subscription = self.slot_subscription.as_mut().unwrap();
        while self.back_slot_notifications > 1 {
            self.back_slot_notifications -= 1;
            subscription.next().await.unwrap().unwrap();
        }

        self.back_slot_notifications -= 1;
        subscription.next().await.unwrap().unwrap()
    }

    /// Gets the next state update notification, clearing any *known* updates from the queue first.
    /// Unless you've been using `produce_and_wait_for_slot` for all block production, this notification might possibly be stale.
    /// This is inevitable because `update_state` doesn't run on every block, so (unlike the slot subscription) we don't know
    /// how many state update notifications we should ultimately be receiving.
    pub async fn next_state_update_notification(&mut self) -> StateUpdateNotification {
        let subscription = self.state_update_subscription.as_mut().unwrap();
        subscription.next().await.unwrap().unwrap()
    }

    /// Produces a slot and waits for the state update and slot notifications.
    pub async fn produce_and_wait_for_slot(&mut self) -> api_types::Slot {
        self.produce_block().await.unwrap();
        self.next_state_update_notification().await;
        self.wait_for_new_slot_notification().await
    }

    pub async fn produce_and_wait_for_n_slots(&mut self, n: u64) {
        for _ in 0..n {
            self.produce_and_wait_for_slot().await;
        }
    }
}

/// All the interesting "things" that can happen during sequencer operations, and to
/// which the sequencer ought to know how to respond.
#[derive(Debug, Clone, Arbitrary)]
pub(crate) enum TestingAction {
    /// Never generated automatically because tests would slow down wayyy too
    /// much. Useful for debugging.
    #[weight(0)]
    Sleep { duration_ms: u64 },
    /// The node is immediately shutdown and restarted, to catch possible losses
    /// of soft-confirmed transactions and state initialization bugs.
    Restart,
    /// A client submits a valid transaction to be included in the next batch,
    /// and for which a soft confirmation ought to be provided immediately.
    #[weight(5)] // Make it more likely to be picked (this is where all juicy stuff happens)
    AcceptTx,
    /// Shorthand for a bunch of transactions in quick succession.
    AcceptTxs {
        #[strategy(0..10usize)]
        count: usize,
    },
    /// A client submits an **invalid** transactions, asking for it to be
    /// included in the next batch (it won't, as it's invalid).
    #[weight(2)]
    TryAcceptBadTx { invalid_reason: InvalidGeneration },
    /// A client queries the nonce for a given address.
    ///
    /// This is an easy and effective way for us to check that all pending
    /// transactions have actually been processed by the sequencer, and that
    /// its state changes are visible to REST API clients.
    ///
    /// TODO(@neysofu): Switch to the message generator.
    #[weight(8)]
    QuerySetValue,
    /// Like [`TestingAction::QuerySetValue`], but historical queries.
    ///
    /// FIXME(@neysofu): historical queries only work for node-processed slots,
    /// and not soft confirmations. This is arguably fine, but this test feature
    /// doesn't take that into account and is currently broken.
    #[weight(0)]
    #[allow(dead_code)]
    QuerySetValueHistorical,
    /// A new DA slot will be produced and made available to the node and sequencer.
    NewDaSlot,
    /// Sets the pause state of update state execution to provided value.
    /// Allows disabling update_state from running inside the sequencer.
    PauseUpdateStateExecution(bool),
}

/// An invalid nonce.
#[derive(Debug, Clone, Arbitrary)]
pub(crate) enum InvalidGeneration {
    DuplicateTransaction,
    TooOld,
}

#[allow(clippy::too_many_arguments)]
async fn create_test_rollup(
    minimum_profit_per_tx: u128,
    max_batch_size: usize,
    blob_processing_timeout_secs: u64,
    max_batch_execution_time_millis: u64,
) -> (Option<TestRollup<TestBlueprint>>, TestUser<TestSpec>) {
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

    (
        new_test_rollup::<TestRuntime<TestSpec>>(
            dir.clone(),
            genesis_params
                .runtime
                .sequencer_registry
                .sequencer_config
                .seq_da_address,
            genesis_params,
            minimum_profit_per_tx,
            true,
            max_batch_size,
            BlockProducingConfig::Manual,
            None,
            blob_processing_timeout_secs,
            1,
            max_batch_execution_time_millis,
            None,
            TEST_FINALIZATION_BLOCKS,
        )
        .await
        .map(|v| v.into_iter().next().unwrap()),
        admin,
    )
}

#[derive(Debug)]
pub(crate) struct TestState {
    value_by_slot_number: HashMap<SlotNumber, u64>,
    _current_slot_number: SlotNumber,
    next_generation: u64,
    current_value: u64,
}

impl Default for TestState {
    fn default() -> Self {
        Self {
            value_by_slot_number: Default::default(),
            _current_slot_number: Default::default(),
            next_generation: 10, // initialize to a higher generation so that "invalid generation" actions are always possible
            current_value: Default::default(),
        }
    }
}

// FIXME(@neysofu): this test is not broken due to correctness bugs in the
// sequencer, but rather because generated testing scenarios sometimes are
// oversized and the node can't keep up with the sequencer. TODO: find a solution.
//#[test]
//fn random_edge_cases_and_complex_scenarios() {
//    use proptest::prelude::*;
//    use proptest::test_runner::{Config, TestRunner};
//
//    let mut runner = TestRunner::new(Config::with_cases(1));
//    let result = runner.run(
//        &proptest::collection::vec(any::<TestingAction>(), 0..50),
//        |actions| {
//            let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
//                .enable_all()
//                .build()
//                .unwrap();
//            tokio_runtime.block_on(async {
//                preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
//            });
//
//            Ok(())
//        },
//    );
//
//    result.unwrap();
//}

#[tokio::test(flavor = "multi_thread")]
async fn txs_below_min_fee_are_rejected() {
    let (test_rollup, admin) = create_test_rollup(
        1,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(5).await;

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 7);
    let Err(e) = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
    else {
        panic!("Tx must have been rejected for insufficient fee");
    };
    let err_message = e.to_string();
    assert!(
        err_message.contains("This transaction did not pay a sufficient net fee."),
        "Full error message does not contain expect part: {err_message}"
    );
}

/// Test what happens when the sequencer fills up its gas limit. This tests that...
/// 1. Transactions which would exceed the gas limit are rejected.
/// 2. Really large transactions don't cause the sequencer to produce a batch too early. It only considers a batch full once we've used 95% of the gas.
/// 3. The sequencer does produce a new batch once the current one gets close the the gas limit
#[tokio::test(flavor = "multi_thread")]
async fn sequencer_filled_up_block() {
    let mut genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let max_exec_gas_per_tx = GasUnit::from(config_value!("MAX_SEQUENCER_EXEC_GAS_PER_TX"));

    let gas_limit = max_exec_gas_per_tx.clone();
    let gas_limit = gas_limit.checked_scalar_product(100).unwrap();
    // The sequencer only gets 90% of the overall gas limit, so we need to set the slot gas limit to 10/9ths of what we
    // want the sequencer to have.
    let gas_limit_array = gas_limit
        .as_ref()
        .iter()
        .map(|x| (x * 10) / 9)
        .collect::<Vec<_>>();
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_INITIAL_GAS_LIMIT",
        format!("{gas_limit_array:?}"),
    );
    // Set very high initial balance for the admin.
    genesis_config.additional_accounts_mut()[0].available_gas_balance =
        Amount::MAX.saturating_div(Amount::new(2));

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

    let Some(test_rollups) = new_test_rollup::<TestRuntime<TestSpec>>(
        dir.clone(),
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
        0,
        true,
        TEST_MAX_BATCH_SIZE,
        BlockProducingConfig::Manual,
        None,
        60,
        1,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
        None,
        TEST_FINALIZATION_BLOCKS,
    )
    .await
    else {
        // Docker issues, don't fail the test and just return early.
        return;
    };
    let test_rollup = test_rollups.into_iter().next().unwrap();

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(5).await;

    let client = test_rollup.api_client().clone();

    {
        let gas_to_charge = gas_limit
            .checked_scalar_product(9)
            .unwrap()
            .scalar_division(10)
            .clone();

        // Produce a transaction that uses 90% of the slot gas limit.
        // This should be accepted.
        let tx = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            0,
            7,
            Some(gas_to_charge.clone()),
            Amount::MAX.saturating_div(Amount::new(4)),
        );

        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        // Produce a second huge transaction
        // This should fail with Out of Gas because...
        //  - the sequencer is below its 95% gas usage threshold so no new batch will have been started.
        //  - there's not nearly enough slot gas limit left to cover this tx
        let tx_2 = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            1,
            7,
            Some(gas_to_charge.clone()),
            Amount::MAX.saturating_div(Amount::new(4)),
        );

        let err = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx_2),
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("The gas to charge is greater than the funds available in the meter"),
            "Expected Out of Gas error, got: {err}"
        );

        // Produce a third transaction that uses 5% of the gas limit.
        // This should be accepted and will cause the sequencer to close out its current batch since usage should now pass 95%.
        let small_gas_amount = gas_limit.clone().scalar_division(20).clone();
        let tx_3 = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            1,
            7,
            Some(small_gas_amount.clone()),
            Amount::MAX.saturating_div(Amount::new(4)),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx_3),
            })
            .await
            .unwrap();

        // Produce another huge transaction
        // This should be accepted because the sequencer starts a new batch.
        let tx_4 = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            1,
            7,
            Some(gas_to_charge.clone()),
            Amount::MAX.saturating_div(Amount::new(4)),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx_4),
            })
            .await
            .unwrap();
    }
}

const SEQUENCER_RECOVERY_ERROR: &str = "The preferred sequencer is recovering from downtime and cannot provide soft-confirmations at this time";

#[tokio::test(flavor = "multi_thread")]
async fn flaky_seq_behind_deferred_slots_count_simple_lagging() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "40");
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;
    let Some(test_rollup) = test_rollup else {
        return;
    };
    let client = test_rollup.api_client().clone();
    // Sleep for the rollup to start up
    sleep(Duration::from_millis(500)).await;

    // Finalise some blocks
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(8).await;

    // Sanity check tx that the rollup works
    let tx_update_one = tx_set_value(&admin.private_key, 0, 8);
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_one),
        })
        .await
        .unwrap();

    tracing::info!("Producing DA blocks for inclusion sanity check tx inclusion");
    da_layer.produce_and_wait_for_n_slots(10).await;

    // Pause sequencer update_state and run some blocks so deferred_slots_count is reached
    test_rollup.pause_preferred_batches().await;
    tracing::info!("Preferred sequencer batch production paused.");

    // Preferred sequencer should accept the transaction (and keep it as an in-progress batch since
    // we've stopped update_state and thus aren't producing batches)
    const UPDATE_TWO_VALUE: u64 = 19;
    let tx_update_two = tx_set_value(&admin.private_key, 0, UPDATE_TWO_VALUE);
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_two),
        })
        .await
        .unwrap();

    tracing::info!(
        "Producing subsequent DA blocks while sequencer is paused, to exceet deferred_slots_count"
    );
    // This can be lower than DEFERRED_SLOTS_COUNT because the sequencer takes into account a)
    // possible node lag and b) a 90% threshold.
    for _ in 0..30 {
        let _ = da_layer.produce_block().await; // Don't wait for state updates since we've just paused them
    }
    tokio::time::sleep(Duration::from_millis(1500)).await; // Sleep to give time for at least some of these to be processed.

    tracing::info!("Resuming preferred sequencer batch production.");
    test_rollup.resume_preferred_batches().await;
    // Produce two blocks that will trigger update_state() and cause the sequencer to go into
    // recovery
    // A single block is usually enough but was very rarely flaky. Producing two blocks fixes that
    // and doesn't hurt
    for _ in 0..2 {
        let _ = da_layer.produce_block().await; // Now updates are not working because we're in recovery mode.
        sleep(Duration::from_millis(100)).await; // Sleep to give time for the sequencer to go into recovery.
    }

    // Create transaction that should fail: sequencer should not accept transactions while in
    // recovery.
    // We use set_many_values so we don't overwrite the value from set_value earlier, so we can
    // check that both had an effect by querying the separate state items.
    const UPDATE_VEC_VALUE: u8 = 12;
    let tx_update_vec = tx_set_many_values(&admin.private_key, 2, vec![UPDATE_VEC_VALUE]);
    tracing::info!("Trying to send transaction during recovery - expecing rejection");
    let err = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_vec),
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(SEQUENCER_RECOVERY_ERROR),
        "Expected recovery error, got: {err}"
    );

    // Give time for the sequencer to catch up its visible state number
    tracing::info!("Producing DA blocks to let the sequencer resync.");
    for _ in 0..10 {
        let _ = da_layer.produce_block().await;
        sleep(Duration::from_millis(100)).await; // Notifications don't work during recovery.
    }

    // Submit the same transaction to the now-working sequencer
    // This transaction will be soft-confirmed. The assertion should pass at this stage.
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_vec),
        })
        .await
        .unwrap();

    tracing::info!("Producing final run of blocks to include the last tx and confirm the rollup is running normally");
    for _ in 0..10 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        sleep(Duration::from_millis(50)).await; // have the node process them
    }

    // Assert that the earlier transactions sent just before the sequencer went into recovery was
    // flushed and processed by the node
    #[derive(Debug, serde::Deserialize)]
    struct ValueResponse {
        value: u32,
    }
    let response = test_rollup
        .client
        .query_rest_endpoint::<ValueResponse>("/modules/value-setter/state/value")
        .await
        .unwrap();
    let actual_value = response.value;
    assert_eq!(
        actual_value, UPDATE_TWO_VALUE as u32,
        "Expected value to be {UPDATE_TWO_VALUE}, but got {actual_value}"
    );

    // Assert that the transaction sent after the sequencer exited recovery was processed (i.e.
    // that the rollup is now functional)
    #[derive(Debug, serde::Deserialize)]
    struct IdxResponse {
        #[allow(unused)]
        index: u64,
        value: Option<u8>,
    }
    let many_values_response = test_rollup
        .client
        .query_rest_endpoint::<IdxResponse>("/modules/value-setter/state/many-values/items/0")
        .await
        .unwrap();
    let actual_many_value = many_values_response.value.unwrap();
    assert_eq!(
        actual_many_value, 12u8,
        "Expected many_values[0] to be 12, but got {actual_many_value}"
    );

    tracing::info!("All asserts successful, shutting down rollup");
    test_rollup.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_behind_deferred_slots_count_with_shutdown() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "40");
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;
    let Some(test_rollup) = test_rollup else {
        return;
    };
    let client = test_rollup.api_client().clone();
    // Sleep for the rollup to start up
    sleep(Duration::from_millis(500)).await;

    // Finalise some blocks
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(8).await;

    // Sanity check tx that the rollup works
    let tx_update_one = tx_set_value(&admin.private_key, 0, 8);
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_one),
        })
        .await
        .unwrap();

    tracing::info!("Producing DA blocks for inclusion sanity check tx inclusion");
    da_layer.produce_and_wait_for_n_slots(10).await;

    // Send a transaction that will be queued while rollup is shut down
    const UPDATE_TWO_VALUE: u64 = 19;
    let tx_update_two = tx_set_value(&admin.private_key, 0, UPDATE_TWO_VALUE);
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_two),
        })
        .await
        .unwrap();

    // Keep a reference to the DA service before shutting down
    let da_service = test_rollup.da_service.clone();

    // Shutdown the rollup while preserving the DA layer
    tracing::info!("Shutting down rollup to test restart behavior");
    let builder = test_rollup.shutdown().await.unwrap();

    tracing::info!("Producing DA blocks while rollup is shut down, to exceed deferred_slots_count");
    // This can be lower than DEFERRED_SLOTS_COUNT because the sequencer takes into account a)
    // possible node lag and b) a 90% threshold.
    da_service.produce_n_blocks_now(30).await.unwrap();

    // Restart the rollup
    tracing::info!("Restarting rollup after exceeding deferred_slots_count");
    let test_rollup = builder.start().await.unwrap();
    let client = test_rollup.api_client().clone();

    // Give the rollup time to process the backlog and enter recovery mode
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Produce a few more blocks to trigger recovery detection
    for _ in 0..3 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        sleep(Duration::from_millis(1000)).await;
    }

    // Create transaction that should fail: sequencer should not accept transactions while in recovery
    const UPDATE_VEC_VALUE: u8 = 12;
    let tx_update_vec = tx_set_many_values(&admin.private_key, 2, vec![UPDATE_VEC_VALUE]);
    tracing::info!("Trying to send transaction during recovery - expecting rejection");
    let err = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_vec),
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(SEQUENCER_RECOVERY_ERROR),
        "Expected recovery error, got: {err}"
    );

    // Give time for the sequencer to catch up its visible state number
    tracing::info!("Producing DA blocks to let the sequencer resync.");
    for _ in 0..10 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }

    // Submit the same transaction to the now-working sequencer
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_vec),
        })
        .await
        .unwrap();

    tracing::info!("Producing final run of blocks to include the last tx and confirm the rollup is running normally");
    for _ in 0..10 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }

    // Assert that the earlier transaction sent before shutdown was processed
    #[derive(Debug, serde::Deserialize)]
    struct ValueResponse {
        value: u32,
    }
    let response = test_rollup
        .client
        .query_rest_endpoint::<ValueResponse>("/modules/value-setter/state/value")
        .await
        .unwrap();
    let actual_value = response.value;
    assert_eq!(
        actual_value, UPDATE_TWO_VALUE as u32,
        "Expected value to be {UPDATE_TWO_VALUE}, but got {actual_value}"
    );

    // Assert that the transaction sent after recovery was processed
    #[derive(Debug, serde::Deserialize)]
    struct IdxResponse {
        #[allow(unused)]
        index: u64,
        value: Option<u8>,
    }
    let many_values_response = test_rollup
        .client
        .query_rest_endpoint::<IdxResponse>("/modules/value-setter/state/many-values/items/0")
        .await
        .unwrap();
    let actual_many_value = many_values_response.value.unwrap();
    assert_eq!(
        actual_many_value, 12u8,
        "Expected many_values[0] to be 12, but got {actual_many_value}"
    );

    tracing::info!("All asserts successful, shutting down rollup");
    test_rollup.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_out_of_gas_for_pre_checks() {
    let mut genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let max_exec_gas_per_tx = GasUnit::from(config_value!("MAX_SEQUENCER_EXEC_GAS_PER_TX"));

    // We want to set the initial gas limit to be 3/2 of the max exec gas per tx.
    let gas_limit = max_exec_gas_per_tx.clone();
    let mut gas_limit = gas_limit.checked_scalar_product(3).unwrap();
    gas_limit.scalar_division(2);
    let gas_limit_array = gas_limit.as_ref();
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_INITIAL_GAS_LIMIT",
        format!("{gas_limit_array:?}"),
    );
    let price_array = config_value!("INITIAL_BASE_FEE_PER_GAS");
    let gas_price = GasPrice::<2>::from([
        Amount::from(price_array[0] as u64),
        Amount::from(price_array[1] as u64),
    ]);

    let max_amount_limit = gas_limit.value(&gas_price);

    // Set very high initial balance for the admin.
    genesis_config.additional_accounts_mut()[0].available_gas_balance =
        max_amount_limit.checked_mul(Amount::new(10)).unwrap();

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

    let Some(test_rollups) = new_test_rollup::<TestRuntime<TestSpec>>(
        dir.clone(),
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
        0,
        true,
        TEST_MAX_BATCH_SIZE,
        BlockProducingConfig::Manual,
        None,
        60,
        1,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
        None,
        TEST_FINALIZATION_BLOCKS,
    )
    .await
    else {
        // Docker issues, don't fail the test and just return early.
        return;
    };
    let test_rollup = test_rollups.into_iter().next().unwrap();

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(5).await;

    let client = test_rollup.api_client().clone();
    test_rollup.pause_preferred_batches().await;

    // Produce the first transaction that nearly exhausts the gas slot limit.
    {
        let gas_to_charge = gas_limit.checked_sub(&max_exec_gas_per_tx).unwrap();

        let tx = tx_set_value_with_gas::<TestRuntime<TestSpec>>(
            &admin.private_key,
            0,
            7,
            Some(gas_to_charge),
            max_amount_limit,
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        query_set_value(&test_rollup, None, 7).await.unwrap();
    }

    // The second transaction will be rejected because of the slot gas limit.
    {
        let tx = tx_set_value(&admin.private_key, 1, 8);

        let resp = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap_err();

        let error_str = resp.to_string();
        assert!(error_str.contains("More transactions were submitted that the sequencer is allowed to put into a single batch."));
    }
    test_rollup.resume_preferred_batches().await;
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn max_batch_size() {
    let max_batch_size = 1024;
    let (test_rollup, admin) = create_test_rollup(
        0,
        max_batch_size,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(5).await;

    let client = test_rollup.api_client().clone();

    // The transaction is rejected because it is too large.
    {
        let tx = tx_set_many_values(&admin.private_key, 0, vec![0; 1024]);

        let resp = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap_err();

        let error_str = resp.to_string();
        assert!(
            error_str.contains("Transaction cannot be included in the batch"),
            "actual error: {error_str}"
        );
    }

    test_rollup.pause_preferred_batches().await;
    // The first and third transactions are processed, but the second and fourth are too large to be included in the batch.
    {
        let tx = tx_set_many_values(&admin.private_key, 0, vec![0; 128]);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        let tx = tx_set_many_values(&admin.private_key, 1, vec![0; 1024]);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap_err();

        let tx = tx_set_many_values(&admin.private_key, 1, vec![0; 512]);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        let tx = tx_set_many_values(&admin.private_key, 2, vec![0; 512]);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap_err();
    }

    test_rollup.force_close_batch().await.unwrap();

    // Once we start creating a fresh batch, we can insert a transaction that was previously rejected.
    {
        let tx = tx_set_many_values(&admin.private_key, 2, vec![0; 512]);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
    }
}

/// This test covers various endpoints that the sequencer provides for getting transactions and events.o
/// These tests are all squished into one because we have to do some relatively expensive setup (ie running a few hundred txs),
/// so we want to reuse that work for multiple endpoints.
#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_getters() {
    use sov_api_spec::types::TxInfoWithConfirmation;
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        1000, // Timeout the batch after 1 second of execution time.
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    // Set up the rollup the usual way.
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    for _ in 0..5 {
        let _ = slot_subscription.next().await.unwrap().unwrap();
    }

    let mut responses: Vec<TxInfoWithConfirmation> = Vec::new();
    let mut tx_number = 0;
    // Create a helper function to check that the tx endpoint responds with the expected txs.
    let check_responses = |starting_from: usize,
                           responses: Vec<TxInfoWithConfirmation>,
                           client: sov_api_spec::client::Client| async move {
        let mut tx_ws = client
            .subscribe_to_txs(Some(starting_from as u64))
            .await
            .unwrap();
        for (i, old_response) in responses.iter().enumerate().skip(starting_from) {
            let new_response = tokio::time::timeout(Duration::from_millis(500), tx_ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(
                new_response.id.to_string(),
                old_response.id.to_string(),
                "Mismatch at tx number {i}. Found {new_response:?} \nbut expected {old_response:?}",
            );
            assert_eq!(
                new_response.events, old_response.events,
                "Mismatch at tx {i}",
            );
        }
    };

    // Produce 30 blocks each containing 7 txs. After each block, check that the tx websocket responds with the expected txs. Starting from both zero and 5.
    // This should trigger any edge cases.
    for _ in 0..30 {
        for _ in 0..7 {
            let tx = tx_set_value(&admin.private_key, tx_number, tx_number);
            let resp = test_rollup
                .api_client()
                .send_raw_tx_to_sequencer_with_retry(&tx)
                .await
                .unwrap();
            responses.push(resp.into_inner());
            tx_number += 1;
        }
        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
        check_responses(0, responses.clone(), test_rollup.api_client().clone()).await;
        check_responses(5, responses.clone(), test_rollup.api_client().clone()).await;
    }

    check_responses(0, responses.clone(), test_rollup.api_client().clone()).await;
    check_responses(5, responses.clone(), test_rollup.api_client().clone()).await;

    // Now, check the `get_tx` endpoint by iterating through all the txs we generated.
    // And fetching each tx by its id.
    for response in responses.iter() {
        let tx_response = test_rollup
            .api_client()
            .sequencer_get_tx(&response.id)
            .await
            .unwrap();
        let tx = tx_response.as_ref();
        assert_eq!(tx.id, response.id);
        assert_eq!(tx.events, response.events);
        assert_eq!(&tx.receipt, response.receipt.as_ref().unwrap());
        assert_eq!(&tx.tx_number, response.tx_number.as_ref().unwrap());
    }

    // Finally, test the "list events" endpoint by iterating through all the events we generated.
    let all_events = responses
        .into_iter()
        .flat_map(|response| response.events.into_iter())
        .collect::<Vec<_>>();
    let mut i = 0;
    let mut page = SequencerListEventsPage::First;
    let mut page_cursor: Option<String> = None;
    while i < all_events.len() {
        let response = test_rollup
            .api_client()
            .sequencer_list_events(Some(page), page_cursor.as_deref(), Some(9)) // Use page size 9 because it's relatively prime to our number of events. This should trigger more edge cases
            .await
            .unwrap()
            .into_inner();
        let SequencerListEventsResponse { items, next_cursor } = response;
        for event in items {
            assert_eq!(event.number, i as u64);
            i += 1;
        }
        page = SequencerListEventsPage::Next;
        page_cursor = next_cursor;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_event_stream_filtering() {
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        1000, // Timeout the batch after 1 second of execution time.
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    // Set up the rollup the usual way.
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    for _ in 0..5 {
        let _ = slot_subscription.next().await.unwrap().unwrap();
    }

    let mut all_events = test_rollup
        .api_client()
        .subscribe_to_events()
        .await
        .unwrap();
    let mut value_setter_cpu_heavy_events = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("ValueSetter/RanCPUHeavyOperation")
        .await
        .unwrap();
    let mut bank_events = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();
    let mut value_setter_and_bank_events = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/*,ValueSetter/*")
        .await
        .unwrap();
    let tx = tx_set_value(&admin.private_key, 0, 1000);
    let _ = test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    assert!(
        tokio::time::timeout(Duration::from_secs(1), bank_events.next())
            .await
            .is_err(),
        "Bank event stream should not receive value setter txs"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), value_setter_cpu_heavy_events.next())
            .await
            .is_err(),
        "Value setter cpu heavy event stream should not receive bank events"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), all_events.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok(),
        "All event stream should receive all events"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), value_setter_and_bank_events.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok(),
        "Value setter and bank event stream should receive value setter events"
    );
}

/// This test checks that the sequencer closes its current batch when the tx execution time exceeds its target.
#[tokio::test(flavor = "multi_thread")]
async fn max_batch_execution_time() {
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        1000, // Timeout the batch after 1 second of execution time.
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    for _ in 0..5 {
        let _ = slot_subscription.next().await.unwrap().unwrap();
    }
    tokio::time::sleep(Duration::from_millis(10)).await; // Ensure that the slots have propagated to the sequencer

    let client = test_rollup.api_client().clone();

    // A helper function to get the next block and assert that it has the expected number of batches.
    // Because it's async, we pass a clone of the client to avoid borrow checking headaches.
    let get_next_block = |rollup_client: Client, should_have_batch: bool| {
        let da_service = test_rollup.da_service.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(500)).await; // Ensure the batch has time to close, if applciable
            let mut slot_subscription = rollup_client
                .subscribe_slots_with_children(IncludeChildren::new(true))
                .await
                .unwrap();
            da_service.produce_block_now().await.unwrap();
            let slot = slot_subscription.next().await.unwrap().unwrap();
            let expected_batches = if should_have_batch { 1 } else { 0 };
            assert_eq!(
                slot.batches.len(),
                expected_batches,
                "Expected {} batches, but got {} in slot number {}.",
                expected_batches,
                slot.batches.len(),
                slot.number
            );
        }
    };

    {
        // For now, the first tx should be accepted. Since its execution time exceeds our target of 1000 ms, the batch should be closed now;
        let tx = tx_set_value_and_sleep(&admin.private_key, 0, 1, 1200);
        tracing::info!("Submitting first tx");
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        tracing::info!("Tx received, fetching next block");
        // The fist batch should have been closed
        get_next_block(client.clone(), true).await;

        // The second tx isn't big enough to fill the batch, so it should still be open afterwards
        tracing::info!("Submitting second tx");
        let tx = tx_set_value_and_sleep(&admin.private_key, 0, 2, 500);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        tracing::info!("Tx received, fetching next block");
        // The second batch wasn't full - it should still be open
        get_next_block(client.clone(), false).await;

        // The next tx will put our executoin time over 1000ms causing the batch to be closed
        let tx = tx_set_value_and_sleep(&admin.private_key, 1, 3, 600);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        // The second batch should be full now.
        get_next_block(client.clone(), true).await;

        // This next transaction shouldn't trigger batch production
        let tx = tx_set_value_and_sleep(&admin.private_key, 1, 4, 500);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        get_next_block(client.clone(), false).await;

        // Sleep for 500 ms. This should *not* trigger batch production since only block execution time counts.
        tokio::time::sleep(Duration::from_millis(500)).await;
        // Send a tx that takes 400 ms. This should not trigger batch production since our running total is only 900 ms
        let tx = tx_set_value_and_sleep(&admin.private_key, 2, 5, 400);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        get_next_block(client.clone(), false).await;

        // The fifth transaction should fill the batch and trigger batch production
        let tx = tx_set_value_and_sleep(&admin.private_key, 3, 5, 160);
        let _ = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        get_next_block(client.clone(), true).await;
    }

    test_rollup.shutdown().await.unwrap();
}

/// Test that the sequencer can compute state roots for itself to avoid panics.
///
/// This test works by causing the node to fall far behind the sequencer in processsing rollup blocks.
/// This is done by preventing the seuqencer batches from being included on DA, while still producing lots of DA blocks.
///
/// Since there are always new finalized blocks available, the sequencer will happily create new rollup blocks far in advance of the node, triggering the case we care about.
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_state_root_computation_when_blobs_are_delayed() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_STATE_ROOT_DELAY_BLOCKS", "1");
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

    let Some(test_rollups) = new_test_rollup::<TestRuntime<TestSpec>>(
        dir.clone(),
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
        0,
        true,
        TEST_MAX_BATCH_SIZE,
        BlockProducingConfig::OnAnySubmit {
            block_wait_timeout_ms: None,
        },
        None,
        60,
        1,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
        None,
        TEST_FINALIZATION_BLOCKS,
    )
    .await
    else {
        // Docker issues, don't fail the test and just return early.
        return;
    };
    let test_rollup = test_rollups.into_iter().next().unwrap();

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    sleep(Duration::from_millis(200)).await;
    test_rollup.da_service.set_delay_blobs_by(100).await;

    let client = test_rollup.api_client().clone();
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    for i in 0..100 {
        let tx = tx_set_value(&admin.private_key, i, i);
        client
            .send_raw_tx_to_sequencer_with_retry(&tx)
            .await
            .unwrap();

        test_rollup.da_service.produce_block_now().await.unwrap();
        slot_subscription.next().await;
    }

    test_rollup
        .da_service
        .produce_n_blocks_now(100)
        .await
        .unwrap();
    sleep(Duration::from_millis(200)).await;
    test_rollup.shutdown().await.unwrap();
}

// The sequencer controls emitting ledger slots over websocket
// this test ensures it correctly publishes all the expected slots
#[tokio::test(flavor = "multi_thread")]
async fn test_rollup_emits_all_slot_notifications() {
    let (test_rollup, _) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    let nb_of_blocks = 10;
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(nb_of_blocks)
        .await
        .unwrap();

    for _ in 0..nb_of_blocks {
        let slot = slot_subscription.next().await.unwrap().unwrap();
        tracing::info!("received slot {}", slot.number);
    }

    test_rollup.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn rollup_shuts_down_if_blob_sender_fails() {
    sov_test_utils::initialize_logging();
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    let nb_of_blocks = 5 + default_ideal_lag_behind_finalized_slot() as usize;
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(nb_of_blocks)
        .await
        .unwrap();

    for i in 0..nb_of_blocks {
        tracing::warn!("waiting for slot {}", i);
        slot_subscription.next().await.unwrap().unwrap();
    }

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 9);

    tracing::warn!("accepting tx");
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    test_rollup.da_service.set_fail_send_blob();
    tracing::warn!("setting fail send blob");
    test_rollup.force_close_batch().await.unwrap();
    tracing::warn!("force closed batch");

    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(5))
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rollup_shuts_down_if_blob_processing_timeouts() {
    let (test_rollup, admin) =
        create_test_rollup(0, TEST_MAX_BATCH_SIZE, 1, MAX_BATCH_EXECUTION_TIME_MILLIS).await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    let nb_of_blocks = 5;
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(nb_of_blocks)
        .await
        .unwrap();

    for _ in 0..nb_of_blocks {
        slot_subscription.next().await.unwrap().unwrap();
    }

    // Send a transaction to ensure a batch is created.
    let tx = tx_set_value(&admin.private_key, 0, 9);
    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    // Close the batch and submit it.  It won't be received because we don't create any more blocks
    test_rollup.force_close_batch().await.unwrap();

    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(5))
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rollup_shuts_down_if_panic_is_triggered() {
    let (test_rollup, admin) =
        create_test_rollup(0, TEST_MAX_BATCH_SIZE, 60, MAX_BATCH_EXECUTION_TIME_MILLIS).await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    let nb_of_blocks = 5;
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    test_rollup
        .da_service
        .produce_n_blocks_now(nb_of_blocks)
        .await
        .unwrap();

    for _ in 0..nb_of_blocks {
        slot_subscription.next().await.unwrap().unwrap();
    }

    let client = test_rollup.api_client().clone();
    sleep(Duration::from_millis(500)).await;

    // Send a test transaction to make sure everything is working.
    let tx = tx_set_value(&admin.private_key, 0, 9);
    client
        .send_raw_tx_to_sequencer_with_retry(&tx)
        .await
        .unwrap();

    // Cause a panic in the sequencer.
    let tx = tx_panic(&admin.private_key, 1);
    client.send_raw_tx_to_sequencer(&tx).await.unwrap_err();

    // Ensure that the sequencer rejects subsequent transactions.
    let tx = tx_set_value(&admin.private_key, 2, 5);
    client.send_raw_tx_to_sequencer(&tx).await.unwrap_err();

    // Ensure that the sequencer shuts down promptly
    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(5))
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_seq_back_pressure() {
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();

    sleep(Duration::from_millis(200)).await;

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 9);

    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    // Pause block submission and produce some pending blocks.
    {
        test_rollup.da_service.set_blob_submission_pause().await;
        let num_blocks_needed =
            default_ideal_lag_behind_finalized_slot() + TEST_MAX_CONCURRENT_BLOBS as u64;
        for _ in 0..num_blocks_needed + 8 {
            // Add a little cushion to reduce flakiness
            test_rollup.da_service.produce_block_now().await.unwrap();
            sleep(Duration::from_millis(800)).await;
        }

        let tx = tx_set_value(&admin.private_key, 1, 9);
        let err = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("The sequencer is waiting for the blob sender to be ready"),
            "Unexpected error: {err}"
        );

        test_rollup.da_service.resume_blob_submission().await;
    }

    for _ in 0..10 {
        test_rollup.da_service.produce_block_now().await.unwrap();
        sleep(Duration::from_millis(800)).await;
    }
    sleep(Duration::from_millis(1000)).await;

    let tx = tx_set_value(&admin.private_key, 1, 9);
    client
        .send_raw_tx_to_sequencer_with_retry(&tx)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_many_invalid_txs() {
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    for _ in 0..5 {
        slot_subscription.next().await.unwrap().unwrap();
    }

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 100, 0);

    client
        .send_raw_tx_to_sequencer_with_retry(&tx)
        .await
        .unwrap();

    let mut handles = Vec::default();
    for i in 0..100 {
        let client = client.clone();
        let da_service = test_rollup.da_service.clone();

        let tx = tx_set_value(&admin.private_key, 0, i);
        handles.push(tokio::spawn(async move {
            let res = client.send_raw_tx_to_sequencer_with_retry(&tx).await;
            da_service.produce_block_now().await.unwrap();
            res
        }));
    }

    test_rollup
        .da_service
        .produce_n_blocks_now(50)
        .await
        .unwrap();

    let results = future::join_all(handles).await;
    for res in results {
        assert!(res.unwrap().is_err());
    }
}

/// Ensure that we use the correct visible slot number when replaying transactions after a call to `update_state` in the sequencer.
/// The key thing that this test does is to execute the same transaction 3 times - once in the sequencer via `accept_tx`, once via `update_state`
/// and once in the node. Everything else is implementation details.
///
/// # How it works (currently)
/// Here's how the test works currently - feel free to change this as the sequencer logic evolves.
///  1. Produce enough empty *DA blocks* that the sequencer will produce an empty batch
///  2. Before including that first empty batch on DA, submit a transaction to the sequencer which asserts the correct visible slot number.
///      This will cause the sequencer to start bulding a new batch on top of the updated state.
///  3. Include the empty batch on DA, and wait for the node to process it. This triggers a call to `update_state` in the sequencer, which will panic on error.
///  4. Accept the sequencer's new batch (which contains the transaction asserting the correct visible slot number) onto the DA layer. Defensively assert that it
///     gets processed correctly by the node.
#[tokio::test(flavor = "multi_thread")]
async fn query_historical_values() {
    let panicked: Arc<std::sync::atomic::AtomicBool> =
        Arc::new(std::sync::atomic::AtomicBool::new(false));
    let panicked_ref = panicked.clone();
    let prev_hook = std::panic::take_hook();
    // Set a new panic hook
    std::panic::set_hook(Box::new(move |panic_info| {
        // Your custom panic handling logic here
        panicked_ref.store(true, std::sync::atomic::Ordering::SeqCst);
        prev_hook(panic_info);
    }));
    let tx_builder = |key| tx_set_value(&key, 0, 7);
    let assertions = |test_rollup| async move {
        query_set_value(&test_rollup, Some(2), 7).await.unwrap();
        query_set_value(&test_rollup, Some(0), 0).await.unwrap();
        query_set_value_by_slot_number(&test_rollup, Some(8), 7)
            .await
            .unwrap();
        query_set_value_by_slot_number(&test_rollup, Some(7), 0)
            .await
            .unwrap();
        query_set_value_by_slot_number(&test_rollup, Some(1), 0)
            .await
            .unwrap();
        // Query some future heights/slot numbers to be sure they don't panic
        assert!(query_set_value(&test_rollup, Some(100000), 0)
            .await
            .is_err());
        assert!(
            query_set_value_by_slot_number(&test_rollup, Some(100000), 0)
                .await
                .is_err()
        );
        test_rollup.shutdown().await.unwrap();
        assert!(
            !panicked.load(std::sync::atomic::Ordering::SeqCst),
            "Panicked during assertions"
        );
    };
    do_manual_block_production_test(tx_builder, assertions, 22222).await;
}

async fn do_manual_block_production_test<Fut: Future<Output = ()>>(
    tx_builder: impl Fn(Ed25519PrivateKey) -> RawTx,
    assertions: impl FnOnce(TestRollup<TestBlueprint>) -> Fut,
    port: u16,
) {
    const FINALIZATION_BLOCKS: u32 = 0;
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

    let dir = Arc::new(tempfile::tempdir().unwrap());

    let da_layer = Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(FINALIZATION_BLOCKS)
            .await
            .unwrap(),
    ));
    let test_rollup = {
        let sequencer_addr = genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address;

        RollupBuilder::<TestBlueprint>::new(
            GenesisSource::CustomParams(genesis_params),
            BlockProducingConfig::Manual, // Use manual block production to be sure that the changes are happening in the sequencer only, not the node.
            FINALIZATION_BLOCKS,
        )
        .set_config(|c| {
            c.rollup_prover_config = None;
            c.storage = dir;
            c.axum_port = port;
        })
        .set_da_config(|c| {
            c.sender_address = sequencer_addr;
            c.da_layer = Some(da_layer.clone());
        })
        .start()
        .await
        .unwrap()
    };
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    // Produce some empty blocks to make sure we have a finalized slot.
    da_layer.produce_and_wait_for_n_slots(5).await;

    // Note: The exact number height asserted here is not important, as long as it's correct
    // at the time we submit the transaction - if we change the sequencer logic, this number may need to be updated.
    let tx = tx_set_value(&admin.private_key, 0, 0);
    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();
    test_rollup.force_close_batch().await.unwrap();
    sleep(Duration::from_millis(200)).await; // Sleep to make sure the batch is published before we produce a block. If this gets flaky, we'll need to add a blob_sender subscription.
    da_layer.produce_and_wait_for_slot().await;

    // Note: The exact number height asserted here is not important, as long as it's correct
    // at the time we submit the transaction - if we change the sequencer logic, this number may need to be updated.
    let tx = tx_builder(admin.private_key.clone());
    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    // Produce a block. Make sure that it's empty, meaning that the preferred sequencer still has the previous tx in memory and will replay it.
    // as part of update_state
    let slot = da_layer.produce_and_wait_for_slot().await;
    assert_eq!(slot.number, 7);
    assert_eq!(slot.batches.len(), 0);

    // Close the batch and submit to DA
    test_rollup.force_close_batch().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await; // Sleep to make sure the batch is published before we produce a block. If this gets flaky, we'll need to add a blob_sender subscription.

    // Ensure that the sequencer has time to see the updated state and submit its batch containing the transaction to DA.
    let next = da_layer.produce_and_wait_for_slot().await;

    assert_eq!(next.number, 8);
    assert_eq!(
        next.batches[0].txs[0].receipt.result,
        TxReceiptResult::Successful,
        "Replay in the sequencer was successful, but replay on the node failed: {:?}",
        next.batches[0],
    );

    assertions(test_rollup).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn events_are_returned_in_tx_response() {
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    sleep(Duration::from_millis(200)).await;

    let client = test_rollup.api_client().clone();
    let tx = tx_set_value(&admin.private_key, 0, 7);
    let response = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    assert_eq!(response.events.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn delayed_tx_is_processed_after_delay() {
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    test_rollup
        .da_service
        .produce_n_blocks_now(5)
        .await
        .unwrap();
    sleep(Duration::from_millis(200)).await;

    let client = test_rollup.api_client().clone();

    let delayed_tx = tx_delayed_call(&admin.private_key, 0);
    let regular_tx = tx_set_value(&admin.private_key, 0, 7);

    let binding = api_types::AcceptTxBody {
        body: BASE64_STANDARD.encode(&delayed_tx),
    };
    let delayed_tx_future = client.accept_tx(&binding);

    // Sleep to ensure that the tx would surely be processed if it were not delayed.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let binding = api_types::AcceptTxBody {
        body: BASE64_STANDARD.encode(&regular_tx),
    };
    let regular_tx_future = client.accept_tx(&binding);

    tokio::select! {
        _ = regular_tx_future => {},
        _ = delayed_tx_future => {
            panic!("Delayed tx was processed before the regular tx");
        },
    };
}

/// This test checks our "nuke the queue" functionality, which ensures fairness when the sequencer has downtime.
///
/// Recall that some transaction types have a "speedbump" where their handlers sleep for a short period of time
/// before entering the execution queue. If the sequencer has downtime during that sleep, these transactions need
/// to be rejected for safety - otherwise, they might "time travel" and be executed before other transactions that
/// arrived earlier - since those transactions were rejected during the downtime. This test covers that functionality.
///
/// This test works by...
/// - Sending a tx that has to go through the speedbump
/// - Intentionally sending enough transactions to cause downtime while that delayed tx is waiting on the speedbump
/// - Producing blocks so that the sequencer recovers from the downtime
/// - Ensuring that the delayed tx still fails with a 503
#[tokio::test(flavor = "multi_thread")]
async fn flaky_txs_that_enter_before_downtime_are_dropped() {
    use futures::future::Either;
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        1000, // Set a small batch time limit to ensure that the sequencer will be overloaded after the first tx.
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    // Produce a the exact minimum number of blocks to ensure that the sequencer has a finalized slot.
    // If we change the finalized slot in the test framework, this number will need to be updated.
    test_rollup
        .da_service
        .produce_n_blocks_now(4)
        .await
        .unwrap();
    sleep(Duration::from_millis(200)).await;

    let client = test_rollup.api_client().clone();

    let first_tx = tx_set_value_and_sleep(&admin.private_key, 0, 0, 1200);
    let delayed_tx = tx_delayed_call(&admin.private_key, 1);
    let third_tx = tx_set_value(&admin.private_key, 1, 8);
    let fourth_tx = tx_set_value(&admin.private_key, 2, 9);

    // Submit the first tx. This should succeed. This verifies that our initialization works fine *and* causes the sequencer to be close out its current batch.
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&first_tx),
        })
        .await
        .unwrap();

    // Produce a new block that includes this first tx. It will take 1200 ms to get processed, so start soon.
    test_rollup
        .da_service
        .produce_n_blocks_now(1)
        .await
        .unwrap();
    // Wait until the new batch is almost processed
    sleep(Duration::from_millis(1100)).await;

    // Send off the delayed tx. It should arrive at the seqeuncer immediately and begin sleeping.
    let delayed_tx_handle = tokio::spawn({
        let client = client.clone();
        async move {
            let response = client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&delayed_tx),
                })
                .await
                .unwrap_err()
                .to_string();
            assert!(
                response.contains("The sequencer is temporarily overloaded"),
                "Expected error to contain 'The sequencer is temporarily overloaded', got: {response}"
            );
        }
    });
    // Sleep to ensure that the delayed tx arrives before this one.
    // The third tx to be sent (second to be processed because of the speedbump) should be rejected since the batch is full.
    // This is the "downtime" that we're testing for.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let third_tx_response = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&third_tx),
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        third_tx_response.contains("The sequencer is temporarily overloaded"),
        "Expected error to contain 'The sequencer is temporarily overloaded', got: {third_tx_response}"
    );
    // Produce blocks to ensure that the sequencer has room to process the following txs
    test_rollup
        .da_service
        .produce_n_blocks_now(3)
        .await
        .unwrap();
    // Sleep until these new blocks can be processed
    tokio::time::sleep(Duration::from_millis(220)).await;

    // Send a fourth tx. It should succeed *before* the speedbumped tx is processed.
    let fourth_tx_handle = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&fourth_tx),
                })
                .await
                .unwrap()
        }
    });
    // Check that the 4th tx was processed successfully before the delayed tx, and that the delayed tx task didn't panic (i.e. that its assertion of receiving 503 passed)
    let result = futures::future::select(delayed_tx_handle, fourth_tx_handle).await;
    match result {
        Either::Left(_) => {
            panic!("Delayed tx was processed before the regular tx. This is just a timing issue, but we fail to be extra safe");
        }
        Either::Right((fourth_tx_result, delayed_tx_handle)) => {
            delayed_tx_handle.await.unwrap();
            fourth_tx_result.unwrap();
        }
    }

    // Send a delayed tx to ensure that it's processed as expected. This rules out unforunate errors like a bug in our handling of this tx type.
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(tx_delayed_call(&admin.private_key, 3)),
        })
        .await
        .unwrap();
}

/// Ensure that we use the correct visible slot number when replaying transactions after a call to `update_state` in the sequencer.
/// The key thing that this test does is to execute the same transaction 3 times - once in the sequencer via `accept_tx`, once via `update_state`
/// and once in the node. Everything else is implementation details.
#[tokio::test(flavor = "multi_thread")]
async fn replay_uses_correct_visible_slot_number() {
    let tx_builder = |key| tx_assert_visible_slot_number(&key, 0, 2);
    do_manual_block_production_test(tx_builder, |_| async {}, 22223).await;
}

/// This test checks that the visible hash of the rollup block is the same in the node and the sequencer.
///
/// This test currently works by running a loop of...
/// - Send a transaction to trigger the sequencer to start a batch.
/// - Query the current state root from the sequencer.
/// - Send a transaction that asserts the correct state root. Ensure it is accepted
/// - Produce a block, triggering the sequencer to close out its current batch and post it on DA
/// - Check that the state root assertion suceeded on the node as well.
#[tokio::test(flavor = "multi_thread")]
async fn visible_hashes_match_across_node_and_sequencer() {
    const FINALIZATION_BLOCKS: u32 = 0;
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

    let dir = Arc::new(tempfile::tempdir().unwrap());

    let da_layer = Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(FINALIZATION_BLOCKS)
            .await
            .unwrap(),
    ));
    let test_rollup = {
        let sequencer_addr = genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address;
        RollupBuilder::<TestBlueprint>::new(
            GenesisSource::CustomParams(genesis_params),
            BlockProducingConfig::Manual, // Use manual block production to be sure that the changes are happening in the sequencer only, not the node.
            FINALIZATION_BLOCKS,
        )
        .set_config(|c| {
            c.rollup_prover_config = None;
            c.storage = dir;
        })
        .set_da_config(|c| {
            c.sender_address = sequencer_addr;
            c.da_layer = Some(da_layer.clone());
        })
        .start()
        .await
        .unwrap()
    };

    let mut state_update_subscription = test_rollup.subscribe_state_updates().await.unwrap();
    let mut slot_subscription = test_rollup
        .api_client()
        .subscribe_slots_with_children(IncludeChildren::new(true))
        .await
        .unwrap();

    #[derive(Debug, serde::Deserialize)]
    struct StateRootResponse {
        root_hashes: Vec<u8>,
    }
    #[derive(Debug, serde::Deserialize)]
    struct ValueResponse {
        value: StateRootResponse,
    }
    async fn get_state_root(test_rollup: &TestRollup<TestBlueprint>) -> StateRootResponse {
        let state_root_url = format!(
            "{}/modules/hooks-count/state/latest-state-root/",
            test_rollup.api_client().baseurl()
        );
        let response = test_rollup
            .api_client()
            .client()
            .get(state_root_url)
            .send()
            .await
            .unwrap();
        let response = response
            .json::<ValueResponse>()
            .await
            .expect("Hooks must have run");
        response.value
    }
    // Produce some empty blocks to ensure that the sequencer has a batch in progress.
    da_layer.write().await.produce_block().await.unwrap();
    slot_subscription.next().await.unwrap().unwrap();
    state_update_subscription.next().await.unwrap().unwrap();
    da_layer.write().await.produce_block().await.unwrap();
    slot_subscription.next().await.unwrap().unwrap();
    state_update_subscription.next().await.unwrap().unwrap();

    // Run a few rounds of checking the state root to be extra sure nothing gets screwed up over time.
    let mut current_nonce = 0;
    for i in 0..10 {
        // Send a transaction to ensure that the sequencer has a batch in progress. This is necessary
        // because we start a new rollup block (with a new visible hash) each time we start a batch.
        let tx = tx_set_value(&admin.private_key, current_nonce, i).clone();
        test_rollup
            .api_client()
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        current_nonce += 1;

        // Query the current state root from the node.
        let root = get_state_root(&test_rollup).await;
        tracing::info!(
            "Sending assert state root tx: {}",
            hex::encode(&root.root_hashes)
        );

        // Send a transaction that asserts the correct state root.
        let tx = tx_assert_state_root(&admin.private_key, current_nonce, root.root_hashes.clone())
            .clone();
        test_rollup
            .api_client()
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        current_nonce += 1;

        // Produce a block. This will trigger the sequencer to close out its current batch and start a new one.
        test_rollup.force_close_batch().await.unwrap();
        da_layer.write().await.produce_block().await.unwrap();
        let slot = slot_subscription.next().await.unwrap().unwrap();
        if !slot.batches.is_empty() && !slot.batches[0].txs.is_empty() {
            // Assert that the second transaction in the batch (the one that asserts the state root) succeeded.
            assert_eq!(
                slot.batches[0].txs[1].receipt.result,
                TxReceiptResult::Successful
            );
        }
        state_update_subscription.next().await.unwrap().unwrap();
    }
}

/// This test checks what happens when the DA layer takes a long time to publish blobs under load.
/// This is a regression test for behavior which would cause the sequencer to produce ever-larger batches
/// and then blow up.
///
/// If this test is flaky, it can be made more reliable by simply increasing the `worker_timeout_secs` variable.
/// This makes the test take longer to run, but also reduces the chance of it flaking.
// Note that this test is marked heavy, so it will be ignored by nextest unless you run `make test-all` or manually activate the `ci` test profile
#[tokio::test(flavor = "multi_thread")]
async fn heavy_blob_submission_long_delay() {
    let worker_timeout_secs = 90;
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "150000");
    let max_batch_size = 1 << 30;
    let blob_processing_timeout_secs = 500;
    let (task_completed_sender, task_completed_receiver) = tokio::sync::oneshot::channel();
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

    let Some(test_rollups) = new_test_rollup::<TestRuntime<TestSpec>>(
        dir.clone(),
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
        0,
        true,
        max_batch_size,
        BlockProducingConfig::Periodic { block_time_ms: 200 },
        None,
        blob_processing_timeout_secs,
        1,
        400, // Set the batch time limit to twice the block time
        None,
        TEST_FINALIZATION_BLOCKS,
    )
    .await
    else {
        // Docker issues, don't fail the test and just return early.
        return;
    };
    let test_rollup = test_rollups.into_iter().next().unwrap();

    test_rollup.da_service.set_delay_blobs_by(30).await;

    let nonce = Arc::new(AtomicU64::new(0));
    let timeout_handle = tokio::spawn(async move {
        let timeout = worker_timeout_secs + 10;
        tokio::time::timeout(Duration::from_secs(timeout), task_completed_receiver)
            .await
            .unwrap()
    });

    // Spawn 20 workers to spam the sequencer with load.
    let workers = (0..50)
        .map(|_| {
            let client = test_rollup.api_client().clone();
            let nonce = nonce.clone();
            let key = admin.private_key.clone();
            tokio::spawn(async move {
                let start = std::time::Instant::now();
                loop {
                    if start.elapsed() > Duration::from_secs(worker_timeout_secs) {
                        break;
                    }
                    let nonce = nonce.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let tx = tx_set_many_values(&key, nonce, vec![nonce as u8; 1024]);

                    let resp = client
                        .accept_tx(&api_types::AcceptTxBody {
                            body: BASE64_STANDARD.encode(&tx),
                        })
                        .await;
                    if let Err(e) = resp {
                        tracing::warn!("Error sending tx: {:?}", e);
                    }
                }
            })
        })
        .collect::<Vec<_>>();

    // Wait for the workers to finish.
    futures::future::join_all(workers).await;

    tokio::select! {
        _ = timeout_handle => {
            panic!("Test timed out! This means the sequencer has regressed!");
        }
        shutdown_result = test_rollup.shutdown() => {
            shutdown_result.unwrap();
            task_completed_sender.send(()).expect("Failed to send task completed signal");
        }
    }
}

/// This test checks that state changes from the begin/end slot and finalize hooks are visible via the sequencer's REST API.
///
/// It works by producing several batches in the sequencer (causing the hooks to be run) without every publishing those batches
/// to DA (ensuring that the state changes are not visible to the node), then querying the state via the REST API.
// TODO(@neysofu): unflaky it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Broken by removal of /batches endpoint, will be testable again when we add node/sequencer state comparision support"]
async fn flaky_test_hooks_state_is_visible() {
    const FINALIZATION_BLOCKS: u32 = 3;
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

    let dir = Arc::new(tempfile::tempdir().unwrap());

    let da_layer = Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(FINALIZATION_BLOCKS)
            .await
            .unwrap(),
    ));
    let test_rollup = {
        let sequencer_addr = genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address;
        RollupBuilder::<TestBlueprint>::new(
            GenesisSource::CustomParams(genesis_params),
            BlockProducingConfig::Manual, // Use manual block production to be sure that the changes are happening in the sequencer only, not the node.
            FINALIZATION_BLOCKS,
        )
        .set_config(|c| {
            c.automatic_batch_production = false;
            c.rollup_prover_config = None;
            c.storage = dir;
        })
        .set_da_config(|c| {
            c.sender_address = sequencer_addr;
            c.da_layer = Some(da_layer.clone());
        })
        .start()
        .await
        .unwrap()
    };

    test_rollup
        .da_service
        .produce_n_blocks_now(8)
        .await
        .unwrap();
    sleep(Duration::from_millis(200)).await;

    // By the time we query here, the sequencer has *started* the next slot, so it has run the begin slot hook a second time.
    let client = NodeClient::new(test_rollup.api_client().baseurl())
        .await
        .unwrap();

    let query_hook_counter = |hook_name: &'static str| async {
        #[derive(Debug, serde::Deserialize)]
        struct ValueResponse {
            value: u32,
        }
        let hook_name = hook_name.to_string();
        client
            .query_rest_endpoint::<ValueResponse>(&format!(
                "/modules/hooks-count/state/{hook_name}-hook-count"
            ))
            .await
            .unwrap()
            .value
    };

    let begin_slot_count = query_hook_counter("begin-rollup-block").await;
    assert_eq!(begin_slot_count, 0);
    let begin_slot_count = query_hook_counter("end-rollup-block").await;
    assert_eq!(begin_slot_count, 0);

    {
        let txs = generate_txs(admin.private_key.clone()).clone();
        for tx in txs {
            client
                .client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx.raw_tx),
                })
                .await
                .unwrap();
        }
    }

    let begin_slot_count = query_hook_counter("begin-rollup-block").await;
    assert_eq!(begin_slot_count, 1);

    //  since we haven't finished building this batch, the end slot hook hasn't been run - so its value is still 0
    let end_slot_count = query_hook_counter("end-rollup-block").await;
    assert_eq!(end_slot_count, 0);
    // was run once during genesis
    let finalize_count = query_hook_counter("finalize").await;
    assert_eq!(finalize_count, 1);

    test_rollup
        .da_service
        .produce_n_blocks_now(10)
        .await
        .unwrap();
    sleep(Duration::from_millis(200)).await;

    let begin_slot_count = query_hook_counter("begin-rollup-block").await;
    assert_eq!(begin_slot_count, 1);
    let end_slot_count = query_hook_counter("end-rollup-block").await;
    assert_eq!(end_slot_count, 1);
    let finalize_count = query_hook_counter("finalize").await;
    assert_eq!(finalize_count, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_production_and_accept_tx() {
    let mut actions = vec![];
    for i in 1..20 {
        actions.push(TestingAction::AcceptTxs { count: i });
        actions.push(TestingAction::AcceptTx);
    }

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

// Checks that transactions that are not sequencer safe are rejected
// when the sender address is not configured as an admin in the sequencer config.
#[tokio::test(flavor = "multi_thread")]
async fn not_sequencer_safe_txs_are_restricted() {
    let (test_rollup, admin) = create_test_rollup(
        0,
        TEST_MAX_BATCH_SIZE,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
    )
    .await;

    let Some(test_rollup) = test_rollup else {
        return;
    };

    test_rollup
        .da_service
        .produce_n_blocks_now(10)
        .await
        .unwrap();

    // Wait for all blocks to be processed by the node+sequencer. TODO: better
    // logic not prone to race conditions.
    sleep(Duration::from_millis(500)).await;

    let tx = generate_paymaster_tx::<TestRuntime<TestSpec>>(admin.private_key.clone());
    {
        if let Err(e) = test_rollup
            .client
            .client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
        {
            assert!(
                e.to_string().contains("Only designated admins are allowed"),
                "Unexpected error: {e}"
            );
        } else {
            panic!("Sequencer accepted admin tx from non-admin sender");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_and_query_value() {
    let actions = vec![TestingAction::Restart, TestingAction::QuerySetValue];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_production_and_state_query() {
    let actions = vec![
        TestingAction::AcceptTx,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn api_state_race_condition_regression() {
    let actions = vec![
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::DuplicateTransaction,
        },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::NewDaSlot {},
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_after_big_batch_regression() {
    let actions = vec![
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::AcceptTxs { count: 5 },
        TestingAction::AcceptTx,
        TestingAction::Restart,
        TestingAction::AcceptTxs { count: 10 },
        TestingAction::Restart,
        TestingAction::AcceptTx,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[derive(BorshSerialize, BorshDeserialize, Clone)]
struct X {
    data: Vec<Vec<u8>>,
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_batch_production_with_immediate_finalization() {
    let actions = vec![
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::AcceptTxs { count: 50 },
        TestingAction::Restart,
        // Restarting is consistently slow in this test because of the big batches, so sleep extra
        TestingAction::Sleep { duration_ms: 2000 },
        TestingAction::AcceptTx,
        TestingAction::Sleep { duration_ms: 50 },
        TestingAction::Restart,
        TestingAction::Sleep { duration_ms: 2000 },
        TestingAction::AcceptTxs { count: 50 },
        TestingAction::Restart,
        TestingAction::Sleep { duration_ms: 2000 },
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTxs { count: 3 },
        TestingAction::AcceptTxs { count: 50 },
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_state_and_node_state_matches() {
    let actions = vec![
        TestingAction::PauseUpdateStateExecution(true),
        TestingAction::AcceptTxs { count: 50 },
        // Query the state the sequencer has produced
        // Sequencer state updates are currently paused so we dont
        // update with state from the node.
        TestingAction::QuerySetValue,
        TestingAction::PauseUpdateStateExecution(false),
        // Produce a block to trigger an update_state operation
        // which will replace the state with that from the node.
        // It should produce the same result as our previous query
        TestingAction::NewDaSlot,
        // Give time for `update_state` to run
        // This should produce a batch containing previous transactions
        TestingAction::Sleep { duration_ms: 500 },
        // Trigger another block so we get updated state
        TestingAction::NewDaSlot,
        // give time for update state to run
        TestingAction::Sleep { duration_ms: 500 },
        // Should be unchanged
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn api_state_race_condition_regression2() {
    let actions = vec![
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::DuplicateTransaction,
        },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::NewDaSlot {},
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

async fn preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions: Vec<TestingAction>) {
    let mut genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    genesis_config.initial_sequencer.bond = genesis_config
        .initial_sequencer
        .bond
        .checked_mul(Amount::new(100))
        .unwrap();

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

    let Some(test_rollups) = new_test_rollup::<TestRuntime<TestSpec>>(
        dir.clone(),
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
        0,
        false,
        TEST_MAX_BATCH_SIZE,
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        Some(RollupProverConfig::Skip),
        60,
        1,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
        None,
        TEST_FINALIZATION_BLOCKS,
    )
    .await
    else {
        // Docker issues, don't fail the test and just return early.
        return Default::default();
    };
    let test_rollup = test_rollups.into_iter().next().unwrap();

    let (test_rollup, test_state) = setup_test_rollup_with_initial_state(test_rollup, &admin).await;
    run_actions_against_test_rollup(actions, test_rollup, &admin, test_state).await;
}

pub(crate) async fn setup_test_rollup_with_initial_state(
    test_rollup: TestRollup<TestBlueprint>,
    admin: &TestUser<TestSpec>,
) -> (TestRollup<TestBlueprint>, TestState) {
    test_rollup
        .da_service
        .produce_n_blocks_now(10)
        .await
        .unwrap();

    // Wait for all blocks to be processed by the node+sequencer. TODO: better
    // logic not prone to race conditions.
    sleep(Duration::from_millis(500)).await;

    let client = test_rollup.api_client().clone();

    let mut test_state = TestState::default();

    {
        let txs = generate_txs(admin.private_key.clone()).clone();
        for tx in txs {
            client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx.raw_tx),
                })
                .await
                .unwrap();
            test_state.next_generation += 1;
        }
    }

    // initialize nonce value
    {
        let tx = tx_set_value(
            &admin.private_key,
            test_state.next_generation,
            test_state.current_value,
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        test_state.next_generation += 1;
    }

    (test_rollup, test_state)
}

pub(crate) async fn run_actions_against_test_rollup(
    actions: Vec<TestingAction>,
    test_rollup: TestRollup<TestBlueprint>,
    admin: &TestUser<TestSpec>,
    mut test_state: TestState,
) -> (TestRollup<TestBlueprint>, TestState) {
    let mut test_rollup = Some(test_rollup);

    for (i, action) in actions.iter().enumerate() {
        let new_test_rollup_res = run_action_against_test_rollup(
            test_rollup.take().unwrap(),
            &admin.private_key,
            action.clone(),
            &mut test_state,
        )
        .await;

        match new_test_rollup_res {
            Ok(new_test_rollup) => test_rollup = Some(new_test_rollup),
            Err(e) => {
                println!("Action history: {:#?}", actions[..=i].to_vec());
                println!("test state: {test_state:#?}");
                panic!("Error: {e:#?}");
            }
        }
    }

    (test_rollup.unwrap(), test_state)
}

pub(crate) async fn run_action_against_test_rollup(
    test_rollup: TestRollup<TestBlueprint>,
    key: &Ed25519PrivateKey,
    action: TestingAction,
    test_state: &mut TestState,
) -> anyhow::Result<TestRollup<TestBlueprint>> {
    assert!(test_state.next_generation > 0);

    info!(
        ?action,
        test_state.next_generation, "Executing testing action"
    );

    match action {
        TestingAction::PauseUpdateStateExecution(v) => pause_update_state::set(v),
        TestingAction::Sleep { duration_ms } => {
            sleep(Duration::from_millis(duration_ms)).await;
        }
        TestingAction::Restart => {
            // This is a more complex action, as the sequencer cannot accept transactions on
            // startup until a StateUpdateInfo from the node has been processed.
            let test_rollup = test_rollup.restart().await?;
            test_rollup.da_service.produce_block_now().await.unwrap();
            sleep(Duration::from_millis(500)).await;
            return Ok(test_rollup);
        }
        TestingAction::TryAcceptBadTx { invalid_reason } => {
            let tx = match invalid_reason {
                InvalidGeneration::DuplicateTransaction => tx_set_value(
                    key,
                    test_state.next_generation - 1,
                    test_state.current_value,
                ),
                InvalidGeneration::TooOld => {
                    let bad_generation = test_state.next_generation
                        - 1
                        - config_value!("PAST_TRANSACTION_GENERATIONS");
                    tx_set_value(key, bad_generation, test_state.current_value + 1)
                }
            };

            anyhow::ensure!(test_rollup
                .api_client()
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await
                .is_err());
        }
        TestingAction::AcceptTx => {
            let tx = tx_set_value(
                key,
                test_state.next_generation,
                test_state.current_value + 1,
            );

            test_state.next_generation += 1;
            test_state.current_value += 1;

            test_rollup
                .api_client()
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await?;
        }
        TestingAction::AcceptTxs { count } => {
            for _ in 0..count {
                let tx = tx_set_value(
                    key,
                    test_state.next_generation,
                    test_state.current_value + 1,
                );

                test_state.next_generation += 1;
                test_state.current_value += 1;

                test_rollup
                    .api_client()
                    .accept_tx(&api_types::AcceptTxBody {
                        body: BASE64_STANDARD.encode(&tx),
                    })
                    .await?;
            }
        }
        TestingAction::NewDaSlot => {
            test_rollup.da_service.produce_block_now().await.unwrap();
        }
        TestingAction::QuerySetValueHistorical => {
            for (slot_number, value) in test_state.value_by_slot_number.iter() {
                info!(
                    %slot_number,
                    %value,
                    "Historical query of value",
                );
                query_set_value(&test_rollup, Some(slot_number.get()), *value).await?;
            }
        }
        TestingAction::QuerySetValue => {
            query_set_value(&test_rollup, None, test_state.current_value).await?;
        }
    }

    Ok(test_rollup)
}

async fn query_set_value(
    test_rollup: &TestRollup<TestBlueprint>,
    rollup_height: Option<u64>,
    expected: u64,
) -> anyhow::Result<()> {
    query_set_value_helper(
        test_rollup,
        rollup_height.map(|n| format!("rollup_height={n}")),
        expected,
    )
    .await
}

async fn query_set_value_by_slot_number(
    test_rollup: &TestRollup<TestBlueprint>,
    slot_number: Option<u64>,
    expected: u64,
) -> anyhow::Result<()> {
    query_set_value_helper(
        test_rollup,
        slot_number.map(|n| format!("slot_number={n}")),
        expected,
    )
    .await
}

async fn query_set_value_helper(
    test_rollup: &TestRollup<TestBlueprint>,
    query_param: Option<String>,
    expected: u64,
) -> anyhow::Result<()> {
    let url = format!(
        "/modules/value-setter/state/value{}",
        if let Some(query_param) = query_param {
            format!("?{query_param}")
        } else {
            "".to_string()
        }
    );
    let response = test_rollup
        .client
        .query_rest_endpoint::<serde_json::Value>(&url)
        .await?;
    // quick/easy way to detect if the request returned a error, i.e 404
    if response.get("message").is_some() {
        return Err(anyhow::anyhow!("API request failed: {:?}", response));
    }

    debug!(?response, "Querying value");
    let found_value = response["value"].as_u64().unwrap_or_default();

    anyhow::ensure!(found_value == expected);

    Ok(())
}

fn tx_set_value(key: &Ed25519PrivateKey, nonce: u64, value_to_set: u64) -> RawTx {
    tx_set_value_with_gas::<TestRuntime<TestSpec>>(
        key,
        nonce,
        value_to_set,
        None,
        sov_test_utils::TEST_DEFAULT_MAX_FEE,
    )
}

fn tx_delayed_call(key: &Ed25519PrivateKey, nonce: u64) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::HooksCount(
        sov_test_modules::hooks_count::CallMessage::DelayedCallMsg {},
    );
    encode_call(key, nonce, &msg)
}

fn tx_set_many_values(key: &Ed25519PrivateKey, nonce: u64, values_to_set: Vec<u8>) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetManyValues(values_to_set),
    );
    encode_call(key, nonce, &msg)
}

fn tx_set_value_and_sleep(
    key: &Ed25519PrivateKey,
    nonce: u64,
    value_to_set: u64,
    sleep_millis: u64,
) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetValueAndSleep {
            value: value_to_set as u32,
            sleep_millis,
        },
    );
    encode_call(key, nonce, &msg)
}

fn tx_panic(key: &Ed25519PrivateKey, nonce: u64) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::Panic,
    );
    encode_call(key, nonce, &msg)
}

fn tx_assert_visible_slot_number(
    key: &Ed25519PrivateKey,
    nonce: u64,
    assert_visible_slot_number: u64,
) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::HooksCount(
        sov_test_modules::hooks_count::CallMessage::AssertVisibleSlotNumber {
            expected_visible_slot_number: assert_visible_slot_number,
        },
    );

    encode_call(key, nonce, &msg)
}

fn tx_assert_state_root(
    key: &Ed25519PrivateKey,
    nonce: u64,
    expected_state_root: Vec<u8>,
) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::HooksCount(
        sov_test_modules::hooks_count::CallMessage::AssertStateRoot {
            expected_state_root,
        },
    );

    encode_call(key, nonce, &msg)
}

fn encode_call(
    key: &Ed25519PrivateKey,
    nonce: u64,
    call_message: &<TestRuntime<TestSpec> as DispatchCall>::Decodable,
) -> RawTx {
    let tx = default_test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        call_message,
        nonce,
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

mod tests_with_basic_kernel {
    use sov_modules_stf_blueprint::GenesisParams;
    use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder};
    use sov_test_utils::{RtAgnosticBlueprint, TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING};

    use super::{
        generate_optimistic_runtime_with_kernel, HighLevelOptimisticGenesisConfig, TestSpec,
    };
    generate_optimistic_runtime_with_kernel!(
        RtWithBasicKernel <=
        kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
        modules: [],
    );

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic(
        expected = "Attempting to use preferred sequencer with an incompatible rollup. Set your sequencer config to `standard` in your rollup's config.toml file or change your kernel to be compatible with soft confirmations."
    )]
    async fn preferred_sequencer_panics_with_basic_kernel() {
        let genesis_config = HighLevelOptimisticGenesisConfig::generate();
        let genesis_params = GenesisParams {
            runtime: GenesisConfig::from_minimal_config(genesis_config.into()),
        };

        RollupBuilder::<RtAgnosticBlueprint<TestSpec, RtWithBasicKernel<TestSpec>>>::new(
            GenesisSource::CustomParams(genesis_params),
            TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
            0,
        )
        .set_config(|conf| {
            conf.sequencer_config =
                sov_sequencer::SequencerKindConfig::Preferred(Default::default());
        })
        .start()
        .await
        .unwrap();
    }
}
