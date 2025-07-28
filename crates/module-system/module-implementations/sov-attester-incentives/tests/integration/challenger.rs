use std::convert::Infallible;

use sov_attester_incentives::{AttesterIncentives, CallMessage, SlashingReason};
use sov_bank::Amount;
use sov_mock_da::MockHash;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{InvalidProofError, ProofOutcome};
use sov_rollup_interface::common::SlotNumber;
use sov_state::StorageRoot;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    assert_matches, AsUser, AtomicAmount, BondedTestChallenger, ProofInput, ProofTestCase,
    TestAttester, TransactionTestCase, TEST_ROLLUP_FINALITY_PERIOD,
};

use crate::helpers::{
    build_challenge, build_proof, make_attestation_blob, make_challenge_blob, setup,
    TestAttesterIncentives, RT, S,
};

/// Helper that sets up a configuration where:
/// - the challenger is bonded and
/// - there is a wrong attestation to challenge in the first slot.
fn setup_with_wrong_attestation() -> (
    TestRunner<RT, S>,
    TestAttester<S>,
    BondedTestChallenger<S>,
    Amount,
) {
    let (mut runner, genesis_attester, mut genesis_challenger, _) = setup();

    let genesis_attester_address = genesis_attester.user_info.address();
    let genesis_attester_bond = genesis_attester.bond;

    let genesis_challenger_address = genesis_challenger.user_info.address();
    let genesis_challenger_bond = runner.query_visible_state(|state| {
        TestAttesterIncentives::default().get_minimal_challenger_bond_value(state)
    });

    let expected_challenger_balance = AtomicAmount::new(genesis_challenger.user_info.balance());
    let expected_challenger_balance_2 = expected_challenger_balance.clone();
    let expected_challenger_balance_3 = expected_challenger_balance.clone();

    let bond_challenger = TransactionTestCase {
        input: genesis_challenger.create_plain_message::<RT, TestAttesterIncentives>(
            CallMessage::RegisterChallenger(genesis_challenger_bond),
        ),
        assert: Box::new(move |result, state| {
            assert_eq!(
                TestAttesterIncentives::default()
                    .get_challenger_bond_amount(genesis_challenger_address, state)
                    .unwrap_infallible()
                    .value,
                genesis_challenger_bond,
                "Challenger not bonded"
            );

            // Update the challenger balance (because they consumed some gas and bonded)
            expected_challenger_balance.sub(result.gas_value_used);

            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&genesis_challenger_address, state),
                Some(
                    expected_challenger_balance_2
                        .get()
                        .checked_sub(genesis_challenger_bond)
                        .unwrap()
                ),
                "The attester should have the correct bond amount from genesis"
            );
        }),
    };

    runner
        .execute_transaction(bond_challenger)
        // Then execute empty transactions to reach finality
        .advance_slots(TEST_ROLLUP_FINALITY_PERIOD as usize);

    genesis_challenger.user_info.available_gas_balance = expected_challenger_balance_3.get();

    let bonded_challenger =
        BondedTestChallenger::from_challenger(genesis_challenger, genesis_challenger_bond);

    let initial_attester_balance = runner
        .query_visible_state(|state| {
            TestRunner::<RT, S>::bank_gas_balance(&genesis_attester_address, state)
        })
        .unwrap();

    {
        let mut attestation_proof = runner
            .query_visible_state(|state| build_proof(state, 1, &genesis_attester_address))
            .unwrap();

        attestation_proof.post_state_root = StorageRoot::new([255; 32], [255; 32]);

        runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
            input: ProofInput(make_attestation_blob(attestation_proof)),
            assert: Box::new(move |result, state| {
                assert_matches!(
                    result.proof_receipt.unwrap().outcome,
                    ProofOutcome::Invalid(_)
                );

                // Check that the attester was slashed
                assert!(TestAttesterIncentives::default()
                    .bonded_attesters
                    .get(&genesis_attester_address, state)
                    .unwrap()
                    .is_none(),);

                // Check that the transition was added to the challengeable set
                // The attestation should be part of the challengeable set and its associated value should be the BOND_AMOUNT
                assert_eq!(
                    AttesterIncentives::<S>::default()
                        .bad_transition_pool
                        .get(&SlotNumber::ONE, state)
                        .unwrap_infallible(),
                    Some(genesis_attester_bond),
                    "The transition should exist in the pool"
                );

                assert_eq!(
                    TestRunner::<RT, S>::bank_gas_balance(&genesis_attester_address, state),
                    Some(
                        initial_attester_balance
                            .checked_sub(result.gas_value_used)
                            .unwrap()
                    ),
                    "The attester should have the correct bond amount from genesis"
                );
            }),
        });
    }

    (
        runner,
        genesis_attester,
        bonded_challenger,
        genesis_attester_bond,
    )
}

/// Test that given an invalid transition, a challenger can successfully challenge it and get rewarded
/// This tests the happy path of challenge processing.

#[test]
fn test_valid_challenge() -> Result<(), Infallible> {
    let (mut runner, _, bonded_challenger, expected_reward) = setup_with_wrong_attestation();
    let bonded_challenger_address = bonded_challenger.user_info.address();

    let challenge_proof = runner
        .query_visible_state(|state| {
            build_challenge(
                state,
                SlotNumber::new_dangerous(1),
                bonded_challenger_address,
            )
        })
        .unwrap();

    let initial_challenger_balance = runner
        .query_visible_state(|state| {
            TestRunner::<RT, S>::bank_gas_balance(&bonded_challenger_address, state)
        })
        .unwrap();

    runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
        input: ProofInput(make_challenge_blob(
            challenge_proof,
            true,
            SlotNumber::new_dangerous(1),
        )),
        assert: Box::new(move |result, state| {
            assert_eq!(
                TestAttesterIncentives::default()
                    .bad_transition_pool
                    .get(&SlotNumber::ONE, state)
                    .unwrap_infallible(),
                None,
                "The transition should have disappeared from the pool"
            );

            let reward = TestAttesterIncentives::default()
                .burn_rate()
                .apply(expected_reward);

            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&bonded_challenger_address, state).unwrap(),
                initial_challenger_balance
                    .checked_sub(result.gas_value_used)
                    .unwrap()
                    .checked_add(reward)
                    .unwrap(),
            );
        }),
    });

    Ok(())
}

fn test_invalid_challenge_helper(
    runner: &mut TestRunner<RT, S>,
    expected_reward: u128,
    bonded_challenger: &BondedTestChallenger<S>,
    challenge_blob: Vec<u8>,
    slashing_reason: SlashingReason,
) {
    let bonded_challenger_address = bonded_challenger.user_info.address();

    let initial_challenger_balance = runner
        .query_visible_state(|state| {
            TestRunner::<RT, S>::bank_gas_balance(&bonded_challenger_address, state)
        })
        .unwrap();

    runner.execute_proof::<TestAttesterIncentives>(ProofTestCase {
        input: ProofInput(challenge_blob),
        assert: Box::new(move |result, state| {
            match &result.proof_receipt.unwrap().outcome {
                ProofOutcome::Invalid(InvalidProofError::ProverSlashed(msg)) => {
                    assert_eq!(msg, &slashing_reason.to_string());
                }
                _ => panic!("Expected invalid outcome"),
            }

            // Check that the challenger was slashed
            assert_eq!(
                TestAttesterIncentives::default()
                    .bonded_challengers
                    .get(&bonded_challenger_address, state)
                    .unwrap_infallible(),
                None,
                "The challenger was not removed from the bonded challengers set"
            );

            // Check that the challenge set is *not* empty
            assert_eq!(
                TestAttesterIncentives::default()
                    .bad_transition_pool
                    .get(&SlotNumber::ONE, state)
                    .unwrap_infallible(),
                Some(expected_reward.into()),
                "The transition should *not* have disappeared from the pool"
            );

            // Check that the challenger was not rewarded

            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&bonded_challenger_address, state).unwrap(),
                initial_challenger_balance
                    .checked_sub(result.gas_value_used)
                    .unwrap(),
            );
        }),
    });
}

#[test]
fn test_invalid_challenge_initial_state_root() {
    let (mut runner, _, bonded_challenger, expected_reward) = setup_with_wrong_attestation();
    let bonded_challenger_address = bonded_challenger.user_info.address();

    let mut challenge_proof = runner
        .query_visible_state(|state| {
            build_challenge(
                state,
                SlotNumber::new_dangerous(1),
                bonded_challenger_address,
            )
        })
        .unwrap();

    challenge_proof.initial_state_root = StorageRoot::new([255; 32], [255; 32]);

    test_invalid_challenge_helper(
        &mut runner,
        expected_reward.0,
        &bonded_challenger,
        make_challenge_blob(challenge_proof, true, SlotNumber::new_dangerous(1)),
        SlashingReason::InvalidInitialHash,
    );
}

#[test]

fn test_invalid_challenge_transition() {
    let (mut runner, _, bonded_challenger, expected_reward) = setup_with_wrong_attestation();
    let bonded_challenger_address = bonded_challenger.user_info.address();

    let mut challenge_proof = runner
        .query_visible_state(|state| {
            build_challenge(
                state,
                SlotNumber::new_dangerous(1),
                bonded_challenger_address,
            )
        })
        .unwrap();

    challenge_proof.slot_hash = MockHash([255; 32]);

    test_invalid_challenge_helper(
        &mut runner,
        expected_reward.0,
        &bonded_challenger,
        make_challenge_blob(challenge_proof, true, SlotNumber::new_dangerous(1)),
        SlashingReason::TransitionInvalid,
    );
}

#[test]
fn test_invalid_challenge_proof() {
    let (mut runner, _, bonded_challenger, expected_reward) = setup_with_wrong_attestation();
    let bonded_challenger_address = bonded_challenger.user_info.address();

    let challenge_proof = runner
        .query_visible_state(|state| {
            build_challenge(
                state,
                SlotNumber::new_dangerous(1),
                bonded_challenger_address,
            )
        })
        .unwrap();

    test_invalid_challenge_helper(
        &mut runner,
        expected_reward.0,
        &bonded_challenger,
        make_challenge_blob(challenge_proof, false, SlotNumber::new_dangerous(1)),
        SlashingReason::InvalidZkProof,
    );
}
