mod celestia;
mod mock;
mod sp1;

pub use celestia::*;
pub use mock::*;
pub use sp1::*;

use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::MockZkvmCryptoSpec;
use sov_modules_api::Amount;
use sov_paymaster::{
    PayeePolicy, PayerGenesisConfig, Paymaster, PaymasterConfig, PaymasterPolicyInitializer,
    SafeVec,
};
use sov_rollup_interface::zk::CryptoSpec;
use sov_sequencer::preferred::{ConfiguredNodeRole, PostgresConfig};
pub use sov_soak_testing_lib::*;
use sov_synthetic_load::SyntheticLoad;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;
use sov_test_utils::{
    generate_runtime, TestProver, TestSequencer, TestSpec, TestUser, TEST_DEFAULT_USER_BALANCE,
};

pub const DEFAULT_BLOCK_TIME_MS: u64 = 200;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};

pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 5;

pub type TestRT = TestRuntime<TestSpec>;

pub(crate) type SoakHasher = <MockZkvmCryptoSpec as CryptoSpec>::Hasher;

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

pub struct Setup {
    /// A user who is pre-registered as a payer for [`Setup::sequencer`].
    #[allow(dead_code)]
    pub paymaster: TestUser<TestSpec>,
    /// The pre-registered sequencer.
    pub sequencer: TestSequencer<TestSpec>,
    /// The pre-registered prover.
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

pub(crate) fn make_postgres_config(db_connection_url: Option<String>) -> Option<PostgresConfig> {
    db_connection_url.map(|url| PostgresConfig {
        postgres_connection_string: url,
        node_id: "Primary".to_string(),
        node_role: ConfiguredNodeRole::Leader,
        leader_election: Default::default(),
    })
}
