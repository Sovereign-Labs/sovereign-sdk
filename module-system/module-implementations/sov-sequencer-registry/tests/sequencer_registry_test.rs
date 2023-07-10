use sov_modules_api::{Context, Module};
use sov_sequencer_registry::call::CallMessage;
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

use helpers::*;

// Happy path for registration and exit
// This test checks:
//  - genesis sequencer is present after genesis
//  - registration works, and funds are deducted
//  - exit works and funds are returned
#[test]
fn test_registration_lifecycle() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    // Check genesis
    {
        let sequencer_address = generate_address(GENESIS_SEQUENCER_KEY);
        let registry_response = test_sequencer
            .registry
            .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set);
        assert_eq!(Some(sequencer_address), registry_response.address);
    }

    // Check normal lifecycle

    let da_address = ANOTHER_SEQUENCER_DA_ADDRESS.to_vec();

    let sequencer_address = generate_address(ANOTHER_SEQUENCER_KEY);
    let sender_context = C::new(sequencer_address.clone());

    let balance_before = test_sequencer
        .query_balance(sequencer_address.clone(), working_set)
        .amount
        .unwrap();

    let registry_response_before = test_sequencer
        .registry
        .sequencer_address(da_address.clone(), working_set);
    assert!(registry_response_before.address.is_none());

    let register_message = CallMessage::Register {
        da_address: da_address.clone(),
    };
    test_sequencer
        .registry
        .call(register_message, &sender_context, working_set)
        .expect("Sequencer registration has failed");

    let balance_after_registration = test_sequencer
        .query_balance(sequencer_address.clone(), working_set)
        .amount
        .unwrap();
    assert_eq!(balance_before - LOCKED_AMOUNT, balance_after_registration);

    let registry_response_after_registration = test_sequencer
        .registry
        .sequencer_address(da_address.clone(), working_set);
    assert_eq!(
        Some(sequencer_address.clone()),
        registry_response_after_registration.address
    );

    let exit_message = CallMessage::Exit {
        da_address: da_address.clone(),
    };
    test_sequencer
        .registry
        .call(exit_message, &sender_context, working_set)
        .expect("Sequencer exit has failed");

    let balance_after_exit = test_sequencer
        .query_balance(sequencer_address, working_set)
        .amount
        .unwrap();
    assert_eq!(balance_before, balance_after_exit);

    let registry_response_after_exit = test_sequencer
        .registry
        .sequencer_address(da_address, working_set);
    assert!(registry_response_after_exit.address.is_none());
}

#[test]
fn test_registration_not_enough_funds() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let da_address = ANOTHER_SEQUENCER_DA_ADDRESS.to_vec();

    let sequencer_address = generate_address(LOW_FUND_KEY);
    let sender_context = C::new(sequencer_address.clone());

    let register_message = CallMessage::Register { da_address };
    let response = test_sequencer
        .registry
        .call(register_message, &sender_context, working_set);

    assert!(
        response.is_err(),
        "insufficient funds registration should fail"
    );
    let expected_error_message = format!("Insufficient funds for {}", sequencer_address);
    let actual_error_message = response.err().unwrap().to_string();

    assert_eq!(expected_error_message, actual_error_message);
}

#[test]
fn test_registration_second_time() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let da_address = GENESIS_SEQUENCER_DA_ADDRESS.to_vec();

    let sequencer_address = generate_address(GENESIS_SEQUENCER_KEY);
    let sender_context = C::new(sequencer_address.clone());

    let register_message = CallMessage::Register { da_address };
    let response = test_sequencer
        .registry
        .call(register_message, &sender_context, working_set);

    assert!(response.is_err(), "duplicate registration should fail");
    let expected_error_message = format!("sequencer {} already registered", sequencer_address);
    let actual_error_message = response.err().unwrap().to_string();

    assert_eq!(expected_error_message, actual_error_message);
}

#[test]
fn test_exit_different_sender() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let da_address = ANOTHER_SEQUENCER_DA_ADDRESS.to_vec();

    let sequencer_address = generate_address(ANOTHER_SEQUENCER_KEY);
    let sender_context = C::new(sequencer_address);
    let attacker_address = generate_address("some_random_key");
    let attacker_context = C::new(attacker_address);

    let register_message = CallMessage::Register {
        da_address: da_address.clone(),
    };
    test_sequencer
        .registry
        .call(register_message, &sender_context, working_set)
        .expect("Sequencer registration has failed");

    let exit_message = CallMessage::Exit { da_address };
    let response = test_sequencer
        .registry
        .call(exit_message, &attacker_context, working_set);

    assert!(
        response.is_err(),
        "exit by non authorized sender should fail"
    );
    let actual_error_message = response.err().unwrap().to_string();

    assert_eq!("Unauthorized exit attempt", actual_error_message);
}

#[test]
fn test_allow_exit_last_sequencer() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let sequencer_address = generate_address(GENESIS_SEQUENCER_KEY);
    let sender_context = C::new(sequencer_address);
    let exit_message = CallMessage::Exit {
        da_address: GENESIS_SEQUENCER_DA_ADDRESS.to_vec(),
    };
    test_sequencer
        .registry
        .call(exit_message, &sender_context, working_set)
        .expect("Last sequencer exit has failed");
}
