use std::collections::HashSet;

use sov_accounts::{AccountConfig, AccountData};
use sov_attester_incentives::AttesterIncentivesConfig;
use sov_modules_api::{Amount, CodeCommitmentFor, DaSpec, GasArray, Spec, ZkVerifier, Zkvm};
use sov_prover_incentives::ProverIncentivesConfig;
use sov_rollup_interface::common::SlotNumber;

use crate::interface::AsUser;
use crate::runtime::genesis::{
    generate_config_details, sequencer_da_addr_inner, BasicGenesisConfig, HighLevelBasicConfig,
    TestTokenName,
};
use crate::{
    TestAttester, TestChallenger, TestSequencer, TestUser, TEST_DEFAULT_USER_BALANCE,
    TEST_DEFAULT_USER_STAKE, TEST_GAS_TOKEN_NAME, TEST_LIGHT_CLIENT_FINALIZED_HEIGHT,
    TEST_MAX_ATTESTED_HEIGHT, TEST_ROLLUP_FINALITY_PERIOD,
};

/// A genesis config for a minimal optimistic runtime
pub struct MinimalOptimisticGenesisConfig<S: Spec> {
    /// Basic genesis config.
    pub config: BasicGenesisConfig<S>,
}

/// A convenient high-level representation of an optimistic genesis config. This config
/// is expressed in terms of abstract entities like Attesters and Sequencers, rather than
/// the low level details of accounts with balances held by several different modules.
///
/// This type can be converted into a low-level [`MinimalOptimisticGenesisConfig`] using
/// the [`From`] trait.
#[derive(Debug, Clone)]
pub struct HighLevelOptimisticGenesisConfig<S: Spec> {
    /// The initial attester.
    pub initial_attester: TestAttester<S>,
    /// The initial challenger.
    pub initial_challenger: TestChallenger<S>,
    /// The initial sequencer.
    pub initial_sequencer: TestSequencer<S>,
    high_level_basic: HighLevelBasicConfig<S>,
}

#[allow(missing_docs)]
impl<S: Spec> HighLevelOptimisticGenesisConfig<S> {
    pub fn additional_accounts(&self) -> &Vec<TestUser<S>> {
        &self.high_level_basic.additional_accounts
    }

    pub fn additional_accounts_mut(&mut self) -> &mut Vec<TestUser<S>> {
        &mut self.high_level_basic.additional_accounts
    }

    pub fn gas_token_name(&self) -> &str {
        &self.high_level_basic.gas_token_name
    }
}

impl<S: Spec> HighLevelOptimisticGenesisConfig<S> {
    /// Creates a new high-level genesis config with the given initial attester and sequencer using
    /// the default gas token name.
    pub fn with_defaults(
        initial_attester: TestAttester<S>,
        initial_challenger: TestChallenger<S>,
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
            initial_attester,
            initial_challenger,
            initial_sequencer,
            high_level_basic,
        }
    }

    /// Returns the list of accounts that have a balance for the given token. The account vector is cloned.
    pub fn get_accounts_for_token(&self, token_name: &TestTokenName) -> Vec<TestUser<S>> {
        self.additional_accounts()
            .clone()
            .into_iter()
            .filter(|user| user.token_balance(token_name).is_some())
            .collect()
    }

    /// Returns the list of token names that are used in the genesis config.
    /// Clones the underlying list of token names.
    pub fn token_names(&self) -> Vec<TestTokenName> {
        let mut token_names = HashSet::<&TestTokenName>::new();

        self.additional_accounts().iter().for_each(|user| {
            user.token_balances.iter().for_each(|token_info| {
                token_names.insert(&token_info.token_name);
            });
        });

        token_names.into_iter().cloned().collect()
    }
}

impl<S: Spec> HighLevelOptimisticGenesisConfig<S>
where
    <S::Da as DaSpec>::Address: From<[u8; 32]>,
    <<<S as Spec>::InnerZkvm as Zkvm>::Verifier as ZkVerifier>::CodeCommitment: Default,
    <<<S as Spec>::OuterZkvm as Zkvm>::Verifier as ZkVerifier>::CodeCommitment: Default,
{
    /// The sequencer address used by [`HighLevelOptimisticGenesisConfig::generate`].
    pub fn sequencer_da_addr() -> <S::Da as DaSpec>::Address {
        sequencer_da_addr_inner::<S>()
    }

    fn generate_with_additional_accounts(num_accounts: usize) -> Self {
        let (user_stake_value, prover_sequencer, sequencer, additional_accounts) =
            generate_config_details(num_accounts);

        let attester = TestAttester {
            user_info: prover_sequencer.clone(),
            bond: user_stake_value,
            slot_to_attest: 1,
        };

        let challenger = TestChallenger {
            user_info: prover_sequencer,
        };

        let inner_code_commitment = Default::default();
        let outer_code_commitment = Default::default();

        Self::with_defaults(
            attester,
            challenger,
            sequencer,
            additional_accounts,
            inner_code_commitment,
            outer_code_commitment,
        )
    }

    /// Generates a new high-level genesis config with random addresses, constant amounts (1_000_000_000 tokens)
    /// and no additional accounts.
    pub fn generate() -> Self {
        Self::generate_with_additional_accounts(0)
    }

    /// Generates a new high-level genesis config with random addresses and constant amounts (1_000_000_000 tokens)
    /// and `num_accounts` additional accounts.
    ///
    /// This is a convenience function for [`Self::add_accounts`]
    pub fn add_accounts_with_default_balance(self, num_accounts: usize) -> Self {
        self.add_accounts_with_balance(num_accounts, TEST_DEFAULT_USER_BALANCE)
    }

    /// Generates a new high-level genesis config with random addresses and constant amounts (1_000_000_000 tokens)
    /// and `num_accounts` additional accounts.
    ///
    /// This is a convenience function for [`Self::add_accounts`]
    pub fn add_accounts_with_balance(mut self, num_accounts: usize, balance: Amount) -> Self {
        self.high_level_basic
            .add_accounts_with_balance(num_accounts, balance);

        self
    }

    /// Adds a token to the genesis config. Generates a token with an (optional) given name and adds it to the list of tokens.
    /// The token is associated with the given number of accounts, each of which has an initial balance. It is also
    /// possible to specify a minter for the token using the `with_minter` parameter.
    ///
    /// This is a convenience function for [`Self::add_accounts`]
    pub fn add_accounts_with_token(
        mut self,
        token_name: &TestTokenName,
        with_minter: bool,
        num_accounts: usize,
        account_initial_balance: Amount,
    ) -> Self {
        self.high_level_basic.add_accounts_with_token_inner(
            token_name,
            with_minter,
            num_accounts,
            account_initial_balance,
        );

        self
    }

    /// Adds additional accounts to the genesis config.
    pub fn add_accounts(mut self, additional_accounts: Vec<TestUser<S>>) -> Self {
        self.high_level_basic
            .add_accounts_inner(additional_accounts);

        self
    }
}

impl<S: Spec> From<HighLevelOptimisticGenesisConfig<S>> for MinimalOptimisticGenesisConfig<S> {
    fn from(high_level: HighLevelOptimisticGenesisConfig<S>) -> Self {
        Self::from_args(
            high_level.initial_attester,
            high_level.initial_challenger,
            high_level.initial_sequencer,
            &high_level.high_level_basic.additional_accounts,
            high_level.high_level_basic.gas_token_name,
            high_level.high_level_basic.inner_code_commitment,
            high_level.high_level_basic.outer_code_commitment,
        )
    }
}

impl<S: Spec> MinimalOptimisticGenesisConfig<S> {
    /// Creates a new [`MinimalOptimisticGenesisConfig`] from the given arguments.
    pub fn from_args(
        initial_attester: TestAttester<S>,
        initial_challenger: TestChallenger<S>,
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
                    initial_attester.as_user().address().clone(),
                ),

                attester_incentives: AttesterIncentivesConfig {
                    minimum_attester_bond: default_user_stake.clone(),
                    minimum_challenger_bond: default_user_stake.clone(),
                    initial_attesters: vec![(
                        initial_attester.as_user().address().clone(),
                        initial_attester.bond,
                    )],
                    rollup_finality_period: SlotNumber::new(TEST_ROLLUP_FINALITY_PERIOD),
                    maximum_attested_height: TEST_MAX_ATTESTED_HEIGHT,
                    light_client_finalized_height: TEST_LIGHT_CLIENT_FINALIZED_HEIGHT,
                },
                // unused in optimistic mode
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
                    initial_attester.as_user(),
                    initial_attester.bond,
                    Some(initial_challenger.as_user()),
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
                    sov_modules_api::OperatingMode::Optimistic,
                    inner_code_commitment,
                    outer_code_commitment,
                ),
            },
        }
    }
}
