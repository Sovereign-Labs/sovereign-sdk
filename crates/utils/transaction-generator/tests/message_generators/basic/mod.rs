use sov_modules_api::prelude::arbitrary::Unstructured;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{TestSpec as S, TransactionTestCase, TransactionType};
use sov_transaction_generator::interface::rng_utils::get_random_bytes;
use sov_transaction_generator::interface::{MessageValidity, Percent};
use sov_transaction_generator::Distribution;

use crate::{
    plain_tx_with_default_details, setup_harness, setup_roles_and_config, GeneratorOutput,
    ModulesToUse, MAXIMUM_HOOKS_OPS, MAXIMUM_WRITE_BEGIN_INDEX, MAXIMUM_WRITE_DATA_LENGTH,
    MAXIMUM_WRITE_SIZE, MAX_VEC_LEN_VALUE_SETTER, RT, USER_BALANCE,
};

/// The number of transactions to generate.
pub const TXS_TO_GENERATE: u64 = 100;

mod combined;
mod successful_generation;
mod unsuccessful_generation;

fn test_with_modules(
    modules: Distribution<ModulesToUse>,
    validity: Distribution<MessageValidity>,
    transaction_exec_closure: &mut impl FnMut(
        TransactionType<RT, S>,
        GeneratorOutput,
        &mut TestRunner<RT, S>,
    ),
) -> (TestRunner<RT, S>, Vec<GeneratorOutput>) {
    let random_bytes = get_random_bytes(100_000_000, 1);
    let u = &mut Unstructured::new(&random_bytes[..]);

    let setup = setup_roles_and_config(USER_BALANCE);
    let mut generator = setup_harness::<RT>(
        Percent::one_hundred(),
        &setup.value_setter_admin,
        MAX_VEC_LEN_VALUE_SETTER,
        MAXIMUM_WRITE_DATA_LENGTH,
        MAXIMUM_WRITE_BEGIN_INDEX,
        MAXIMUM_WRITE_SIZE,
        MAXIMUM_HOOKS_OPS,
        &modules,
    );

    let mut runner = TestRunner::<RT, S>::new_with_genesis(
        setup.genesis_config.into_genesis_params(),
        Default::default(),
    );

    let init_txs = generator
        .initial_transactions
        .iter()
        .take(generator.initial_transactions.len());

    // Execute initial transactions if there are some
    for init_tx in init_txs {
        runner.execute_transaction(TransactionTestCase {
            input: plain_tx_with_default_details::<RT>(init_tx),
            assert: Box::new(move |receipt, _state| {
                assert!(
                    receipt.tx_receipt.is_successful(),
                    "The initial transactions should always be successful"
                );
            }),
        });
    }

    let modules = modules.map_values(&mut |module| {
        module.select(
            generator.bank_harness.clone(),
            generator.value_setter_harness.clone(),
            generator.access_pattern_harness.clone(),
        )
    });

    // Generate and execute [`TXS_TO_GENERATE`] txs
    let outputs: Vec<_> = (0..TXS_TO_GENERATE)
        .map(|_| {
            let validity = validity.select_value(u).expect("Ran out of randomness");
            let expected_output = generator.generate(&modules, *validity);

            let tx = plain_tx_with_default_details(&expected_output);

            transaction_exec_closure(tx, expected_output.clone(), &mut runner);

            expected_output
        })
        .collect();

    (runner, outputs)
}
