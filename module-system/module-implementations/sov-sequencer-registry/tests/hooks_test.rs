use sov_modules_api::hooks::ApplyBlobHooks;
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

use helpers::*;
use sov_modules_api::Address;
use sov_rollup_interface::mocks::TestBlob;
use sov_sequencer_registry::SequencerOutcome;

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
        let mut test_blob = TestBlob::new(
            Vec::new(),
            Address::from(GENESIS_SEQUENCER_DA_ADDRESS),
            [0_u8; 32],
        );

        test_sequencer
            .registry
            .begin_blob_hook(&mut test_blob, working_set)
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
            .end_blob_hook(SequencerOutcome::Completed, working_set)
            .unwrap();
        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE, resp.data.unwrap().balance);
    }

    // Unknown sequencer
    {
        let mut test_blob = TestBlob::new(
            Vec::new(),
            Address::from(UNKNOWN_SEQUENCER_DA_ADDRESS),
            [0_u8; 32],
        );

        let result = test_sequencer
            .registry
            .begin_blob_hook(&mut test_blob, working_set);
        assert!(result.is_err());
        assert_eq!("Invalid next sequencer.", result.err().unwrap().to_string());
    }
}

// TODO: Last sequencer exit
// TODO: Genesis with low balance
