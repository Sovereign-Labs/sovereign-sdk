use crate::preferred::db::{BatchToStore, StoredBlob};
use crate::preferred::exit_rollup;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sov_modules_api::FullyBakedTx;
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::PgPool;
use sqlx::Row;
use std::str::FromStr;
use tokio::sync::watch;
use tracing::{debug, error, trace};

const MAX_DB_ERRORS_ALLOWED: u32 = 10;

// Process events in pages to avoid excessive memory consumption
const PAGE_SIZE: i64 = 2000;

/// Event type enum for type-safe parsing
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "event_type", rename_all = "snake_case")]
pub(crate) enum EventType {
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
pub(crate) struct EventsNotificationPayload {
    pub(crate) event_id: u64,
    pub(crate) sequence_number: u64,
    pub(crate) event_type: EventType,
    pub(crate) index_in_batch: Option<u64>, // Only present for transaction events
}

impl EventsNotificationPayload {
    pub(crate) fn parse_csv(payload: &str) -> anyhow::Result<Self> {
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

#[derive(thiserror::Error, Debug)]
pub(crate) enum EventReceiverError {
    #[error("Error while querying for  db data: {0}")]
    DbError(#[from] sqlx::Error),

    #[error("Error while parsing the db notification: {0}")]
    ParsingError(#[from] anyhow::Error),
}

pub(crate) struct EventReceiver {
    connection_string: String,
    db_data_sender: tokio::sync::mpsc::Sender<DbData>,
    db_data_receiver: tokio::sync::mpsc::Receiver<DbData>,
    shutdown_sender: watch::Sender<()>,
}

impl EventReceiver {
    pub(crate) async fn new(connection_string: String, shutdown_sender: watch::Sender<()>) -> Self {
        let (db_data_sender, db_data_receiver) = tokio::sync::mpsc::channel(PAGE_SIZE as usize);
        Self {
            connection_string,
            db_data_sender,
            db_data_receiver,
            shutdown_sender,
        }
    }

    pub(crate) async fn spawn_db_data_fetcher(&mut self) {
        // Create a separate persistent connection pool for querying transaction data
        let query_pool = match PgPoolOptions::default()
            .max_connections(5) // Small pool since we're just doing simple queries
            .connect(&self.connection_string)
            .await
        {
            Ok(pool) => pool,
            Err(e) => {
                error!("Failed to connect to PostgreSQL: {e:?}. Replica shutting down.");
                exit_rollup(&self.shutdown_sender).await;
                unreachable!("EventReceiver: impossible happened rollup didn't exit");
            }
        };

        let mut start_event_id = match latest_event_id(&query_pool).await {
            Ok(latest_event_id) => latest_event_id,
            Err(e) => {
                error!("Failed to get latest event id: {e:?}. Replica shutting down.");
                exit_rollup(&self.shutdown_sender).await;
                unreachable!("EventReceiver: impossible happened rollup didn't exit");
            }
        };

        // Create a dedicated listener for PostgreSQL LISTEN/NOTIFY
        let mut listener = match PgListener::connect(&self.connection_string).await {
            Ok(listener) => listener,
            Err(e) => {
                error!("Failed to create PostgreSQL listener: {e:?}. Replica shutting down.");
                exit_rollup(&self.shutdown_sender).await;
                unreachable!("EventReceiver: impossible happened rollup didn't exit");
            }
        };

        if let Err(e) = listener.listen("events_changes").await {
            error!("Failed to listen on events_changes channel: {e:?}. Replica shutting down.");
            exit_rollup(&self.shutdown_sender).await;
            unreachable!("EventReceiver: impossible happened rollup didn't exit");
        }

        let mut nb_of_consecutive_db_errors = 0;
        let shutdown_sender = self.shutdown_sender.clone();
        let shutdown_receiver = self.shutdown_sender.subscribe();

        let mut db_data_sender = self.db_data_sender.clone();

        tokio::spawn(async move {
            loop {
                if shutdown_receiver.has_changed().unwrap_or(true) {
                    break;
                }

                match Self::fetch_data(
                    start_event_id,
                    &query_pool,
                    &mut listener,
                    &mut db_data_sender,
                )
                .await
                {
                    Ok(next_event_id) => {
                        nb_of_consecutive_db_errors = 0;
                        start_event_id = Some(next_event_id + 1);
                    }
                    Err(err) => {
                        match err {
                            EventReceiverError::ParsingError(e) => {
                                // This should never happen, so we shut down the replica immediately
                                error!(
                                    "Failed to parse notification: {e:?}. Shutting down replica."
                                );
                                exit_rollup(&shutdown_sender).await;
                            }

                            EventReceiverError::DbError(e) => {
                                error!("Failed to receive notifications from database: {e:?}. Shutting down replica.");

                                if shutdown_receiver.has_changed().unwrap_or(true) {
                                    break;
                                }

                                // Since network errors can occur, we will retry receiving a few times before initiating replica shutdown.
                                if nb_of_consecutive_db_errors >= MAX_DB_ERRORS_ALLOWED {
                                    error!("Failed to connect to the database after {nb_of_consecutive_db_errors} attempts. Shutting down replica.");
                                    exit_rollup(&shutdown_sender).await;
                                }

                                nb_of_consecutive_db_errors += 1;
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                continue;
                            }
                        }
                    }
                }
            }
        });
    }

    pub(crate) async fn recv(&mut self) -> Option<DbData> {
        self.db_data_receiver.recv().await
    }

    async fn recv_notifications(
        listener: &mut PgListener,
    ) -> Result<EventsNotificationPayload, EventReceiverError> {
        let mut last_notification = None;

        // We only care about the latest notification from the DB,
        // since the backfill logic allows us to skip earlier ones.
        while let Some(p) = listener.next_buffered() {
            last_notification = Some(p);
        }

        let pg_notification = match last_notification {
            Some(notification) => notification,
            None => listener.recv().await?,
        };

        let payload = pg_notification.payload();
        let parsed_notification = EventsNotificationPayload::parse_csv(payload)?;
        Ok(parsed_notification)
    }

    async fn fetch_data(
        start_event_id: Option<u64>,
        query_pool: &PgPool,
        listener: &mut PgListener,
        db_data_sender: &mut tokio::sync::mpsc::Sender<DbData>,
    ) -> Result<u64, EventReceiverError> {
        let notification = Self::recv_notifications(listener).await?;

        Self::backfill_to_event_id(
            query_pool,
            start_event_id,
            notification.event_id,
            db_data_sender,
        )
        .await?;

        Ok(notification.event_id)
    }

    async fn backfill_to_event_id(
        query_pool: &PgPool,
        current_event_id: Option<u64>,
        target_event_id: u64,
        db_data_sender: &mut tokio::sync::mpsc::Sender<DbData>,
    ) -> Result<(), EventReceiverError> {
        let mut current_event_id = current_event_id.unwrap_or(1);

        // If we're already caught up, nothing to do
        if current_event_id > target_event_id {
            return Ok(());
        }

        debug!(
            "Backfilling events from {} to {}",
            current_event_id, target_event_id
        );

        // Currently, we fetch data in a loop. One possible (but not yet necessary) optimization would be to issue
        // multiple parallel queries to the DB for different `event_id`` ranges.
        // After analyzing real-world workloads, we can revisit this optimization. Implementing it would only affect
        // the contents of this method and would not require significant refactoring.
        while current_event_id <= target_event_id {
            let page_end = std::cmp::min(current_event_id + PAGE_SIZE as u64, target_event_id);

            trace!(
                "Processing backfill page: events {} to {}",
                current_event_id,
                page_end
            );

            // Query and process events for this page
            let events = sqlx::query(
                "SELECT event_id, sequence_number, index_in_batch, event_type, hash, data FROM events 
                 WHERE event_id >= $1 AND event_id <= $2
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
                    EventType::BatchStart => {
                        let batch_data: Vec<u8> = row.get("data");
                        let batch_to_store = parse_serialized_batch(batch_data, sequence_number)?;

                        let _ = db_data_sender
                            .send(DbData::BatchStart(batch_to_store))
                            .await;
                    }
                    EventType::Transaction => {
                        let tx_data: Vec<u8> = row.get("data");

                        let baked_tx = FullyBakedTx::new(tx_data);
                        let _ = db_data_sender.send(DbData::Transaction(baked_tx)).await;
                    }

                    EventType::BatchEnd => {
                        let batch_data: Vec<u8> = row.get("data");
                        let batch_to_store = parse_serialized_batch(batch_data, sequence_number)?;

                        let _ = db_data_sender.send(DbData::BatchEnd(batch_to_store)).await;
                    }
                    EventType::NewProof => {
                        let _ = db_data_sender.send(DbData::NewProof).await;
                    }
                }
            }

            current_event_id = page_end + 1;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DbData {
    BatchStart(BatchToStore),
    Transaction(FullyBakedTx),
    BatchEnd(BatchToStore),
    NewProof,
}

fn parse_serialized_batch(data: Vec<u8>, sequence_number: u64) -> anyhow::Result<BatchToStore> {
    let stored_blob: StoredBlob = borsh::from_slice(&data)?;

    match stored_blob {
        StoredBlob::Batch {
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            blob_id,
        } => Ok(BatchToStore {
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            blob_id,
            sequence_number,
        }),
        StoredBlob::Proof { .. } => Err(anyhow::anyhow!(
            "Expected batch blob but found proof blob for sequence_number {}",
            sequence_number
        )),
    }
}

async fn latest_event_id(query_pool: &PgPool) -> Result<Option<u64>, sqlx::Error> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(event_id) FROM events")
            .fetch_one(query_pool)
            .await?
            .map(|id| id as u64),
    )
}
