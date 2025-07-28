use sov_modules_api::{
    AggregatedProofPublicData, ApiStateAccessor, InvalidProofError, ProofOutcome, Spec,
};
use sov_rollup_interface::common::IntoSlotNumber;
use sov_state::Storage;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    assert_matches, ProofAssertContext, ProofInput, ProofTestCase, TestProver, TestSpec, TestUser,
};

use crate::helpers::{
    build_proof, consume_gas_tx_for_signer, serialize_proof, setup, TestProverIncentives, RT,
};

type S = TestSpec;

fn assert_slashed(
    context: ProofAssertContext<S>,
    state: &mut ApiStateAccessor<S>,
    prover: &TestUser<S>,
    slash_reason: &str,
) {
    assert_matches!(
        &context.proof_receipt.unwrap().outcome,
        ProofOutcome::Invalid(e) if matches!(e, InvalidProofError::ProverSlashed(s) if s == slash_reason)
    );
    assert!(TestProverIncentives::default()
        .bonded_provers
        .get(&prover.address(), state)
        .unwrap()
        .is_none());
}

#[allow(clippy::type_complexity)]
fn prepare_for_slashing() -> (
    TestRunner<RT, S>,
    TestProver<S>,
    AggregatedProofPublicData<
        <S as Spec>::Address,
        <S as Spec>::Da,
        <<S as Spec>::Storage as Storage>::Root,
    >,
) {
    let (mut runner, prover, other_user) = setup();

    for _ in 0..3 {
        // execute some transactions that will generate gas to reward the prover
        runner.execute(consume_gas_tx_for_signer(&other_user));
    }

    let aggregated_proof = runner
        .query_visible_state(|state| {
            build_proof(
                state,
                1.to_slot_number(),
                2.to_slot_number(),
                prover.user_info.address(),
            )
        })
        .unwrap();

    (runner, prover, aggregated_proof)
}

#[test]
fn test_invalid_proof_slashed() {
    let (mut runner, prover, _) = setup();

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(())),
        assert: Box::new(move |result, state| {
            assert_slashed(result, state, &prover.user_info, "Verification failed");
        }),
    });
}

#[test]
fn test_invalid_genesis_hash_slashed() {
    let (mut runner, prover, mut aggregated_proof) = prepare_for_slashing();
    aggregated_proof
        .genesis_state_root
        .clone_from(&aggregated_proof.final_state_root);

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_slashed(
                result,
                state,
                &prover.user_info,
                "Invalid output IncorrectGenesisHash",
            );
        }),
    });
}

#[test]
fn test_invalid_initial_state_root() {
    let (mut runner, prover, mut aggregated_proof) = prepare_for_slashing();
    aggregated_proof
        .initial_state_root
        .clone_from(&aggregated_proof.final_state_root);

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_slashed(
                result,
                state,
                &prover.user_info,
                "Invalid output IncorrectInitialStateRoot",
            );
        }),
    });
}

#[test]
fn test_invalid_final_slot_hash() {
    let (mut runner, prover, mut aggregated_proof) = prepare_for_slashing();
    aggregated_proof
        .final_slot_hash
        .clone_from(&aggregated_proof.initial_slot_hash);

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_slashed(
                result,
                state,
                &prover.user_info,
                "Invalid output IncorrectFinalSlotHash",
            );
        }),
    });
}

#[test]
fn test_invalid_final_state_root() {
    let (mut runner, prover, mut aggregated_proof) = prepare_for_slashing();
    aggregated_proof
        .final_state_root
        .clone_from(&aggregated_proof.initial_state_root);

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_slashed(
                result,
                state,
                &prover.user_info,
                "Invalid output IncorrectFinalStateRoot",
            );
        }),
    });
}

#[test]
fn test_invalid_initial_slot_hash() {
    let (mut runner, prover, mut aggregated_proof) = prepare_for_slashing();
    aggregated_proof
        .initial_slot_hash
        .clone_from(&aggregated_proof.final_slot_hash);

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_slashed(
                result,
                state,
                &prover.user_info,
                "Invalid output IncorrectInitialSlotHash",
            );
        }),
    });
}

#[test]
fn test_invalid_initial_slot_number() {
    let (mut runner, prover, mut aggregated_proof) = prepare_for_slashing();
    aggregated_proof.initial_slot_number = 5555.to_slot_number();

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_slashed(
                result,
                state,
                &prover.user_info,
                "Invalid output InitialTransitionDoesNotExist",
            );
        }),
    });
}

#[test]
fn test_invalid_final_slot_number() {
    let (mut runner, prover, mut aggregated_proof) = prepare_for_slashing();
    aggregated_proof.final_slot_number = 6.to_slot_number();

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_slashed(
                result,
                state,
                &prover.user_info,
                "Invalid output FinalTransitionDoesNotExist",
            );
        }),
    });
}
