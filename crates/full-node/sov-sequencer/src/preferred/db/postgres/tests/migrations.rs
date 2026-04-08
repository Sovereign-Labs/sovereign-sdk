use super::*;
use sov_test_utils::postgres::{
    connection_string_from_postgres_container, ContainerAsync, Postgres,
};
use sqlx::postgres::{PgPool, PgPoolOptions};

#[tokio::test(flavor = "multi_thread")]
async fn test_003_step1_deduplicates_transaction_events() {
    let Some((_postgres, pool)) = setup_pre_003_pool().await else {
        return;
    };

    let tx_hash = vec![0x11; 32];
    let tx_data = vec![0x21, 0x22];
    let control_same_batch_hash = vec![0x12; 32];
    let control_same_batch_data = vec![0x23, 0x24];
    let control_other_batch_hash = vec![0x13; 32];
    let control_other_batch_data = vec![0x25, 0x26];

    let keep_id = insert_transaction_event(&pool, 7, 0, &tx_hash, &tx_data).await;
    let drop_id = insert_transaction_event(&pool, 7, 0, &tx_hash, &tx_data).await;
    let control_same_batch_id = insert_transaction_event(
        &pool,
        7,
        1,
        &control_same_batch_hash,
        &control_same_batch_data,
    )
    .await;
    let control_other_batch_id = insert_transaction_event(
        &pool,
        8,
        0,
        &control_other_batch_hash,
        &control_other_batch_data,
    )
    .await;

    assert_eq!(count_transaction_events(&pool, 7, 0).await, 2);
    assert_eq!(count_transaction_events(&pool, 7, 1).await, 1);
    assert_eq!(count_transaction_events(&pool, 8, 0).await, 1);

    apply_003_migration(&pool).await;

    assert_eq!(count_transaction_events(&pool, 7, 0).await, 1);
    assert_eq!(count_transaction_events(&pool, 7, 1).await, 1);
    assert_eq!(count_transaction_events(&pool, 8, 0).await, 1);
    assert_eq!(count_event_by_id(&pool, keep_id).await, 1);
    assert_eq!(count_event_by_id(&pool, drop_id).await, 0);
    assert_eq!(count_event_by_id(&pool, control_same_batch_id).await, 1);
    assert_eq!(count_event_by_id(&pool, control_other_batch_id).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_003_step2_deduplicates_non_transaction_events() {
    let Some((_postgres, pool)) = setup_pre_003_pool().await else {
        return;
    };

    let batch_start_data = vec![0x31, 0x32];
    let batch_end_data = vec![0x41, 0x42];
    let tx_hash = vec![0x51; 32];
    let tx_data = vec![0x61, 0x62];

    let batch_start_keep_id =
        insert_non_transaction_event(&pool, 9, "batch_start", Some(&batch_start_data)).await;
    let batch_start_drop_id =
        insert_non_transaction_event(&pool, 9, "batch_start", Some(&batch_start_data)).await;
    let batch_end_keep_id =
        insert_non_transaction_event(&pool, 9, "batch_end", Some(&batch_end_data)).await;
    let batch_end_drop_id =
        insert_non_transaction_event(&pool, 9, "batch_end", Some(&batch_end_data)).await;
    let new_proof_keep_id = insert_non_transaction_event(&pool, 10, "new_proof", None).await;
    let new_proof_drop_id = insert_non_transaction_event(&pool, 10, "new_proof", None).await;
    let tx_control_id = insert_transaction_event(&pool, 9, 0, &tx_hash, &tx_data).await;

    assert_eq!(
        count_non_transaction_events(&pool, 9, "batch_start").await,
        2
    );
    assert_eq!(count_non_transaction_events(&pool, 9, "batch_end").await, 2);
    assert_eq!(
        count_non_transaction_events(&pool, 10, "new_proof").await,
        2
    );
    assert_eq!(count_transaction_events(&pool, 9, 0).await, 1);

    apply_003_migration(&pool).await;

    assert_eq!(
        count_non_transaction_events(&pool, 9, "batch_start").await,
        1
    );
    assert_eq!(count_non_transaction_events(&pool, 9, "batch_end").await, 1);
    assert_eq!(
        count_non_transaction_events(&pool, 10, "new_proof").await,
        1
    );
    assert_eq!(count_event_by_id(&pool, batch_start_keep_id).await, 1);
    assert_eq!(count_event_by_id(&pool, batch_end_keep_id).await, 1);
    assert_eq!(count_event_by_id(&pool, new_proof_keep_id).await, 1);
    assert_eq!(count_event_by_id(&pool, batch_start_drop_id).await, 0);
    assert_eq!(count_event_by_id(&pool, batch_end_drop_id).await, 0);
    assert_eq!(count_event_by_id(&pool, new_proof_drop_id).await, 0);
    assert_eq!(count_event_by_id(&pool, tx_control_id).await, 1);
}

async fn setup_pre_003_pool() -> Option<(ContainerAsync<Postgres>, PgPool)> {
    let postgres = setup_test_postgres().await?;
    let connection_string = connection_string_from_postgres_container(&postgres)
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .connect(&connection_string)
        .await
        .unwrap();

    sqlx::raw_sql(include_str!("../migrations/001_init.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/002_nodes.sql"))
        .execute(&pool)
        .await
        .unwrap();

    Some((postgres, pool))
}

async fn apply_003_migration(pool: &PgPool) {
    sqlx::raw_sql(include_str!("../migrations/003_unique_tx_events.sql"))
        .execute(pool)
        .await
        .unwrap();
}

async fn insert_transaction_event(
    pool: &PgPool,
    sequence_number: i64,
    index_in_batch: i64,
    hash: &[u8],
    data: &[u8],
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
         VALUES ($1, 'transaction', $2, $3, $4)
         RETURNING event_id",
    )
    .bind(sequence_number)
    .bind(index_in_batch)
    .bind(hash)
    .bind(data)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_non_transaction_event(
    pool: &PgPool,
    sequence_number: i64,
    event_type: &str,
    data: Option<&[u8]>,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
         VALUES ($1, $2::event_type, NULL, NULL, $3)
         RETURNING event_id",
    )
    .bind(sequence_number)
    .bind(event_type)
    .bind(data)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn count_event_by_id(pool: &PgPool, event_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn count_transaction_events(pool: &PgPool, sequence_number: i64, index_in_batch: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM events
         WHERE sequence_number = $1
           AND event_type = 'transaction'
           AND index_in_batch = $2",
    )
    .bind(sequence_number)
    .bind(index_in_batch)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn count_non_transaction_events(
    pool: &PgPool,
    sequence_number: i64,
    event_type: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM events
         WHERE sequence_number = $1
           AND event_type = $2::event_type",
    )
    .bind(sequence_number)
    .bind(event_type)
    .fetch_one(pool)
    .await
    .unwrap()
}
