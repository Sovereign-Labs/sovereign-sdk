use sov_accounts::AccountConfig;
use sov_attester_incentives::AttesterIncentivesConfig;
use sov_mock_da::MockDaSpec;
use sov_modules_api::{Amount, CodeCommitmentFor, GasArray, Spec};
use sov_rollup_interface::common::SlotNumber;

use crate::interface::AsUser;
use crate::runtime::genesis::{generate_config_details, BasicGenesisConfig, HighLevelBasicConfig};
use crate::runtime::sov_sequencer_registry::SequencerConfig;
use crate::runtime::{OperatorIncentivesConfig, ProverIncentivesConfig, SequencerRegistryConfig};
use crate::{
    TestProver, TestSequencer, TestSpec, TestUser, TEST_DEFAULT_USER_BALANCE,
    TEST_DEFAULT_USER_STAKE, TEST_GAS_TOKEN_NAME, TEST_MIN_SEQ_BOND,
};
/// Minimal genesis configuration for the zk runtime.
pub struct MinimalZkGenesisConfig<S: Spec> {
    /// Basic genesis config.
    pub config: BasicGenesisConfig<S>,
}

/// A convenient high-level representation of a ZK genesis config.
#[derive(Debug, Clone)]
pub struct HighLevelZkGenesisConfig<S: Spec> {
    /// The initial prover.
    pub initial_prover: TestProver<S>,
    /// The initial sequencer.
    pub initial_sequencer: TestSequencer<S>,
    high_level_basic: HighLevelBasicConfig<S>,
}

#[allow(missing_docs)]
impl<S: Spec> HighLevelZkGenesisConfig<S> {
    pub fn additional_accounts(&self) -> &Vec<TestUser<S>> {
        &self.high_level_basic.additional_accounts
    }

    pub fn additional_accounts_mut(&mut self) -> &mut Vec<TestUser<S>> {
        &mut self.high_level_basic.additional_accounts
    }
}

impl<S: Spec> HighLevelZkGenesisConfig<S> {
    /// Creates a new high-level genesis config with the given initial prover and sequencer using
    /// the default gas token name.
    pub fn with_defaults(
        initial_prover: TestProver<S>,
        initial_sequencer: TestSequencer<S>,
        additional_accounts: Vec<TestUser<S>>,
        inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,
        outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
    ) -> Self {
        let high_level_basic = HighLevelBasicConfig {
            additional_accounts,
            gas_token_name: TEST_GAS_TOKEN_NAME.to_string(),
            inner_code_commitment,
            outer_code_commitment,
        };
        Self {
            initial_prover,
            initial_sequencer,
            high_level_basic,
        }
    }

    /// Generates a new high-level genesis config with random addresses and constant amounts (1_000_000_000 tokens)
    /// and `num_accounts` additional accounts.
    pub fn generate_with_additional_accounts_and_code_commitments(
        num_accounts: usize,
        inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,
        outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
    ) -> Self
    where
        S: Spec<Da = MockDaSpec>,
    {
        let (user_stake_value, prover_sequencer, sequencer, additional_accounts) =
            generate_config_details(num_accounts);

        let prover = TestProver {
            // By default we generate the prover as the same user as the sequencer
            // because provers must be registered sequencers.
            user_info: prover_sequencer,
            bond: user_stake_value,
        };

        Self::with_defaults(
            prover,
            sequencer,
            additional_accounts,
            inner_code_commitment,
            outer_code_commitment,
        )
    }

    /// Generates a new high-level genesis config with a given number of additional accounts with random addresses and given balance.
    ///
    pub fn add_accounts_with_balance(mut self, num_accounts: usize, balance: Amount) -> Self {
        self.high_level_basic
            .add_accounts_with_balance(num_accounts, balance);

        self
    }
}

impl HighLevelZkGenesisConfig<TestSpec> {
    /// Generates a new high-level genesis config with random addresses, constant amounts (1_000_000_000 tokens)
    /// and no additional accounts.
    pub fn generate() -> Self {
        Self::generate_with_additional_accounts(0)
    }

    /// Generates a new high-level genesis config with random addresses and constant amounts (1_000_000_000 tokens)
    /// and `num_accounts` additional accounts.
    pub fn generate_with_additional_accounts(num_accounts: usize) -> Self {
        Self::generate_with_additional_accounts_and_code_commitments(
            num_accounts,
            Default::default(),
            Default::default(),
        )
    }
}

impl<S: Spec> From<HighLevelZkGenesisConfig<S>> for MinimalZkGenesisConfig<S> {
    fn from(high_level: HighLevelZkGenesisConfig<S>) -> Self {
        Self::from_args(
            high_level.initial_prover,
            high_level.initial_sequencer,
            high_level.high_level_basic.additional_accounts.as_slice(),
            high_level.high_level_basic.gas_token_name,
            high_level.high_level_basic.inner_code_commitment,
            high_level.high_level_basic.outer_code_commitment,
        )
    }
}

impl<S: Spec> MinimalZkGenesisConfig<S> {
    /// Creates a new [`MinimalZkGenesisConfig`] from the given arguments.
    pub fn from_args(
        initial_prover: TestProver<S>,
        initial_sequencer: TestSequencer<S>,
        additional_accounts: &[TestUser<S>],
        gas_token_name: String,
        inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,
        outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
    ) -> Self {
        let placeholder = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
        let default_user_stake = S::Gas::from(TEST_DEFAULT_USER_STAKE);
        Self {
            config: BasicGenesisConfig {
                sequencer_registry: SequencerRegistryConfig {
                    minimum_bond: TEST_MIN_SEQ_BOND,
                    sequencer_config: SequencerConfig {
                        seq_rollup_address: initial_sequencer.as_user().address().clone(),
                        seq_da_address: initial_sequencer.da_address.clone(),
                        seq_bond: initial_sequencer.bond,
                        is_preferred_sequencer: true,
                    },
                },
                operator_incentives: OperatorIncentivesConfig {
                    reward_address: initial_prover.as_user().address().clone(),
                },
                // unused in zk mode
                attester_incentives: AttesterIncentivesConfig {
                    minimum_attester_bond: default_user_stake.clone(),
                    minimum_challenger_bond: default_user_stake.clone(),
                    initial_attesters: vec![(placeholder.address().clone(), placeholder.balance())],
                    rollup_finality_period: SlotNumber::GENESIS,
                    maximum_attested_height: SlotNumber::GENESIS,
                    light_client_finalized_height: SlotNumber::GENESIS,
                },
                prover_incentives: ProverIncentivesConfig {
                    minimum_bond: default_user_stake.clone(),
                    proving_penalty: {
                        let mut proving_penalty = default_user_stake.clone();
                        proving_penalty.scalar_division(2);
                        proving_penalty
                    },
                    initial_provers: vec![(
                        initial_prover.as_user().address().clone(),
                        initial_prover.bond,
                    )],
                },

                bank: BasicGenesisConfig::bank(
                    initial_prover.as_user(),
                    initial_prover.bond,
                    None,
                    &initial_sequencer,
                    additional_accounts,
                    gas_token_name,
                    placeholder,
                ),
                accounts: AccountConfig { accounts: vec![] },
                uniqueness: (),
                blob_storage: (),
                chain_state: BasicGenesisConfig::chain_state(
                    sov_modules_api::OperatingMode::Zk,
                    inner_code_commitment,
                    outer_code_commitment,
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use sov_bank::config_gas_token_id;

    use super::HighLevelZkGenesisConfig;
    use crate::runtime::TestRunner;
    use crate::{generate_zk_runtime, TestSpec};

    type S = TestSpec;

    generate_zk_runtime!(TestRuntime <= );

    #[test]
    fn test_default_genesis_zk_runtime_config() {
        let genesis_config = HighLevelZkGenesisConfig::generate();
        let sequencer = genesis_config.initial_sequencer.clone();
        let prover = genesis_config.initial_prover.clone();

        assert_eq!(
            sequencer.user_info.address(),
            prover.user_info.address(),
            "Sequencer and prover should be the same user"
        );

        let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
        let mut runner = TestRunner::<_, _>::new_with_genesis(
            genesis.into_genesis_params(),
            TestRuntime::default(),
        );

        runner.advance_slots(1).query_visible_state(|state| {
            let bank = crate::runtime::Bank::<S>::default();

            assert_eq!(
                bank.get_balance_of(&sequencer.user_info.address(), config_gas_token_id(), state)
                    .unwrap(),
                Some(sequencer.user_info.balance()),
            );

            let prover_incentives = crate::runtime::ProverIncentives::<S>::default();

            assert_eq!(
                prover_incentives
                    .bonded_provers
                    .get(&prover.user_info.address(), state)
                    .unwrap(),
                Some(prover.bond),
                "Should be bonded prover"
            );

            let sequencer_registry = crate::runtime::SequencerRegistry::<S>::default();

            assert_eq!(
                sequencer_registry.get_sender_balance_via_api(&sequencer.da_address, state),
                Some(sequencer.bond),
                "Should be bonded sequencer"
            );
        });
    }
}
