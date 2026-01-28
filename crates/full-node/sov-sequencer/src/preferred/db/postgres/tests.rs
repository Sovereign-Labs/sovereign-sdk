use super::*;
use sov_full_node_configs::sequencer::ConfiguredNodeRole;
use sov_modules_api::VisibleSlotNumber;
use sov_test_utils::postgres::{
    config_from_postgres_container, create_postgres_container, ContainerAsync, CreatePostgresError,
    Postgres,
};
use std::num::NonZero;

async fn setup_test_postgres() -> Option<ContainerAsync<Postgres>> {
    match create_postgres_container().await {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => None,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_leader_election() {
    let Some(postgres) = setup_test_postgres().await else {
        return;
    };

    let db_1 = &mut DB::new(
        &postgres,
        String::from("node_id_1"),
        ConfiguredNodeRole::Leader,
    )
    .await;
    let db_2 = &mut DB::new(
        &postgres,
        String::from("node_id_2"),
        ConfiguredNodeRole::Replica,
    )
    .await;

    {
        // Updating the same node_id should change the last updated time in the db.
        let leader_1 = db_1.maybe_update_leader().await.unwrap();
        let updated_leader_1 = db_1.maybe_update_leader().await.unwrap();

        assert_eq!(leader_1.node_id, db_1.node_id);
        assert_eq!(leader_1.node_id, updated_leader_1.node_id);
        assert!(
            leader_1.last_updated < updated_leader_1.last_updated,
            "Leader timestamp should increase on refresh"
        );

        // Updating a different node id shouldn't change anything as the time delta is too big.
        let leader_2 = db_2.maybe_update_leader().await;
        assert!(
            leader_2.is_none(),
            "Replica should not become leader within timeout"
        );

        let leader_node_id = db_2.get_sequencer_leader().await.unwrap().unwrap();
        assert_eq!(updated_leader_1.node_id, leader_node_id);
    }

    {
        db_2.override_leader_timeout(Duration::ZERO);
        // Now we should be able to update db as the leader_timeout is zero.
        let leader_2 = db_2.maybe_update_leader().await.unwrap();
        assert_eq!(leader_2.node_id, db_2.node_id);
    }

    {
        db_1.override_leader_timeout(Duration::from_millis(100));
        let leader_1 = db_1.maybe_update_leader().await;
        assert!(
            leader_1.is_none(),
            "Old leader should not reclaim within timeout"
        );

        // Wait for more than 100ms and update the leader.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let leader_1 = db_1.maybe_update_leader().await.unwrap();
        assert_eq!(leader_1.node_id, db_1.node_id);
    }
}

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
        .add_proof_blob(sequence_number, 3, Arc::new([1, 2, 3]))
        .await
        .unwrap();

    db.as_mut().end_rollup_block(batch_to_store).await.unwrap();

    let data = db.as_mut().current_data().await.unwrap();
    assert!(
        !data.is_empty(),
        "Data should exist after adding transactions"
    );

    db.as_mut().prune(2).await.unwrap();
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
        .add_proof_blob(sequence_number, 3, Arc::new([1, 2, 3]))
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

struct DB {
    backend: PostgresBackend,
    node_id: String,
    leader_timeout: Duration,
}

impl AsRef<PostgresBackend> for DB {
    fn as_ref(&self) -> &PostgresBackend {
        &self.backend
    }
}

impl AsMut<PostgresBackend> for DB {
    fn as_mut(&mut self) -> &mut PostgresBackend {
        &mut self.backend
    }
}

impl DB {
    async fn new(
        postgres: &sov_test_utils::postgres::ContainerAsync<sov_test_utils::postgres::Postgres>,
        node_id: String,
        node_role: ConfiguredNodeRole,
    ) -> Self {
        let leader_timeout = Duration::from_millis(100_000);
        let postgres_config =
            config_from_postgres_container(postgres, node_id.clone(), node_role)
                .await
                .unwrap();
        let backend =
            PostgresBackend::connect_internal(&postgres_config, format!("{node_id}_address"))
                .await
                .unwrap();

        Self {
            backend,
            node_id,
            leader_timeout,
        }
    }

    fn override_leader_timeout(&mut self, leader_timeout: Duration) {
        self.leader_timeout = leader_timeout;
    }

    async fn maybe_update_leader(&self) -> Option<SequencerLeader> {
        self.backend
            .try_update_leader_and_register_node(self.leader_timeout)
            .await
            .unwrap()
    }

    pub(crate) async fn get_sequencer_leader(&self) -> Result<Option<String>, sqlx::Error> {
        let mut tx = self.backend.pool.begin().await?;
        let res = self.backend.get_sequencer_leader_inner(&mut tx).await?;
        tx.commit().await?;
        Ok(res)
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

#[tokio::test(flavor = "multi_thread")]
async fn test_nodes_table_notifications() {
    let Some(postgres) = setup_test_postgres().await else {
        return;
    };

    let db = DB::new(
        &postgres,
        String::from("node_1"),
        ConfiguredNodeRole::Leader,
    )
    .await;

    let mut listener = sqlx::postgres::PgListener::connect_with(&db.backend.pool)
        .await
        .unwrap();

    listener.listen("nodes_changes").await.unwrap();

    // Test INSERT notification via try_update_leader_and_register_node
    db.maybe_update_leader().await.unwrap();

    let notification = tokio::time::timeout(Duration::from_secs(5), listener.recv())
        .await
        .expect("Timed out waiting for INSERT notification")
        .unwrap();

    assert_eq!(notification.channel(), "nodes_changes");
    let parts: Vec<&str> = notification.payload().split(',').collect();
    assert_eq!(parts, vec!["node_1", "node_1_address", "INSERT"]);

    // Test UPDATE notification via try_update_leader_and_register_node
    db.maybe_update_leader().await.unwrap();

    let notification = tokio::time::timeout(Duration::from_secs(5), listener.recv())
        .await
        .expect("Timed out waiting for UPDATE notification")
        .unwrap();

    assert_eq!(notification.channel(), "nodes_changes");
    let parts: Vec<&str> = notification.payload().split(',').collect();
    assert_eq!(parts, vec!["node_1", "node_1_address", "UPDATE"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_leader_acquired_at() {
    let Some(postgres) = setup_test_postgres().await else {
        return;
    };

    let db_1 = &mut DB::new(
        &postgres,
        String::from("node_1"),
        ConfiguredNodeRole::Leader,
    )
    .await;
    let db_2 = &mut DB::new(
        &postgres,
        String::from("node_2"),
        ConfiguredNodeRole::Replica,
    )
    .await;

    // Node 1 becomes leader
    db_1.maybe_update_leader().await.unwrap();

    let (initial_leader_acquired_at,): (OffsetDateTime,) =
        sqlx::query_as("SELECT leader_acquired_at FROM sequencer_leader WHERE singleton = 1")
            .fetch_one(&db_1.backend.pool)
            .await
            .unwrap();

    // Small delay to ensure time difference
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Same node refreshes leadership (heartbeat)
    db_1.maybe_update_leader().await.unwrap();

    let (after_refresh_leader_acquired_at,): (OffsetDateTime,) =
        sqlx::query_as("SELECT leader_acquired_at FROM sequencer_leader WHERE singleton = 1")
            .fetch_one(&db_1.backend.pool)
            .await
            .unwrap();

    assert_eq!(
        initial_leader_acquired_at, after_refresh_leader_acquired_at,
        "leader_acquired_at should not change when same node refreshes leadership"
    );

    // Different node takes over leadership after timeout
    db_2.override_leader_timeout(Duration::ZERO);
    db_2.maybe_update_leader().await.unwrap();

    let (after_takeover_leader_acquired_at,): (OffsetDateTime,) =
        sqlx::query_as("SELECT leader_acquired_at FROM sequencer_leader WHERE singleton = 1")
            .fetch_one(&db_2.backend.pool)
            .await
            .unwrap();

    assert!(
        after_takeover_leader_acquired_at > initial_leader_acquired_at,
        "leader_acquired_at should be updated when a different node takes over leadership"
    );
}
