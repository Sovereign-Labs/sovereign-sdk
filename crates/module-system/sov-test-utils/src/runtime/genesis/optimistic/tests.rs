use sov_accounts::AccountConfig;
use sov_attester_incentives::AttesterIncentivesConfig;
use sov_bank::{Bank, BankConfig};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    Amount, CodeCommitmentFor, DaSpec, EncodeCall, Gas, GasArray, GasSpec, PrivateKey, PublicKey,
    Spec,
};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::{PaymasterConfig, SafeVec};
use sov_prover_incentives::ProverIncentivesConfig;
use sov_rollup_interface::common::SlotNumber;
use sov_sequencer_registry::{SequencerConfig, SequencerRegistryConfig};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

use crate::interface::AsUser;
use crate::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use crate::runtime::genesis::TestTokenName;
use crate::runtime::{
    config_gas_token_id, ChainStateConfig, Coins, TestOptimisticRuntime, TestRunner,
};
use crate::{
    default_test_tx_details, generate_optimistic_runtime, TestPrivateKey, TestSpec, TestUser,
    TransactionTestAssert, TransactionTestCase, TransactionType, UserTokenInfo,
    TEST_DEFAULT_USER_BALANCE, TEST_DEFAULT_USER_STAKE, TEST_LIGHT_CLIENT_FINALIZED_HEIGHT,
    TEST_MAX_ATTESTED_HEIGHT, TEST_MIN_SEQ_BOND, TEST_ROLLUP_FINALITY_PERIOD,
};

const SEQUENCER_ADDR: [u8; 28] = [42u8; 28];
const SEQUENCER_DA_ADDR: [u8; 32] = [42u8; 32];

#[test]
// Tests the test setup by running the value setter module and checking if the value was set correctly
fn test_value_setter_tx_success() {
    let value_to_set = 18;
    let assertion: TransactionTestAssert<TestOptimisticRuntime<TestSpec>, TestSpec> =
        Box::new(move |_result, state| {
            let value_setter = ValueSetter::<TestSpec>::default();
            let value = value_setter
                .value
                .get(state)
                .unwrap_infallible()
                .expect("We should be able to get a value from the state");
            assert_eq!(value, value_to_set);
        });

    run_value_setter_txs_with_assertions(vec![(value_to_set, assertion)]);
}

#[test]
#[should_panic]
// Tests the test setup by running the value setter with an assertion that should fail and then trying to
// run another transaction afterward. This would cause subsequent tests to block forever if the test runtime
// failed to handle panics.
fn test_value_setter_tx_bad_assertion() {
    let value_to_set = 18;
    let bad_assertion: TransactionTestAssert<TestOptimisticRuntime<TestSpec>, TestSpec> =
        Box::new(move |_result, state| {
            let value_setter = ValueSetter::<TestSpec>::default();
            let value = value_setter
                .value
                .get(state)
                .unwrap_infallible()
                .expect("We should be able to get a value from the state");
            assert_eq!(value, value_to_set + 1); // This will fail!
        });

    run_value_setter_txs_with_assertions(vec![
        (value_to_set, bad_assertion),
        (1, Box::new(|_result, _state| {})),
    ]);
}

#[allow(clippy::type_complexity)]
fn run_value_setter_txs_with_assertions(
    values_and_assertions: Vec<(
        u32,
        TransactionTestAssert<TestOptimisticRuntime<TestSpec>, TestSpec>,
    )>,
) {
    let sequencer_rollup_addr = <TestSpec as Spec>::Address::from(SEQUENCER_ADDR);
    let admin_pkey = TestPrivateKey::generate();
    let admin_addr = admin_pkey.pub_key().credential_id().into();
    let genesis_config = create_test_rt_genesis_config(
        admin_addr,
        &[],
        sequencer_rollup_addr,
        SEQUENCER_DA_ADDR.into(),
        <TestSpec as Spec>::Gas::from(TEST_DEFAULT_USER_STAKE)
            .value(&TestSpec::initial_base_fee_per_gas()),
        "SovereignToken".to_string(),
        TEST_DEFAULT_USER_BALANCE,
        Default::default(),
        Default::default(),
    );
    let params = GenesisParams {
        runtime: genesis_config,
    };
    let mut runner: TestRunner<TestOptimisticRuntime<TestSpec>, TestSpec> =
        TestRunner::new_with_genesis(params, Default::default());

    for (value, assert) in values_and_assertions {
        let input = TransactionType::Plain {
            message:
                <TestOptimisticRuntime<TestSpec> as EncodeCall<ValueSetter<TestSpec>>>::to_decodable(
                    sov_value_setter::CallMessage::SetValue { value, gas: None },
                ),
            key: admin_pkey.clone(),
            details: default_test_tx_details(),
        };
        runner.execute_transaction(TransactionTestCase { input, assert });
    }
}

// TODO: generate this function in macro. We'll change the return type to a fixed `BasicGenesisConfig`
// and then implement a helper function to combine this basic config with config for other modules to
// create the full genesis config.
//
// This function should also take fewer arguments and generate data more aggressively.
// <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/682>
#[allow(clippy::too_many_arguments)]
fn create_test_rt_genesis_config<S: Spec>(
    admin: S::Address,
    additional_accounts: &[(S::Address, Amount)],
    seq_rollup_address: S::Address,
    seq_da_address: <S::Da as DaSpec>::Address,
    seq_bond: Amount,
    token_name: String,
    init_balance: Amount,
    inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,
    outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
) -> crate::runtime::GenesisConfig<S> {
    let user_stake = <S as Spec>::Gas::from(TEST_DEFAULT_USER_STAKE);
    let prover_placeholder = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    crate::runtime::GenesisConfig {
        value_setter: ValueSetterConfig {
            admin: admin.clone(),
        },
        paymaster: PaymasterConfig {
            payers: SafeVec::new(),
        },
        sequencer_registry: SequencerRegistryConfig {
            minimum_bond: TEST_MIN_SEQ_BOND,
            sequencer_config: SequencerConfig {
                seq_rollup_address: seq_rollup_address.clone(),
                seq_da_address,
                seq_bond,
                is_preferred_sequencer: true,
            },
        },
        operator_incentives: sov_operator_incentives::OperatorIncentivesConfig {
            reward_address: prover_placeholder.address(),
        },
        attester_incentives: AttesterIncentivesConfig {
            minimum_attester_bond: user_stake.clone(),
            minimum_challenger_bond: user_stake.clone(),
            initial_attesters: vec![(
                admin.clone(),
                user_stake.value(&S::initial_base_fee_per_gas()),
            )],
            rollup_finality_period: SlotNumber::new_dangerous(TEST_ROLLUP_FINALITY_PERIOD),
            maximum_attested_height: TEST_MAX_ATTESTED_HEIGHT,
            light_client_finalized_height: TEST_LIGHT_CLIENT_FINALIZED_HEIGHT,
        },
        prover_incentives: ProverIncentivesConfig {
            minimum_bond: user_stake.clone(),
            proving_penalty: {
                let mut proving_penalty = user_stake.clone();
                proving_penalty.scalar_division(2);
                proving_penalty
            },
            initial_provers: vec![(prover_placeholder.address(), prover_placeholder.balance())],
        },
        bank: BankConfig {
            gas_token_config: sov_bank::GasTokenConfig {
                token_name: token_name.clone(),
                token_decimals: None,
                supply_cap: None,
                address_and_balances: {
                    let mut additional_accounts_vec = additional_accounts.to_vec();
                    additional_accounts_vec.append(&mut vec![
                        (seq_rollup_address, init_balance),
                        (admin.clone(), init_balance),
                        (prover_placeholder.address(), prover_placeholder.balance()),
                    ]);
                    additional_accounts_vec
                },
                admins: vec![admin.clone()],
            },
            tokens: vec![],
        },

        blob_storage: (),

        chain_state: ChainStateConfig {
            current_time: Default::default(),
            genesis_da_height: 0,
            operating_mode: sov_modules_api::OperatingMode::Optimistic,
            inner_code_commitment,
            outer_code_commitment,
        },

        accounts: AccountConfig { accounts: vec![] },

        uniqueness: (),
    }
}

generate_optimistic_runtime!(TestRuntime <=);

#[test]
fn test_rollup_height() {
    let genesis_config = HighLevelOptimisticGenesisConfig::<TestSpec>::generate();
    let genesis_config = GenesisConfig::from_minimal_config(genesis_config.clone().into());

    let runtime = TestRuntime::default();

    let mut runner =
        TestRunner::<_, _>::new_with_genesis(genesis_config.into_genesis_params(), runtime);
    assert_eq!(runner.true_slot_number().get(), 0);

    runner.advance_slots(2);

    assert_eq!(runner.true_slot_number().get(), 2);

    runner.advance_slots(2);

    assert_eq!(runner.true_slot_number().get(), 4);
}

#[test]
fn test_define_token() {
    let token_0_name = &TestTokenName::new("0".to_string());
    let token_1_name = &TestTokenName::new("MyTestToken".to_string());

    let genesis_config = HighLevelOptimisticGenesisConfig::<TestSpec>::generate()
        .add_accounts_with_default_balance(1)
        .add_accounts_with_token(token_0_name, true, 2, Amount::new(100_000))
        .add_accounts_with_token(token_1_name, false, 1, Amount::new(10));

    let admin = genesis_config.additional_accounts()[0].clone();

    let genesis_config = crate::runtime::GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
        PaymasterConfig {
            payers: SafeVec::new(),
        },
    );

    assert_eq!(genesis_config.bank.tokens.len(), 2);
    let token_0 = genesis_config.bank.tokens.first().unwrap();
    assert_eq!(token_0.token_name, "TestToken(0)");
    assert_eq!(token_0.admins.len(), 1);
    assert_eq!(token_0.address_and_balances.len(), 3);
    assert!(token_0
        .address_and_balances
        .iter()
        .all(|(_, balance)| { *balance == 100_000 }));
    assert!(token_0.address_and_balances.iter().all(|(address, _)| {
        genesis_config
            .bank
            .gas_token_config
            .address_and_balances
            .contains(&(*address, TEST_DEFAULT_USER_BALANCE))
    }));

    let token_1 = genesis_config.bank.tokens.get(1).unwrap();
    assert_eq!(token_1.token_name, "TestToken(MyTestToken)");
    assert_eq!(token_1.admins.len(), 0);
    assert_eq!(token_1.address_and_balances.len(), 1);
    assert!(token_1
        .address_and_balances
        .iter()
        .all(|(_, balance)| { *balance == 10 }));
    assert!(token_1.address_and_balances.iter().all(|(address, _)| {
        genesis_config
            .bank
            .gas_token_config
            .address_and_balances
            .contains(&(*address, TEST_DEFAULT_USER_BALANCE))
    }));
}

#[test]
fn test_define_token_with_state() {
    const BALANCE_TOKEN_0: Amount = Amount::new(100_000);

    let token_0_name = &TestTokenName::new("0".to_string());
    let token_1_name = &TestTokenName::new("MyTestToken".to_string());

    let genesis_config = HighLevelOptimisticGenesisConfig::generate()
        .add_accounts_with_default_balance(1)
        .add_accounts_with_token(token_0_name, false, 2, BALANCE_TOKEN_0)
        .add_accounts_with_token(token_1_name, true, 0, Amount::ZERO);

    let admin = genesis_config.additional_accounts()[0].clone();

    let token_names = genesis_config.token_names();

    assert!(token_names.contains(&TestTokenName::new("0".to_string())));
    assert!(token_names.contains(&TestTokenName::new("MyTestToken".to_string())));

    let token_0_holders = genesis_config.get_accounts_for_token(token_0_name);

    let genesis_config = crate::runtime::GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
        PaymasterConfig {
            payers: SafeVec::new(),
        },
    );

    let runner = TestRunner::<_, _>::new_with_genesis(
        genesis_config.into_genesis_params(),
        TestOptimisticRuntime::<TestSpec>::default(),
    );

    runner.query_visible_state(|state| {
        assert_eq!(
            Bank::<TestSpec>::default()
                .get_token_name(&token_0_name.id(), state)
                .unwrap_infallible()
                .unwrap(),
            "TestToken(0)"
        );
        assert_eq!(
            Bank::<TestSpec>::default()
                .get_token_name(&token_1_name.id(), state)
                .unwrap_infallible()
                .unwrap(),
            "TestToken(MyTestToken)"
        );

        token_0_holders.into_iter().for_each(|user| {
            assert_eq!(
                Bank::<TestSpec>::default()
                    .get_balance_of(&user.address(), config_gas_token_id(), state)
                    .unwrap_infallible()
                    .unwrap(),
                user.balance(),
                "The new token's user balance should be equal to the initial gas balance"
            );

            assert_eq!(
                Bank::<TestSpec>::default()
                    .get_balance_of(&user.address(), token_0_name.id(), state)
                    .unwrap_infallible()
                    .unwrap(),
                user.token_balance(token_0_name).unwrap(),
                "The new token's user balance should be equal to the initial token balance"
            );

            assert_eq!(
                Bank::<TestSpec>::default()
                    .get_balance_of(&user.address(), token_1_name.id(), state)
                    .unwrap_infallible(),
                None,
                "The user should not have any balance for the second token"
            );

            assert_eq!(
                Bank::<TestSpec>::default()
                    .get_balance_of(&user.address(), token_0_name.id(), state)
                    .unwrap_infallible()
                    .unwrap(),
                BALANCE_TOKEN_0,
                "The user should have the initial token balance specified"
            );

            assert_eq!(
                Bank::<TestSpec>::default()
                    .get_balance_of(&user.address(), config_gas_token_id(), state)
                    .unwrap_infallible()
                    .unwrap(),
                TEST_DEFAULT_USER_BALANCE,
                "The user should have the default initial gas balance"
            );
        });
    });
}

#[test]
fn test_define_token_with_mint() {
    let token_0_name = &TestTokenName::new("0".to_string());

    let genesis_config = HighLevelOptimisticGenesisConfig::generate()
        .add_accounts_with_default_balance(1)
        .add_accounts_with_token(token_0_name, true, 0, Amount::ZERO);

    let token_0_name = genesis_config.token_names().pop().unwrap();
    let mut token_0_holders = genesis_config.get_accounts_for_token(&token_0_name);

    assert_eq!(token_0_holders.len(), 1);

    let minter = token_0_holders.pop().unwrap();

    let admin = genesis_config.additional_accounts()[0].clone();

    let minter_address = minter.as_user().address();

    let genesis_config = crate::runtime::GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
        PaymasterConfig {
            payers: SafeVec::new(),
        },
    );

    let mut runner = TestRunner::<_, _>::new_with_genesis(
        genesis_config.into_genesis_params(),
        TestOptimisticRuntime::<TestSpec>::default(),
    );

    runner.execute_transaction(TransactionTestCase {
        input: minter
            .create_plain_message::<TestOptimisticRuntime<TestSpec>, sov_bank::Bank<TestSpec>>(
                sov_bank::CallMessage::Mint {
                    coins: Coins {
                        amount: Amount::new(100),
                        token_id: token_0_name.id(),
                    },
                    mint_to_address: minter_address,
                },
            ),
        assert: Box::new(move |receipt, state| {
            assert!(receipt.tx_receipt.is_successful());

            assert_eq!(
                Bank::<TestSpec>::default()
                    .get_balance_of(&minter_address, token_0_name.id(), state)
                    .unwrap_infallible()
                    .unwrap(),
                100,
                "The minter should have the minted amount"
            );
        }),
    });
}

#[test]
fn test_define_genesis_config_additional_accounts_with_default_balance() {
    let mut genesis_config = HighLevelOptimisticGenesisConfig::<TestSpec>::generate();

    // By default we don't have any additional accounts
    assert!(genesis_config.additional_accounts().is_empty());

    genesis_config = genesis_config.add_accounts_with_default_balance(5);
    assert_eq!(genesis_config.additional_accounts().len(), 5);

    genesis_config
        .additional_accounts()
        .iter()
        .for_each(|user| {
            assert_eq!(user.balance(), TEST_DEFAULT_USER_BALANCE);
            assert_eq!(user.token_balances.len(), 0);
        });
}

#[test]
fn test_define_genesis_config_additional_accounts_test_user() {
    let mut genesis_config = HighLevelOptimisticGenesisConfig::<TestSpec>::generate();

    // By default we don't have any additional accounts
    assert!(genesis_config.additional_accounts().is_empty());

    let balance_1 = Amount::new(100);
    let balance_2 = Amount::new(1);
    genesis_config = genesis_config.add_accounts(vec![
        TestUser::<TestSpec>::generate(balance_1),
        TestUser::<TestSpec>::generate(balance_2).add_token_info(UserTokenInfo {
            token_name: TestTokenName::new("Token".to_string()),
            balance: Amount::new(5),
            is_minter: false,
        }),
    ]);
    assert_eq!(genesis_config.additional_accounts().len(), 2);

    let first_user = genesis_config.additional_accounts().first().unwrap();
    let second_user = genesis_config.additional_accounts().get(1).unwrap();

    assert_eq!(first_user.balance(), balance_1);
    assert_eq!(first_user.token_balances.len(), 0);

    assert_eq!(second_user.balance(), balance_2);
    assert_eq!(second_user.token_balances.len(), 1);
    assert_eq!(
        second_user.token_balances.first().unwrap().token_name,
        TestTokenName::new("Token".to_string())
    );
    assert_eq!(
        second_user
            .token_balance(&TestTokenName::new("Token".to_string()))
            .unwrap(),
        5
    );
    assert!(!second_user.is_minter(&TestTokenName::new("Token".to_string())));
}
