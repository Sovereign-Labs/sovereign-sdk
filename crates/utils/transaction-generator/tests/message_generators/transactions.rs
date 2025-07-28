use std::sync::Arc;

use sov_modules_api::prelude::*;
use sov_modules_api::Amount;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_optimistic_runtime, TestSpec as S, TestUser, TEST_DEFAULT_USER_BALANCE,
};
use sov_transaction_generator::generators::bank::harness_interface::BankHarness;
use sov_transaction_generator::generators::bank::BankMessageGenerator;
use sov_transaction_generator::generators::basic::{BasicModuleRef, BasicTag};
use sov_transaction_generator::generators::transaction::{
    GasLimitOutcome, GeneratedTransaction, MaxFeeOutcome, RunTest, SovereignContext,
    SovereignGeneratedTransaction, TransactionOutcome,
};
use sov_transaction_generator::generators::value_setter::{
    ValueSetterHarness, ValueSetterMessageGenerator,
};
use sov_transaction_generator::interface::rng_utils::get_random_bytes;
use sov_transaction_generator::{AccountState, Distribution, Percent, State};
use sov_value_setter::{
    CallMessageDiscriminants as ValueSetterDiscriminants, ValueSetter, ValueSetterConfig,
};

generate_optimistic_runtime!(TestRuntime <= value_setter: ValueSetter<S>);

type RT = TestRuntime<S>;

fn test_outcomes(outcomes: Vec<Arc<TransactionOutcome>>, txs_count: usize) {
    std::env::set_var(
        "RUST_LOG",
        "info,sov_metrics=error,sov_modules_stf_blueprint=debug,sov_bank=trace",
    );
    sov_test_utils::initialize_logging();
    use sov_bank::CallMessageDiscriminants::*;
    let user = TestUser::<S>::generate_with_default_balance();

    let bank_harness = BankHarness::new(BankMessageGenerator::<S>::new(
        Distribution::with_equiprobable_values(vec![Transfer, Freeze, Burn, Mint, CreateToken]),
        Percent::one_hundred(),
    ));
    let value_setter_harness = ValueSetterHarness::new(ValueSetterMessageGenerator::<S>::new(
        Distribution::with_equiprobable_values(vec![
            ValueSetterDiscriminants::SetValue,
            ValueSetterDiscriminants::SetManyValues,
        ]),
        sov_transaction_generator::generators::value_setter::ValueSetterGeneratorOptions {
            maximum_vec_length: 1000,
            min_and_max_number_of_individual_state_operations: (0, 0),
            min_and_max_number_of_new_values_for_heavy_state: (0, 0),
            min_and_max_number_of_iterations_for_cpu_heavy_operation: (0, 0),
            max_heavy_state_size: 0,
        },
        user.private_key.clone(),
    ));
    let modules: Vec<BasicModuleRef<S, RT>> =
        vec![Arc::new(bank_harness), Arc::new(value_setter_harness)];

    let random_bytes: Vec<u8> = get_random_bytes(100_000, 0);
    let u = &mut arbitrary::Unstructured::new(&random_bytes[..]);
    let mut state: State<S, BasicTag> = State::with_account_and_tags(
        AccountState {
            private_key: user.private_key.clone(),
            balances: vec![],
            can_mint: Default::default(),
            sequencing_bond: None,
            additional_info: Default::default(),
        },
        vec![],
    );

    let mut context = SovereignContext {
        modules: Distribution::with_equiprobable_values(modules),
        u,
        call_generator_state: &mut state,
        outcomes: Distribution::with_equiprobable_values(outcomes),
    };

    let mut generated_txs = vec![];

    for _ in 0..txs_count {
        generated_txs.push(SovereignGeneratedTransaction::new(&mut context));
    }

    let accounts = generated_txs
        .iter()
        .map(|tx| {
            TestUser::<S>::new(
                tx.msg.sender.clone(),
                TEST_DEFAULT_USER_BALANCE
                    .checked_mul(Amount::new(10))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let low_genesis_config = HighLevelOptimisticGenesisConfig::generate()
        .add_accounts_with_default_balance(2)
        .add_accounts(vec![user.clone()])
        .add_accounts(accounts);

    let genesis_config = GenesisConfig::from_minimal_config(
        low_genesis_config.into(),
        ValueSetterConfig {
            admin: user.address(),
        },
    );
    let mut runner = TestRunner::<RT, S>::new_with_genesis(
        genesis_config.into_genesis_params(),
        Default::default(),
    );

    for generated_tx in generated_txs {
        generated_tx.run_test(&mut runner);
    }
}

#[test]
#[ignore = "test needs to be revisited: tx needs to be executed even if OOG"]
fn test_transaction_max_fee() {
    let outcomes = vec![
        Arc::new(TransactionOutcome::MaxFee(MaxFeeOutcome::Excess)),
        Arc::new(TransactionOutcome::MaxFee(MaxFeeOutcome::Insufficient)),
        Arc::new(TransactionOutcome::MaxFee(MaxFeeOutcome::Exact)),
    ];
    test_outcomes(outcomes, 10);
}

#[test]
#[ignore = "test needs to be revisited: tx needs to be executed even if OOG"]
fn test_transaction_gas_limit() {
    let outcomes = vec![
        Arc::new(TransactionOutcome::GasLimit(GasLimitOutcome::Excess)),
        Arc::new(TransactionOutcome::GasLimit(GasLimitOutcome::Insufficient)),
    ];
    test_outcomes(outcomes, 10);
}
