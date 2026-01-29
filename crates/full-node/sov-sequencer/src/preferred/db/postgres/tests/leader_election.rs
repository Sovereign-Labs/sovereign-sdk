use super::*;
use time::OffsetDateTime;

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
