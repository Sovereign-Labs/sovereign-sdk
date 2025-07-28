use sov_state::nomt::zk_storage::NomtVerifierStorage;
use sov_state::{ArrayWitness, NodeLeaf, SlotKey, SlotValue, Storage};
use sov_test_utils::storage::SimpleNomtStorageManager;
use sov_test_utils::TestHasher;

use crate::state_tests::compute_state_update::{run_test, ForklessStorageManager, TestCase};
use crate::state_tests::StorageSpec;

fn run_nomt_test(test_case: TestCase) {
    let sm = SimpleNomtStorageManager::new();
    run_test(test_case, sm, NomtVerifierStorage::<StorageSpec>::new());
}

#[test]
fn test_roundtrip_nomt() {
    run_nomt_test(TestCase::single_write());
    run_nomt_test(TestCase::single_write_both_namespaces());
    run_nomt_test(TestCase::single_read_write_different_key());
    run_nomt_test(TestCase::single_read_write_same_key());
    run_nomt_test(TestCase::rounds_of_same_key());
}

/// Add a new read to the first round.
/// This way nomt witness will not have the proof for this read, so it will be considered missed.
#[test]
fn test_missing_reads() {
    let alien_key = SlotKey::from_slice(b"key_alien");
    let alien_value = SlotValue::from("value_alien");

    let native_test_case = TestCase::rounds_of_same_key();
    let mut zk_test_case = TestCase::rounds_of_same_key();

    let malformed_round = zk_test_case.rounds.get_mut(0).unwrap();
    malformed_round.kernel.ordered_reads.push((
        alien_key,
        // Note: we are testing some value, because reading none value is similar as not reading at all,
        // because absence will be proven correct.
        Some(NodeLeaf::make_leaf::<TestHasher>(&alien_value)),
    ));

    check_malicious_case(
        native_test_case,
        zk_test_case,
        "Failed to verify inclusion of key",
    );
}

/// Remove reads in some of the round.
/// This way passed nomt witness will contain proof for extra read.
#[test]
fn test_modified_read() {
    let alien_value = SlotValue::from("value_alien");
    let native_test_case = TestCase::rounds_of_same_key();
    let mut zk_test_case = TestCase::rounds_of_same_key();

    let malformed_round = zk_test_case.rounds.get_mut(2).unwrap();
    malformed_round.kernel.ordered_reads.get_mut(0).unwrap().1 =
        Some(NodeLeaf::make_leaf::<TestHasher>(&alien_value));

    check_malicious_case(
        native_test_case,
        zk_test_case,
        "Failed to verify inclusion of key",
    );
}

#[test]
fn test_modified_read_to_none() {
    let native_test_case = TestCase::rounds_of_same_key();
    let mut zk_test_case = TestCase::rounds_of_same_key();

    let malformed_round = zk_test_case.rounds.get_mut(2).unwrap();
    malformed_round.kernel.ordered_reads.get_mut(0).unwrap().1 = None;

    check_malicious_case(
        native_test_case,
        zk_test_case,
        "Failed to verify non-existence of key",
    );
}

/// Crafting malicious witness from scratch is tedious.
/// To emulate it, we use "inverse approach".
/// We modify build normal nomt witness, but ZK receives a modified version of state accesses.
/// For example,
///  - Extra value in state accesses means absent proof
///  - Missing value in state accesses means extra proof.
///
/// Note:
///  - we don't test extra or missing writes, because all writes originate from within ZKVM
///  - we don't test extra proof for reads, because zk guest only cares about reads it made.
fn check_malicious_case(native_case: TestCase, zk_case: TestCase, expected_error: &str) {
    let mut sm = SimpleNomtStorageManager::new();

    for (native_state_accesses, zk_state_accesses) in native_case
        .rounds
        .into_iter()
        .zip(zk_case.rounds.into_iter())
    {
        let (prover_storage, prev_state_root) = sm.create_storage_with_root();

        let witness = ArrayWitness::default();

        let (native_root, change_set) = prover_storage
            .compute_state_update(native_state_accesses, &witness, prev_state_root)
            .expect("state update computation must succeed");

        let zk_storage = NomtVerifierStorage::<StorageSpec>::new();

        match zk_storage.compute_state_update(zk_state_accesses, &witness, prev_state_root) {
            Ok((zk_root, _)) => {
                // If the update is correct, do normal operations.
                // This allows having a more sophisticated error case to be detected.
                assert_eq!(native_root.as_ref(), zk_root.as_ref());
                sm.commit_state_update(prover_storage, change_set, native_root);
            }
            Err(err) => {
                let error_message = err.to_string();
                assert!(
                    error_message.contains(expected_error),
                    "Error message does not contain expected text. Error: {error_message}, expected pattern: '{expected_error}'"
                );
                return;
            }
        }
    }
    panic!("No error has been detected");
}
