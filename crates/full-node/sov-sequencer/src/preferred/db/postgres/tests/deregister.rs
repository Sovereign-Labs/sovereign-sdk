use super::*;

// Counts the rows in the `nodes` table matching `node_id` (0 or 1, since node_id is the
// primary key). Reads `backend.pool` directly, which is accessible from this submodule.
async fn count_nodes_row(backend: &PostgresBackend, node_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE node_id = $1")
        .bind(node_id)
        .fetch_one(&backend.pool)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_deregister_removes_only_own_nodes_row() {
    let Some(postgres) = setup_test_postgres().await else {
        return;
    };

    let node_a = DB::new(&postgres, String::from("node_a"), ConfiguredNodeRole::Replica).await;
    let node_b = DB::new(&postgres, String::from("node_b"), ConfiguredNodeRole::Replica).await;

    // Register both nodes via a replica heartbeat (no leadership competition).
    node_a.backend.heartbeat(None).await.unwrap();
    node_b.backend.heartbeat(None).await.unwrap();
    assert_eq!(
        count_nodes_row(&node_a.backend, "node_a").await,
        1,
        "node_a should be registered before deregistration"
    );

    // Deregister only node_a.
    node_a.backend.deregister_node_on_shutdown().await.unwrap();

    assert_eq!(
        count_nodes_row(&node_a.backend, "node_a").await,
        0,
        "node_a's row should be removed after deregistration"
    );
    assert_eq!(
        count_nodes_row(&node_a.backend, "node_b").await,
        1,
        "node_b's row must remain untouched when node_a deregisters"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_deregister_is_idempotent_for_unregistered_node() {
    let Some(postgres) = setup_test_postgres().await else {
        return;
    };

    let node = DB::new(
        &postgres,
        String::from("never_registered"),
        ConfiguredNodeRole::Replica,
    )
    .await;

    // Deleting a row that was never inserted must be a no-op, not an error.
    node.backend
        .deregister_node_on_shutdown()
        .await
        .expect("deregistering an unregistered node should be a no-op");
}
