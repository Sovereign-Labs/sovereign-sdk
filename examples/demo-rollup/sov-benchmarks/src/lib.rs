//! Benchmarking utilities

use core::time::Duration;
use std::sync::Arc;

use demo_stf::genesis_config::EvmConfig;
use demo_stf::runtime::{GenesisConfig, Runtime};
use sov_address::MultiAddressEvm;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{Amount, CryptoSpecExt, Spec, ZkVerifier, Zkvm};
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::zk::ZkvmHost;
use sov_sp1_adapter::SP1;
use sov_test_modules::access_pattern::AccessPatternGenesisConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::sov_paymaster::{
    self, PayeePolicy, PayerGenesisConfig, PaymasterPolicyInitializer, SafeVec,
};
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    MockDaSpec, MockZkvm, RtAgnosticBlueprint, TestPreferredSequencer, TestProver, TestUser,
};

use crate::sov_paymaster::PaymasterConfig;

pub const DEFAULT_BLOCK_TIME_MS: u64 = 150;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};
pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 0;
pub const DEFAULT_TXS_PER_BATCH: u64 = 10;
pub const MAX_TXS_PER_BATCH: u64 = 1000;
pub const DEFAULT_TIMEOUT: Duration = Duration::new(10, 0);

/// Node benchmarking utilities
pub mod node;

pub mod bench_runner;

/// Benchmark transaction generator. Stores the transactions generated in benchmark files.
pub mod bench_generator;

/// [`ConfigurableSpec`] with [`MockDaSpec`] and a custom inner vm
pub type BenchSpec<Vm> = ConfigurableSpec<MockDaSpec, Vm, MockZkvm, MultiAddressEvm, Native>;
/// [`ConfigurableSpec`] with [`MockDaSpec`] and a [`Risc0`] inner vm
pub type BenchRisc0Spec = BenchSpec<Risc0>;
/// [`ConfigurableSpec`] with [`MockDaSpec`] and a [`SP1`] inner vm
pub type BenchSP1Spec = BenchSpec<SP1>;

type RT<S> = Runtime<S>;

type Runner<S> = TestRunner<RT<S>, S>;

type RollupBlueprint<S> = RtAgnosticBlueprint<S, RT<S>>;

/// Benchmark user roles
pub struct Roles<S: Spec> {
    /// Admin of the value setter module
    pub value_setter_admin: TestUser<S>,
    /// Admin of the bank module
    pub bank_admin: TestUser<S>,
    /// Default Prover
    pub prover: TestProver<S>,
    /// Initial preferred sequencer.
    pub preferred_sequencer: TestPreferredSequencer<S>,
    /// Transaction senders
    pub senders: Vec<TestUser<S>>,
}

/// Setups benchmarks and returns the genesis config along with benchmark roles
pub fn setup<Vm: Zkvm>(
    num_senders: u64,
    inner_code_commitment: <Vm::Verifier as ZkVerifier>::CodeCommitment,
) -> (GenesisConfig<BenchSpec<Vm>>, Roles<BenchSpec<Vm>>)
where
    <Vm::Verifier as ZkVerifier>::CryptoSpec: CryptoSpecExt,
{
    let mut genesis_config =
        HighLevelZkGenesisConfig::generate_with_additional_accounts_and_code_commitments(
            (3 + num_senders) as usize,
            inner_code_commitment,
            Default::default(),
        );

    genesis_config.initial_sequencer.bond = genesis_config
        .initial_sequencer
        .bond
        .checked_mul(Amount::new(num_senders as u128 * 10))
        .unwrap();

    let sequencer = TestPreferredSequencer::new(genesis_config.initial_sequencer.clone());
    let prover = genesis_config.initial_prover.clone();

    let payer = genesis_config.additional_accounts()[0].clone();
    let admin_account = genesis_config.additional_accounts()[1].clone();
    let extra_account = genesis_config.additional_accounts()[2].clone();

    let senders = (0..num_senders)
        .map(|i| genesis_config.additional_accounts()[i as usize + 3].clone())
        .collect::<Vec<_>>();

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
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
                sequencers_to_register: [sequencer.sequencer_info.da_address]
                    .as_ref()
                    .try_into()
                    .unwrap(),
            }]
            .as_ref()
            .try_into()
            .unwrap(),
        },
        AccessPatternGenesisConfig {
            admin: admin_account.address(),
        },
    );

    (
        genesis,
        Roles {
            value_setter_admin: admin_account,
            bank_admin: extra_account,
            senders,
            preferred_sequencer: sequencer,
            prover,
        },
    )
}

/// Setups benchmarks and returns the a [`TestRunner`] along with benchmark roles
pub fn setup_with_runner<Vm: Zkvm>(
    num_senders: u64,
    inner_code_commitment: <Vm::Verifier as ZkVerifier>::CodeCommitment,
) -> (Runner<BenchSpec<Vm>>, Roles<BenchSpec<Vm>>)
where
    <Vm::Verifier as ZkVerifier>::CryptoSpec: CryptoSpecExt,
{
    let (genesis_config, roles) = setup(num_senders, inner_code_commitment);

    (
        TestRunner::<_, _>::new_with_genesis(
            genesis_config.into_genesis_params(),
            Default::default(),
        ),
        roles,
    )
}

/// Setups benchmarks and returns the a [`TestRollup`] along with benchmark roles
pub async fn setup_with_rollup<Vm: Zkvm>(
    num_senders: u64,
    host_args: Arc<<Vm::Host as ZkvmHost>::HostArgs>,
    inner_code_commitment: <Vm::Verifier as ZkVerifier>::CodeCommitment,
) -> (
    TestRollup<RollupBlueprint<BenchSpec<Vm>>>,
    Roles<BenchSpec<Vm>>,
)
where
    <Vm::Verifier as ZkVerifier>::CryptoSpec: CryptoSpecExt,
{
    let (genesis_config, roles) = setup(num_senders, inner_code_commitment);

    let rollup_builder = RollupBuilder::new(
        GenesisSource::CustomParams(genesis_config.into_genesis_params()),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        DEFAULT_FINALIZATION_BLOCKS,
    )
    .with_zkvm_host_args(host_args)
    .set_config(|config| {
        config.prover_address = roles.prover.user_info.address().to_string();
    })
    .set_da_config(|da_config| {
        da_config.sender_address = roles.preferred_sequencer.sequencer_info.da_address;
    });

    let rollup = rollup_builder
        .start()
        .await
        .expect("Impossible to start rollup");

    (rollup, roles)
}

/// Returns the risc0 host arguments for a rollup with mock da. This is the code that is zk-proven by the rollup
pub fn mock_da_risc0_host_args() -> Arc<&'static [u8]> {
    let should_skip_guest_build = {
        match std::env::var("SKIP_GUEST_BUILD")
            .as_ref()
            .map(|arg0: &String| String::as_str(arg0))
        {
            Ok("1") | Ok("true") | Ok("risc0") => true,
            Ok("0") | Ok("false") | Ok(_) | Err(_) => false,
        }
    };

    // Don't try to read the elf file if we're not building the risc0 guest!
    if should_skip_guest_build {
        return Arc::new(vec![].leak());
    }

    Arc::new(risc0::MOCK_DA_ELF)
}
