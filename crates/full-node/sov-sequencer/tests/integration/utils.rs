use std::sync::Arc;

use proptest::bits::u64;
use sov_chain_state::ChainState;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaService};
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::{RollupHeight, TransactionAuthenticator};
use sov_modules_api::digest::Digest;
use sov_modules_api::prelude::*;
use sov_modules_api::rest::HasRestApi;
use sov_modules_api::transaction::{Transaction, TxDetails};
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
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    default_test_signed_transaction, default_test_tx_details, test_signed_transaction, EncodeCall,
    MessageGenerator, RtAgnosticBlueprint, TestPrivateKey, TestSpec, TransactionType,
    TEST_DEFAULT_GAS_LIMIT, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE,
    TEST_MAX_CONCURRENT_BLOBS,
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
    nonce: u64,
    call_message: &<RT as DispatchCall>::Decodable,
) -> RawTx {
    let tx = default_test_signed_transaction::<RT, TestSpec>(
        &setup.admin_private_key,
        call_message,
        nonce,
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
    nonce: u64,
    value_to_set: u32,
) -> RawTx {
    let msg = <RT as EncodeCall<ValueSetter<TestSpec>>>::to_decodable(
        sov_value_setter::CallMessage::SetValue {
            value: value_to_set,
            gas: None,
        },
    );

    build_tx(setup, nonce, &msg)
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

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
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
    num_replicas: u64,
    max_batch_execution_time_millis: u64,
    stop_at_rollup_height: Option<RollupHeight>,
    finalization_blocks: u32,
) -> Option<Vec<TestRollup<RtAgnosticBlueprint<TestSpec, RT>>>> {
    // We skip all docker (i.e. postgres) tests on our dev server due to firewall false positives
    // bricking the machine.
    // The dev machine has 96 threads, which we detect to disable postgres. Currently no dev or CI
    // setup uses a machine of exactly this size, though if this ever changes this will cause
    // false positives.
    const DEV_SERVER_CPUS: usize = 96;

    let builder = RollupBuilder::<RtAgnosticBlueprint<TestSpec, RT>>::new(
        GenesisSource::CustomParams(genesis_params),
        block_producing_config,
        finalization_blocks,
    )
    .set_config(|c| {
        c.rollup_prover_config = rollup_prover_config;
        c.automatic_batch_production = automatic_batch_production;
        c.storage = dir;
        c.max_batch_size_bytes = max_batch_size_bytes;
        c.blob_processing_timeout_secs = blob_processing_timeout_secs;
        c.stop_at_rollup_height = stop_at_rollup_height;
        if let SequencerKindConfig::Preferred(preferred_sequencer_config) = &mut c.sequencer_config
        {
            preferred_sequencer_config.batch_execution_time_limit_millis =
                max_batch_execution_time_millis;
        }
        c.max_concurrent_blobs = TEST_MAX_CONCURRENT_BLOBS;
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .set_persistent_da()
    .with_preferred_seq_min_profit_per_tx(minimum_profit_per_tx)
    .with_preferred_seq_recovery_strategy(sov_sequencer::preferred::RecoveryStrategy::TryToSave);

    let builder_res = if num_cpus::get() != DEV_SERVER_CPUS {
        builder.with_postgres_sequencer().await
    } else {
        tracing::warn!("Running tests with postgres disabled in the sequencer! Detected machine with {DEV_SERVER_CPUS} threads, assuming we are running on the dev server.");
        if num_replicas > 1 {
            tracing::warn!(
                "Replica test cannot run, postgres is disabled due to detecting the dev server"
            );
            return None;
        } else {
            Ok(builder)
        }
    };

    match builder_res {
        Ok(builder) => match num_replicas {
            0 => panic!("At least one node needs to be started"),
            1 => Some(vec![builder.start().await.unwrap()]),
            2.. => Some(builder.start_with_replicas(num_replicas).await.unwrap()),
        },
        Err(e) => {
            if std::env::var("SOV_TEST_SKIP_DOCKER") == Ok("1".to_string()) {
                None
            } else {
                eprintln!("Error starting rollup builder: {e:?}");
                eprintln!("To skip docker based tests run with the env var SOV_TEST_SKIP_DOCKER=1");
                panic!("Unable to proceed without docker");
            }
        }
    }
}

pub fn encode_call_with_fee<RT: Runtime<TestSpec>>(
    key: &Ed25519PrivateKey,
    nonce: u64,
    call_message: &<RT as DispatchCall>::Decodable,
    max_fee: Amount,
) -> RawTx {
    let mut tx_details = default_test_tx_details();
    tx_details.max_fee = max_fee;
    let tx = test_signed_transaction::<RT, TestSpec>(
        key,
        call_message,
        nonce,
        &<RT as Runtime<TestSpec>>::CHAIN_HASH,
        tx_details,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

pub fn tx_set_value_with_gas<RT: Runtime<TestSpec> + EncodeCall<ValueSetter<TestSpec>>>(
    key: &Ed25519PrivateKey,
    nonce: u64,
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

    encode_call_with_fee::<RT>(key, nonce, &msg, max_fee)
}

// This allows for easily setting file sharing when using Docker Desktop.
pub fn tempdir_inside_codebase_dir() -> Arc<tempfile::TempDir> {
    Arc::new(tempfile::tempdir_in(std::env!("CARGO_TARGET_TMPDIR")).unwrap())
}
