use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use sov_address::MultiAddressEvm;
use sov_bank::Bank;
use sov_bank::CallMessageDiscriminants::Transfer;
use sov_celestia_adapter::verifier::CelestiaSpec;
use sov_mock_da::{BlockProducingConfig, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::config_chain_id;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::arbitrary::{self, Unstructured};
use sov_modules_api::prelude::tracing;
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::{Amount, CryptoSpec, DispatchCall, EncodeCall, PrivateKey, Runtime, Spec};
use sov_paymaster::{
    PayeePolicy, PayerGenesisConfig, Paymaster, PaymasterConfig, PaymasterPolicyInitializer,
    SafeVec,
};
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_synthetic_load::CallMessageDiscriminants::{
    ReadAndSetHeavyState, ReadAndSetManyIndividualValues, RunCPUHeavyOperation,
};
use sov_synthetic_load::SyntheticLoad;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    generate_runtime, RtAgnosticBlueprint, TestProver, TestSequencer, TestSpec, TestUser,
    TransactionType, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE,
    TEST_DEFAULT_USER_BALANCE,
};
use sov_transaction_generator::generators::bank::harness_interface::BankHarness;
use sov_transaction_generator::generators::bank::BankMessageGenerator;
use sov_transaction_generator::generators::basic::{
    BasicCallMessageFactory, BasicChangeLogEntry, BasicModuleRef, BasicTag,
};
use sov_transaction_generator::generators::synthetic_load::{
    SyntheticLoadHarness, SyntheticLoadMessageGenerator,
};
use sov_transaction_generator::interface::rng_utils::{get_random_bytes, randomize_buffer};
use sov_transaction_generator::interface::MessageValidity;
use sov_transaction_generator::{Distribution, GeneratedMessage, Percent, State};
use tokio::sync::watch::Receiver;

pub const DEFAULT_BLOCK_TIME_MS: u64 = 200;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};

pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 5;

generate_runtime! {
    name: TestRuntime,
    modules: [paymaster: Paymaster<S>, synthetic_load: SyntheticLoad<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
    minimal_genesis_config_type: MinimalZkGenesisConfig<S>,
    gas_enforcer: paymaster: Paymaster<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, Self>,
    auth_call_wrapper: |call| call,
}

pub type TestRT = TestRuntime<TestSpec>;
pub type RollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRT>;
pub type TestRollupBuilder = RollupBuilder<RollupBlueprint, PathBuf>;

// Celestia
pub type CelestiaRollupSpec =
    ConfigurableSpec<CelestiaSpec, MockZkvm, MockZkvm, MultiAddressEvm, Native>;
pub type DemoCelestiaRT = demo_stf::runtime::Runtime<CelestiaRollupSpec>;

// Mock
pub type MockDemoRollupSpec =
    ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, MultiAddressEvm, Native>;
pub type DemoMockRT = demo_stf::runtime::Runtime<MockDemoRollupSpec>;

pub const BUFFER_SIZE: usize = 100_000;
// The minimum randomness needed to guarantee successful transaction generation
pub const SAFE_MIN_RANDOMNESS: usize = 1_000;

pub fn plain_tx_with_default_details<R: Runtime<S>, S: Spec>(
    gen_output: &GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>>,
) -> TransactionType<R, S> {
    TransactionType::Plain {
        message: gen_output.message.clone(),
        key: gen_output.sender.clone(),
        details: TxDetails {
            max_priority_fee_bips: TEST_DEFAULT_MAX_PRIORITY_FEE,
            max_fee: TEST_DEFAULT_MAX_FEE,
            gas_limit: None,
            chain_id: config_chain_id(),
        },
    }
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum TxType {
    /// Only [`SyntheticLoad`] transactions - includes many heavy txs
    SyntheticLoad,
    /// Only [`Bank`] transactions
    Bank,
    /// Mixed [`SyntheticLoad`] and [`Bank`] transactions
    Mixed,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ValidityProfile {
    /// Only valid transactions
    Clean,
    /// 5% of invalid transactions
    Buzzy,
    /// 50/50
    Half,
    /// 90% of invalid transactions
    Spammy,
}

impl ValidityProfile {
    pub fn get_validity(&self) -> Distribution<MessageValidity> {
        match self {
            ValidityProfile::Clean => Distribution::with_values(vec![(1, MessageValidity::Valid)]),
            ValidityProfile::Buzzy => Distribution::with_values(vec![
                (20, MessageValidity::Valid),
                (1, MessageValidity::Invalid),
            ]),
            ValidityProfile::Half => Distribution::with_values(vec![
                (50, MessageValidity::Valid),
                (50, MessageValidity::Invalid),
            ]),
            ValidityProfile::Spammy => Distribution::with_values(vec![
                (10, MessageValidity::Valid),
                (90, MessageValidity::Invalid),
            ]),
        }
    }
}
pub struct TestGenerator<R: Runtime<S>, S: Spec> {
    generator: BasicCallMessageFactory<S, R>,
    state: State<S, BasicTag>,
    randomness: Vec<u8>,
    remaining_randomness: usize,
    target_buffer_size: usize,
    salt: u128,
}

impl<R: Runtime<S>, S: Spec> TestGenerator<R, S> {
    pub fn generate(
        &mut self,
        modules_distribution: &Distribution<BasicModuleRef<S, R>>,
        validity: MessageValidity,
    ) -> GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>> {
        for _ in 0..20 {
            if self.has_enough_randomness() {
                let u =
                    &mut arbitrary::Unstructured::new(&self.randomness[self.randomness_offset()..]);

                if let Ok(output) = self.generator.generate_call_message(
                    modules_distribution,
                    u,
                    &mut self.state,
                    validity,
                ) {
                    self.remaining_randomness = u.len();
                    return output;
                } else {
                    self.target_buffer_size *= 2;
                }
            }
            self.re_randomize();
        }
        unreachable!("Could not get enough randomness to generate a transaction");
    }

    fn re_randomize(&mut self) {
        if self.randomness.len() < self.target_buffer_size {
            self.randomness = vec![0; self.target_buffer_size];
        }
        randomize_buffer(&mut self.randomness[..], self.salt);
        self.remaining_randomness = self.randomness.len();
        self.salt += 1;
    }

    fn randomness_offset(&self) -> usize {
        self.randomness.len() - self.remaining_randomness
    }

    fn has_enough_randomness(&self) -> bool {
        self.remaining_randomness > std::cmp::min(SAFE_MIN_RANDOMNESS, self.target_buffer_size / 10)
    }
}

// Setup generation with the given params
pub fn setup_harness<R: Runtime<S> + Clone, S: Spec>(rng_salt: u128) -> TestGenerator<R, S> {
    let factory = BasicCallMessageFactory::<S, R>::new();
    let state: State<S, BasicTag> = State::new();

    let random_bytes: Vec<u8> = get_random_bytes(100_000, rng_salt);
    let u = &mut arbitrary::Unstructured::new(&random_bytes[..]);
    let remaining_randomness = u.len();
    TestGenerator {
        randomness: random_bytes,
        remaining_randomness,
        generator: factory,
        state,
        target_buffer_size: BUFFER_SIZE,
        salt: rng_salt,
    }
}

pub struct Setup {
    /// A user who is pre-registered as a payer for [`Setup::sequencer`].
    #[allow(dead_code)]
    pub paymaster: TestUser<TestSpec>,
    /// The pre-registered sequencer
    pub sequencer: TestSequencer<TestSpec>,
    /// The pre-registered prover
    pub prover: TestProver<TestSpec>,
    #[allow(missing_docs)]
    pub genesis_config: GenesisConfig<TestSpec>,
}

pub fn setup_roles_and_config() -> Setup {
    let mut genesis_config = HighLevelZkGenesisConfig::generate();

    let sequencer = genesis_config.initial_sequencer.clone();
    let prover = genesis_config.initial_prover.clone();
    let paymaster = TestUser::generate(
        TEST_DEFAULT_USER_BALANCE
            .checked_mul(Amount::new(10))
            .unwrap(),
    );
    genesis_config
        .additional_accounts_mut()
        .push(paymaster.clone());

    let users: Vec<TestUser<TestSpec>> = vec![TestUser::generate_with_default_balance(); 20];

    genesis_config.additional_accounts_mut().extend(users);
    let genesis_config = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        PaymasterConfig {
            payers: [PayerGenesisConfig {
                payer_address: paymaster.address(),
                policy: PaymasterPolicyInitializer {
                    default_payee_policy: PayeePolicy::Allow {
                        max_fee: None,
                        gas_limit: None,
                        max_gas_price: None,
                        transaction_limit: None,
                    },
                    payees: SafeVec::new(),
                    authorized_sequencers: sov_paymaster::AuthorizedSequencers::All,
                    authorized_updaters: [paymaster.address()].as_ref().try_into().unwrap(),
                },
                sequencers_to_register: [sequencer.da_address].as_ref().try_into().unwrap(),
            }]
            .as_ref()
            .try_into()
            .unwrap(),
        },
        (),
    );
    Setup {
        paymaster,
        sequencer,
        prover,
        genesis_config,
    }
}

pub async fn setup_rollup(
    storage_path: PathBuf,
    axum_port: u16,
    setup: Setup,
    db_connection_url: Option<String>,
) -> TestRollup<RollupBlueprint, PathBuf> {
    let rollup_builder = TestRollupBuilder::new_with_storage_path(
        GenesisSource::CustomParams(setup.genesis_config.clone().into_genesis_params()),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        DEFAULT_FINALIZATION_BLOCKS,
        storage_path,
    )
    .set_config(|config| {
        config.telegraf_address = sov_metrics::MonitoringConfig::standard().telegraf_address;
        config.automatic_batch_production = true;
        config.rollup_prover_config = None;
        config.sequencer_config = SequencerKindConfig::Preferred(PreferredSequencerConfig {
            minimum_profit_per_tx: 0,
            postgres_connection_string: db_connection_url,
            ..Default::default()
        });
        config.prover_address = setup.prover.user_info.address().to_string();
        config.aggregated_proof_block_jump = 3;
        config.axum_port = axum_port;
        if let SequencerKindConfig::Preferred(preferred_sequencer_config) =
            &mut config.sequencer_config
        {
            preferred_sequencer_config.batch_execution_time_limit_millis = 400;
        }
    })
    .set_da_config(|da_config| {
        da_config.sender_address = setup.sequencer.da_address;
    });
    rollup_builder
        .start()
        .await
        .expect("Impossible to start rollup")
}

/// The passed client is responsible for handling timeouts (otherwise calls can block).
pub async fn run_generator_task_for_bank_and_synthetic_load<
    R: Runtime<S> + EncodeCall<Bank<S>> + EncodeCall<SyntheticLoad<S>> + Clone,
    S: Spec,
>(
    client: sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    validity: Distribution<MessageValidity>,
    tx_type: TxType,
) -> anyhow::Result<()> {
    let bank_harness = BankHarness::new(BankMessageGenerator::<S>::new(
        Distribution::with_equiprobable_values(vec![Transfer]),
        Percent::fifty(),
    ));
    let synthetic_load_admin = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    let synthetic_load_harness = SyntheticLoadHarness::new(SyntheticLoadMessageGenerator::new(
        Distribution::with_equiprobable_values(vec![
            ReadAndSetManyIndividualValues,
            ReadAndSetHeavyState,
            RunCPUHeavyOperation,
        ]),
        sov_transaction_generator::generators::synthetic_load::SyntheticLoadGeneratorOptions {
            maximum_vec_length: 10,
            min_and_max_number_of_individual_state_operations: (1, 10000),
            min_and_max_number_of_new_values_for_heavy_state: (100, 1000),
            min_and_max_number_of_iterations_for_cpu_heavy_operation: (1000, 5000),
            max_heavy_state_size: 1_000_000,
        },
        synthetic_load_admin,
    ));
    let modules: Vec<BasicModuleRef<S, R>> = match tx_type {
        TxType::SyntheticLoad => vec![Arc::new(synthetic_load_harness.clone())],
        TxType::Bank => vec![Arc::new(bank_harness.clone())],
        TxType::Mixed => vec![
            Arc::new(bank_harness.clone()),
            Arc::new(synthetic_load_harness.clone()),
        ],
    };

    prepare_and_send_txs(modules, client, rx, worker_id, num_workers, validity).await
}

/// The passed client is responsible for handling timeouts (otherwise calls can block).
pub async fn run_generator_task_for_bank<R: Runtime<S> + EncodeCall<Bank<S>> + Clone, S: Spec>(
    client: sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    validity: Distribution<MessageValidity>,
) -> anyhow::Result<()> {
    let bank_harness = BankHarness::new(BankMessageGenerator::<S>::new(
        Distribution::with_equiprobable_values(vec![Transfer]),
        Percent::fifty(),
    ));

    let modules: Vec<BasicModuleRef<S, R>> = vec![Arc::new(bank_harness.clone())];
    prepare_and_send_txs(modules, client, rx, worker_id, num_workers, validity).await
}

async fn prepare_and_send_txs<R: Runtime<S> + Clone, S: Spec>(
    modules: Vec<BasicModuleRef<S, R>>,
    client: sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    validity: Distribution<MessageValidity>,
) -> anyhow::Result<()> {
    let mut nonces: HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64> =
        Default::default();

    let modules = Distribution::with_equiprobable_values(modules);
    let random_bytes = get_random_bytes(100_000_000, worker_id);
    let u = &mut Unstructured::new(&random_bytes[..]);

    let mut generator: TestGenerator<R, S> = setup_harness::<R, _>(worker_id);
    let past_transaction_generations = config_value!("PAST_TRANSACTION_GENERATIONS") + 1;
    let worker_start = std::time::Instant::now();
    let mut total_txns = 0;

    while !*rx.borrow() {
        let txn_count = {
            // rng must fall out of scope before awaiting anything so this fn is Send
            let mut rng = rand::thread_rng();

            // Do this at the start so we add some jitter to initial API requests
            let sleep_ms = rng.gen_range(25..100);
            std::thread::sleep(Duration::from_millis(sleep_ms));

            rng.gen_range(10..100)
        };

        let mut txns = vec![];
        for _ in 0..txn_count {
            let validity = validity.select_value(u)?;
            let msg = generator.generate(&modules, *validity);
            let tx = plain_tx_with_default_details::<R, S>(&msg);
            let signed_tx = {
                let TransactionType::Plain {
                    message,
                    key,
                    details,
                } = tx
                else {
                    panic!("The method `plain_tx_with_default_details` should return a plain transaction!");
                };

                let pub_key = key.clone().pub_key();
                let nonce = nonces.get(&pub_key).unwrap_or(&0);

                // If message is invalid, create a future nonce and send it to the sequencer
                // The sequencer should reject the transaction as invalid, and more importantly, the sequencer should not update the nonce for the given account.
                if *validity == MessageValidity::Invalid {
                    let mut future_nonce = HashMap::from([(pub_key, nonce + 1000)]);
                    let invalid_tx = TransactionType::<R, S>::sign(
                        message.clone(),
                        key.clone(),
                        &R::CHAIN_HASH,
                        details.clone(),
                        &mut future_nonce,
                    );
                    txns.push((invalid_tx, true));
                    continue;
                }

                // Create an outdated but valid transaction (using the valid transaction that was just generated)
                if *nonce != 0 && *nonce % past_transaction_generations == 0 {
                    let mut outdated_nonce =
                        HashMap::from([(pub_key, nonce - past_transaction_generations)]);
                    let outdated_tx = TransactionType::<R, S>::sign(
                        message.clone(),
                        key.clone(),
                        &R::CHAIN_HASH,
                        details.clone(),
                        &mut outdated_nonce,
                    );
                    txns.push((outdated_tx, true));
                }

                TransactionType::<R, S>::sign(message, key, &R::CHAIN_HASH, details, &mut nonces)
            };
            txns.push((signed_tx, false));
        }

        let start = std::time::Instant::now();
        for (tx, is_invalid) in &txns {
            if *is_invalid {
                client
                    .send_tx_to_sequencer(tx)
                    .await
                    .expect_err("Outdated transaction should have failed");
            } else {
                client.send_tx_to_sequencer_with_retry(tx).await?;
            }
            total_txns += 1;
        }
        let elapsed = start.elapsed();
        tracing::debug!(id = %worker_id, "Sent {} transactions in {}ms. Current throughput: {:.2} txs per second. Running throughput: {:.2} txs per second", txns.len(), elapsed.as_millis(), (txns.len() * num_workers as usize) as f64 / elapsed.as_secs_f64(), (total_txns * num_workers as usize) as f64 / worker_start.elapsed().as_secs_f64());
    }

    Ok(())
}
