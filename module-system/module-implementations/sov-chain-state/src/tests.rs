use std::marker::PhantomData;

use sov_bank::TotalSupplyResponse;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, DispatchCall, Spec};
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::mocks::{MockZkvm, TestBlob, TestValidityCond};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_state::{DefaultStorageSpec, ProverStorage, Storage, WorkingSet, ZkStorage};

use crate::tests_helpers::{
    create_demo_genesis_config, generate_address, mint_token_tx, transfer_token_tx, TestRuntime,
};
use crate::{call, ChainState};

type C = DefaultContext;

/// This test generates a new mock rollup having a simple bank module
/// with an associated chain state, and checks that the height, the genesis hash
/// and the state transitions are correctly stored and updated.
#[test]
fn test_value_setter() {
    // Build an app template with the module configurations
    let runtime = TestRuntime::default();

    let tmpdir = tempfile::tempdir().unwrap();

    let mut storage: ProverStorage<sov_state::DefaultStorageSpec> =
        ProverStorage::with_path(tmpdir.path()).unwrap();

    let app_template = AppTemplate::<
        C,
        TestRuntime<C>,
        MockZkvm,
        TestValidityCond,
        TestBlob<<DefaultContext as Spec>::Address>,
    > {
        current_storage: storage,
        runtime,
        checkpoint: None,
        phantom_vm: PhantomData,
        phantom_cond: PhantomData,
        phantom_blob: PhantomData,
    };

    // Genesis
    let init_root_hash = app_template
        .init_chain(create_demo_genesis_config())
        .unwrap();

    let initial_balance = 1000;
    let salt = 10;
    let token_name = "Token1".to_owned();

    let (token_address, mint_message) =
        mint_token_tx(token_name, initial_balance, salt, app_template.runtime.bank);

    // Helpers
    app_template
        .runtime
        .dispatch_call(mint_message, &mut working_set, context)
        .expect("Failed to mint token");

    let transfer_message = transfer_token_tx(
        initial_balance,
        sender_address,
        receiver_address,
        token_address,
    );

    let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);
    let blob = new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS);

    let mut blobs = [blob];

    let data = TestBlock::default();

    let result = StateTransitionFunction::<MockZkvm, TestBlob>::apply_slot(
        &mut demo,
        Default::default(),
        &data,
        &mut blobs,
    );

    assert_eq!(1, result.batch_receipts.len());
    let apply_blob_outcome = result.batch_receipts[0].clone();
    assert_eq!(
        SequencerOutcome::Rewarded(0),
        apply_blob_outcome.inner,
        "Sequencer execution should have succeeded but failed "
    );

    assert!(has_tx_events(&apply_blob_outcome),);

    // No events at the moment. If there are, needs to be checked
    bank.call(transfer_message, &sender_context, &mut working_set)
        .expect("Transfer call failed");
    assert!(working_set.events().is_empty());

    assert_eq!(
        Some(initial_balance - transfer_amount),
        sender_balance_after
    );
    assert_eq!(Some(transfer_amount), receiver_balance_after);
    let total_supply_after = query_total_supply(&mut working_set);
    assert_eq!(total_supply_before, total_supply_after);

    // Generate a new storage instance after dumping data to the db.
    {
        let runtime = &mut Runtime::<DefaultContext>::default();
        let storage = ProverStorage::with_path(path).unwrap();
        let mut working_set = WorkingSet::new(storage);

        let resp = runtime.election.results(&mut working_set).unwrap();

        assert_eq!(
            resp,
            sov_election::GetResultResponse::Result(Some(sov_election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        );
        let resp = runtime.value_setter.query_value(&mut working_set).unwrap();

        assert_eq!(resp, sov_value_setter::Response { value: Some(33) });
    }
}
