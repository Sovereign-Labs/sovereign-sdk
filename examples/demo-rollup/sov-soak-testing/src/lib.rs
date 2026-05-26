use std::path::{Path, PathBuf};

use anyhow::Context as _;
use demo_stf::MultiAddressEvmSolana;
use sov_celestia_adapter::verifier::CelestiaSpec;
use sov_mock_da::{BlockProducingConfig, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::prelude::serde_json;
use sov_modules_api::{Amount, CryptoSpec as ModuleCryptoSpec, Spec};
use sov_paymaster::{
    PayeePolicy, PayerGenesisConfig, Paymaster, PaymasterConfig, PaymasterPolicyInitializer,
    SafeVec,
};
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::preferred::{ConfiguredNodeRole, PostgresConfig, PreferredSequencerConfig};
use sov_sequencer::SequencerKindConfig;
pub use sov_soak_testing_lib::*;
use sov_synthetic_load::SyntheticLoad;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath, TestRollup};
use sov_test_utils::{
    generate_runtime, RtAgnosticBlueprint, TestProver, TestSequencer, TestSpec, TestUser,
    TEST_DEFAULT_USER_BALANCE,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

pub const DEFAULT_BLOCK_TIME_MS: u64 = 6000;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};

pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 5;
pub const VALUE_SETTER_ADMIN_PRIVATE_KEY_FILE: &str = "tx_signer_private_key.json";

pub type TestRT = TestRuntime<TestSpec>;
pub type RollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRT>;
pub type TestRollupBuilder = RollupBuilder<RollupBlueprint>;

// Celestia
pub type CelestiaRollupSpec =
    ConfigurableSpec<CelestiaSpec, MockZkvm, MockZkvm, MultiAddressEvmSolana, Native>;
pub type DemoCelestiaRT = demo_stf::runtime::Runtime<CelestiaRollupSpec>;

// Mock
pub type MockDemoRollupSpec =
    ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, MultiAddressEvmSolana, Native>;
pub type DemoMockRT = demo_stf::runtime::Runtime<MockDemoRollupSpec>;

generate_runtime! {
    name: TestRuntime,
    modules: [
        paymaster: Paymaster<S>,
        synthetic_load: SyntheticLoad<S>,
        value_setter: ValueSetter<S>
    ],
    operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
    minimal_genesis_config_type: MinimalZkGenesisConfig<S>,
    gas_enforcer: paymaster: Paymaster<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, Self>,
    auth_call_wrapper: |call| call,
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
    let value_setter_admin_private_key = read_value_setter_admin_private_key::<TestSpec>()
        .expect("failed to read value setter admin private key");
    let value_setter_admin = TestUser::new(
        value_setter_admin_private_key.clone(),
        TEST_DEFAULT_USER_BALANCE,
    );
    genesis_config
        .additional_accounts_mut()
        .push(paymaster.clone());
    genesis_config
        .additional_accounts_mut()
        .push(value_setter_admin.clone());

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
        ValueSetterConfig {
            admin: value_setter_admin.address(),
        },
    );
    Setup {
        paymaster,
        sequencer,
        prover,
        genesis_config,
    }
}

pub fn read_value_setter_admin_private_key<S>(
) -> anyhow::Result<<<S as Spec>::CryptoSpec as ModuleCryptoSpec>::PrivateKey>
where
    S: Spec,
    <<S as Spec>::CryptoSpec as ModuleCryptoSpec>::PrivateKey: serde::de::DeserializeOwned,
{
    let private_key_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-data/keys")
        .join(VALUE_SETTER_ADMIN_PRIVATE_KEY_FILE);
    let data = std::fs::read_to_string(&private_key_path)
        .with_context(|| format!("unable to read {}", private_key_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&data)
        .with_context(|| format!("unable to parse {}", private_key_path.display()))?;

    let private_key = value
        .get("private_key")
        .cloned()
        .context("private key file is missing `private_key`")?;
    serde_json::from_value(private_key)
        .context("unable to deserialize value setter admin private key")
}

pub async fn setup_rollup(
    storage_path: PathBuf,
    axum_port: u16,
    setup: Setup,
    db_connection_url: Option<String>,
) -> TestRollup<RollupBlueprint> {
    let postgres_config = db_connection_url.map(|url| PostgresConfig {
        postgres_connection_string: url,
        node_id: "Primary".to_string(),
        node_role: ConfiguredNodeRole::Leader,
        leader_election: Default::default(),
    });

    let rollup_builder = TestRollupBuilder::new_with_storage_path(
        GenesisSource::CustomParams(setup.genesis_config.clone().into_genesis_params()),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        DEFAULT_FINALIZATION_BLOCKS,
        StoragePath::Buf(storage_path),
        false,
    )
    .set_config(|config| {
        config.telegraf_address = sov_metrics::MonitoringConfig::standard().telegraf_address;
        config.automatic_batch_production = true;
        config.rollup_prover_config = None;
        config.sequencer_config = SequencerKindConfig::Preferred(PreferredSequencerConfig {
            minimum_profit_per_tx: 0,
            postgres_config,
            ..Default::default()
        });
        config.prover_address = setup.prover.user_info.address().to_string();
        config.aggregated_proof_block_jump = 3;
        config.axum_port = axum_port;
        if let SequencerKindConfig::Preferred(preferred_sequencer_config) =
            &mut config.sequencer_config
        {
            preferred_sequencer_config.batch_execution_time_limit_millis = 3000;
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
