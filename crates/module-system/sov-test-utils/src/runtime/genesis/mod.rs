use sha2::Digest;
use sov_accounts::Accounts;
use sov_attester_incentives::AttesterIncentives;
use sov_bank::{Bank, BankConfig, TokenConfig, DEFAULT_TOKEN_DECIMALS};
use sov_blob_storage::BlobStorage;
use sov_chain_state::{ChainState, ChainStateConfig};
use sov_modules_api::{
    Amount, CodeCommitmentFor, CryptoSpec, DaSpec, Gas, GasSpec, Genesis, OperatingMode, Spec,
};
use sov_operator_incentives::{OperatorIncentives, OperatorIncentivesConfig};
use sov_prover_incentives::ProverIncentives;
use sov_sequencer_registry::{SequencerConfig, SequencerRegistry, SequencerRegistryConfig};
use sov_uniqueness::Uniqueness;

use crate::interface::AsUser;
use crate::runtime::TokenId;
use crate::{
    TestSequencer, TestSpec, TestUser, UserTokenInfo, TEST_DEFAULT_USER_BALANCE,
    TEST_DEFAULT_USER_STAKE, TEST_MIN_SEQ_BOND,
};

/// Utilities for testing a runtime in the optimistic execution context.
pub mod optimistic;

/// Utilities for testing a runtime in the ZK execution context.
pub mod zk;

/// Utilities for testing a runtime in the operator context.
pub mod operator;

/// A wrapper around a string that can be used to easily identify a test token.
#[derive(Debug, Eq, Hash, Clone, PartialEq, derive_more::Display)]
#[display("TestToken({})", self.0)]
pub struct TestTokenName(
    /// The name of the token. Can be any human-readable string.
    pub String,
);

impl TestTokenName {
    /// Creates a new token name from a string.
    pub fn new(name: String) -> Self {
        Self(name)
    }

    /// Returns the ID of the token.
    pub fn id(&self) -> TokenId {
        let mut bytes: [u8; 32] =
            <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher::digest(self.to_string())
                .as_slice()
                .try_into()
                .unwrap();
        bytes[31] = DEFAULT_TOKEN_DECIMALS;
        TokenId::from(bytes)
    }
}

#[test]
fn test_display_token_name() {
    let token_name = TestTokenName::new("test".to_string());
    assert_eq!("TestToken(test)", token_name.to_string());
}

/// Common config for all the rollup types.
pub struct BasicGenesisConfig<S: Spec> {
    /// The sequencer registry config.
    pub sequencer_registry: <SequencerRegistry<S> as Genesis>::Config,
    /// The operator incentives config.
    pub operator_incentives: <OperatorIncentives<S> as Genesis>::Config,
    /// The attester incentives config.
    pub attester_incentives: <AttesterIncentives<S> as Genesis>::Config,
    /// The prover incentives config.
    pub prover_incentives: <ProverIncentives<S> as Genesis>::Config,
    /// The bank config.
    pub bank: <Bank<S> as Genesis>::Config,
    /// The accounts config.
    pub accounts: <Accounts<S> as Genesis>::Config,
    /// The uniqueness config.
    pub uniqueness: <Uniqueness<S> as Genesis>::Config,
    /// The chain state config.
    pub chain_state: <ChainState<S> as Genesis>::Config,
    /// The blob storage config.
    pub blob_storage: <BlobStorage<S> as Genesis>::Config,
}

impl<S: Spec> BasicGenesisConfig<S> {
    fn sequencer_registry(initial_sequencer: &TestSequencer<S>) -> SequencerRegistryConfig<S> {
        SequencerRegistryConfig {
            minimum_bond: TEST_MIN_SEQ_BOND,
            sequencer_config: SequencerConfig {
                seq_rollup_address: initial_sequencer.as_user().address().clone(),
                seq_da_address: initial_sequencer.da_address.clone(),
                seq_bond: initial_sequencer.bond,
                is_preferred_sequencer: true,
            },
        }
    }

    fn operator_incentives(reward_address: S::Address) -> OperatorIncentivesConfig<S> {
        OperatorIncentivesConfig { reward_address }
    }

    fn bank(
        prover: &TestUser<S>,
        bond: Amount,
        initial_challenger: Option<&TestUser<S>>,
        initial_sequencer: &TestSequencer<S>,
        additional_accounts: &[TestUser<S>],
        gas_token_name: String,
        placeholder: TestUser<S>,
    ) -> BankConfig<S> {
        BankConfig {
            gas_token_config: sov_bank::GasTokenConfig {
                token_name: gas_token_name,
                token_decimals: None,
                supply_cap: None,
                address_and_balances: {
                    let mut additional_accounts_vec: Vec<_> = additional_accounts
                        .iter()
                        .map(|user| (user.address(), user.balance()))
                        .collect();

                    additional_accounts_vec.push((placeholder.address(), placeholder.balance()));

                    let sequencer = initial_sequencer.as_user();

                    if sequencer.address() == prover.address() {
                        assert_eq!(sequencer.available_gas_balance, prover.available_gas_balance, "Sequencer and prover balances should be equal if they are the same user");

                        additional_accounts_vec.append(&mut vec![(
                            sequencer.address(),
                            initial_sequencer
                                .bond
                                .checked_add(bond)
                                .unwrap()
                                .checked_add(sequencer.available_gas_balance)
                                .unwrap(),
                        )]);
                    } else {
                        // We need to add the bond to the initial balance because genesis deduces the bond from the bank balance.
                        additional_accounts_vec.append(&mut vec![
                            (
                                initial_sequencer.as_user().address(),
                                initial_sequencer
                                    .bond
                                    .checked_add(initial_sequencer.as_user().available_gas_balance)
                                    .unwrap(),
                            ),
                            (
                                prover.address(),
                                bond.checked_add(prover.available_gas_balance).unwrap(),
                            ),
                        ]);

                        if let Some(challenger) = initial_challenger {
                            additional_accounts_vec
                                .push((challenger.address(), challenger.available_gas_balance));
                        }
                    }

                    additional_accounts_vec
                },
                admins: vec![],
            },
            tokens: parse_token_configs(additional_accounts),
        }
    }

    fn chain_state(
        operating_mode: OperatingMode,
        inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,
        outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
    ) -> ChainStateConfig<S> {
        ChainStateConfig {
            current_time: Default::default(),
            genesis_da_height: 0,
            operating_mode,
            inner_code_commitment,
            outer_code_commitment,
        }
    }
}

/// A convenient high-level representation of a ZK genesis config.
#[derive(derivative::Derivative, Clone)]
#[derivative(Debug(bound = ""))]
struct HighLevelBasicConfig<S: Spec> {
    additional_accounts: Vec<TestUser<S>>,
    gas_token_name: String,
    inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,
    outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
}

impl<S: Spec> HighLevelBasicConfig<S> {
    fn add_accounts_with_balance(&mut self, num_accounts: usize, balance: Amount) {
        for _ in 0..num_accounts {
            self.additional_accounts
                .push(TestUser::<S>::generate(balance));
        }
    }

    fn add_accounts_inner(&mut self, mut additional_accounts: Vec<TestUser<S>>) {
        self.additional_accounts.append(&mut additional_accounts);
    }

    fn add_accounts_with_token_inner(
        &mut self,
        token_name: &TestTokenName,
        with_minter: bool,
        num_accounts: usize,
        account_initial_balance: Amount,
    ) {
        let mut additional_accounts = Vec::with_capacity(num_accounts);

        if with_minter {
            additional_accounts.push(
                TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE).add_token_info(UserTokenInfo {
                    token_name: token_name.clone(),
                    balance: account_initial_balance,
                    is_minter: true,
                }),
            );
        }

        for _ in 0..num_accounts {
            additional_accounts.push(
                TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE).add_token_info(UserTokenInfo {
                    token_name: token_name.clone(),
                    balance: account_initial_balance,
                    is_minter: false,
                }),
            );
        }

        self.add_accounts_inner(additional_accounts);
    }
}

/// Helper function that parses the token configs from the list of test users.
fn parse_token_configs<S: Spec>(test_users: &[TestUser<S>]) -> Vec<TokenConfig<S>> {
    let mut token_configs = Vec::<TokenConfig<S>>::new();

    test_users.iter().for_each(|user| {
        let user_address = user.address();

        user.token_balances.iter().for_each(|token_info| {
            let token_name = &token_info.token_name;
            // If there is no entry for that specific token name, we create a new one.
            let token_config = if let Some(config) = token_configs
                .iter_mut()
                .find(|config| config.token_name == token_name.to_string())
            {
                config
            } else {
                let initial_token_config = TokenConfig {
                    token_name: token_name.to_string(),
                    token_decimals: None,
                    token_id: token_name.id(),
                    address_and_balances: Vec::new(),
                    admins: Vec::new(),
                    supply_cap: None,
                };

                token_configs.push(initial_token_config);

                token_configs.last_mut().unwrap()
            };

            if token_info.is_minter {
                token_config.admins.push(user_address.clone());
            }

            token_config
                .address_and_balances
                .push((user_address.clone(), token_info.balance));
        });
    });

    token_configs
}

fn sequencer_da_addr_inner<S: Spec>() -> <S::Da as DaSpec>::Address
where
    <S::Da as DaSpec>::Address: From<[u8; 32]>,
{
    [172; 32].into()
}

fn generate_config_details<S: Spec>(
    num_accounts: usize,
) -> (Amount, TestUser<S>, TestSequencer<S>, Vec<TestUser<S>>)
where
    <S::Da as DaSpec>::Address: From<[u8; 32]>,
{
    let user_stake_value = <S as Spec>::Gas::from(TEST_DEFAULT_USER_STAKE)
        .value(&S::initial_base_fee_per_gas())
        .saturating_mul(Amount::new(10));

    let prover_sequencer = TestUser::generate(
        user_stake_value
            .saturating_mul(Amount::new(2))
            .checked_add(TEST_DEFAULT_USER_BALANCE)
            .unwrap(),
    );

    let sequencer = TestSequencer {
        user_info: prover_sequencer.clone(),
        da_address: sequencer_da_addr_inner::<S>(),
        bond: user_stake_value,
    };

    let mut additional_accounts = Vec::with_capacity(num_accounts);

    for _ in 0..num_accounts {
        additional_accounts.push(TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE));
    }

    (
        user_stake_value,
        prover_sequencer,
        sequencer,
        additional_accounts,
    )
}
