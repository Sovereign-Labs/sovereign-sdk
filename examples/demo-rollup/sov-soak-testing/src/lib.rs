use std::time::Duration;

use demo_stf::MultiAddressEvmSolana;
use std::sync::LazyLock;
use sov_celestia_adapter::verifier::CelestiaSpec;
use sov_db::storage_manager::NomtStorageManager;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockDaSpec, MockHash};
use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec, MockZkvmNetwork};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::default_spec::DefaultNomtSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::{Amount, CryptoSpec, Spec};
use sov_modules_rollup_blueprint::FullNodeBlueprint;
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
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{DefaultStorageSpec, Storage};
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


pub type TestRT = TestRuntime<TestSpec>;
pub type MockRollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRT>;
pub type TestRollupBuilder = RollupBuilder<MockRollupBlueprint>;

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


pub type SP1TestSpec = DefaultNomtSpec<MockDaSpec, SP1, MockZkvm, Native>;
pub type SP1TestRT = TestRuntime<SP1TestSpec>;

type SP1StorageManager = NomtStorageManager<
    MockDaSpec,
    <<SP1TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher,
    NomtProverStorage<
        DefaultStorageSpec<<<SP1TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
        MockHash,
    >,
>;


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


pub static SOAK_INNER_GUEST_SP1_ELF: LazyLock<&'static [u8]> = LazyLock::new(|| {
    let path = format!(
        "{}/inner-guest-sp1/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-soak-testing-inner-guest-sp1",
        env!("CARGO_MANIFEST_DIR")
    );
    let elf = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read SP1 inner guest ELF at '{path}': {e}. Build it first (unset SKIP_GUEST_BUILD)."));
    assert!(
        !elf.is_empty(),
        "SP1 inner guest ELF at '{path}' is empty. Build it first (unset SKIP_GUEST_BUILD)."
    );
    Vec::leak(elf)
});


/// Prover factory that submits proofs to the SP1 Succinct proving network.
pub struct NetworkProverFactory;

#[async_trait]
impl ProverFactory<SP1TestSpec> for NetworkProverFactory {
    type ProverService = NetworkProverService<
        <SP1TestSpec as Spec>::Address,
        <<SP1TestSpec as Spec>::Storage as Storage>::Root,
        <<SP1TestSpec as Spec>::Storage as Storage>::Witness,
        StorableMockDaService,
        SP1,
        MockZkvm,
    >;

    async fn create(
        _prover_config: RollupProverConfig<SP1>,
        rollup_config: &RollupConfig<<SP1TestSpec as Spec>::Address, StorableMockDaService>,
    ) -> Self::ProverService {
        let inner_vm = SP1Network::new(*SOAK_INNER_GUEST_SP1_ELF)
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

pub type NetworkProvingBlueprint =
    RtAgnosticBlueprint<SP1TestSpec, SP1TestRT, SP1StorageManager, NetworkProverFactory>;


pub struct Setup<S: Spec = TestSpec> {
    /// A user who is pre-registered as a payer for [`Setup::sequencer`].
    #[allow(dead_code)]
    pub paymaster: TestUser<S>,
    /// The pre-registered sequencer
    pub sequencer: TestSequencer<S>,
    /// The pre-registered prover
    pub prover: TestProver<S>,
    #[allow(missing_docs)]
    pub genesis_config: GenesisConfig<S>,
}

fn finalize_genesis_config<S: Spec<Da = MockDaSpec>>(
    mut genesis_config: HighLevelZkGenesisConfig<S>,
) -> Setup<S> {
    let sequencer = genesis_config.initial_sequencer.clone();
    let prover = genesis_config.initial_prover.clone();
    let paymaster = TestUser::<S>::generate(
        TEST_DEFAULT_USER_BALANCE
            .checked_mul(Amount::new(10))
            .unwrap(),
    );
    genesis_config
        .additional_accounts_mut()
        .push(paymaster.clone());

    let users: Vec<TestUser<S>> = vec![TestUser::generate_with_default_balance(); 20];
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

pub fn setup_roles_and_config() -> Setup<TestSpec> {
    finalize_genesis_config(HighLevelZkGenesisConfig::generate())
}

pub fn setup_roles_and_config_sp1() -> Setup<SP1TestSpec> {
    finalize_genesis_config(
        HighLevelZkGenesisConfig::<SP1TestSpec>::generate_with_additional_accounts_and_code_commitments(
            0,
            sov_sp1_adapter::code_commitment_from_elf(*SOAK_INNER_GUEST_SP1_ELF)
                .expect("Failed to compute SP1 code commitment from guest ELF"),
            Default::default(), // MockCodeCommitment for outer
        ),
    )
}


pub fn create_rollup_builder<R>(
    storage_path: PathBuf,
    axum_port: u16,
    setup: &Setup<R::Spec>,
    db_connection_url: Option<String>,
) -> RollupBuilder<R>
where
    R: FullNodeBlueprint<Native, DaService = StorableMockDaService> + Default + 'static,
    R::Spec: Spec<Da = MockDaSpec>,
    R::Runtime: sov_modules_stf_blueprint::Runtime<R::Spec, GenesisConfig = GenesisConfig<R::Spec>>,
{
    let postgres_config = db_connection_url.map(|url| PostgresConfig {
        postgres_connection_string: url,
        node_id: "Primary".to_string(),
        node_role: ConfiguredNodeRole::Leader,
        leader_election: Default::default(),
    });
    let da_address = setup.sequencer.da_address;

    RollupBuilder::<R>::new_with_storage_path(
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
