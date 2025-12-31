use crate::preferred::db::BatchToStore;
use crate::preferred::db::StoredBlob;
use crate::Serialize;
use serde::Deserialize;
use sov_modules_api::FullyBakedTx;
use sov_modules_api::TxHash;
use sqlx::postgres::PgRow;
use sqlx::PgPool;
use sqlx::Row;
use std::io;
use std::num::ParseIntError;
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ParsingError {
    #[error("Invalid event_type: {0}")]
    InvalidEventType(String),

    #[error("Invalid CSV format: expected 4 fields, got {0}")]
    InvalidCsvFormat(usize),

    #[error("Invalid event_id '{0}': {1}")]
    InvalidEventId(String, #[source] ParseIntError),

    #[error("Invalid sequence_number '{0}': {1}")]
    InvalidSequenceNumber(String, #[source] ParseIntError),

    #[error("Invalid index_in_batch '{0}': {1}")]
    InvalidIndexInBatch(String, #[source] ParseIntError),

    #[error("Unexpected proof: sequence number: '{0}'")]
    UnexpectedProof(u64),

    #[error("Borsh deserialization error: '{0}'")]
    Borsh(io::Error),
}

/// Event type enum for type-safe parsing
#[derive(Debug, Deserialize, Serialize, Copy, Clone, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "event_type", rename_all = "snake_case")]
pub(crate) enum EventType {
    Transaction,
    BatchStart,
    BatchEnd,
    NewProof,
}

impl EventType {
    #[allow(clippy::match_same_arms)]
    pub(crate) fn is_event_sequence_valid(prev: Option<Self>, current: Self) -> bool {
        match (prev, current) {
            (Some(EventType::BatchStart), EventType::BatchStart) => false,
            (Some(EventType::Transaction), EventType::BatchStart) => false,
            (Some(EventType::BatchEnd), EventType::Transaction) => false,
            (Some(EventType::BatchEnd), EventType::BatchEnd) => false,
            (None, EventType::BatchStart) => true,
            (None, _) => false,
            (_, _) => true,
        }
    }
}

impl FromStr for EventType {
    type Err = ParsingError;

    fn from_str(s: &str) -> Result<Self, ParsingError> {
        match s {
            "transaction" => Ok(EventType::Transaction),
            "batch_start" => Ok(EventType::BatchStart),
            "batch_end" => Ok(EventType::BatchEnd),
            "new_proof" => Ok(EventType::NewProof),
            _ => Err(ParsingError::InvalidEventType(s.to_string())),
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
    pub(crate) fn parse_csv(payload: &str) -> Result<Self, ParsingError> {
        let parts: Vec<&str> = payload.split(',').collect();
        if parts.len() != 4 {
            return Err(ParsingError::InvalidCsvFormat(parts.len()));
        }

        Ok(EventsNotificationPayload {
            event_id: parts[0]
                .parse()
                .map_err(|e| ParsingError::InvalidEventId(parts[0].to_string(), e))?,
            sequence_number: parts[1]
                .parse()
                .map_err(|e| ParsingError::InvalidSequenceNumber(parts[1].to_string(), e))?,
            event_type: parts[2].parse()?,
            index_in_batch: if parts[3].is_empty() {
                None
            } else {
                Some(
                    parts[3]
                        .parse()
                        .map_err(|e| ParsingError::InvalidIndexInBatch(parts[3].to_string(), e))?,
                )
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DbData {
    BatchStart(BatchToStore),
    Transaction(u64, FullyBakedTx, TxHash),
    BatchEnd(BatchToStore),
    NewProof,
}

impl DbData {
    pub(crate) fn is_batch_end(&self) -> bool {
        matches!(self, DbData::BatchEnd(_))
    }

    pub(crate) fn sequence_number(&self) -> u64 {
        match self {
            DbData::BatchStart(batch_to_store) | DbData::BatchEnd(batch_to_store) => {
                batch_to_store.sequence_number
            }
            DbData::Transaction(sequence_number, _, _) => *sequence_number,
            DbData::NewProof => 0,
        }
    }
}

pub(crate) async fn rows(
    query_pool: &PgPool,
    page_end: u64,
    current_event_id: u64,
) -> Result<Vec<PgRow>, sqlx::Error> {
    // Query and process events for this page
    sqlx::query(
        "SELECT event_id, sequence_number, index_in_batch, event_type, hash, data FROM events
                 WHERE event_id >= $1 AND event_id <= $2
                 ORDER BY event_id ASC",
    )
    .bind(current_event_id as i64)
    .bind(page_end as i64)
    .fetch_all(query_pool)
    .await
}

pub(crate) fn row_to_event(row: PgRow) -> Result<(DbData, EventType), ParsingError> {
    let event_type: EventType = row.get("event_type");
    let sequence_number = row.get::<i64, _>("sequence_number") as u64;
    let data: Vec<u8> = row.get("data");

    let event = match event_type {
        EventType::BatchStart => {
            let batch_to_store = parse_serialized_batch(data, sequence_number)?;
            DbData::BatchStart(batch_to_store)
        }
        EventType::Transaction => {
            // Deserialize the full FullyBakedTx (including sequencing_data)
            let baked_tx = borsh::from_slice::<FullyBakedTx>(&data).map_err(ParsingError::Borsh)?;
            let tx_hash: TxHash = TxHash::new(row.get("hash"));
            DbData::Transaction(sequence_number, baked_tx, tx_hash)
        }

        EventType::BatchEnd => {
            let batch_to_store = parse_serialized_batch(data, sequence_number)?;
            DbData::BatchEnd(batch_to_store)
        }
        EventType::NewProof => DbData::NewProof,
    };

    Ok((event, event_type))
}

fn parse_serialized_batch(
    data: Vec<u8>,
    sequence_number: u64,
) -> Result<BatchToStore, ParsingError> {
    let stored_blob: StoredBlob = borsh::from_slice(&data).map_err(ParsingError::Borsh)?;
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
        StoredBlob::Proof { .. } => Err(ParsingError::UnexpectedProof(sequence_number)),
    }
}
