use std::num::NonZero;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use futures::stream::FuturesOrdered;
use futures::{Future, StreamExt};
use serde::{Deserialize, Serialize};
use sov_blob_storage::SequenceNumber;
use sov_modules_api::{FullyBakedTx, Runtime, Spec, TxHash, VisibleSlotNumber};
use sov_rollup_interface::node::da::DaService;
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::{PgPool, Row};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace, warn};

use super::db::StoredBlob;
use crate::preferred::{exit_rollup, DbEvent, PreferredSequencer};
use crate::ProofBlobSender;

/// Event type enum for type-safe parsing
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "event_type", rename_all = "snake_case")]
enum EventType {
    Transaction,
    BatchStart,
    BatchEnd,
    NewProof,
}

impl FromStr for EventType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "transaction" => Ok(EventType::Transaction),
            "batch_start" => Ok(EventType::BatchStart),
            "batch_end" => Ok(EventType::BatchEnd),
            "new_proof" => Ok(EventType::NewProof),
            _ => Err(anyhow!("Invalid event type: {}", s)),
        }
    }
}

/// Structure representing the CSV payload from PostgreSQL NOTIFY
#[derive(Debug)]
struct EventsNotificationPayload {
    event_id: u64,
    sequence_number: u64,
    event_type: EventType,
    index_in_batch: Option<u64>, // Only present for transaction events
}

impl EventsNotificationPayload {
    fn parse_csv(payload: &str) -> anyhow::Result<Self> {
        let parts: Vec<&str> = payload.split(',').collect();
        if parts.len() != 4 {
            return Err(anyhow!(
                "Invalid CSV format: expected 4 fields, got {}",
                parts.len()
            ));
        }

        Ok(EventsNotificationPayload {
            event_id: parts[0]
                .parse()
                .map_err(|e| anyhow!("Invalid event_id '{}': {}", parts[0], e))?,
            sequence_number: parts[1]
                .parse()
                .map_err(|e| anyhow!("Invalid sequence_number '{}': {}", parts[1], e))?,
            event_type: parts[2]
                .parse()
                .map_err(|e| anyhow!("Invalid event_type '{}': {}", parts[2], e))?,
            index_in_batch: if parts[3].is_empty() {
                None
            } else {
                Some(
                    parts[3]
                        .parse()
                        .map_err(|e| anyhow!("Invalid index_in_batch '{}': {}", parts[3], e))?,
                )
            },
        })
    }
}

/// Structure representing batch metadata stored by the master sequencer
#[derive(Debug)]
struct BatchMetadata {
    visible_slot_number_after_increase: VisibleSlotNumber,
    visible_slots_to_advance: NonZero<u8>,
}

/// Type alias for futures in the concurrent processing pipeline
type PendingEventFuture = Pin<Box<dyn Future<Output = anyhow::Result<CompletedEvent>> + Send>>;

/// Represents a completed event ready for processing
#[derive(Debug)]
enum CompletedEvent {
    Event(DbEvent),
    Backfill {
        current_latest: Option<u64>,
        target: u64,
    },
}

/// Background task for replica sequencers to sync state from the master sequencer's database.
/// This task monitors the `txs` table for new batches and updates the replica's state accordingly.
pub async fn spawn_replica_sync_task<S, Rt, Da>(
    sequencer: Arc<PreferredSequencer<S, Rt, Da>>,
    shutdown_receiver: watch::Receiver<()>,
    replay_receiver: watch::Receiver<Option<SequenceNumber>>,
) -> JoinHandle<()>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    tokio::spawn(replica_sync_task_inner(
        sequencer,
        shutdown_receiver,
        replay_receiver,
    ))
}

async fn replica_sync_task_inner<S, Rt, Da>(
    sequencer: Arc<PreferredSequencer<S, Rt, Da>>,
    mut shutdown_receiver: watch::Receiver<()>,
    mut replay_receiver: watch::Receiver<Option<SequenceNumber>>,
) where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    assert!(
        sequencer.config.sequencer_kind_config.is_replica,
        "Master sequencer trying to spawn replica task, this should not be possible"
    );

    info!("Starting replica sync task for a replica sequencer");

    // Ensure we're running postgres, else replicas are not supported
    let Some(connection_string) = sequencer
        .config
        .sequencer_kind_config
        .postgres_connection_string
        .as_ref()
    else {
        warn!("Replicas are not supported on the rocksdb backend. Configure a postgres database that will be shared between all sequencers, and provide the `sequencer.preferred.postgres_connection_string` config value.");
        return;
    };

    // Create a separate persistent connection pool for querying transaction data
    let query_pool = match PgPoolOptions::default()
        .max_connections(5) // Small pool since we're just doing simple queries
        .connect(connection_string)
        .await
    {
        Ok(pool) => pool,
        Err(e) => {
            error!("Failed to connect to PostgreSQL: {e:?}. Replica shutting down.");
            exit_rollup(&sequencer.shutdown_sender).await;
            return;
        }
    };

    // Create a dedicated listener for PostgreSQL LISTEN/NOTIFY
    let mut listener = match PgListener::connect(connection_string).await {
        Ok(listener) => listener,
        Err(e) => {
            error!("Failed to create PostgreSQL listener: {e:?}. Replica shutting down.");
            exit_rollup(&sequencer.shutdown_sender).await;
            return;
        }
    };

    if let Err(e) = listener.listen("events_changes").await {
        error!("Failed to listen on events_changes channel: {e:?}. Replica shutting down.");
        exit_rollup(&sequencer.shutdown_sender).await;
        return;
    }

    debug!("Replica sync task: Successfully connected and listening for PostgreSQL notifications on 'events_changes' channel");

    // Use concurrent event processing for better throughput
    if let Err(e) = concurrent_event_processing_loop(
        sequencer.clone(),
        &mut listener,
        &query_pool,
        &mut shutdown_receiver,
        &mut replay_receiver,
    )
    .await
    {
        error!("Concurrent event processing failed: {e:?}. Replica shutting down.");
        exit_rollup(&sequencer.shutdown_sender).await;
    }
}

/// Concurrent event processing loop that handles notifications with parallel DB queries
async fn concurrent_event_processing_loop<S, Rt, Da>(
    sequencer: Arc<PreferredSequencer<S, Rt, Da>>,
    listener: &mut PgListener,
    query_pool: &PgPool,
    shutdown_receiver: &mut watch::Receiver<()>,
    replay_receiver: &mut watch::Receiver<Option<SequenceNumber>>,
) -> anyhow::Result<()>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    // This determines the maximum number of futures. The futures are used to fetch extra info from
    // the DB when a notification comes in, so that notification processing doesn't block on this.
    // In theory, this will only grow above the pool's max_connections() if either the replica is
    // processing transactions slower than the master, or when the replica hits a backfill - in
    // both cases, accumulating completed futures in the queue.
    // The PgListener has its own internal queue, so hitting this limit will just shift where the
    // notifications are queued. Having a large number of completed futures just ensures we'll be
    // able to start processing events immediately with all relevant DB lookups already completed,
    // in the case of a backfill. (In case the replica is too slow, we're in trouble anyway.)
    const MAX_CONCURRENT: usize = 1000;
    let mut pending_events: FuturesOrdered<PendingEventFuture> = FuturesOrdered::new();

    let mut latest_received_event_id: Option<u64> = None;
    let mut last_fully_processed_batch: Option<SequenceNumber> = None;
    loop {
        tokio::select! {
            // Only poll listener if we have capacity
            notification_result = listener.recv(), if pending_events.len() < MAX_CONCURRENT => {
                match notification_result {
                    Ok(notification) => {
                        trace!("Replica sync: received PostgreSQL notification: {:?}", notification);

                        let payload = notification.payload();
                        let parsed_notification = EventsNotificationPayload::parse_csv(payload)
                            .map_err(|e| anyhow!("Failed to parse CSV notification payload: {e}"))?;

                        let replayed_sequence_number = *replay_receiver.borrow();
                        match replayed_sequence_number {
                            // Node is not synced yet - ignore all incoming events
                            None => {
                                trace!("Replica receiving event but node is not synced yet, ignoring. Event: {payload}");
                                continue;
                            }
                            // We are indeed synced. Is the last replayed batch ahead of our event
                            // stream?
                            Some(last_replayed_sequence_number) => {
                                if last_fully_processed_batch.is_none_or(|seq| seq < last_replayed_sequence_number) {
                                    // Query the batch_close event with this sequence number to find the event_id
                                    // Set our latest_received_event_id to that event's id so we continue from there
                                    let new_latest_event_id = query_event_id_for_batch_close(query_pool, last_replayed_sequence_number).await?;
                                    if latest_received_event_id.is_some_and(|latest| latest >= new_latest_event_id) {
                                        // Return an error rather than using assert! - this way the
                                        // sequencer will call exit_rollup() cleanly
                                        anyhow::bail!("Sanity check failed - update_state fast-forwarded our sequence number, but this would not have fast-forwarded our event ID");
                                    }
                                    latest_received_event_id = Some(new_latest_event_id);
                                    last_fully_processed_batch = Some(last_replayed_sequence_number);
                                    debug!("Replica event processing fast-forwarding to sequence_number {}, event_id {}, due to update_state running ahead", last_replayed_sequence_number, new_latest_event_id);
                                }
                            }
                        }

                        // Check for gaps and add backfill if needed
                        if detect_gap(latest_received_event_id, parsed_notification.event_id).is_some() {
                            let backfill_future: PendingEventFuture = Box::pin(async move {
                                Ok(CompletedEvent::Backfill {
                                    current_latest: latest_received_event_id,
                                    target: parsed_notification.event_id,
                                })
                            });
                            pending_events.push_back(backfill_future);
                        } else if latest_received_event_id.map(|latest_id| parsed_notification.event_id <= latest_id).unwrap_or(false) {
                            continue;
                        }

                        latest_received_event_id = Some(parsed_notification.event_id);


                        // Create future for current notification
                        let event_future = create_event_future(parsed_notification, query_pool);
                        pending_events.push_back(event_future);

                    }
                    Err(e) => {
                        error!("Error in replica receiving PostgreSQL notification: {e:?}. Replica will sleep then retry.");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            },

            // Always process completions in FIFO order
            Some(completed_result) = pending_events.next() => {
                match completed_result? {
                    CompletedEvent::Event(db_event) => {
                        process_db_event(sequencer.clone(), db_event, query_pool, &mut last_fully_processed_batch).await?;
                    }
                    CompletedEvent::Backfill {
                        current_latest,
                        target,
                    } => {
                        backfill_to_event_id(sequencer.clone(), query_pool, current_latest, target).await?;
                    }
                }
            },

            _ = shutdown_receiver.changed() => {
                info!("Shutdown signal received, stopping concurrent event processing");
                break;
            }
        }
    }

    Ok(())
}

/// Detects if there's a gap between the last processed event and the current event
fn detect_gap(latest_received_event_id: Option<u64>, current_event_id: u64) -> Option<u64> {
    match latest_received_event_id {
        Some(last_id) if current_event_id > last_id + 1 => Some(last_id + 1),
        None if current_event_id > 1 => Some(1),
        _ => None,
    }
}

/// Creates a future for processing a single event notification
fn create_event_future(
    notification: EventsNotificationPayload,
    query_pool: &PgPool,
) -> PendingEventFuture {
    let pool = query_pool.clone();

    match notification.event_type {
        EventType::Transaction => {
            let sequence_number = notification.sequence_number;
            let index_in_batch = notification.index_in_batch.unwrap() as i64;

            Box::pin(async move {
                let (tx_hash, baked_tx) =
                    query_transaction_body_from_db(&pool, sequence_number, index_in_batch).await?;

                Ok(CompletedEvent::Event(DbEvent::TxAccepted(
                    baked_tx, tx_hash,
                )))
            })
        }
        EventType::BatchStart => {
            let sequence_number = notification.sequence_number;

            Box::pin(async move {
                let metadata = query_batch_metadata_from_db(&pool, sequence_number).await?;

                Ok(CompletedEvent::Event(DbEvent::BatchStarted {
                    sequence_number,
                    visible_slot_number_after_increase: metadata.visible_slot_number_after_increase,
                    visible_slots_to_advance: metadata.visible_slots_to_advance,
                }))
            })
        }
        EventType::BatchEnd => {
            let sequence_number = notification.sequence_number;

            Box::pin(
                async move { Ok(CompletedEvent::Event(DbEvent::BatchClosed(sequence_number))) },
            )
        }
        EventType::NewProof => {
            let sequence_number = notification.sequence_number;

            Box::pin(async move {
                Ok(CompletedEvent::Event(DbEvent::ProofBlobAccepted(
                    sequence_number,
                )))
            })
        }
    }
}

/// Processes a completed DB event
async fn process_db_event<S, Rt, Da>(
    sequencer: Arc<PreferredSequencer<S, Rt, Da>>,
    db_event: DbEvent,
    query_pool: &PgPool,
    last_fully_processed_batch: &mut Option<u64>,
) -> anyhow::Result<()>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    match db_event {
        DbEvent::TxAccepted(baked_tx, tx_hash) => {
            sequencer
                .synchronized_state_updator
                .do_new_tx_msg(tx_hash, baked_tx, "replica_sync_task:do_new_tx")
                .await;
            Ok(())
        }
        DbEvent::BatchClosed(_) => {
            *last_fully_processed_batch = Some(last_fully_processed_batch.map_or(0, |n| n + 1));
            sequencer
                .synchronized_state_updator
                .close_current_batch_msg("replica_sync_task:close_current_batch")
                .await;
            Ok(())
        }
        DbEvent::BatchStarted {
            sequence_number: _,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        } => {
            do_batch_start(
                sequencer,
                visible_slot_number_after_increase,
                visible_slots_to_advance,
            )
            .await
        }
        DbEvent::ProofBlobAccepted(sequence_number) => {
            do_proof_blob(sequencer, sequence_number, query_pool).await
        }
    }
}

async fn do_batch_start<S, Rt, Da>(
    sequencer: Arc<PreferredSequencer<S, Rt, Da>>,
    visible_slot_number_after_increase: VisibleSlotNumber,
    visible_slots_to_advance: NonZero<u8>,
) -> anyhow::Result<()>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    while {
        let process_latest_slot_number = sequencer
            .synchronized_state_updator
            .latest_slot_number_msg(
                "replica_sync_task:do_batch_start::wait_for_visible_slot_catchup",
            )
            .await;

        process_latest_slot_number < visible_slot_number_after_increase.as_true()
    } {
        // TODO: once read APIs get an is_ready state, set it to not ready here and reset
        // afterwards
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    sequencer
        .synchronized_state_updator
        .do_batch_start_msg(
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            "replica_sync_task:do_batch_start::start_batch",
        )
        .await;

    Ok(())
}

async fn do_proof_blob<S: Spec, Rt: Runtime<S>, Da: DaService<Spec = S::Da>>(
    sequencer: Arc<PreferredSequencer<S, Rt, Da>>,
    sequence_number: u64,
    query_pool: &PgPool,
) -> anyhow::Result<()> {
    let proof_blob = query_proof_blob_from_db(query_pool, sequence_number).await?;
    sequencer.produce_and_publish_proof_blob(proof_blob).await
}

/// Backfill missing batches and transactions to catch up to the current notification.
/// Fetches events in the open interval between latest_received_event_id and target_event_id: the
/// former is treated as already having been received, and the latter is presumably the ID of an
/// event that just came in that we noticed isn't consecutive with the current
/// latest_received_event_id (so we don't fetch it either).
async fn backfill_to_event_id<S: Spec, Rt: Runtime<S>, Da: DaService<Spec = S::Da>>(
    sequencer: Arc<PreferredSequencer<S, Rt, Da>>,
    query_pool: &PgPool,
    latest_received_event_id: Option<u64>,
    target_event_id: u64,
) -> anyhow::Result<()> {
    let mut current_event_id = latest_received_event_id.map(|id| id + 1).unwrap_or(0);

    // If we're already caught up, nothing to do
    if current_event_id > target_event_id {
        return Ok(());
    }

    debug!(
        "Backfilling events from {} to {}",
        current_event_id, target_event_id
    );

    // Process events in pages (batches, not to be confused with sequencer batches) to avoid
    // excessive memory consumption
    const PAGE_SIZE: i64 = 2000;
    while current_event_id < target_event_id {
        let page_end = std::cmp::min(current_event_id + PAGE_SIZE as u64, target_event_id);

        trace!(
            "Processing backfill page: events {} to {}",
            current_event_id,
            page_end
        );

        // Query and process events for this page
        let events = sqlx::query(
            "SELECT event_id, sequence_number, index_in_batch, event_type, hash, data FROM events 
             WHERE event_id >= $1 AND event_id < $2
             ORDER BY event_id ASC",
        )
        .bind(current_event_id as i64)
        .bind(page_end as i64)
        .fetch_all(query_pool)
        .await?;

        for row in events {
            let event_type: EventType = row.get("event_type");
            let sequence_number = row.get::<i64, _>("sequence_number") as u64;

            match event_type {
                EventType::Transaction => {
                    let hash_bytes: Vec<u8> = row.get("hash");
                    let tx_data: Vec<u8> = row.get("data");

                    let tx_hash = TxHash::new(hash_bytes.try_into().map_err(|_| {
                        anyhow::anyhow!("Invalid transaction hash length from database")
                    })?);
                    let baked_tx = FullyBakedTx::new(tx_data);

                    sequencer
                        .synchronized_state_updator
                        .do_new_tx_msg(tx_hash, baked_tx, "replica_sync_task:do_new_tx")
                        .await;
                }
                EventType::BatchStart => {
                    let batch_data: Vec<u8> = row.get("data");
                    let batch_metadata = parse_serialized_batch(batch_data, sequence_number)?;

                    do_batch_start(
                        sequencer.clone(),
                        batch_metadata.visible_slot_number_after_increase,
                        batch_metadata.visible_slots_to_advance,
                    )
                    .await?;
                }
                EventType::BatchEnd => {
                    sequencer
                        .synchronized_state_updator
                        .close_current_batch_msg("replica_sync_task:close_current_batch")
                        .await;
                }
                EventType::NewProof => {
                    do_proof_blob(sequencer.clone(), sequence_number, query_pool).await?;
                }
            }
        }

        current_event_id = page_end;
    }

    debug!("Backfill completed successfully");
    Ok(())
}

async fn query_batch_metadata_from_db(
    query_pool: &PgPool,
    sequence_number: u64,
) -> anyhow::Result<BatchMetadata> {
    let blob_data: Vec<u8> = sqlx::query(
        "SELECT data FROM events WHERE sequence_number = $1 AND event_type = 'batch_start'",
    )
    .bind(i64::try_from(sequence_number)?)
    .fetch_one(query_pool)
    .await?
    .get("data");

    parse_serialized_batch(blob_data, sequence_number)
}

fn parse_serialized_batch(data: Vec<u8>, sequence_number: u64) -> anyhow::Result<BatchMetadata> {
    let stored_blob: StoredBlob = borsh::from_slice(&data)?;

    match stored_blob {
        StoredBlob::Batch {
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            ..
        } => Ok(BatchMetadata {
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        }),
        StoredBlob::Proof { .. } => Err(anyhow::anyhow!(
            "Expected batch blob but found proof blob for sequence_number {}",
            sequence_number
        )),
    }
}

async fn query_transaction_body_from_db(
    query_pool: &PgPool,
    sequence_number: u64,
    index_in_batch: i64,
) -> anyhow::Result<(TxHash, FullyBakedTx)> {
    let row =
        sqlx::query("SELECT hash, data FROM events WHERE sequence_number = $1 AND index_in_batch = $2 AND event_type = 'transaction'")
            .bind(i64::try_from(sequence_number)?)
            .bind(index_in_batch)
            .fetch_one(query_pool)
            .await?;

    let hash_bytes: Vec<u8> = row.get("hash");
    let tx_data: Vec<u8> = row.get("data");

    let tx_hash = TxHash::new(
        hash_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid transaction hash length from database"))?,
    );

    let baked_tx = FullyBakedTx::new(tx_data);

    Ok((tx_hash, baked_tx))
}

async fn query_proof_blob_from_db(
    query_pool: &PgPool,
    sequence_number: u64,
) -> anyhow::Result<Arc<[u8]>> {
    let row = sqlx::query("SELECT borsh_value FROM proof_blobs WHERE sequence_number = $1")
        .bind(i64::try_from(sequence_number)?)
        .fetch_one(query_pool)
        .await?;

    let blob_data: Vec<u8> = row.get("borsh_value");
    let stored_blob: StoredBlob = borsh::from_slice(&blob_data)?;

    match stored_blob {
        StoredBlob::Proof { data, .. } => Ok(data),
        StoredBlob::Batch { .. } => Err(anyhow::anyhow!(
            "Expected proof blob but found batch blob for sequence_number {}",
            sequence_number
        )),
    }
}

async fn query_event_id_for_batch_close(
    query_pool: &PgPool,
    sequence_number: u64,
) -> anyhow::Result<u64> {
    let row = sqlx::query(
        "SELECT event_id FROM events WHERE sequence_number = $1 AND event_type = 'batch_end'",
    )
    .bind(i64::try_from(sequence_number)?)
    .fetch_one(query_pool)
    .await?;

    let event_id: i64 = row.get("event_id");
    Ok(event_id as u64)
}
