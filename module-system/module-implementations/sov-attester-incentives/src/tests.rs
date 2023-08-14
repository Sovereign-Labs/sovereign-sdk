use sov_chain_state::{StateTransitionId, TransitionInProgress};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::SlotHooks;
use sov_rollup_interface::mocks::{
    TestBlock, TestBlockHeader, TestHash, TestValidityCond, TestValidityCondChecker,
};
use sov_rollup_interface::optimistic::Attestation;
use sov_rollup_interface::services::da::SlotData;
use sov_state::storage::{StorageKey, StorageProof};
use sov_state::{ArrayWitness, ProverStorage, Storage, WorkingSet};

use crate::helpers::{
    commit_get_new_working_set, setup, BOND_AMOUNT, INITIAL_BOND_AMOUNT, INIT_HEIGHT,
};

const DEFAULT_MAX_LIGHT_CLIENT_HEIGHT: u64 = 6;

/// Start by testing the positive case where the attestations are valid
#[test]
fn test_process_valid_attestation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage.clone());
    let (module, attester_address, _) =
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

    // Commit the working set
    let mut working_set = commit_get_new_working_set(&storage, working_set);

    // First get the bond proof that the attester was bonded at genesis.
    let proof_genesis =
        module.get_bond_proof(attester_address, &ArrayWitness::default(), &mut working_set);

    // Then process the first transaction. Only sets the genesis hash and a transition in progress.
    let slot_data = TestBlock {
        curr_hash: [1; 32],
        header: TestBlockHeader {
            prev_hash: TestHash([0; 32]),
        },
        height: INIT_HEIGHT + 1,
        validity_cond: TestValidityCond { is_valid: true },
    };
    module
        .chain_state
        .begin_slot_hook(&slot_data, &mut working_set);

    // Commit the working set
    let mut working_set = commit_get_new_working_set(&storage, working_set);

    // Get bond proof that the attester was bonded after first transition
    let proof_transition_1 =
        module.get_bond_proof(attester_address, &ArrayWitness::default(), &mut working_set);

    // Then process the next transition. Store the first transition and a new transition in progress.
    let slot_data = TestBlock {
        curr_hash: [2; 32],
        header: TestBlockHeader {
            prev_hash: TestHash([1; 32]),
        },
        height: INIT_HEIGHT + 2,
        validity_cond: TestValidityCond { is_valid: true },
    };
    module
        .chain_state
        .begin_slot_hook(&slot_data, &mut working_set);

    // Commit the working set
    let mut working_set = commit_get_new_working_set(&storage, working_set);

    // Process one last transition so that we can store the slot data.
    let slot_data = TestBlock {
        curr_hash: [3; 32],
        header: TestBlockHeader {
            prev_hash: TestHash([2; 32]),
        },
        height: INIT_HEIGHT + 3,
        validity_cond: TestValidityCond { is_valid: true },
    };
    module
        .chain_state
        .begin_slot_hook(&slot_data, &mut working_set);

    // Commit the working set
    let mut working_set = commit_get_new_working_set(&storage, working_set);

    // Get the roots of the transitions
    let initial_state_root = module
        .chain_state
        .get_genesis_hash(&mut working_set)
        .expect("Should have a genesis hash");

    let transition_1 = module
        .chain_state
        .get_historical_transitions(INIT_HEIGHT + 1, &mut working_set)
        .unwrap();

    let root_transition_1 = transition_1.post_state_root();

    let transition_2 = module
        .chain_state
        .get_historical_transitions(INIT_HEIGHT + 2, &mut working_set)
        .unwrap();

    let root_transition_2 = transition_2.post_state_root();

    // Process a valid attestation for the first transition
    {
        let context = DefaultContext {
            sender: attester_address,
        };

        let attestation = Attestation {
            initial_state_root,
            da_block_hash: [1; 32],
            post_state_root: root_transition_1,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 1,
                proof: proof_genesis,
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
            initial_state_root: root_transition_1,
            da_block_hash: [2; 32],
            post_state_root: root_transition_2,
            proof_of_bond: sov_rollup_interface::optimistic::ProofOfBond {
                transition_num: INIT_HEIGHT + 2,
                proof: proof_transition_1,
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
}

#[test]
fn test_burn_on_invalid_proof() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let (module, attester_address, _challenger_address) =
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
    let (module, attester_address, _challenger_address) =
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
    let (module, attester_address, _challenger_address) =
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
    let (module, attester_address, _challenger_address) =
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
