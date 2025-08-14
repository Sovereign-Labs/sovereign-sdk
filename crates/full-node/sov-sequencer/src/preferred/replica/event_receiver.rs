use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::PgPool;
use std::str::FromStr;
use tracing::error;

/// Event type enum for type-safe parsing
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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

pub(crate) struct EventReceiver {
    listener: PgListener,
    query_pool: PgPool,
}

impl EventReceiver {
    pub(crate) async fn new(connection_string: &str) -> anyhow::Result<Self> {
        // Create a separate persistent connection pool for querying transaction data
        let query_pool = match PgPoolOptions::default()
            .max_connections(5) // Small pool since we're just doing simple queries
            .connect(connection_string)
            .await
        {
            Ok(pool) => pool,
            Err(e) => {
                error!("Failed to connect to PostgreSQL: {e:?}. Replica shutting down.");
                //exit_rollup(&sequencer.shutdown_sender).await;
                todo!("Error handling")
            }
        };

        // Create a dedicated listener for PostgreSQL LISTEN/NOTIFY
        let mut listener = match PgListener::connect(connection_string).await {
            Ok(listener) => listener,
            Err(e) => {
                error!("Failed to create PostgreSQL listener: {e:?}. Replica shutting down.");
                //exit_rollup(&sequencer.shutdown_sender).await;
                todo!("Error handling")
            }
        };

        if let Err(e) = listener.listen("events_changes").await {
            error!("Failed to listen on events_changes channel: {e:?}. Replica shutting down.");
            //exit_rollup(&sequencer.shutdown_sender).await;
            todo!("Error handling")
        }

        Ok(Self {
            listener,
            query_pool,
        })
    }

    pub(crate) async fn recv(
        &mut self,
    ) -> anyhow::Result<anyhow::Result<EventsNotificationPayload>> {
        Ok(self.listener.recv().await.map(|notification| {
            let payload = notification.payload();
            let parsed_notification = EventsNotificationPayload::parse_csv(payload)
                .map_err(|e| anyhow!("Failed to parse CSV notification payload: {e}"));

            parsed_notification
        })?)
    }
}
