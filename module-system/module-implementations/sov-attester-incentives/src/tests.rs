
use sov_modules_api::default_context::DefaultContext;

use sov_rollup_interface::mocks::{
    TestValidityCond, TestValidityCondChecker,
};
use sov_rollup_interface::optimistic::Attestation;

use sov_state::storage::{StorageKey, StorageProof};
use sov_state::{ArrayWitness, ProverStorage, Storage, WorkingSet};

use crate::helpers::{
    execution_simulation, setup, BOND_AMOUNT, INITIAL_BOND_AMOUNT,
    INIT_HEIGHT,
};

/// Start by testing the positive case where the attestations are valid
#[test]
fn test_process_valid_attestation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, token_address, attester_address, _) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);

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

    // Simulate the execution of a chain, with the genesis hash and two transitions after.
    // Update the chain_state module and the optimistic module accordingly
    let (exec_vars, mut working_set) =
        execution_simulation(&module, &storage, attester_address, working_set);

    // Process a valid attestation for the first transition
    {
        let context = DefaultContext {
            sender: attester_address,
        };

        let attestation = Attestation {
            initial_state_root: exec_vars.initial_state_root,
            da_block_hash: [1; 32],
            post_state_root: exec_vars.transition_1_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 1,
                proof: exec_vars.initial_state_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is an error");
    }

    // We can now proceed with the next attestation
    {
        let context = DefaultContext {
            sender: attester_address,
        };

        let attestation = Attestation {
            initial_state_root: exec_vars.transition_1_root,
            da_block_hash: [2; 32],
            post_state_root: exec_vars.transition_2_root,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 2,
                proof: exec_vars.transition_1_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is an error");
    }

    // Assert that the attester's bond amount has not been burned
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

    // Assert that the attester has been awarded the tokens
    assert_eq!(
        module
            .bank
            .get_balance_of(attester_address, token_address, &mut working_set)
            .unwrap(),
        // The attester is bonded at the beginning so he loses BOND_AMOUNT
        INITIAL_BOND_AMOUNT - BOND_AMOUNT + 2 * BOND_AMOUNT
    );

    assert_eq!(
        module
            .bank
            .get_balance_of(
                module.get_reward_token_supply_address(&mut working_set),
                token_address,
                &mut working_set
            )
            .unwrap(),
        INITIAL_BOND_AMOUNT - 2 * BOND_AMOUNT
    );
}

#[test]
fn test_burn_on_invalid_attestation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, _token_address, attester_address, _challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);

    // Assert that the prover has the correct bond amount before processing the proof
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

    // Process an invalid proof
    {
        let context = DefaultContext {
            sender: attester_address,
        };

        let proof =
            module.get_bond_proof(attester_address, &ArrayWitness::default(), &mut working_set);

        let storage_proof = StorageProof {
            key: StorageKey::new(module.bonded_attesters.prefix(), &attester_address),
            value: Some(INITIAL_BOND_AMOUNT.to_le_bytes().to_vec().into()),
            proof: proof.proof,
        };

        let attestation = Attestation {
            initial_state_root: [0; 32],
            da_block_hash: [0; 32],
            post_state_root: [0; 32],
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: 0,
                proof: storage_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is not an error");
    }

    // Assert that the prover's bond amount has been burned
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        0
    );
}

#[test]
fn test_valid_proof() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, _token_address, attester_address, _challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);

    // Assert that the prover has the correct bond amount before processing the proof
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

    // Process a valid proof
    {
        let context = DefaultContext {
            sender: attester_address,
        };

        let proof =
            module.get_bond_proof(attester_address, &ArrayWitness::default(), &mut working_set);

        let storage_proof = StorageProof {
            key: StorageKey::new(module.bonded_attesters.prefix(), &attester_address.clone()),
            value: Some(INITIAL_BOND_AMOUNT.to_le_bytes().to_vec().into()),
            proof: proof.proof,
        };

        let attestation = Attestation {
            initial_state_root: [0; 32],
            da_block_hash: [0; 32],
            post_state_root: [0; 32],
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: 0,
                proof: storage_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is not an error");
    }

    // Assert that the prover's bond amount has not been burned
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
}

#[test]
fn test_unbonding() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, _token_address, attester_address, _challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);
    let context = DefaultContext {
        sender: attester_address,
    };
    let _token_address = module
        .bonding_token_address
        .get(&mut working_set)
        .expect("bonding token address was set at genesis");

    // Assert that the prover has bonded tokens
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

    // Get their *unlocked* balance before undbonding
    // let initial_unlocked_balance = {
    //     module
    //         .bank
    //         .get_balance_of(
    //             attester_address.clone(),
    //             token_address.clone(),
    //             &mut working_set,
    //         )
    //         .unwrap_or_default()
    // };

    // Unbond the prover
    module
        .unbond_challenger(&context, &mut working_set)
        .expect("Unbonding should succeed");

    // Assert that the prover no longer has bonded tokens
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        0
    );

    // Assert that the prover's unlocked balance has increased by the amount they unbonded
    // let unlocked_balance =
    //     module
    //         .bank
    //         .get_balance_of(attester_address, token_address, &mut working_set);
    // assert_eq!(
    //     unlocked_balance,
    //     Some(BOND_AMOUNT + initial_unlocked_balance)
    // );
}

#[test]
fn test_prover_not_bonded() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, _token_address, attester_address, _challenger_address) =
        setup::<TestValidityCond, TestValidityCondChecker<TestValidityCond>>(&mut working_set);
    let context = DefaultContext {
        sender: attester_address,
    };

    // Unbond the prover
    module
        .unbond_challenger(&context, &mut working_set)
        .expect("Unbonding should succeed");

    // Assert that the prover no longer has bonded tokens
    assert_eq!(
        module
            .get_bond_amount(
                attester_address,
                crate::call::Role::Attester,
                &mut working_set
            )
            .value,
        0
    );

    // Process a valid proof
    {
        let proof =
            module.get_bond_proof(attester_address, &ArrayWitness::default(), &mut working_set);

        let storage_proof = StorageProof {
            key: StorageKey::new(module.bonded_attesters.prefix(), &attester_address),
            value: Some(INITIAL_BOND_AMOUNT.to_le_bytes().to_vec().into()),
            proof: proof.proof,
        };

        let attestation = Attestation {
            initial_state_root: [0; 32],
            da_block_hash: [0; 32],
            post_state_root: [0; 32],
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: 0,
                proof: storage_proof,
            },
        };

        module
            .process_attestation(attestation, &context, &mut working_set)
            .expect("An invalid proof is not an error");
    }
}
