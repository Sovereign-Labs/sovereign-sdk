use anyhow::Context as AnyhowContext;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use proptest::bits::u64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_chain_state::ChainState;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaService};
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::{RollupHeight, TransactionAuthenticator, UniquenessData};
use sov_modules_api::digest::Digest;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::rest::HasRestApi;
use sov_modules_api::transaction::TransactionCallable;
use sov_modules_api::transaction::{Transaction, TxDetails};
use sov_modules_api::{prelude::*, EventEmitter};
use sov_modules_api::{
    Amount, BlockHooks, CryptoSpec, DispatchCall, FullyBakedTx, GasUnit, Module, ModuleId,
    ModuleInfo, RawTx, StateCheckpoint, TxState,
};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::PaymasterPolicyInitializer;
use sov_rollup_interface::TxHash;
use sov_sequencer::standard::{StdSequencer, StdSequencerConfig};
use sov_sequencer::SequencerKindConfig;
use sov_state::Storage;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::generators::bank::BankMessageGenerator;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::sov_paymaster::{AuthorizedSequencers, PayeePolicy, SafeVec};
use sov_test_utils::runtime::{
    Paymaster, Runtime, TestOptimisticRuntime, TestOptimisticRuntimeCall,
};
use sov_test_utils::sequencer::TestSequencerSetup;
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::test_rollup::{FullNodeBlueprint, GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    default_test_signed_transaction, default_test_tx_details, test_signed_transaction, EncodeCall,
    ManualProofPostingControl, ManualProofPostingRtAgnosticBlueprint, MessageGenerator,
    RtAgnosticBlueprint, TestPrivateKey, TestSpec, TransactionType, TEST_DEFAULT_GAS_LIMIT,
    TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE, TEST_MAX_CONCURRENT_BLOBS,
};
use sov_value_setter::ValueSetter;
use tokio::time::timeout;
use tokio_stream::StreamExt;

pub const MAX_BATCH_EXECUTION_TIME_MILLIS: u64 = 1_000 * 60 * 5; // Allow batches to take up to 5 minutes by default.

pub type MySequencer = StdSequencer<TestSpec, RT, MockDaService>;
pub type RT = TestOptimisticRuntime<TestSpec>;
pub type RTCall = TestOptimisticRuntimeCall<TestSpec>;

#[allow(clippy::too_many_arguments)]
fn configured_test_rollup_builder<B, RT>(
    builder: RollupBuilder<B>,
    dir: Arc<tempfile::TempDir>,
    seq_da_address: MockAddress,
    minimum_profit_per_tx: u128,
    automatic_batch_production: bool,
    max_batch_size_bytes: usize,
    rollup_prover_config: Option<RollupProverConfig<MockZkvm>>,
    blob_processing_timeout_secs: u64,
    max_batch_execution_time_millis: u64,
    stop_at_rollup_height: Option<RollupHeight>,
) -> RollupBuilder<B>
where
    B: FullNodeBlueprint<Native, Spec = TestSpec, Runtime = RT, DaService = StorableMockDaService>
        + Default
        + 'static,
    RT: Runtime<TestSpec> + HasRestApi<TestSpec>,
{
    builder
        .set_config(|c| {
            c.rollup_prover_config = rollup_prover_config;
            c.automatic_batch_production = automatic_batch_production;
            c.storage = StoragePath::Tmp(dir);
            c.max_batch_size_bytes = max_batch_size_bytes;
            c.blob_processing_timeout_secs = blob_processing_timeout_secs;
            c.stop_at_rollup_height = stop_at_rollup_height;
            if let SequencerKindConfig::Preferred(preferred_sequencer_config) =
                &mut c.sequencer_config
            {
                preferred_sequencer_config.batch_execution_time_limit_millis =
                    max_batch_execution_time_millis;
                // Proof generation and sequencer state-root consistency checks are currently
                // incompatible in these integration tests.
                if c.rollup_prover_config.is_some() {
                    preferred_sequencer_config.disable_state_root_consistency_checks = true;
                }
            }
            c.max_concurrent_blobs = TEST_MAX_CONCURRENT_BLOBS;
        })
        .set_da_config(|c| c.sender_address = seq_da_address)
        .set_persistent_da()
        .with_preferred_seq_min_profit_per_tx(minimum_profit_per_tx)
        .with_preferred_seq_recovery_strategy(sov_sequencer::preferred::RecoveryStrategy::TryToSave)
}

pub async fn new_sequencer() -> TestSequencerSetup<RT> {
    let dir = tempfile::tempdir().unwrap();
    let da_service = StorableMockDaService::new_in_memory(
        HighLevelOptimisticGenesisConfig::<TestSpec>::sequencer_da_addr(),
        0,
    )
    .await;

    let sequencer_config = StdSequencerConfig {
        mempool_max_txs_count: None,
        max_batch_size_bytes: None,
    };

    TestSequencerSetup::<RT>::new(dir, da_service, sequencer_config, true)
        .await
        .unwrap()
}

pub fn build_tx<RT: Runtime<TestSpec>>(
    setup: &TestSequencerSetup<RT>,
    generation: u64,
    call_message: &<RT as DispatchCall>::Decodable,
) -> RawTx {
    let tx = default_test_signed_transaction::<RT, TestSpec>(
        &setup.admin_private_key,
        call_message,
        generation,
        &RT::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

pub fn wrap_with_auth(raw_tx: RawTx) -> FullyBakedTx {
    <<TestOptimisticRuntime<TestSpec> as Runtime<TestSpec>>::Auth as TransactionAuthenticator<
        TestSpec,
    >>::encode_with_standard_auth(raw_tx)
}

/// Includes transaction data encoded in several ways, for use with different
/// APIs as needed.
#[derive(Debug, Clone)]
pub struct GeneratedTx {
    pub tx_hash: TxHash,
    pub tx_object: Transaction<RT, TestSpec>,
    pub raw_tx: RawTx,
    pub fully_baked_tx: FullyBakedTx,
}

/// Generates a handful of transactions.
pub fn generate_txs(admin_private_key: TestPrivateKey) -> Vec<GeneratedTx> {
    let bank_generator =
        BankMessageGenerator::<TestSpec>::with_minter_and_transfer(admin_private_key);
    let messages_iter = bank_generator.create_default_messages().into_iter();

    let mut txs = Vec::default();
    for message in messages_iter {
        let tx_object = message.to_tx::<TestOptimisticRuntime<TestSpec>>();
        let raw_tx = RawTx::new(borsh::to_vec(&tx_object).unwrap());

        let tx_hash = TxHash::new(
            <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher::digest(&raw_tx).into(),
        );

        let fully_baked_tx = wrap_with_auth(raw_tx.clone());

        txs.push(GeneratedTx {
            tx_hash,
            tx_object,
            raw_tx,
            fully_baked_tx,
        });
    }

    txs
}

/// Generates a paymaster tx signed with the provided key
pub fn generate_paymaster_tx<RT: Runtime<TestSpec> + EncodeCall<Paymaster<TestSpec>>>(
    key: TestPrivateKey,
) -> RawTx {
    let message = sov_test_utils::runtime::sov_paymaster::CallMessage::RegisterPaymaster {
        policy: PaymasterPolicyInitializer {
            default_payee_policy: PayeePolicy::Deny,
            payees: SafeVec::new(),
            authorized_updaters: SafeVec::new(),
            authorized_sequencers: AuthorizedSequencers::All,
        },
    };
    let details = TxDetails::<TestSpec> {
        max_priority_fee_bips: TEST_DEFAULT_MAX_PRIORITY_FEE,
        max_fee: TEST_DEFAULT_MAX_FEE,
        gas_limit: Some(TEST_DEFAULT_GAS_LIMIT.into()),
        chain_id: config_value!("CHAIN_ID"),
    };
    TransactionType::<RT, TestSpec>::sign_and_serialize(
        <RT as EncodeCall<Paymaster<TestSpec>>>::to_decodable(message),
        key,
        &<RT as Runtime<TestSpec>>::CHAIN_HASH,
        details,
        &mut Default::default(),
    )
}

pub fn valid_tx_bytes<RT: Runtime<TestSpec> + EncodeCall<ValueSetter<TestSpec>>>(
    setup: &TestSequencerSetup<RT>,
    generation: u64,
    value_to_set: u32,
) -> RawTx {
    let msg = <RT as EncodeCall<ValueSetter<TestSpec>>>::to_decodable(
        sov_value_setter::CallMessage::SetValue {
            value: value_to_set,
            gas: None,
        },
    );

    build_tx(setup, generation, &msg)
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    JsonSchema,
    UniversalWallet,
)]
pub enum EventEmitterCallMessage {
    EmitEvents { events: Vec<bool> },
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub enum EventEmitterEvent {
    Event1 {}, // Our rust client requires that events have bodies
    Event2 {},
}

#[derive(ModuleInfo, Clone)]
pub struct EventEmitterModule<S: Spec> {
    #[id]
    id: ModuleId,
    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for EventEmitterModule<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = EventEmitterCallMessage;
    type Event = EventEmitterEvent;
    type Error = anyhow::Error;

    fn call(
        &mut self,
        msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        match msg {
            EventEmitterCallMessage::EmitEvents { events } => {
                for event in events {
                    if event {
                        self.emit_event(state, EventEmitterEvent::Event1 {});
                    } else {
                        self.emit_event(state, EventEmitterEvent::Event2 {});
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(ModuleInfo, Clone)]
pub struct ModuleWithVersionedStateAccessInSlotHook<S: Spec> {
    #[id]
    id: ModuleId,
    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for ModuleWithVersionedStateAccessInSlotHook<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = ();
    type Event = ();
    type Error = anyhow::Error;

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<S: Spec> BlockHooks for ModuleWithVersionedStateAccessInSlotHook<S> {
    type Spec = S;

    fn begin_rollup_block_hook(
        &mut self,
        _visible_hash: &<S::Storage as Storage>::Root,
        state: &mut StateCheckpoint<Self::Spec>,
    ) {
        ChainState::<S>::default()
            .get_time(state)
            .unwrap_infallible();
    }

    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<Self::Spec>) {
        ChainState::<S>::default()
            .get_time(state)
            .unwrap_infallible();
    }
}

pub mod pause_update_state {
    const ENV_VAR: &str = "SOV_TEST_PAUSE_SEQUENCER_UPDATE_STATE";

    pub fn set(value: bool) {
        let v = if value { "1" } else { "0" };
        std::env::set_var(ENV_VAR, v);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn new_test_rollup<RT: Runtime<TestSpec> + HasRestApi<TestSpec>>(
    dir: Arc<tempfile::TempDir>,
    seq_da_address: MockAddress,
    genesis_params: GenesisParams<<RT as Runtime<TestSpec>>::GenesisConfig>,
    minimum_profit_per_tx: u128,
    automatic_batch_production: bool,
    max_batch_size_bytes: usize,
    block_producing_config: BlockProducingConfig,
    rollup_prover_config: Option<RollupProverConfig<MockZkvm>>,
    blob_processing_timeout_secs: u64,
    max_batch_execution_time_millis: u64,
    stop_at_rollup_height: Option<RollupHeight>,
    finalization_blocks: u32,
) -> TestRollup<RtAgnosticBlueprint<TestSpec, RT>> {
    let builder = configured_test_rollup_builder(
        RollupBuilder::<RtAgnosticBlueprint<TestSpec, RT>>::new(
            GenesisSource::CustomParams(genesis_params),
            block_producing_config,
            finalization_blocks,
        ),
        dir,
        seq_da_address,
        minimum_profit_per_tx,
        automatic_batch_production,
        max_batch_size_bytes,
        rollup_prover_config,
        blob_processing_timeout_secs,
        max_batch_execution_time_millis,
        stop_at_rollup_height,
    );

    builder.start().await.unwrap()
}

#[allow(clippy::too_many_arguments)]
pub async fn new_test_rollup_with_manual_proof_posting<
    RT: Runtime<TestSpec> + HasRestApi<TestSpec>,
>(
    dir: Arc<tempfile::TempDir>,
    seq_da_address: MockAddress,
    genesis_params: GenesisParams<<RT as Runtime<TestSpec>>::GenesisConfig>,
    minimum_profit_per_tx: u128,
    automatic_batch_production: bool,
    max_batch_size_bytes: usize,
    block_producing_config: BlockProducingConfig,
    rollup_prover_config: Option<RollupProverConfig<MockZkvm>>,
    blob_processing_timeout_secs: u64,
    max_batch_execution_time_millis: u64,
    stop_at_rollup_height: Option<RollupHeight>,
    finalization_blocks: u32,
) -> (
    TestRollup<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>,
    ManualProofPostingControl,
) {
    new_test_rollup_with_manual_proof_posting_and_proof_jump(
        dir,
        seq_da_address,
        genesis_params,
        minimum_profit_per_tx,
        automatic_batch_production,
        max_batch_size_bytes,
        block_producing_config,
        rollup_prover_config,
        blob_processing_timeout_secs,
        max_batch_execution_time_millis,
        stop_at_rollup_height,
        1,
        finalization_blocks,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn new_test_rollup_with_manual_proof_posting_and_proof_jump<
    RT: Runtime<TestSpec> + HasRestApi<TestSpec>,
>(
    dir: Arc<tempfile::TempDir>,
    seq_da_address: MockAddress,
    genesis_params: GenesisParams<<RT as Runtime<TestSpec>>::GenesisConfig>,
    minimum_profit_per_tx: u128,
    automatic_batch_production: bool,
    max_batch_size_bytes: usize,
    block_producing_config: BlockProducingConfig,
    rollup_prover_config: Option<RollupProverConfig<MockZkvm>>,
    blob_processing_timeout_secs: u64,
    max_batch_execution_time_millis: u64,
    stop_at_rollup_height: Option<RollupHeight>,
    aggregated_proof_block_jump: usize,
    finalization_blocks: u32,
) -> (
    TestRollup<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>,
    ManualProofPostingControl,
) {
    assert!(
        rollup_prover_config.is_some(),
        "manual proof posting requires a prover-enabled rollup"
    );

    let (blueprint, control) =
        ManualProofPostingRtAgnosticBlueprint::<TestSpec, RT>::new_with_control();

    let builder = configured_test_rollup_builder(
        RollupBuilder::<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>::new(
            GenesisSource::CustomParams(genesis_params),
            block_producing_config,
            finalization_blocks,
        )
        .with_blueprint(blueprint.clone()),
        dir,
        seq_da_address,
        minimum_profit_per_tx,
        automatic_batch_production,
        max_batch_size_bytes,
        rollup_prover_config,
        blob_processing_timeout_secs,
        max_batch_execution_time_millis,
        stop_at_rollup_height,
    )
    .set_config(|c| {
        c.aggregated_proof_block_jump = aggregated_proof_block_jump;
        c.max_concurrent_blobs = 256;
    });

    let test_rollup = builder.start().await.unwrap();

    (test_rollup, control)
}

pub async fn produce_block_and_wait_for_sync<RT>(
    test_rollup: &TestRollup<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>,
) -> anyhow::Result<()>
where
    RT: Runtime<TestSpec> + HasRestApi<TestSpec>,
{
    test_rollup.da_service.produce_block_now().await?;
    test_rollup.wait_for_node_synced().await?;
    Ok(())
}

pub async fn pause_preferred_batches_and_confirm<RT>(
    test_rollup: &TestRollup<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>,
) -> anyhow::Result<()>
where
    RT: Runtime<TestSpec> + HasRestApi<TestSpec>,
{
    let mut updates = test_rollup.subscribe_state_updates().await.unwrap();
    test_rollup.pause_preferred_batches().await;
    produce_block_and_wait_for_sync(test_rollup).await?;

    timeout(
        TestRollup::<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>::POLLING_TIMEOUT,
        async {
            loop {
                let Some(next) = updates.next().await else {
                    anyhow::bail!(
                        "state update subscription closed while waiting for pause acknowledgment"
                    );
                };
                let notification = next?;
                if notification.update_skipped_due_to_pause {
                    return Ok::<(), anyhow::Error>(());
                }
            }
        },
    )
    .await
    .context("Timed out waiting for preferred batch pause acknowledgment")??;

    Ok(())
}

pub async fn wait_until_ready_proof_count<RT>(
    test_rollup: &TestRollup<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>,
    control: &ManualProofPostingControl,
    target_ready_proof_count: usize,
    phase: &str,
) -> anyhow::Result<()>
where
    RT: Runtime<TestSpec> + HasRestApi<TestSpec>,
{
    timeout(
        TestRollup::<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>::POLLING_TIMEOUT,
        async {
            loop {
                if control.ready_proof_count() >= target_ready_proof_count {
                    return Ok::<(), anyhow::Error>(());
                }

                if control.blocks_until_next_aggregate_proof() == 0 {
                    let next_ready_proof_count =
                        control.ready_proof_count().saturating_add(1).min(target_ready_proof_count);
                    control
                        .wait_for_ready_proof_count(next_ready_proof_count)
                        .await;
                    continue;
                }

                produce_block_and_wait_for_sync(test_rollup).await?;
            }
        },
    )
    .await
    .with_context(|| {
        format!(
            "Timed out waiting for {target_ready_proof_count} aggregate proofs to become ready to post during {phase}"
        )
    })??;

    Ok(())
}

pub async fn spawn_aggregated_proof_counter(
    client: &sov_api_spec::client::Client,
) -> anyhow::Result<(
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
)> {
    let mut aggregated_proofs = client.subscribe_aggregated_proof().await?;
    let visible_proofs = Arc::new(AtomicUsize::new(0));
    let visible_proofs_counter = visible_proofs.clone();

    let task = tokio::spawn(async move {
        while let Some(next) = aggregated_proofs.next().await {
            next?;
            visible_proofs_counter.fetch_add(1, Ordering::SeqCst);
        }

        anyhow::bail!("aggregated proof subscription closed unexpectedly");
    });

    Ok((visible_proofs, task))
}

pub async fn wait_until_visible_proof_count<RT>(
    test_rollup: &TestRollup<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>,
    visible_proofs: &AtomicUsize,
    target_visible_proofs: usize,
    phase: &str,
) -> anyhow::Result<()>
where
    RT: Runtime<TestSpec> + HasRestApi<TestSpec>,
{
    timeout(
        TestRollup::<ManualProofPostingRtAgnosticBlueprint<TestSpec, RT>>::POLLING_TIMEOUT,
        async {
            loop {
                let current_visible_proofs = visible_proofs.load(Ordering::SeqCst);
                if current_visible_proofs >= target_visible_proofs {
                    break;
                }

                produce_block_and_wait_for_sync(test_rollup).await?;
            }

            Ok::<(), anyhow::Error>(())
        },
    )
    .await
    .with_context(|| {
        format!(
            "Timed out waiting for {target_visible_proofs} aggregate proofs to become visible on the node during {phase}; observed {}",
            visible_proofs.load(Ordering::SeqCst)
        )
    })??;

    Ok(())
}

pub fn encode_call_with_fee<RT: Runtime<TestSpec>>(
    key: &Ed25519PrivateKey,
    generation: u64,
    call_message: &<RT as DispatchCall>::Decodable,
    max_fee: Amount,
) -> RawTx {
    let mut tx_details = default_test_tx_details();
    tx_details.max_fee = max_fee;
    let tx = test_signed_transaction::<RT, TestSpec>(
        key,
        call_message,
        UniquenessData::Generation(generation),
        &<RT as Runtime<TestSpec>>::CHAIN_HASH,
        tx_details,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

pub fn tx_set_value_with_gas<RT: Runtime<TestSpec> + EncodeCall<ValueSetter<TestSpec>>>(
    key: &Ed25519PrivateKey,
    generation: u64,
    value_to_set: u64,
    gas: Option<GasUnit<2>>,
    max_fee: Amount,
) -> RawTx {
    let msg = <RT as EncodeCall<ValueSetter<TestSpec>>>::to_decodable(
        sov_value_setter::CallMessage::SetValue {
            value: value_to_set as u32,
            gas,
        },
    );

    encode_call_with_fee::<RT>(key, generation, &msg, max_fee)
}

// This allows for easily setting file sharing when using Docker Desktop.
pub fn tempdir_inside_codebase_dir() -> Arc<tempfile::TempDir> {
    Arc::new(tempfile::tempdir_in(std::env!("CARGO_TARGET_TMPDIR")).unwrap())
}

pub(crate) fn encode_call<
    RT: Runtime<TestSpec> + TransactionCallable<Call = <RT as DispatchCall>::Decodable>,
>(
    key: &Ed25519PrivateKey,
    generation: u64,
    call_message: &<RT as DispatchCall>::Decodable,
) -> RawTx {
    let tx = default_test_signed_transaction::<RT, TestSpec>(
        key,
        call_message,
        generation,
        &RT::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}
