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
    let db = storage_manager.get_db();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    // Add a few slots
    for i in 0..3 {
        let mut block = MockBlock::default();
        block.header.height = i;
        let slot_commit = SlotCommit::<_, MockBlob, ()>::new(block, Default::default());
        let mut schema_batch = ledger_db
            .materialize_slot(slot_commit, &i.to_be_bytes())
            .unwrap();

        let finalized_slot_number = ledger_db
            .materialize_latest_finalize_slot(SlotNumber::new(i), SlotNumber::new(i))
            .unwrap();

        schema_batch.merge(finalized_slot_number);
        storage_manager.commit(&schema_batch);
    }

    // Verify we have 3 slots (slots 0, 1, 2)
    assert_slot_numbers(2, &ledger_db).await;

    // Rollback slot 2
    {
        LedgerDb::rollback_last_slot(db.clone()).unwrap();
        // Verify the head slot is now slot 1
        assert_slot_numbers(1, &ledger_db).await;
    }

    // Rollback another slot (slot 1)
    {
        LedgerDb::rollback_last_slot(db.clone()).unwrap();
        // Verify the head slot is now slot 0
        assert_slot_numbers(0, &ledger_db).await;
    }

    // Rollback the last slot (slot 0)
    {
        LedgerDb::rollback_last_slot(db.clone()).unwrap();
        // Verify there are no more slots
        assert!(ledger_db.get_head_slot().unwrap().is_none());
        // Try to rollback when there are no slots (should succeed without error)
        LedgerDb::rollback_last_slot(db.clone()).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rollback_with_data() {
    use sov_rollup_interface::common::IntoSlotNumber;
    use sov_rollup_interface::stf::{BatchReceipt, TransactionReceipt, TxEffect};
    use sov_rollup_interface::TxHash;
    use sov_test_utils::TestTxReceiptContents;

    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    let db = storage_manager.get_db();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    // Create slots with actual data (batches, transactions, events)
    for slot_num in 0..3 {
        let mut block = MockBlock::default();
        block.header.height = slot_num;
        let mut slot_commit = SlotCommit::<_, i32, TestTxReceiptContents>::new(block, vec![]);

        // Add 2 batches per slot
        for batch_num in 0..2 {
            let mut tx_receipts = vec![];

            // Add 3 transactions per batch
            for tx_num in 0..3 {
                let tx_hash = TxHash::new([
                    (slot_num * 100 + batch_num * 10 + tx_num) as u8,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                ]);

                let events = vec![
                    sov_rollup_interface::stf::StoredEvent::new(
                        format!(
                            "event_key_slot{}_batch{}_tx{}_evt0",
                            slot_num, batch_num, tx_num
                        )
                        .as_bytes(),
                        format!("event_value_{}_{}_{}_0", slot_num, batch_num, tx_num).as_bytes(),
                        [0u8; 32],
                    ),
                    sov_rollup_interface::stf::StoredEvent::new(
                        format!(
                            "event_key_slot{}_batch{}_tx{}_evt1",
                            slot_num, batch_num, tx_num
                        )
                        .as_bytes(),
                        format!("event_value_{}_{}_{}_1", slot_num, batch_num, tx_num).as_bytes(),
                        [0u8; 32],
                    ),
                ];

                tx_receipts.push(TransactionReceipt {
                    tx_hash,
                    body_to_save: None,
                    events,
                    receipt: TxEffect::Successful(
                        (slot_num * 100 + batch_num * 10 + tx_num) as u32,
                    ),
                });
            }

            let batch_receipt = BatchReceipt {
                batch_hash: [
                    (slot_num * 100 + batch_num * 10) as u8,
                    1,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                ],
                tx_receipts,
                ignored_tx_receipts: vec![],
                inner: batch_num as i32,
            };

            slot_commit.add_batch(batch_receipt);
        }

        let schema_batch = ledger_db
            .materialize_slot(slot_commit, b"state-root")
            .unwrap();
        storage_manager.commit(&schema_batch);
    }

    // Verify we have 3 slots with data
    let (head_slot_number, head_slot) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number, 2.to_slot_number());
    assert_eq!(head_slot.batches.start.0, 4); // Slots 0 and 1 each have 2 batches
    assert_eq!(head_slot.batches.end.0, 6); // Slot 2 has batches 4 and 5

    // Verify item numbers before rollback
    let item_numbers_before_rollback = ledger_db.get_next_items_numbers().unwrap();
    assert_eq!(item_numbers_before_rollback.slot_number, 3.to_slot_number());
    assert_eq!(item_numbers_before_rollback.batch_number, 6); // 3 slots × 2 batches
    assert_eq!(item_numbers_before_rollback.tx_number, 18); // 6 batches × 3 txs
    assert_eq!(item_numbers_before_rollback.event_number, 36); // 18 txs × 2 events

    // Rollback slot 2
    LedgerDb::rollback_last_slot(db.clone()).unwrap();

    // Verify slot 2 is gone
    let (head_slot_number, head_slot) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number, 1.to_slot_number());
    assert_eq!(head_slot.batches.start.0, 2); // Slot 1 starts at batch 2
    assert_eq!(head_slot.batches.end.0, 4); // Slot 1 ends at batch 4

    // Verify the item numbers reflect the rollback
    let item_numbers_after_rollback = ledger_db.get_next_items_numbers().unwrap();
    assert_eq!(item_numbers_after_rollback.slot_number, 2.to_slot_number());
    assert_eq!(item_numbers_after_rollback.batch_number, 4); // 2 slots × 2 batches
    assert_eq!(item_numbers_after_rollback.tx_number, 12); // 4 batches × 3 txs
    assert_eq!(item_numbers_after_rollback.event_number, 24); // 12 txs × 2 events

    // Verify slot 1 data is intact by checking item numbers
    let item_numbers_slot_1 = ledger_db.get_next_items_numbers().unwrap();
    assert_eq!(item_numbers_slot_1.slot_number, 2.to_slot_number());
    assert_eq!(item_numbers_slot_1.batch_number, 4); // 2 slots × 2 batches

    // Rollback slot 1
    LedgerDb::rollback_last_slot(db.clone()).unwrap();

    // Verify slot 1 is gone but slot 0 remains
    let (head_slot_number, head_slot) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number, 0.to_slot_number());
    assert_eq!(head_slot.batches.start.0, 0);
    assert_eq!(head_slot.batches.end.0, 2);

    let item_numbers_after_second_rollback = ledger_db.get_next_items_numbers().unwrap();
    assert_eq!(
        item_numbers_after_second_rollback.slot_number,
        1.to_slot_number()
    );
    assert_eq!(item_numbers_after_second_rollback.batch_number, 2);
    assert_eq!(item_numbers_after_second_rollback.tx_number, 6);
    assert_eq!(item_numbers_after_second_rollback.event_number, 12);
}

async fn assert_slot_numbers(n: u64, ledger_db: &LedgerDb) {
    let (head_slot_number, _) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number.get(), n);
    assert_eq!(
        head_slot_number,
        ledger_db.get_latest_finalized_slot_number().await.unwrap()
    );
}
