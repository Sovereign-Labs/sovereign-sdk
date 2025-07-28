//! CLI utilities for benchmark generation.

use clap::{Args, Parser, Subcommand};
use demo_stf::genesis_config::EvmConfig;
use demo_stf::runtime::{GenesisConfig, Runtime};
use sov_modules_api::{Amount, CryptoSpec, Spec};
use sov_risc0_adapter::host::Risc0Host;
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::zk::ZkvmHost;
use sov_test_modules::access_pattern::AccessPatternGenesisConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::sov_paymaster::{
    self, PayeePolicy, PayerGenesisConfig, PaymasterConfig, PaymasterPolicyInitializer, SafeVec,
};
use sov_transaction_generator::generators::basic::BasicModuleRef;
use sov_transaction_generator::{Distribution, MessageValidity};

use super::{Benchmark, DEFAULT_RANDOMIZATION_BUFFER_SIZE};
use crate::{mock_da_risc0_host_args, BenchSpec};

type S = BenchSpec<Risc0>;
type RT = Runtime<S>;

const DEFAULT_GENERATION_PATH: &str = "./src/bench_files/generated/";

const DEFAULT_INITIAL_SEED: u128 = 0;

const NUM_SLOTS: u64 = 500;

/// Benchmark parameters to define a small benchmark
pub const SMALL_TXS_LARGE_SMALL_BATCH_PARAMS: BenchCLICustomArgs = BenchCLICustomArgs {
    min_txs_per_batch: 1,
    max_txs_per_batch: 1,
    min_batches_per_slot: 1,
    max_batches_per_slot: 1,
    randomization_buffer_size: DEFAULT_RANDOMIZATION_BUFFER_SIZE / 10,
    max_value_setter_vec_len: 2,
    pattern_maximum_write_data_length: 10,
    pattern_maximum_write_begin_index: 10,
    pattern_maximum_write_size: 10,
    pattern_maximum_hooks_ops: 10,
};

/// Benchmark parameters to define a small benchmark
pub const SMALL_TXS_LARGE_BATCH_PARAMS: BenchCLICustomArgs = BenchCLICustomArgs {
    min_txs_per_batch: 100,
    max_txs_per_batch: 750,
    min_batches_per_slot: 1,
    max_batches_per_slot: 1,
    randomization_buffer_size: DEFAULT_RANDOMIZATION_BUFFER_SIZE / 10,
    max_value_setter_vec_len: 10,
    pattern_maximum_write_data_length: 10,
    pattern_maximum_write_begin_index: 60,
    pattern_maximum_write_size: 20,
    pattern_maximum_hooks_ops: 10,
};

/// Benchmark parameters to define a standard benchmark
pub const STANDARD_PARAMS: BenchCLICustomArgs = BenchCLICustomArgs {
    min_txs_per_batch: 50,
    max_txs_per_batch: 300,
    min_batches_per_slot: 1,
    max_batches_per_slot: 1,
    randomization_buffer_size: DEFAULT_RANDOMIZATION_BUFFER_SIZE / 10,
    max_value_setter_vec_len: 300,
    pattern_maximum_write_data_length: 300,
    pattern_maximum_write_begin_index: 40,
    pattern_maximum_write_size: 40,
    pattern_maximum_hooks_ops: 10,
};

/// Benchmark parameters to define a large benchmark
pub const LARGE_TXS_SMALL_BATCH_PARAMS: BenchCLICustomArgs = BenchCLICustomArgs {
    min_txs_per_batch: 10,
    max_txs_per_batch: 70,
    min_batches_per_slot: 1,
    max_batches_per_slot: 1,
    randomization_buffer_size: DEFAULT_RANDOMIZATION_BUFFER_SIZE / 10,
    max_value_setter_vec_len: 900,
    pattern_maximum_write_data_length: 900,
    pattern_maximum_write_begin_index: 30,
    pattern_maximum_write_size: 10,
    pattern_maximum_hooks_ops: 10,
};

/// This program automatically generates benchmarks using the
/// `sov-transaction-generator`. Please note that:
///
/// - The files that are generated systematically come from the
///   `basic_benches`. In the future we may want to add more control over
///   the benchmark to generate.
/// - The path argument should either define a relative path from the root of
///   this crate (ie `sov-benchmarks`), or any absolute path.
#[derive(Parser, Debug)]
#[command(
    version,
    author,
    about = "This program automatically generates benchmarks using the `sov-transaction-generator`."
)]
pub struct BenchCLI {
    /// The base path to store the bench files. It must be a folder that currently exists.
    #[arg(short, long, default_value_t=DEFAULT_GENERATION_PATH.to_string())]
    pub path: String,

    /// Number of slots to execute as part of the benchmark.
    #[arg(short, long, default_value_t=NUM_SLOTS)]
    pub slots: u64,

    /// The seed value used for non-determinism.
    #[arg(long, default_value_t=DEFAULT_INITIAL_SEED)]
    pub seed: u128,

    /// Defines the benchmark size.
    #[command(subcommand)]
    pub size: BenchSize,
}

impl BenchCLI {
    /// Parses the size argument into a [`BenchCLICustomArgs`]
    pub fn parse_size(&self) -> &BenchCLICustomArgs {
        match &self.size {
            BenchSize::VerySmall => &SMALL_TXS_LARGE_SMALL_BATCH_PARAMS,
            BenchSize::Small => &SMALL_TXS_LARGE_BATCH_PARAMS,
            BenchSize::Standard => &STANDARD_PARAMS,
            BenchSize::Large => &LARGE_TXS_SMALL_BATCH_PARAMS,
            BenchSize::Custom(params) => params,
        }
    }
}

/// The different sizes available for benchmarking.
#[derive(Subcommand, Debug)]
pub enum BenchSize {
    /// Runs a benchmark of small transactions and small batches. Ie, with [`SMALL_TXS_LARGE_SMALL_BATCH_PARAMS`] values.
    VerySmall,
    /// Runs a benchmark of small transactions and large batches. Ie, with [`SMALL_TXS_LARGE_BATCH_PARAMS`] values.
    Small,
    /// Runs a benchmark of standard transactions and batches. Ie, with [`STANDARD_PARAMS`] values.
    Standard,
    /// Runs a benchmark of large transactions and small batches. Ie, with [`LARGE_TXS_SMALL_BATCH_PARAMS`] values.
    Large,
    /// Generates a benchmark with custom arguments.
    Custom(BenchCLICustomArgs),
}

/// Custom benchmark arguments.
/// The range parameters ([`BenchCLICustomArgs::min_txs_per_batch`], [`BenchCLICustomArgs::max_txs_per_batch`], [`BenchCLICustomArgs::min_batches_per_slot`], [`BenchCLICustomArgs::max_batches_per_slot`]) define closed ranges, ie of the form `min_val..=max_val`
#[derive(Args, Debug)]
pub struct BenchCLICustomArgs {
    /// Minimum number of transactions to include per batch, included.
    pub min_txs_per_batch: u64,

    /// Maximum number of transactions to include per batch, included.
    pub max_txs_per_batch: u64,

    /// Minimum number of batches to include per slot, included.
    pub min_batches_per_slot: u64,

    /// Maximum number of batches to include per slot, included.
    pub max_batches_per_slot: u64,

    /// The randomization buffer size.
    pub randomization_buffer_size: u64,

    /// The maximum value setter vector length.
    pub max_value_setter_vec_len: usize,

    /// The maximum length of the data written to the storage.
    pub pattern_maximum_write_data_length: usize,

    /// The maximum begin index of the writes to the storage.
    pub pattern_maximum_write_begin_index: u64,

    /// The maximum size of the writes to the storage.
    pub pattern_maximum_write_size: u64,

    /// Max number of hooks ops per storage pattern
    pub pattern_maximum_hooks_ops: u64,
}

impl BenchCLICustomArgs {
    /// Generates a new benchmark with custom params
    pub fn new_benchmark_with_params(
        &self,
        name: String,
        slots: u64,
        seed: u128,
        module_distribution: Distribution<BasicModuleRef<S, RT>>,
        validity_distribution: Distribution<MessageValidity>,
        passed_admin: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    ) -> Benchmark<S> {
        let risc0_host_args = mock_da_risc0_host_args();
        let risc0_commitment = Risc0Host::from_args(&*risc0_host_args).code_commitment();
        let mut genesis_config =
            HighLevelZkGenesisConfig::generate_with_additional_accounts_and_code_commitments(
                2,
                risc0_commitment,
                Default::default(),
            );

        let quarter_max = Amount::MAX.checked_div(Amount::new(4)).unwrap();
        genesis_config
            .initial_prover
            .user_info
            .available_gas_balance = quarter_max;
        genesis_config.initial_prover.bond = quarter_max;
        genesis_config.initial_sequencer.bond = quarter_max;
        genesis_config
            .initial_sequencer
            .user_info
            .available_gas_balance = quarter_max;

        let sequencer = genesis_config.initial_sequencer.clone();
        let payer = genesis_config
            .additional_accounts()
            .first()
            .unwrap()
            .clone();

        let mut admin = genesis_config.additional_accounts().get(1).unwrap().clone();
        admin.private_key = passed_admin;

        let genesis_config = GenesisConfig::from_minimal_config(
            genesis_config.into(),
            EvmConfig::default(),
            PaymasterConfig {
                payers: [PayerGenesisConfig {
                    payer_address: payer.address(),
                    policy: PaymasterPolicyInitializer {
                        default_payee_policy: PayeePolicy::Allow {
                            max_fee: None,
                            gas_limit: None,
                            max_gas_price: None,
                            transaction_limit: None,
                        },
                        payees: SafeVec::new(),
                        authorized_sequencers: sov_paymaster::AuthorizedSequencers::All,
                        authorized_updaters: [payer.address()].as_ref().try_into().unwrap(),
                    },
                    sequencers_to_register: [sequencer.da_address].as_ref().try_into().unwrap(),
                }]
                .as_ref()
                .try_into()
                .unwrap(),
            },
            AccessPatternGenesisConfig {
                admin: admin.address(),
            },
        );

        Benchmark {
            name,
            module_distribution,
            message_validity_distribution: validity_distribution,
            transactions_per_batch_range: self.min_txs_per_batch..=self.max_txs_per_batch,
            batches_per_slot_range: self.min_batches_per_slot..=self.max_batches_per_slot,
            number_of_slots: slots,
            initial_seed: seed,
            initial_randomization_buffer_size: self.randomization_buffer_size,
            genesis_config,
        }
    }
}
