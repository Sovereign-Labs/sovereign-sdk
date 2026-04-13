use futures::StreamExt;
use rockbound::SchemaBatch;
use sov_db::ledger_db::{LedgerDb, SlotCommit};
use sov_db::schema::types::StoredStfInfo;
use sov_mock_da::{MockAddress, MockBlob, MockBlock, MockDaSpec, MockHash};
use sov_mock_zkvm::MockZkvmHost;
use sov_rollup_interface::common::{HexHash, IntoSlotNumber, SlotNumber};
use sov_rollup_interface::node::ledger_api::LedgerStateProvider;
use sov_rollup_interface::stf::{BatchReceipt, BlobDiscardReason, TransactionReceipt, TxEffect};
use sov_rollup_interface::stf::{DiscardedBlob, StoredEvent};
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, CodeCommitmentHash, SerializedAggregatedProof,
};
use sov_rollup_interface::TxHash;
use sov_test_utils::ledger_db::sov_api_spec::types::IntOrHash;
use sov_test_utils::ledger_db::{LedgerTestService, LedgerTestServiceData};
use sov_test_utils::storage::SimpleLedgerStorageManager;
use sov_test_utils::TestTxReceiptContents;

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
            inner_vkey_hash: CodeCommitmentHash::default(),
            outer_vk_hash: CodeCommitmentHash::default(),
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
            .materialize_slot(slot_commit, &[i as u8; 64])
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
        LedgerDb::rollback_head_slot(db.clone()).unwrap();
        // Verify the head slot is now slot 1
        let slot_nr_after_rollback = 1;
        assert_slot_numbers(slot_nr_after_rollback, &ledger_db).await;

        let state_root_hash_from_ledger =
            LedgerDb::get_head_root_hash(db.clone()).unwrap().unwrap();
        assert_eq!(
            state_root_hash_from_ledger,
            [slot_nr_after_rollback as u8; 64]
        );
    }

    // Rollback another slot (slot 1)
    {
        LedgerDb::rollback_head_slot(db.clone()).unwrap();
        // Verify the head slot is now slot 0
        let slot_nr_after_rollback = 0;
        assert_slot_numbers(slot_nr_after_rollback, &ledger_db).await;

        let state_root_hash_from_ledger =
            LedgerDb::get_head_root_hash(db.clone()).unwrap().unwrap();
        assert_eq!(
            state_root_hash_from_ledger,
            [slot_nr_after_rollback as u8; 64]
        );
    }

    // Rollback the last slot (slot 0)
    {
        LedgerDb::rollback_head_slot(db.clone()).unwrap();
        // Verify there are no more slots
        assert!(ledger_db.get_head_slot().unwrap().is_none());
        // Try to rollback when there are no slots (should succeed without error)
        LedgerDb::rollback_head_slot(db.clone()).unwrap();

        let state_root_hash_from_ledger = LedgerDb::get_head_root_hash(db.clone()).unwrap();
        assert!(state_root_hash_from_ledger.is_none());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rollback_with_data() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    let db = storage_manager.get_db();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    // Create slots with actual data (batches, transactions, events)
    for slot_num in 0..3 {
        let schema_batch = create_slot_schema_batch(slot_num, &ledger_db);
        storage_manager.commit(&schema_batch);
    }

    // Verify item numbers before rollback
    {
        let expected_slot_number = 2;
        let (head_slot_number, head_slot) = ledger_db.get_head_slot().unwrap().unwrap();
        assert_eq!(head_slot_number.get(), expected_slot_number);
        assert_eq!(head_slot.batches.end.0 - head_slot.batches.start.0, 2);
        assert_next_items_numbers(expected_slot_number, &ledger_db);
    }

    // Rollback slot 2
    {
        LedgerDb::rollback_head_slot(db.clone()).unwrap();
        let expected_slot_number = 1;

        // Verify slot 2 is gone
        let (head_slot_number, head_slot) = ledger_db.get_head_slot().unwrap().unwrap();
        assert_eq!(head_slot_number.get(), expected_slot_number);
        assert_eq!(head_slot.batches.end.0 - head_slot.batches.start.0, 2);
        assert_next_items_numbers(expected_slot_number, &ledger_db);
    }

    {
        // Rollback slot 1
        LedgerDb::rollback_head_slot(db.clone()).unwrap();
        let expected_slot_number = 0;

        // Verify slot 1 is gone but slot 0 remains
        let (head_slot_number, head_slot) = ledger_db.get_head_slot().unwrap().unwrap();
        assert_eq!(head_slot_number.get(), expected_slot_number);
        assert_eq!(head_slot.batches.end.0 - head_slot.batches.start.0, 2);
        assert_next_items_numbers(expected_slot_number, &ledger_db);
    }
}

async fn assert_slot_numbers(n: u64, ledger_db: &LedgerDb) {
    let (head_slot_number, _) = ledger_db.get_head_slot().unwrap().unwrap();
    assert_eq!(head_slot_number.get(), n);
    assert_eq!(
        head_slot_number,
        ledger_db.get_latest_finalized_slot_number().await.unwrap()
    );
}

fn assert_next_items_numbers(slot_number: u64, ledger_db: &LedgerDb) {
    let next_slot_number = slot_number + 1;
    // Verify item numbers before rollback
    let item_numbers_before_rollback = ledger_db.get_next_items_numbers().unwrap();
    assert_eq!(
        item_numbers_before_rollback.slot_number.get(),
        next_slot_number
    );
    assert_eq!(
        item_numbers_before_rollback.batch_number,
        next_slot_number * 2
    ); // n slots × 2 batches

    assert_eq!(
        item_numbers_before_rollback.discarded_batch_number,
        next_slot_number * 3
    );

    assert_eq!(item_numbers_before_rollback.tx_number, next_slot_number * 6); // batch_number × 3 txs
    assert_eq!(
        item_numbers_before_rollback.event_number,
        next_slot_number * 12
    );
}

fn create_slot_with_keys(slot_num: u64, keys: &[&str], ledger_db: &LedgerDb) -> SchemaBatch {
    let mut block = MockBlock::default();
    block.header.height = slot_num;

    let mut slot_commit =
        SlotCommit::<_, i32, TestTxReceiptContents>::new(block, Default::default());

    let mut tx_receipts = vec![];
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&u64::to_le_bytes(1000 + slot_num));
    let tx_hash = TxHash::new(out);

    let events = keys
        .iter()
        .map(|k| StoredEvent::new(k.as_bytes(), b"val", tx_hash.0))
        .collect();

    tx_receipts.push(TransactionReceipt {
        tx_hash,
        body_to_save: None,
        events,
        receipt: TxEffect::Successful(0),
    });

    let batch_receipt = BatchReceipt {
        batch_hash: [slot_num as u8; 32],
        tx_receipts,
        ignored_tx_receipts: vec![],
        inner: 0,
    };

    slot_commit.add_batch(batch_receipt);

    ledger_db
        .materialize_slot(slot_commit, b"state-root")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_event_key_counts_basic() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    // Insert a slot with 3 events: 2x "alpha", 1x "beta"
    let schema_batch = create_slot_with_keys(0, &["alpha", "alpha", "beta"], &ledger_db);
    storage_manager.commit(&schema_batch);

    let counts: std::collections::HashMap<String, u64> = ledger_db
        .get_event_key_counts()
        .await
        .unwrap()
        .into_iter()
        .collect();

    assert_eq!(counts.get("alpha"), Some(&2), "alpha should have count 2");
    assert_eq!(counts.get("beta"), Some(&1), "beta should have count 1");
    assert_eq!(counts.len(), 2, "should have exactly 2 distinct keys");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_event_key_counts_across_slots() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    // Slot 0: 1x "alpha", 1x "beta"
    let schema_batch = create_slot_with_keys(0, &["alpha", "beta"], &ledger_db);
    storage_manager.commit(&schema_batch);

    // Slot 1: 2x "alpha", 1x "gamma"
    let schema_batch = create_slot_with_keys(1, &["alpha", "alpha", "gamma"], &ledger_db);
    storage_manager.commit(&schema_batch);

    let counts: std::collections::HashMap<String, u64> = ledger_db
        .get_event_key_counts()
        .await
        .unwrap()
        .into_iter()
        .collect();

    assert_eq!(
        counts.get("alpha"),
        Some(&3),
        "alpha should accumulate across slots"
    );
    assert_eq!(counts.get("beta"), Some(&1));
    assert_eq!(counts.get("gamma"), Some(&1));
    assert_eq!(counts.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_event_key_counts_after_rollback() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
    let ledger_storage = storage_manager.create_ledger_storage();
    let db = storage_manager.get_db();
    let ledger_db = LedgerDb::with_reader(ledger_storage).unwrap();

    // Slot 0: 1x "alpha", 1x "beta"
    let schema_batch = create_slot_with_keys(0, &["alpha", "beta"], &ledger_db);
    storage_manager.commit(&schema_batch);

    // Slot 1: 2x "alpha", 1x "beta"
    let schema_batch = create_slot_with_keys(1, &["alpha", "alpha", "beta"], &ledger_db);
    storage_manager.commit(&schema_batch);

    // Before rollback: alpha=3, beta=2
    let counts: std::collections::HashMap<String, u64> = ledger_db
        .get_event_key_counts()
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(counts.get("alpha"), Some(&3));
    assert_eq!(counts.get("beta"), Some(&2));

    // Rollback slot 1
    LedgerDb::rollback_head_slot(db.clone()).unwrap();

    // After rollback: alpha=1, beta=1
    let counts: std::collections::HashMap<String, u64> = ledger_db
        .get_event_key_counts()
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(
        counts.get("alpha"),
        Some(&1),
        "alpha should be decremented after rollback"
    );
    assert_eq!(
        counts.get("beta"),
        Some(&1),
        "beta should be decremented after rollback"
    );

    // Rollback slot 0: all counts should go to zero (entries deleted)
    LedgerDb::rollback_head_slot(db.clone()).unwrap();

    let counts: std::collections::HashMap<String, u64> = ledger_db
        .get_event_key_counts()
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert!(
        counts.is_empty(),
        "all event count entries should be deleted after full rollback"
    );
}

fn create_slot_schema_batch(slot_num: u64, ledger_db: &LedgerDb) -> SchemaBatch {
    let mut block = MockBlock::default();
    block.header.height = slot_num;

    let mut discarded_blobs = Vec::new();

    for i in 0..3 {
        let discarded_blob = DiscardedBlob {
            hash: HexHash::new([(slot_num + i) as u8; 32]),
            reason: BlobDiscardReason::OutOfCapacity,
        };
        discarded_blobs.push(discarded_blob);
    }

    let mut slot_commit = SlotCommit::<_, i32, TestTxReceiptContents>::new(block, discarded_blobs);

    // Add 2 batches per slot
    for batch_num in 0..2 {
        let mut tx_receipts = vec![];

        // Add 3 transactions per batch
        for tx_num in 0..3 {
            let mut out = [0u8; 32];
            out[..8].copy_from_slice(&u64::to_le_bytes(10 * batch_num + tx_num));
            let tx_hash = TxHash::new(out);

            let events = vec![
                StoredEvent::new("k1".as_bytes(), "v1".as_bytes(), tx_hash.0),
                StoredEvent::new("k2".as_bytes(), "v2".as_bytes(), tx_hash.0),
            ];

            tx_receipts.push(TransactionReceipt {
                tx_hash,
                body_to_save: None,
                events,
                receipt: TxEffect::Successful(0),
            });
        }

        let mut batch_hash: [u8; 32] = [0u8; 32];
        batch_hash[..8].copy_from_slice(&u64::to_le_bytes(10 * slot_num + batch_num));

        let batch_receipt = BatchReceipt {
            batch_hash,
            tx_receipts,
            ignored_tx_receipts: vec![],
            inner: batch_num as i32,
        };

        slot_commit.add_batch(batch_receipt);
    }

    ledger_db
        .materialize_slot(slot_commit, b"state-root")
        .unwrap()
}
