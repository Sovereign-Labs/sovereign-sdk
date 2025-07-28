use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, InvalidProofError, ProofOutcome, SovAttestation};
use sov_rollup_interface::common::IntoSlotNumber;
use sov_state::StorageRoot;
use sov_test_utils::runtime::sov_attester_incentives::{AttesterIncentives, CallMessage, Event};
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    assert_matches, AsUser, AtomicAmount, ProofInput, ProofTestCase, TestAttester,
    TransactionTestCase,
};

use super::helpers::{setup, TestRuntimeEvent, S};
use crate::helpers::{
    build_proof, consume_gas_tx_for_signer, create_test_case, make_attestation_blob,
    TestAttesterIncentives, RT,
};

#[test]
fn test_process_valid_attestation() {
    let nb_tests = 3;
    let (mut runner, genesis_attester, _, other_user) = setup();

    let attester_address = genesis_attester.user_info.address();
    let mut rewards = Vec::with_capacity(nb_tests);

    for _ in 0..nb_tests {
        let reward = AtomicAmount::new(Amount::ZERO);
        let reward_clone = reward.clone();
        runner.execute_transaction(TransactionTestCase {
            input: consume_gas_tx_for_signer(&other_user),
            assert: Box::new(move |result, _state| {
                reward_clone.add(result.gas_value_used);
            }),
        });
        rewards.push(reward);
    }

    runner.execute(consume_gas_tx_for_signer(&other_user));

    let mut runner = runner.advance_slots(nb_tests);

    // Submit the attestations
    for (i, reward) in rewards.into_iter().enumerate() {
        let initial_balance = runner
            .query_visible_state(|state| {
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state)
            })
            .unwrap();

        let attestation_proof = runner
            .query_visible_state(|state| build_proof(state, (i + 1) as u64, &attester_address))
            .unwrap();

        let attest_slot = create_test_case(
            genesis_attester.clone(),
            make_attestation_blob(attestation_proof),
            initial_balance,
            reward,
        );

        runner = runner.execute_proof::<TestAttesterIncentives>(attest_slot);
    }
}

#[test]
fn test_burn_on_invalid_attestation() {
    let (mut runner, genesis_attester, _, other_user) = setup();

    let reward_1 = AtomicAmount::new(Amount::ZERO);
    {
        let reward_clone = reward_1.clone();
        runner.execute_transaction(TransactionTestCase {
            input: consume_gas_tx_for_signer(&other_user),
            assert: Box::new(move |result, _state| {
                reward_clone.add(result.gas_value_used);
            }),
        });
    }

    let reward_2 = AtomicAmount::new(Amount::ZERO);
    {
        let reward_clone = reward_2.clone();
        runner.execute_transaction(TransactionTestCase {
            input: consume_gas_tx_for_signer(&other_user),
            assert: Box::new(move |result, _state| {
                reward_clone.add(result.gas_value_used);
            }),
        });
    }

    runner.execute(consume_gas_tx_for_signer(&other_user));

    let attester_address = genesis_attester.user_info.address();
    let attester_bond = genesis_attester.bond;

    // Test that the attester is not slashed when the bond is invalid.
    {
        let initial_balance = runner
            .query_visible_state(|state| {
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state)
            })
            .unwrap();

        let mut attestation_proof = runner
            .query_visible_state(|state| build_proof(state, 1, &attester_address))
            .unwrap();

        attestation_proof.proof_of_bond.claimed_slot_number = 2.to_slot_number();

        let invalid_bond_proof_no_slash =
            invalid_bond_proof_no_slash(&genesis_attester, initial_balance, attestation_proof);

        runner.execute_proof::<TestAttesterIncentives>(invalid_bond_proof_no_slash);
    }

    // Test valid attestation.
    {
        let initial_balance = runner
            .query_visible_state(|state| {
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state)
            })
            .unwrap();
        let valid_attestation = {
            let attestation_proof_2 = runner
                .query_visible_state(|state| build_proof(state, 1, &attester_address))
                .unwrap();

            create_test_case(
                genesis_attester.clone(),
                make_attestation_blob(attestation_proof_2),
                initial_balance,
                reward_1,
            )
        };
        runner.execute_proof::<TestAttesterIncentives>(valid_attestation);
    }

    // Test that the attester is slashed when the initial state is invalid.
    {
        let initial_balance = runner
            .query_visible_state(|state| {
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state)
            })
            .unwrap();

        let mut attestation_proof = runner
            .query_visible_state(|state| build_proof(state, 1, &attester_address))
            .unwrap();

        attestation_proof.initial_state_root = StorageRoot::new([255; 32], [255; 32]);

        let invalid_initial_state_slashed =
            invalid_initial_state_slashed(&genesis_attester, initial_balance, attestation_proof);

        runner.execute_proof::<TestAttesterIncentives>(invalid_initial_state_slashed);
    }

    // Rebond the attester.
    {
        let rebond_attester = {
            TransactionTestCase {
                input: genesis_attester.create_plain_message::<RT, AttesterIncentives<S>>(
                    CallMessage::RegisterAttester(attester_bond),
                ),
                assert: Box::new(move |result, state| {
                    assert!(result.events.iter().any(|event| matches!(
                        event,
                        TestRuntimeEvent::AttesterIncentives(Event::RegisteredAttester { .. })
                    )));
                    assert_eq!(
                        AttesterIncentives::<S>::default()
                            .get_attester_bond_amount(&attester_address, state)
                            .unwrap_infallible()
                            .value,
                        attester_bond,
                    );
                }),
            }
        };

        runner.execute_transaction(rebond_attester);
    }

    // Test that the attester is slashed when the post state is invalid.
    {
        let initial_balance = runner
            .query_visible_state(|state| {
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state)
            })
            .unwrap();

        let mut attestation_proof = runner
            .query_visible_state(|state| build_proof(state, 2, &attester_address))
            .unwrap();

        attestation_proof.post_state_root = StorageRoot::new([255; 32], [255; 32]);

        let invalid_post_state_root_is_challengeable = invalid_post_state_root_is_challengeable(
            &genesis_attester,
            initial_balance,
            attestation_proof,
        );

        runner.execute_proof::<TestAttesterIncentives>(invalid_post_state_root_is_challengeable);
    }
}

fn invalid_bond_proof_no_slash(
    attester: &TestAttester<S>,
    initial_balance: Amount,
    attestation_proof: SovAttestation<S>,
) -> ProofTestCase<S> {
    let attester_address = attester.user_info.address();
    let attester_bond = attester.bond;

    ProofTestCase {
        input: ProofInput(make_attestation_blob(attestation_proof)),
        assert: Box::new(move |result, state| {
            assert_eq!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(InvalidProofError::PreconditionNotMet(
                    "Invalid bonding proof".to_string()
                ))
            );

            assert_eq!(
                TestAttesterIncentives::default()
                    .bonded_attesters
                    .get(&attester_address, state)
                    .unwrap(),
                Some(attester_bond),
                "Bonded amount should not have changed"
            );

            // Attester is not rewarded
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state).unwrap(),
                initial_balance.checked_sub(result.gas_value_used).unwrap()
            );
        }),
    }
}

fn invalid_initial_state_slashed(
    attester: &TestAttester<S>,
    initial_balance: Amount,
    attestation_proof: SovAttestation<S>,
) -> ProofTestCase<S> {
    let attester_address = attester.user_info.address();
    ProofTestCase {
        input: ProofInput(make_attestation_blob(attestation_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(InvalidProofError::ProverSlashed(_))
            );

            assert!(TestAttesterIncentives::default()
                .bonded_attesters
                .get(&attester_address, state)
                .unwrap()
                .is_none());

            // Check that the invalid attestation is not part of the challengeable set.
            // (Since it has the wrong pre-state, no one will be fooled by it so we don't reward challengers)
            assert!(
                AttesterIncentives::<S>::default()
                    .bad_transition_pool
                    .get(&2.to_slot_number(), state)
                    .unwrap_infallible()
                    .is_none(),
                "The transition should not exist in the pool"
            );

            // Attester is not rewarded
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state).unwrap(),
                initial_balance.checked_sub(result.gas_value_used).unwrap()
            );
        }),
    }
}

fn invalid_post_state_root_is_challengeable(
    attester: &TestAttester<S>,
    initial_balance: Amount,
    attestation_proof: SovAttestation<S>,
) -> ProofTestCase<S> {
    let attester_address = attester.user_info.address();
    let attester_bond = attester.bond;
    ProofTestCase {
        input: ProofInput(make_attestation_blob(attestation_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(InvalidProofError::ProverSlashed(_))
            );

            assert!(TestAttesterIncentives::default()
                .bonded_attesters
                .get(&attester_address, state)
                .unwrap()
                .is_none(),);

            // The attestation should be part of the challengeable set and its associated value should be the BOND_AMOUNT
            assert_eq!(
                AttesterIncentives::<S>::default()
                    .bad_transition_pool
                    .get(&2.to_slot_number(), state)
                    .unwrap_infallible(),
                Some(attester_bond),
                "The transition should exist in the bad_transition_pool"
            );

            // Attester is not rewarded
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state).unwrap(),
                initial_balance.checked_sub(result.gas_value_used).unwrap()
            );
        }),
    }
}
