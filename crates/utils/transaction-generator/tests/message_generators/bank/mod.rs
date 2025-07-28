use std::sync::Arc;

use sov_bank::CallMessageDiscriminants;
use sov_modules_api::prelude::arbitrary::Unstructured;
use sov_test_utils::TestSpec as S;
use sov_transaction_generator::generators::bank::BankMessageGenerator;
use sov_transaction_generator::generators::basic::{
    BasicBankHarness, BasicCallMessageFactory, BasicModuleRef, BasicTag,
};
use sov_transaction_generator::rng_utils::get_random_bytes;
use sov_transaction_generator::{Distribution, MessageValidity, Percent, State};

use crate::TestRuntime;

#[test]
fn generate_large_number_of_transfers_should_work() {
    let mut state = State::<S, BasicTag>::new();
    let bank_generator: BasicModuleRef<S, TestRuntime<S>> =
        Arc::new(BasicBankHarness::<S, TestRuntime<S>>::new(
            BankMessageGenerator::new(
                Distribution::with_equiprobable_values(vec![CallMessageDiscriminants::Transfer]),
                Percent::fifty(),
            ),
        ));
    let call_message_factory = BasicCallMessageFactory::new();

    let mut curr_salt = 0;
    let mut rand_bytes = get_random_bytes(1_000_000, curr_salt);
    curr_salt += 1;

    let mut u = Unstructured::new(&rand_bytes);
    call_message_factory
        .generate_setup_messages(&vec![bank_generator.clone()], &mut u, &mut state)
        .unwrap();

    for _ in 0..10 {
        // We rerandomize the state to ensure that there is enough randomness to generate new call messages
        rand_bytes = get_random_bytes(1_000_000, curr_salt);
        curr_salt += 1;
        u = Unstructured::new(&rand_bytes);

        for _ in 0..10_000 {
            call_message_factory
                .generate_call_message(
                    &Distribution::with_equiprobable_values(vec![bank_generator.clone()]),
                    &mut u,
                    &mut state,
                    MessageValidity::Valid,
                )
                .unwrap();
        }
    }
}

#[test]
fn generate_large_number_of_bank_operations_should_work() {
    let mut state = State::<S, BasicTag>::new();
    let bank_generator: BasicModuleRef<S, TestRuntime<S>> =
        Arc::new(BasicBankHarness::<S, TestRuntime<S>>::new(
            BankMessageGenerator::new(
                Distribution::with_equiprobable_values(vec![
                    CallMessageDiscriminants::Transfer,
                    CallMessageDiscriminants::Mint,
                    CallMessageDiscriminants::Burn,
                    CallMessageDiscriminants::Freeze,
                    CallMessageDiscriminants::CreateToken,
                ]),
                Percent::fifty(),
            ),
        ));
    let call_message_factory = BasicCallMessageFactory::new();

    let mut curr_salt = 0;
    let mut rand_bytes = get_random_bytes(1_000_000, curr_salt);
    curr_salt += 1;

    let mut u = Unstructured::new(&rand_bytes);
    call_message_factory
        .generate_setup_messages(&vec![bank_generator.clone()], &mut u, &mut state)
        .unwrap();

    for _ in 0..10 {
        // We rerandomize the state to ensure that there is enough randomness to generate new call messages
        rand_bytes = get_random_bytes(1_000_000, curr_salt);
        curr_salt += 1;
        u = Unstructured::new(&rand_bytes);

        for _ in 0..10_000 {
            call_message_factory
                .generate_call_message(
                    &Distribution::with_equiprobable_values(vec![bank_generator.clone()]),
                    &mut u,
                    &mut state,
                    MessageValidity::Valid,
                )
                .unwrap();
        }
    }
}
