use std::path::Path;
use std::time::Duration;

use demo_stf::genesis_config::GenesisPaths;
use demo_stf::MultiAddressEvmSolana;
use sov_celestia_adapter::verifier::CelestiaSpec;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockDaSpec};
use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec, MockZkvmNetwork};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::{Amount, Spec};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::{
    PayeePolicy, PayerGenesisConfig, Paymaster, PaymasterConfig, PaymasterPolicyInitializer,
    SafeVec,
};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::execution_mode::Native;
use sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash;
use sov_rollup_interface::zk::CryptoSpec;
use sov_sequencer::preferred::{ConfiguredNodeRole, PostgresConfig, PreferredSequencerConfig};
use sov_sequencer::SequencerKindConfig;
use sov_sp1_adapter::network::SP1Network;
use sov_sp1_adapter::SP1;
pub use sov_soak_testing_lib::*;
use sov_state::Storage;
use sov_stf_runner::processes::NetworkProverService;
pub use sov_stf_runner::processes::RollupProverConfig;
use sov_stf_runner::RollupConfig;
use sov_synthetic_load::SyntheticLoad;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath};
use sov_test_utils::{
    generate_runtime, ProverFactory, RtAgnosticBlueprint, TestProver, TestSequencer, TestSpec,
    TestUser, TEST_DEFAULT_USER_BALANCE,
};
use std::path::PathBuf;

pub const DEFAULT_BLOCK_TIME_MS: u64 = 200;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};

pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 5;

// Mock prover types (existing)
pub type TestRT = TestRuntime<TestSpec>;
pub type MockRollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRT>;

type SoakHasher = <MockZkvmCryptoSpec as CryptoSpec>::Hasher;

// Celestia
type CelestiaNativeStorage =
    NomtProverStorage<DefaultStorageSpec<SoakHasher>, <CelestiaSpec as DaSpec>::SlotHash>;
pub type CelestiaRollupSpec = ConfigurableSpec<
    CelestiaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
    MockZkvmCryptoSpec,
    CelestiaNativeStorage,
>;
pub type DemoCelestiaRT = demo_stf::runtime::Runtime<CelestiaRollupSpec>;

// Mock
type MockNativeStorage =
    NomtProverStorage<DefaultStorageSpec<SoakHasher>, <MockDaSpec as DaSpec>::SlotHash>;
pub type MockDemoRollupSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
    MockZkvmCryptoSpec,
    MockNativeStorage,
>;
pub type DemoMockRT = demo_stf::runtime::Runtime<MockDemoRollupSpec>;

// SP1 network proving types — uses demo-stf Runtime with the existing guest-mock ELF
pub type SP1Spec =
    ConfigurableSpec<MockDaSpec, SP1, MockZkvm, MultiAddressEvmSolana, Native>;
pub type SP1RT = demo_stf::runtime::Runtime<SP1Spec>;

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

/// Prover factory that submits proofs to the SP1 Succinct proving network.
pub struct NetworkProverFactory;

#[async_trait]
impl ProverFactory<SP1Spec> for NetworkProverFactory {
    type ProverService = NetworkProverService<
        <SP1Spec as Spec>::Address,
        <<SP1Spec as Spec>::Storage as Storage>::Root,
        <<SP1Spec as Spec>::Storage as Storage>::Witness,
        StorableMockDaService,
        SP1,
        MockZkvm,
    >;

    async fn create(
        _prover_config: RollupProverConfig<SP1>,
        rollup_config: &RollupConfig<<SP1Spec as Spec>::Address, StorableMockDaService>,
    ) -> Self::ProverService {
        let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
        assert!(
            !elf.is_empty(),
            "SP1 guest ELF is empty — build it first (cd provers/sp1 && cargo build)"
        );

        let inner_vm = SP1Network::new(elf)
            .await
            .expect("Failed to create SP1Network — is NETWORK_PRIVATE_KEY set?");
        // auto-complete outer proofs — real outer not supported yet
        let outer_vm = MockZkvmNetwork::new(true);

        NetworkProverService::new(
            inner_vm,
            outer_vm,
            Default::default(),
            CodeCommitmentHash::default(),
            rollup_config.proof_manager.prover_address,
            Duration::from_secs(600),
        )
    }
}

pub type NetworkProvingBlueprint = RtAgnosticBlueprint<
    SP1Spec,
    SP1RT,
    sov_db::storage_manager::NativeStorageManager<
        MockDaSpec,
        <SP1Spec as Spec>::Storage,
    >,
    NetworkProverFactory,
>;

// Mock path setup (TestRuntime)

pub struct Setup {
    #[allow(dead_code)]
    pub paymaster: TestUser<TestSpec>,
    pub sequencer: TestSequencer<TestSpec>,
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

pub fn create_mock_rollup_builder(
    storage_path: PathBuf,
    axum_port: u16,
    setup: &Setup,
    db_connection_url: Option<String>,
) -> RollupBuilder<MockRollupBlueprint> {
    let postgres_config = make_postgres_config(db_connection_url);
    let da_address = setup.sequencer.da_address;

    RollupBuilder::<MockRollupBlueprint>::new_with_storage_path(
        GenesisSource::CustomParams(setup.genesis_config.clone().into_genesis_params()),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        DEFAULT_FINALIZATION_BLOCKS,
        StoragePath::Buf(storage_path),
        false,
    )
    .set_config(|config| {
        config.telegraf_address = sov_metrics::MonitoringConfig::standard().telegraf_address;
        config.automatic_batch_production = true;
        config.sequencer_config = SequencerKindConfig::Preferred(PreferredSequencerConfig {
            minimum_profit_per_tx: 0,
            postgres_config,
            batch_execution_time_limit_millis: 400,
            ..Default::default()
        });
        config.prover_address = setup.prover.user_info.address().to_string();
        config.aggregated_proof_block_jump = 3;
        config.axum_port = axum_port;
    })
    .set_da_config(|da_config| {
        da_config.sender_address = da_address;
    })
}

// SP1 network proving path setup (demo-stf Runtime)

fn sp1_genesis_paths() -> GenesisPaths {
    let dir: &dyn AsRef<Path> = &"../test-data/genesis/integration-tests/";
    let mut paths = GenesisPaths::from_dir(dir.as_ref());
    paths.chain_state_genesis_path = dir.as_ref().join("chain_state_zk.json");
    paths
}

pub fn create_sp1_rollup_builder(
    storage_path: PathBuf,
    axum_port: u16,
    db_connection_url: Option<String>,
) -> RollupBuilder<NetworkProvingBlueprint> {
    let genesis_config =
        demo_stf::genesis_config::create_genesis_config::<SP1Spec>(&sp1_genesis_paths())
            .expect("Failed to create demo-stf genesis config");
    let postgres_config = make_postgres_config(db_connection_url);

    RollupBuilder::<NetworkProvingBlueprint>::new_with_storage_path(
        GenesisSource::CustomParams(GenesisParams {
            runtime: genesis_config,
        }),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        DEFAULT_FINALIZATION_BLOCKS,
        StoragePath::Buf(storage_path),
        false,
    )
    .set_config(|config| {
        config.telegraf_address = sov_metrics::MonitoringConfig::standard().telegraf_address;
        config.automatic_batch_production = true;
        config.sequencer_config = SequencerKindConfig::Preferred(PreferredSequencerConfig {
            minimum_profit_per_tx: 0,
            postgres_config,
            batch_execution_time_limit_millis: 400,
            ..Default::default()
        });
        config.aggregated_proof_block_jump = 3;
        config.axum_port = axum_port;
    })
}

fn make_postgres_config(db_connection_url: Option<String>) -> Option<PostgresConfig> {
    db_connection_url.map(|url| PostgresConfig {
        postgres_connection_string: url,
        node_id: "Primary".to_string(),
        node_role: ConfiguredNodeRole::Leader,
        leader_election: Default::default(),
    })
}
