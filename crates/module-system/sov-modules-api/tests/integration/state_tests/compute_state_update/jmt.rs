// This is a reference point to validate that there are no errors in test

use sov_state::ZkStorage;
use sov_test_utils::storage::SimpleJmtStorageManager;

use crate::state_tests::compute_state_update::{run_test, TestCase};
use crate::state_tests::StorageSpec;

fn run_jmt_test(test_case: TestCase) {
    let mut sm = SimpleJmtStorageManager::new();
    sm.genesis();
    run_test(test_case, sm, ZkStorage::<StorageSpec>::new());
}

#[test]
fn test_roundtrip_jmt() {
    run_jmt_test(TestCase::single_write());
    run_jmt_test(TestCase::single_write_both_namespaces());
    run_jmt_test(TestCase::single_read_write_different_key());
    run_jmt_test(TestCase::single_read_write_same_key());
    run_jmt_test(TestCase::rounds_of_same_key());
    run_jmt_test(TestCase::multi_write_distinct_keys());
    run_jmt_test(TestCase::multi_write_both_namespaces_distinct());
    run_jmt_test(TestCase::mixed_reads_and_multi_writes());
    run_jmt_test(TestCase::multi_round_multi_write());
}
