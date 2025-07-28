use sov_accounts::{AccountConfig, AccountData};
use sov_attester_incentives::AttesterIncentivesConfig;
use sov_modules_api::{Amount, CodeCommitmentFor, DaSpec, GasArray, Spec, ZkVerifier, Zkvm};
use sov_prover_incentives::ProverIncentivesConfig;
use sov_rollup_interface::common::SlotNumber;

use crate::interface::AsUser;
use crate::runtime::genesis::{generate_config_details, BasicGenesisConfig, HighLevelBasicConfig};
use crate::{
    TestSequencer, TestUser, TEST_DEFAULT_USER_BALANCE, TEST_DEFAULT_USER_STAKE,
    TEST_GAS_TOKEN_NAME,
};

/// A genesis config for a minimal operator runtime
pub struct MinimalOperatorGenesisConfig<S: Spec> {
    /// Basic genesis config.
    pub config: BasicGenesisConfig<S>,
}

/// A convenient high-level representation of an operator genesis config.
///
/// This type can be converted into a low-level [`MinimalOperatorGenesisConfig`] using
/// the [`From`] trait.
#[derive(Debug, Clone)]
pub struct HighLevelOperatorGenesisConfig<S: Spec> {
    /// The base fee goes to this address.
    pub reward_user: TestUser<S>,
    /// The initial sequencer.
    pub initial_sequencer: TestSequencer<S>,
    high_level_basic: HighLevelBasicConfig<S>,
}

impl<S: Spec> HighLevelOperatorGenesisConfig<S> {
    /// Creates a new high-level genesis config with the given reward address and sequencer using
    /// the default gas token name.
    pub fn with_defaults(
        reward_user: TestUser<S>,
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
            reward_user,
            initial_sequencer,
            high_level_basic,
        }
    }
}

#[allow(missing_docs)]
impl<S: Spec> HighLevelOperatorGenesisConfig<S> {
    pub fn additional_accounts(&self) -> &Vec<TestUser<S>> {
        &self.high_level_basic.additional_accounts
    }
}

impl<S: Spec> HighLevelOperatorGenesisConfig<S>
where
    <S::Da as DaSpec>::Address: From<[u8; 32]>,
    <<<S as Spec>::InnerZkvm as Zkvm>::Verifier as ZkVerifier>::CodeCommitment: Default,
    <<<S as Spec>::OuterZkvm as Zkvm>::Verifier as ZkVerifier>::CodeCommitment: Default,
{
    /// Generate new high-level genesis config.
    pub fn generate_with_additional_accounts(
        num_accounts: usize,
        reward_user: TestUser<S>,
    ) -> Self {
        let (_, _, sequencer, mut additional_accounts) = generate_config_details(num_accounts);

        additional_accounts.push(reward_user.clone());

        let inner_code_commitment = Default::default();
        let outer_code_commitment = Default::default();

        Self::with_defaults(
            reward_user,
            sequencer,
            additional_accounts,
            inner_code_commitment,
            outer_code_commitment,
        )
    }

    /// Generates a new high-level genesis config.
    pub fn generate(reward_user: TestUser<S>) -> Self {
        Self::generate_with_additional_accounts(0, reward_user)
    }
}

impl<S: Spec> From<HighLevelOperatorGenesisConfig<S>> for MinimalOperatorGenesisConfig<S> {
    /// Creates a new [`HighLevelOperatorGenesisConfig`] from the given arguments.
    fn from(high_level: HighLevelOperatorGenesisConfig<S>) -> Self {
        Self::from_args(
            high_level.reward_user,
            high_level.initial_sequencer,
            &high_level.high_level_basic.additional_accounts,
            high_level.high_level_basic.gas_token_name,
            high_level.high_level_basic.inner_code_commitment,
            high_level.high_level_basic.outer_code_commitment,
        )
    }
}

impl<S: Spec> MinimalOperatorGenesisConfig<S> {
    /// Creates a new [`MinimalOperatorGenesisConfig`] from the given arguments.
    pub fn from_args(
        reward_user: TestUser<S>,
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
                sequencer_registry: BasicGenesisConfig::sequencer_registry(&initial_sequencer),
                operator_incentives: BasicGenesisConfig::operator_incentives(
                    reward_user.address().clone(),
                ),

                // unused in operator mode
                attester_incentives: AttesterIncentivesConfig {
                    minimum_attester_bond: default_user_stake.clone(),
                    minimum_challenger_bond: default_user_stake.clone(),
                    initial_attesters: vec![(
                        placeholder.as_user().address().clone(),
                        Amount::ZERO,
                    )],
                    rollup_finality_period: SlotNumber::GENESIS,
                    maximum_attested_height: SlotNumber::GENESIS,
                    light_client_finalized_height: SlotNumber::GENESIS,
                },
                // unused in operator mode
                prover_incentives: ProverIncentivesConfig {
                    minimum_bond: default_user_stake.clone(),
                    proving_penalty: {
                        let mut user_stake = default_user_stake;
                        user_stake.scalar_division(2);
                        user_stake
                    },
                    initial_provers: vec![(placeholder.address().clone(), placeholder.balance())],
                },

                bank: BasicGenesisConfig::bank(
                    placeholder.clone().as_user(),
                    Amount::ZERO,
                    None,
                    &initial_sequencer,
                    additional_accounts,
                    gas_token_name,
                    placeholder,
                ),
                accounts: AccountConfig {
                    accounts: {
                        additional_accounts
                            .iter()
                            .filter_map(|user| {
                                user.custom_credential_id.map(|credential_id| AccountData {
                                    credential_id,
                                    address: user.address(),
                                })
                            })
                            .collect()
                    },
                },
                uniqueness: (),
                blob_storage: (),
                chain_state: BasicGenesisConfig::chain_state(
                    sov_modules_api::OperatingMode::Operator,
                    inner_code_commitment,
                    outer_code_commitment,
                ),
            },
        }
    }
}
