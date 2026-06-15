//! Tests for the non-admin proof submission path in [`ProverIncentives`].
//!
//! These exercise the slashing checks that the admin role bypasses: when
//! `prover_incentives.admin` is unset, the sequencer who submits proofs is
//! treated like any other prover and is slashed on output mismatches.

use sov_modules_api::{CodeCommitmentHash, InvalidProofError, ProofOutcome};
use sov_rollup_interface::common::IntoSlotNumber;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{assert_matches, ProofInput, ProofTestCase, TestProver, TestSpec, TestUser};
use sov_value_setter::ValueSetterConfig;

use crate::helpers::{
    build_proof, consume_gas_tx_for_signer, serialize_proof, TestProverIncentives, RT,
};

type S = TestSpec;

fn setup_without_admin() -> (TestRunner<RT, S>, TestProver<S>, TestUser<S>) {
    let high_level = HighLevelZkGenesisConfig::<S>::generate_with_additional_accounts(1);
    let unbonded_user = high_level.additional_accounts().first().unwrap().clone();
    let prover = high_level.initial_prover.clone();

    let mut minimal: MinimalZkGenesisConfig<S> = high_level.into();
    minimal.config.prover_incentives.admin = None;

    let genesis_config = crate::helpers::GenesisConfig::from_minimal_config(
        minimal,
        ValueSetterConfig {
            admin: prover.user_info.address(),
        },
    );
    let runner = TestRunner::new_with_genesis(
        genesis_config.into_genesis_params(),
        crate::helpers::TestRuntime::default(),
    );

    (runner, prover, unbonded_user)
}

/// A proof whose `origin_state_root` differs from chain_state's genesis hash
/// must be slashed for `IncorrectGenesisHash`, and no admin upgrade entry
/// should be recorded.
#[test]
fn test_non_admin_invalid_genesis_hash_slashed() {
    let (mut runner, prover, other_user) = setup_without_admin();
    for _ in 0..3 {
        runner.execute(consume_gas_tx_for_signer(&other_user));
    }
    let mut aggregated_proof = runner
        .query_visible_state(|state| {
            build_proof(
                state,
                1.to_slot_number(),
                2.to_slot_number(),
                prover.user_info.address(),
            )
        })
        .unwrap();

    aggregated_proof
        .origin_state_root
        .clone_from(&aggregated_proof.final_state_root);

    let prover_address = prover.user_info.address();
    let final_slot = aggregated_proof.final_slot_number;

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                &result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(e, _) if matches!(
                    e,
                    InvalidProofError::ProverSlashed(reason)
                        if reason == "Invalid output IncorrectGenesisHash"
                )
            );

            let module = TestProverIncentives::default();
            assert!(
                module
                    .bonded_provers
                    .get(&prover_address, state)
                    .unwrap()
                    .is_none(),
                "non-admin should be removed from bonded_provers on slash",
            );
            assert_eq!(
                module.latest_admin_upgrade_slot.get(state).unwrap(),
                None,
                "no upgrade should be recorded when the sender is not the admin",
            );
            assert!(module
                .admin_upgrades
                .get(&final_slot, state)
                .unwrap()
                .is_none());
        }),
    });
}

/// A proof whose `inner_vkey_hash` differs from chain_state's inner code
/// commitment must be slashed for `IncorrectInnerVkeyHash`.
#[test]
fn test_non_admin_invalid_inner_vkey_hash_slashed() {
    let (mut runner, prover, other_user) = setup_without_admin();
    for _ in 0..3 {
        runner.execute(consume_gas_tx_for_signer(&other_user));
    }
    let mut aggregated_proof = runner
        .query_visible_state(|state| {
            build_proof(
                state,
                1.to_slot_number(),
                2.to_slot_number(),
                prover.user_info.address(),
            )
        })
        .unwrap();

    aggregated_proof.inner_vkey_hash = {
        let mut bytes = [0u8; CodeCommitmentHash::HASH_LEN];
        bytes[..8].copy_from_slice(&[0xab; 8]);
        CodeCommitmentHash::from_u8_array(bytes)
    };
    let prover_address = prover.user_info.address();

    runner.execute_proof::<TestProverIncentives>(ProofTestCase {
        input: ProofInput(serialize_proof(aggregated_proof)),
        assert: Box::new(move |result, state| {
            assert_matches!(
                &result.proof_receipt.unwrap().outcome,
                ProofOutcome::Invalid(e, _) if matches!(
                    e,
                    InvalidProofError::ProverSlashed(reason)
                        if reason == "Invalid output IncorrectInnerVkeyHash"
                )
            );

            let module = TestProverIncentives::default();
            assert!(module
                .bonded_provers
                .get(&prover_address, state)
                .unwrap()
                .is_none());
        }),
    });
}
