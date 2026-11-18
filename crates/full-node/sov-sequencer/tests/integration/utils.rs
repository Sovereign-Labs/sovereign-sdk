use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use proptest::bits::u64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_chain_state::ChainState;
use sov_db::config::PrunerConfig;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaService};
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::capabilities::{RollupHeight, TransactionAuthenticator, UniquenessData};
use sov_modules_api::digest::Digest;
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
use sov_rollup_interface::execution_mode::Native;
use sov_rollup_interface::TxHash;
use sov_sequencer::standard::{StdSequencer, StdSequencerConfig};
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
    MessageGenerator, RtAgnosticBlueprint, TestPrivateKey, TestSpec, TransactionType,
    TEST_BLOB_PROCESSING_TIMEOUT, TEST_DEFAULT_GAS_LIMIT, TEST_DEFAULT_MAX_FEE,
    TEST_DEFAULT_MAX_PRIORITY_FEE, TEST_FINALIZATION_BLOCKS, TEST_MAX_BATCH_SIZE,
    TEST_MAX_CONCURRENT_BATCH_BLOBS,
};
use sov_value_setter::ValueSetter;

pub const MAX_BATCH_EXECUTION_TIME_MILLIS: u64 = 1_000 * 60 * 5; // Allow batches to take up to 5 minutes by default.

pub type MySequencer = StdSequencer<TestSpec, RT, MockDaService>;
pub type RT = TestOptimisticRuntime<TestSpec>;
pub type RTCall = TestOptimisticRuntimeCall<TestSpec>;

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
        chain_hash_fragment: 0,
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

/// Builds a preferred-sequencer [`TestRollup`] with the defaults shared by most sequencer
/// integration tests. Override only what a test cares about.
pub struct SeqTestRollup<RT: Runtime<TestSpec>> {
    dir: Arc<tempfile::TempDir>,
    seq_da_address: MockAddress,
    genesis_params: GenesisParams<<RT as Runtime<TestSpec>>::GenesisConfig>,
    minimum_profit_per_tx: u128,
    automatic_batch_production: bool,
    max_batch_size_bytes: usize,
    block_producing_config: BlockProducingConfig,
    rollup_prover_config: RollupProverConfig,
    blob_processing_timeout_secs: u64,
    max_batch_execution_time_millis: u64,
    stop_at_rollup_height: Option<RollupHeight>,
    finalization_blocks: u32,
    pruner: PrunerConfig,
}

impl<RT: Runtime<TestSpec> + HasRestApi<TestSpec>> SeqTestRollup<RT> {
    pub fn new(
        seq_da_address: MockAddress,
        genesis_params: GenesisParams<<RT as Runtime<TestSpec>>::GenesisConfig>,
    ) -> Self {
        Self {
            dir: tempdir_inside_codebase_dir(),
            seq_da_address,
            genesis_params,
            minimum_profit_per_tx: 0,
            automatic_batch_production: true,
            max_batch_size_bytes: TEST_MAX_BATCH_SIZE,
            block_producing_config: BlockProducingConfig::Manual,
            rollup_prover_config: RollupProverConfig::Disabled,
            blob_processing_timeout_secs: TEST_BLOB_PROCESSING_TIMEOUT,
            max_batch_execution_time_millis: MAX_BATCH_EXECUTION_TIME_MILLIS,
            stop_at_rollup_height: None,
            finalization_blocks: TEST_FINALIZATION_BLOCKS,
            // Pruning disabled by default, matching production defaults for fresh nodes.
            pruner: PrunerConfig::Off,
        }
    }

    /// Storage directory. Pass a clone of a directory the test keeps to reuse it across restarts.
    pub fn dir(mut self, dir: Arc<tempfile::TempDir>) -> Self {
        self.dir = dir;
        self
    }

    pub fn minimum_profit_per_tx(mut self, minimum_profit_per_tx: u128) -> Self {
        self.minimum_profit_per_tx = minimum_profit_per_tx;
        self
    }

    pub fn automatic_batch_production(mut self, automatic_batch_production: bool) -> Self {
        self.automatic_batch_production = automatic_batch_production;
        self
    }

    pub fn max_batch_size_bytes(mut self, max_batch_size_bytes: usize) -> Self {
        self.max_batch_size_bytes = max_batch_size_bytes;
        self
    }

    pub fn block_producing_config(mut self, block_producing_config: BlockProducingConfig) -> Self {
        self.block_producing_config = block_producing_config;
        self
    }

    /// Proof generation is incompatible with sequencer state-root consistency checks in these
    /// tests, so enabling the prover also disables those checks.
    pub fn rollup_prover_config(mut self, rollup_prover_config: RollupProverConfig) -> Self {
        self.rollup_prover_config = rollup_prover_config;
        self
    }

    pub fn blob_processing_timeout_secs(mut self, blob_processing_timeout_secs: u64) -> Self {
        self.blob_processing_timeout_secs = blob_processing_timeout_secs;
        self
    }

    pub fn max_batch_execution_time_millis(mut self, max_batch_execution_time_millis: u64) -> Self {
        self.max_batch_execution_time_millis = max_batch_execution_time_millis;
        self
    }

    pub fn stop_at_rollup_height(mut self, stop_at_rollup_height: Option<RollupHeight>) -> Self {
        self.stop_at_rollup_height = stop_at_rollup_height;
        self
    }

    pub fn finalization_blocks(mut self, finalization_blocks: u32) -> Self {
        self.finalization_blocks = finalization_blocks;
        self
    }

    /// State-version pruning in the node DB config, e.g. `PrunerConfig::Periodic { .. }`.
    pub fn pruner(mut self, pruner: PrunerConfig) -> Self {
        self.pruner = pruner;
        self
    }

    pub async fn start(self) -> TestRollup<RtAgnosticBlueprint<TestSpec, RT>> {
        let prover_enabled = self.rollup_prover_config.is_enabled();
        RollupBuilder::<RtAgnosticBlueprint<TestSpec, RT>>::new(
            GenesisSource::CustomParams(self.genesis_params),
            self.block_producing_config,
            self.finalization_blocks,
        )
        .set_config(|c| {
            c.rollup_prover_config = self.rollup_prover_config;
            c.automatic_batch_production = self.automatic_batch_production;
            c.storage = StoragePath::Tmp(self.dir);
            c.max_batch_size_bytes = self.max_batch_size_bytes;
            c.blob_processing_timeout_secs = self.blob_processing_timeout_secs;
            c.stop_at_rollup_height = self.stop_at_rollup_height;
            c.pruner = self.pruner;
            c.max_concurrent_batch_blobs = TEST_MAX_CONCURRENT_BATCH_BLOBS;
        })
        .with_preferred_seq_config(|seq| {
            seq.batch_execution_time_limit_millis = self.max_batch_execution_time_millis;
            if prover_enabled {
                seq.disable_state_root_consistency_checks = true;
            }
        })
        .set_da_config(|c| c.sender_address = self.seq_da_address)
        .set_persistent_da()
        .with_preferred_seq_min_profit_per_tx(self.minimum_profit_per_tx)
        .with_preferred_seq_recovery_strategy(sov_sequencer::preferred::RecoveryStrategy::TryToSave)
        .start()
        .await
        .unwrap()
    }
}

pub fn encode_call_with_fee<RT: Runtime<TestSpec>>(
    key: &Ed25519PrivateKey,
    generation: u64,
    call_message: &<RT as DispatchCall>::Decodable,
    max_fee: Amount,
) -> RawTx {
    encode_call_with_fee_and_uniqueness::<RT>(
        key,
        call_message,
        max_fee,
        UniquenessData::Generation(generation),
    )
}

pub fn encode_call_with_fee_and_uniqueness<RT: Runtime<TestSpec>>(
    key: &Ed25519PrivateKey,
    call_message: &<RT as DispatchCall>::Decodable,
    max_fee: Amount,
    uniqueness: UniquenessData,
) -> RawTx {
    let mut tx_details = default_test_tx_details();
    tx_details.max_fee = max_fee;
    let tx = test_signed_transaction::<RT, TestSpec>(
        key,
        call_message,
        uniqueness,
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

pub fn tx_set_value_nonce<RT: Runtime<TestSpec> + EncodeCall<ValueSetter<TestSpec>>>(
    key: &Ed25519PrivateKey,
    nonce: u64,
    value_to_set: u64,
    gas: Option<<TestSpec as Spec>::Gas>,
) -> RawTx {
    let tx_details = default_test_tx_details::<TestSpec>();
    let msg = <RT as EncodeCall<ValueSetter<TestSpec>>>::to_decodable(
        sov_value_setter::CallMessage::SetValue {
            value: value_to_set as u32,
            gas,
        },
    );

    encode_call_with_fee_and_uniqueness::<RT>(
        key,
        &msg,
        tx_details.max_fee,
        UniquenessData::Nonce(nonce),
    )
}

/// Checks the `ValueSetter` value over REST, optionally at a historical `rollup_height`.
/// `expected: None` asserts that the value is unset.
pub async fn query_set_value<B: FullNodeBlueprint<Native>>(
    test_rollup: &TestRollup<B>,
    rollup_height: Option<u64>,
    expected: Option<u64>,
) -> anyhow::Result<()> {
    query_set_value_helper(
        test_rollup,
        rollup_height.map(|n| format!("rollup_height={n}")),
        expected,
    )
    .await
}

pub async fn query_set_value_helper<B: FullNodeBlueprint<Native>>(
    test_rollup: &TestRollup<B>,
    query_param: Option<String>,
    expected: Option<u64>,
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

    tracing::debug!(?response, "Queried state value");
    let found_value = response["value"].as_u64();

    anyhow::ensure!(
        found_value == expected,
        "Expected value {expected:?}, found {found_value:?}"
    );

    Ok(())
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
