use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::optimistic::Attestation;
use sov_state::{ProverStorage, WorkingSet};

use crate::call::AttesterIncentiveErrors;
use crate::tests::helpers::{
    execution_simulation, setup, BOND_AMOUNT, DEFAULT_ROLLUP_FINALITY, INIT_HEIGHT,
};

#[test]
fn test_two_phase_unbonding() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, token_address, attester_address, _) = setup(&mut working_set);

    // Assert that the attester has the correct bond amount before processing the proof
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        BOND_AMOUNT
    );

    let context = DefaultContext {
        sender: attester_address,
    };

    // Try to skip the first phase of the two phase unbonding. Should fail
    {
        // Should fail
        let err = module
            .end_unbond_attester(&context, &mut working_set)
            .unwrap_err();
        assert_eq!(err, AttesterIncentiveErrors::AttesterIsNotUnbonding);
    }

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (mut exec_vars, mut working_set) =
        execution_simulation(3, &module, &storage, attester_address, working_set);

    // Start unbonding and then try to prove a transition. User slashed
    module
        .begin_unbond_attester(&context, &mut working_set)
        .expect("Should succeed");

    let _transition_2 = exec_vars.pop().unwrap();
    let transition_1 = exec_vars.pop().unwrap();
    let initial_transition = exec_vars.pop().unwrap();

    // Process a valid attestation but get slashed because the attester was trying to unbond.
    {
        let attestation = Attestation {
            initial_state_root: initial_transition.state_root,
            da_block_hash: [1; 32],
            post_state_root: transition_1.state_root,
            proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
                claimed_transition_num: INIT_HEIGHT + 1,
                proof: initial_transition.state_proof,
            },
        };

        let err = module
            .process_attestation(&context, attestation, &mut working_set)
            .unwrap_err();

        assert_eq!(
            err,
            AttesterIncentiveErrors::UserNotBonded,
            "The attester should not be bonded"
        );

        // We cannot try to bond either
        let err = module
            .bond_user_helper(
                BOND_AMOUNT,
                &attester_address,
                crate::call::Role::Attester,
                &mut working_set,
            )
            .unwrap_err();

        assert_eq!(
            err,
            AttesterIncentiveErrors::AttesterIsUnbonding,
            "Should raise an AttesterIsUnbonding error"
        );
    }

    // Cannot bond again while unbonding
    {
        let err = module
            .bond_user_helper(
                BOND_AMOUNT,
                &attester_address,
                crate::call::Role::Attester,
                &mut working_set,
            )
            .unwrap_err();

        assert_eq!(
            err,
            AttesterIncentiveErrors::AttesterIsUnbonding,
            "Should raise that error"
        );
    }

    // Now try to complete the two phase unbonding immediately: the second phase should fail because the
    // first phase cannot get finalized
    {
        // Should fail
        let err = module
            .end_unbond_attester(&context, &mut working_set)
            .unwrap_err();
        assert_eq!(err, AttesterIncentiveErrors::UnbondingNotFinalized);
    }

    // Now unbond the right way.
    {
        let initial_account_balance = module
            .bank
            .get_balance_of(attester_address, token_address, &mut working_set)
            .unwrap();

        // Start unbonding the user: should succeed
        module
            .begin_unbond_attester(&context, &mut working_set)
            .unwrap();

        let unbonding_info = module
            .unbonding_attesters
            .get(&attester_address, &mut working_set)
            .unwrap();

        assert_eq!(
            unbonding_info.unbonding_initiated_height, INIT_HEIGHT,
            "Invalid beginning unbonding height"
        );

        // Wait for the light client to finalize
        module
            .light_client_finalized_height
            .set(&(INIT_HEIGHT + DEFAULT_ROLLUP_FINALITY), &mut working_set);

        // Finish the unbonding: should succeed
        module
            .end_unbond_attester(&context, &mut working_set)
            .unwrap();

        // Check that the final balance is the same as the initial balance
        assert_eq!(
            initial_account_balance + BOND_AMOUNT,
            module
                .bank
                .get_balance_of(attester_address, token_address, &mut working_set)
                .unwrap(),
            "The initial and final account balance don't match"
        );
    }
}
