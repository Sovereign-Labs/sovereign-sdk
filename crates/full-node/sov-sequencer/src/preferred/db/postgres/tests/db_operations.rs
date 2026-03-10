use super::*;
use sov_modules_api::VisibleSlotNumber;
use std::{num::NonZero, sync::Arc};

#[tokio::test(flavor = "multi_thread")]
async fn test_db_operations_leader() {
    let Some(postgres) = setup_test_postgres().await else {
        return;
    };

    let db = &mut DB::new(
        &postgres,
        String::from("node_id_1"),
        ConfiguredNodeRole::Leader,
    )
    .await;
    db.maybe_update_leader().await.unwrap();

    let sequence_number = 1;
    let batch_to_store = batch_to_store(sequence_number);

    db.as_mut()
        .begin_rollup_block(batch_to_store)
        .await
        .unwrap();

    db.as_mut()
        .add_tx(
            sequence_number,
            1,
            FullyBakedTx::new(vec![1, 2, 3]),
            TxHash::new([1; 32]),
        )
        .await
        .unwrap();

    db.as_mut()
        .batch_add_txs(
            sequence_number,
            2,
            &[(FullyBakedTx::new(vec![4, 5, 6]), TxHash::new([1; 32]))],
        )
        .await
        .unwrap();

    db.as_mut()
        .add_proof_blob(
            sequence_number + 1,
            3,
            PreferredProofDataBytes(Arc::new(*b"proof_data")),
        )
        .await
        .unwrap();

    // Add more txs to the batch after the proof blob. They should come back when we read from the DB.
    db.as_mut()
        .add_tx(
            sequence_number,
            3,
            FullyBakedTx::new(vec![7, 8, 9]),
            TxHash::new([3; 32]),
        )
        .await
        .unwrap();

    db.as_mut().end_rollup_block(batch_to_store).await.unwrap();

    let data = db.as_mut().current_data().await.unwrap();
    assert!(
        !data.is_empty(),
        "Data should exist after adding transactions"
    );

    let data = db.as_mut().current_data().await.unwrap();
    assert!(
        data.completed_blobs.len() == 2,
        "Should have 2 completed blobs but found {}",
        data.completed_blobs.len()
    );

    let ReadBlob::Batch(batch) = &data.completed_blobs[0] else {
        panic!("Completed blob must be a batch");
    };
    assert_eq!(batch.sequence_number, sequence_number);
    assert_eq!(batch.txs.len(), 3);

    let ReadBlob::Proof {
        sequence_number: proof_sequence_number,
        data: proof_data,
        ..
    } = &data.completed_blobs[1]
    else {
        panic!("Completed blob must be a proof");
    };
    assert!(
        *proof_sequence_number == (sequence_number + 1),
        "Should have a completed proof blob with sequence number {}",
        sequence_number + 1
    );
    assert_eq!(
        &*proof_data.0,
        b"proof_data".as_slice(),
        "Proof data should be correct"
    );

    db.as_mut().prune(3).await.unwrap();
    let data = db.as_mut().current_data().await.unwrap();
    assert!(data.is_empty(), "Data should be empty after prune");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_db_operations_replica() {
    let Some(postgres) = setup_test_postgres().await else {
        return;
    };
    let db_leader = &mut DB::new(
        &postgres,
        String::from("node_id_1"),
        ConfiguredNodeRole::Leader,
    )
    .await;
    let db_replica = &mut DB::new(
        &postgres,
        String::from("node_id_2"),
        ConfiguredNodeRole::Replica,
    )
    .await;

    let sequence_number = 1;
    let batch_to_store = batch_to_store(sequence_number);

    db_leader.maybe_update_leader().await.unwrap();

    let err = db_replica
        .as_mut()
        .begin_rollup_block(batch_to_store)
        .await
        .unwrap_err();

    assert_replica_disallowed(err, &db_replica.node_id, &FailedOperation::BeginBlock);

    let err = db_replica
        .as_mut()
        .add_tx(
            sequence_number,
            1,
            FullyBakedTx::new(vec![1, 2, 3]),
            TxHash::new([1; 32]),
        )
        .await
        .unwrap_err();

    assert_replica_disallowed(err, &db_replica.node_id, &FailedOperation::AddTx);

    let err = db_replica
        .as_mut()
        .batch_add_txs(
            sequence_number,
            2,
            &[(FullyBakedTx::new(vec![4, 5, 6]), TxHash::new([1; 32]))],
        )
        .await
        .unwrap_err();

    assert_replica_disallowed(err, &db_replica.node_id, &FailedOperation::BatchAddTxs);

    let err = db_replica
        .as_mut()
        .add_proof_blob(
            sequence_number,
            3,
            PreferredProofDataBytes(Arc::new([1, 2, 3])),
        )
        .await
        .unwrap_err();

    assert_replica_disallowed(err, &db_replica.node_id, &FailedOperation::AddProof);

    let err = db_replica
        .as_mut()
        .end_rollup_block(batch_to_store)
        .await
        .unwrap_err();

    assert_replica_disallowed(err, &db_replica.node_id, &FailedOperation::EndBlock);

    let err = db_replica.as_mut().prune(2).await.unwrap_err();
    assert_replica_disallowed(
        err,
        &db_replica.node_id,
        &FailedOperation::Prune {
            db_leader: Some(db_leader.node_id.clone()),
        },
    );

    let err = db_replica.as_mut().current_data().await.unwrap_err();
    assert_replica_disallowed(
        err,
        &db_replica.node_id,
        &FailedOperation::CurrentData {
            db_leader: Some(db_leader.node_id.clone()),
        },
    );
}

fn assert_replica_disallowed(
    err: DbError,
    expected_node_id: &str,
    expected_operation: &FailedOperation,
) {
    match err {
        DbError::ReplicaDisallowed {
            self_node_id,
            operation,
        } => {
            assert_eq!(
                self_node_id, expected_node_id,
                "Unexpected node_id in error"
            );
            assert_eq!(
                &operation, expected_operation,
                "Unexpected operation in error"
            );
        }
        DbError::Database(e) => panic!("Expected ReplicaDisallowed, got Database error: {e:?}"),
    }
}

fn batch_to_store(sequence_number: SequenceNumber) -> BatchToStore {
    BatchToStore {
        blob_id: 42,
        sequence_number,
        visible_slot_number_after_increase: VisibleSlotNumber::new_dangerous(100),
        visible_slots_to_advance: NonZero::new(3).unwrap(),
    }
}
