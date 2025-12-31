use futures::StreamExt;
use sov_db::ledger_db::{LedgerDb, SlotCommit};
use sov_db::schema::types::StoredStfInfo;
use sov_mock_da::{MockAddress, MockBlob, MockBlock, MockDaSpec, MockHash};
use sov_mock_zkvm::MockZkvmHost;
use sov_rollup_interface::common::{IntoSlotNumber, SlotNumber};
use sov_rollup_interface::node::ledger_api::LedgerStateProvider;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, CodeCommitment, SerializedAggregatedProof,
};
use sov_test_utils::ledger_db::sov_api_spec::types::IntOrHash;
use sov_test_utils::ledger_db::{LedgerTestService, LedgerTestServiceData};
use sov_test_utils::storage::SimpleLedgerStorageManager;

#[tokio::test(flavor = "multi_thread")]
async fn get_filtered_slot_events() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::Simple)
        .await
        .unwrap();
    let client = ledger_service.axum_client;

    let events = &client
        .get_slot_filtered_events(&IntOrHash::Integer(0), None)
        .await
        .unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].key, "foo0");

    let events = &client
        .get_slot_filtered_events(&IntOrHash::Integer(0), Some("bar0"))
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].key, "bar0");

    let events = &client
        .get_slot_filtered_events(&IntOrHash::Integer(0), Some("")) // empty prefix
        .await
        .unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].key, "foo0");
    assert_eq!(events[1].key, "bar0");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_slot_subscription() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    let mut slots_subscription = ledger_db.subscribe_slots();
    let _ = ledger_db
        .materialize_slot(
            SlotCommit::<_, MockBlob, ()>::new(MockBlock::default(), Default::default()),
            b"state-root",
        )
        .unwrap();
    ledger_db.send_notifications();

    assert_eq!(
        slots_subscription.next().await.unwrap(),
        SlotNumber::GENESIS
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_save_aggregated_proof() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    // Storage sender is ignored, because data is immediately committed to the database.
    // Existing DeltaReader has a view to this database.
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();
    let _rx = ledger_db.subscribe_proof_saved();

    let proof_from_db = ledger_db.get_latest_aggregated_proof().await.unwrap();
    assert_eq!(None, proof_from_db);

    for i in 0..10 {
        let public_data = AggregatedProofPublicData::<MockAddress, MockDaSpec, Vec<u8>> {
            initial_slot_number: i.to_slot_number(),
            final_slot_number: i.to_slot_number(),
            genesis_state_root: vec![1],
            initial_state_root: vec![i],
            final_state_root: vec![i + 1],
            initial_slot_hash: MockHash([i + 2; 32]),
            final_slot_hash: MockHash([i + 3; 32]),
            code_commitment: CodeCommitment::default(),
            rewarded_addresses: vec![MockAddress::default()],
        };

        let raw_aggregated_proof = MockZkvmHost::create_serialized_proof(true, public_data.clone());

        let agg_proof = SerializedAggregatedProof {
            raw_aggregated_proof,
        };

        let slot_num = ledger_db.get_next_items_numbers().unwrap().slot_number;
        let proof_change_set = ledger_db
            .materialize_aggregated_proof(slot_num, agg_proof.clone())
            .unwrap();
        storage_manager.commit(&proof_change_set);

        let proof_from_db = ledger_db
            .get_latest_aggregated_proof()
            .await
            .unwrap()
            .unwrap();

        assert_eq!(proof_from_db.proof, agg_proof);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stf_info() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();

    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    let original_stored_inf_info = StoredStfInfo {
        data: vec![1, 2, 3],
    };

    let schema_batch = ledger_db
        .materialize_stf_info(&original_stored_inf_info, SlotNumber::GENESIS)
        .unwrap();

    storage_manager.commit(&schema_batch);

    let stored_stf_info = ledger_db
        .get_stf_info(SlotNumber::GENESIS)
        .unwrap()
        .unwrap();
    assert_eq!(original_stored_inf_info, stored_stf_info);
}

#[tokio::test(flavor = "multi_thread")]
async fn next_slot_number_to_receive_is_none_at_startup() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();

    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();
    assert!(ledger_db
        .get_stf_info_next_slot_number_to_receive()
        .await
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rollback() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    // Initially, there should be no slots
    assert!(ledger_db.get_head_slot().unwrap().is_none());

    // Add a few slots
    for i in 0..3 {
        let mut block = MockBlock::default();
        block.header.height = i;
        let slot_commit = SlotCommit::<_, MockBlob, ()>::new(block, Default::default());
        let schema_batch = ledger_db
            .materialize_slot(slot_commit, b"state-root")
            .unwrap();
        storage_manager.commit(&schema_batch);
    }

    // Verify we have 3 slots (slots 0, 1, 2)
    let (head_slot_number, _) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number, 2.to_slot_number());

    // Rollback the last slot (slot 2)
    ledger_db
        .rollback_last_slot(storage_manager.get_db())
        .unwrap();

    // Verify the head slot is now slot 1
    let (head_slot_number, _) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number, 1.to_slot_number());

    // Rollback another slot (slot 1)
    ledger_db
        .rollback_last_slot(storage_manager.get_db())
        .unwrap();

    // Verify the head slot is now slot 0
    let (head_slot_number, _) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number, 0.to_slot_number());

    // Rollback the last slot (slot 0)
    ledger_db
        .rollback_last_slot(storage_manager.get_db())
        .unwrap();

    // Verify there are no more slots
    assert!(ledger_db.get_head_slot().unwrap().is_none());

    // Try to rollback when there are no slots (should succeed without error)
    ledger_db
        .rollback_last_slot(storage_manager.get_db())
        .unwrap();
}
