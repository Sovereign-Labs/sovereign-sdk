//! Integration tests for the preferred sequencer that use [`RollupBuilder`] and
//! thus test sequencer + node interactions.

use crate::utils::{encode_call, EventEmitterModule};
use crate::utils::{
    generate_paymaster_tx, generate_txs, query_set_value, query_set_value_helper,
    tempdir_inside_codebase_dir, tx_set_value_with_gas, ModuleWithVersionedStateAccessInSlotHook,
    SeqTestRollup, MAX_BATCH_EXECUTION_TIME_MILLIS,
};
use anyhow::Context;
use backon::Retryable;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use futures::future;
use serde_json::Number;
use sov_api_spec::types::{
    self as api_types, ApiError, SequencerListEventsPage, SequencerListEventsResponse,
    TxInfoWithConfirmation, TxReceiptResult,
};
use sov_api_spec::{types, ClientInfo, Error, ResponseValue, WsSubscription};
use sov_db::config::{default_max_pruning_batch_size, PrunerConfig};
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::prelude::*;
use sov_modules_api::{
    Amount, DispatchCall, Gas, GasArray, GasPrice, GasUnit, PrivateKey, PublicKey, RawTx, Runtime,
};
use sov_modules_stf_blueprint::GenesisParams;
use sov_node_client::NodeClient;
use sov_paymaster::{Paymaster, PaymasterConfig};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::execution_mode::Native;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::ledger_api::IncludeChildren;
use sov_sequencer::{SequencerKindConfig, StateUpdateNotification};
use sov_test_modules::hooks_count::HooksCount;
use sov_test_utils::runtime::genesis::optimistic::{
    HighLevelOptimisticGenesisConfig, MinimalOptimisticGenesisConfig,
};
use sov_test_utils::test_rollup::FullNodeBlueprint;
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::test_rollup::{
    next_within, GenesisSource, RollupBuilder, RollupProverConfig, TestRollup,
};
use sov_test_utils::{
    generate_optimistic_runtime_with_kernel, RtAgnosticBlueprint, TestSpec, TestUser,
    TEST_DEFAULT_MOCK_DA_ON_SUBMIT, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE,
    TEST_MAX_CONCURRENT_BATCH_BLOBS, TEST_NORMAL_SHUTDOWN_TIMEOUT,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};
use std::collections::HashMap;
use std::future::Future;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use test_strategy::Arbitrary;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_stream::StreamExt;
use tracing::info;

mod action_scenarios;
mod api_streams;
mod archival;
mod batch_limits;
mod downtime_recovery;
mod event_epoch;
mod node_consistency;
mod startup_shutdown;
mod tx_admission;

const DELAYED_TX_DELAY_MS: u64 = 2500;
const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(30);

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [value_setter: ValueSetter<S>, hooks_count: HooksCount<S>, paymaster: Paymaster<S>, slot_hook_checker: ModuleWithVersionedStateAccessInSlotHook<S>, event_emitter: EventEmitterModule<S>],
    transaction_delay_ms_wrapper: |call: &Self::Decodable| {
        match call {
            Self::Decodable::HooksCount(sov_test_modules::hooks_count::CallMessage::DelayedCallMsg) => DELAYED_TX_DELAY_MS,
            _ => 0,
        }
    },
    transaction_priority_wrapper: |call: &sov_modules_api::FullyBakedTx| {
        use sov_modules_api::capabilities::TransactionAuthenticator;
        let Ok(call) = Self::Auth::decode_serialized_tx(call) else {
            return 0;
        };
        match call {
            Self::Decodable::ValueSetter(sov_value_setter::CallMessage::SetValue { value, .. }) =>
                value,
            _ => 0,
        }

    },
);

pub(crate) type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;

const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::OnBatchSubmit {
    block_wait_timeout_ms: None,
};

#[derive(Debug, serde::Deserialize)]
struct ValueResponse {
    value: u32,
}

pub struct DaLayerWithSubscription {
    da_layer: Arc<RwLock<StorableMockDaLayer>>,
    state_update_subscription: WsSubscription<StateUpdateNotification>,
    slot_subscription: WsSubscription<api_types::Slot>,
    /// The number of slots that have been produced but not yet had their notifications received.
    back_slot_notifications: u64,
}

impl DaLayerWithSubscription {
    pub async fn new<
        R: FullNodeBlueprint<Native, DaService = StorableMockDaService, Spec = S> + Default,
        S: Spec,
    >(
        test_rollup: &TestRollup<R>,
    ) -> Self {
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
            next_within(subscription, NOTIFICATION_TIMEOUT)
                .await
                .unwrap();
        }

        self.back_slot_notifications -= 1;
        next_within(subscription, NOTIFICATION_TIMEOUT)
            .await
            .unwrap()
    }

    /// Gets the next state update notification, clearing any *known* updates from the queue first.
    /// Unless you've been using `produce_and_wait_for_slot` for all block production, this notification might possibly be stale.
    /// This is inevitable because `update_state` doesn't run on every block, so (unlike the slot subscription) we don't know
    /// how many state update notifications we should ultimately be receiving.
    pub async fn next_state_update_notification(&mut self) -> StateUpdateNotification {
        let subscription = self.state_update_subscription.as_mut().unwrap();
        next_within(subscription, NOTIFICATION_TIMEOUT)
            .await
            .unwrap()
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
    /// Never generated automatically because tests would slow down waaay too much.
    /// Useful for debugging.
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

fn create_genesis_params() -> (GenesisParams<GenesisConfig<TestSpec>>, TestUser<TestSpec>) {
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
            (),
        );

    (
        GenesisParams {
            runtime: rt_genesis_config.clone(),
        },
        admin,
    )
}

async fn create_test_rollup() -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    create_test_rollup_with(|b| b).await
}

/// Starts a rollup with this file's genesis; `configure` overrides [`SeqTestRollup`] defaults.
pub(crate) async fn create_test_rollup_with(
    configure: impl FnOnce(SeqTestRollup<TestRuntime<TestSpec>>) -> SeqTestRollup<TestRuntime<TestSpec>>,
) -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let (genesis_params, admin) = create_genesis_params();
    let seq_da_address = genesis_params
        .runtime
        .sequencer_registry
        .sequencer_config
        .seq_da_address;
    let builder = SeqTestRollup::<TestRuntime<TestSpec>>::new(seq_da_address, genesis_params);
    (configure(builder).start().await, admin)
}

#[derive(Debug)]
pub(crate) struct TestState {
    value_by_slot_number: HashMap<SlotNumber, u64>,
    _current_slot_number: SlotNumber,
    pub next_generation: u64,
    pub current_value: u64,
}

impl Default for TestState {
    fn default() -> Self {
        Self {
            value_by_slot_number: Default::default(),
            _current_slot_number: Default::default(),
            // Start at the pruning boundary so that "too old" generation actions
            // are possible without exceeding the forward-generation cap during setup.
            next_generation: config_value!("PAST_TRANSACTION_GENERATIONS"),
            current_value: Default::default(),
        }
    }
}

async fn do_manual_block_production_test<Fut: Future<Output = ()>>(
    tx_builder: impl Fn(Ed25519PrivateKey) -> RawTx,
    assertions: impl FnOnce(TestRollup<TestBlueprint>) -> Fut,
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
            c.rollup_prover_config = RollupProverConfig::Disabled;
            c.storage = StoragePath::Tmp(dir);
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
    // Without waiting for publication, the next block can be produced before the batch is visible in DA.
    test_rollup
        .force_close_batch_and_wait_for_publication()
        .await
        .unwrap();
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

    // Close the batch and wait until it is submitted to DA
    test_rollup
        .force_close_batch_and_wait_for_publication()
        .await
        .unwrap();

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
            (),
        );
    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = tempdir_inside_codebase_dir();

    let test_rollup = SeqTestRollup::<TestRuntime<TestSpec>>::new(
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
    )
    .dir(dir.clone())
    .automatic_batch_production(false)
    .block_producing_config(DEFAULT_BLOCK_PRODUCING_CONFIG)
    .rollup_prover_config(RollupProverConfig::Prove)
    .start()
    .await;

    let (test_rollup, test_state) = setup_test_rollup_with_initial_state(test_rollup, &admin).await;
    run_actions_against_test_rollup(actions, test_rollup, &admin, test_state).await;
}

pub(crate) async fn setup_test_rollup_with_initial_state(
    test_rollup: TestRollup<TestBlueprint>,
    admin: &TestUser<TestSpec>,
) -> (TestRollup<TestBlueprint>, TestState) {
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

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
        TestingAction::PauseUpdateStateExecution(true) => {
            test_rollup.pause_preferred_batches().await;
        }
        TestingAction::PauseUpdateStateExecution(false) => {
            test_rollup.resume_preferred_batches().await;
        }
        TestingAction::Sleep { duration_ms } => {
            // This action intentionally models time passing in scenario-driven tests.
            sleep(Duration::from_millis(duration_ms)).await;
        }
        TestingAction::Restart => {
            // This is a more complex action, as the sequencer cannot accept transactions on
            // startup until a StateUpdateInfo from the node has been processed.
            let test_rollup = test_rollup.restart().await?;
            test_rollup.da_service.produce_block_now().await?;
            test_rollup.wait_for_sequencer_ready().await?;
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
                    let bad_generation = test_state
                        .next_generation
                        .checked_sub(1)
                        .expect("Next generation is to low for TooOld scenario")
                        .checked_sub(config_value!("PAST_TRANSACTION_GENERATIONS"))
                        .expect("Next generation is to low for TooOld scenario");
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
                query_set_value(&test_rollup, Some(slot_number.get()), Some(*value)).await?;
            }
        }
        TestingAction::QuerySetValue => {
            query_set_value(&test_rollup, None, Some(test_state.current_value)).await?;
        }
    }

    Ok(test_rollup)
}

async fn query_set_value_by_slot_number(
    test_rollup: &TestRollup<TestBlueprint>,
    slot_number: Option<u64>,
    expected: Option<u64>,
) -> anyhow::Result<()> {
    query_set_value_helper(
        test_rollup,
        slot_number.map(|n| format!("slot_number={n}")),
        expected,
    )
    .await
}

pub(super) fn tx_set_value(key: &Ed25519PrivateKey, generation: u64, value_to_set: u64) -> RawTx {
    tx_set_value_with_gas::<TestRuntime<TestSpec>>(
        key,
        generation,
        value_to_set,
        None,
        sov_test_utils::TEST_DEFAULT_MAX_FEE,
    )
}

fn tx_event_emitter(key: &Ed25519PrivateKey, events: Vec<bool>, generation: u64) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::EventEmitter(
        crate::utils::EventEmitterCallMessage::EmitEvents { events },
    );
    encode_call::<TestRuntime<TestSpec>>(key, generation, &msg)
}

fn tx_delayed_call(key: &Ed25519PrivateKey, generation: u64) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::HooksCount(
        sov_test_modules::hooks_count::CallMessage::DelayedCallMsg {},
    );
    encode_call::<TestRuntime<TestSpec>>(key, generation, &msg)
}

fn tx_set_many_values(key: &Ed25519PrivateKey, generation: u64, values_to_set: Vec<u8>) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetManyValues(values_to_set),
    );
    encode_call::<TestRuntime<TestSpec>>(key, generation, &msg)
}

fn tx_set_value_and_sleep(
    key: &Ed25519PrivateKey,
    generation: u64,
    value_to_set: u64,
    sleep_millis: u64,
) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetValueAndSleep {
            value: value_to_set as u32,
            sleep_millis,
        },
    );
    encode_call::<TestRuntime<TestSpec>>(key, generation, &msg)
}

fn tx_panic(key: &Ed25519PrivateKey, nonce: u64) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::Panic,
    );
    encode_call::<TestRuntime<TestSpec>>(key, nonce, &msg)
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

    encode_call::<TestRuntime<TestSpec>>(key, nonce, &msg)
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

    encode_call::<TestRuntime<TestSpec>>(key, nonce, &msg)
}
