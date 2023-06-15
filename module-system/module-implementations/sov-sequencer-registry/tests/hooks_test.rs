use sov_modules_api::hooks::ApplyBlobHooks;
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

use helpers::*;

#[test]
fn test_sequencer() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    {
        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE, resp.data.unwrap().balance);
    }

    // Lock
    {
        test_sequencer
            .registry
            .begin_blob_hook(&GENESIS_SEQUENCER_DA_ADDRESS, &[], working_set)
            .unwrap();

        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, resp.data.unwrap().balance);
    }

    // Reward
    {
        test_sequencer
            .registry
            .end_blob_hook(0, working_set)
            .unwrap();
        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE, resp.data.unwrap().balance);
    }

    // Unknown sequencer
    {
        let result = test_sequencer.registry.begin_blob_hook(
            &UNKNOWN_SEQUENCER_DA_ADDRESS,
            &[],
            working_set,
        );
        assert!(result.is_err());
        assert_eq!("Invalid next sequencer.", result.err().unwrap().to_string());
    }
}

// TODO: Last sequencer exit
// TODO: Genesis with low balance
