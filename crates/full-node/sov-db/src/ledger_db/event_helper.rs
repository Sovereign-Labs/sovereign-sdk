use anyhow::{ensure, Context};
use borsh::BorshDeserialize;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::ledger_api::{EventIdentifier, PaginatedEventResponse};
use sov_rollup_interface::stf::{EventKey, StoredEvent};

use crate::ledger_db::rpc::LedgerRpcReader;
use crate::ledger_db::rpc_constants::MAX_BATCHES_PER_REQUEST;
use crate::ledger_db::LedgerDb;
use crate::schema::tables::{BatchByNumber, EventByKey, SlotByNumber};
use crate::schema::types::{BatchNumber, EventNumber, TxNumber};

impl LedgerRpcReader {
    async fn get_events_by_key_helper<E>(
        &self,
        event_key: &str,
        txn_range: Option<(u64, u64)>,
        num_events: usize,
        next: Option<&str>,
    ) -> anyhow::Result<PaginatedEventResponse<E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let scan_key_start = match next {
            Some(start_key) => {
                let key_bytes = hex::decode(start_key)?;
                let composite_key: (EventKey, TxNumber, EventNumber) =
                    BorshDeserialize::try_from_slice(&key_bytes)?;
                composite_key
            }
            None => (
                EventKey::new(event_key.as_bytes()),
                TxNumber(txn_range.unwrap_or((0u64, 0u64)).0),
                EventNumber(0u64),
            ),
        };

        let paginated_query_response = self
            .db
            .get_n_from_first_match_async::<EventByKey>(&scan_key_start, num_events)
            .await?;

        let (event_keys, next_key) = (
            paginated_query_response.key_value,
            paginated_query_response.next,
        );
        let event_keys: Vec<((EventKey, TxNumber, EventNumber), ())> = event_keys
            .into_iter()
            .filter(|((e_key, t_num, _), _)| {
                event_match_helper(e_key, event_key, t_num.0, txn_range)
            })
            .collect();

        let event_ids: Vec<EventIdentifier> = event_keys
            .into_iter()
            .map(|(k, _)| EventIdentifier::Number(k.2 .0))
            .collect();
        let events_response: Vec<E> = self
            .get_events::<E>(&event_ids)
            .await?
            .into_iter()
            .flatten()
            .collect();
        let next = next_key
            .and_then(|next_key| {
                if !event_match_helper(&next_key.0, event_key, next_key.2 .0, txn_range) {
                    None
                } else {
                    Some(next_key)
                }
            })
            .map(|next_key| borsh::to_vec(&next_key).map(hex::encode))
            .transpose()?;
        Ok(PaginatedEventResponse {
            events_response,
            next,
        })
    }
    async fn get_events_by_key_slot_range_helper<E>(
        &self,
        event_key: &str,
        slot_height_start: SlotNumber,
        slot_height_end: SlotNumber,
        num_events: usize,
        next: Option<&str>,
    ) -> anyhow::Result<PaginatedEventResponse<E>>
    where
        E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
    {
        let (txn_range, next_key) = match next {
            None => {
                let read_slot = |slot_num| async move {
                    self.db
                        .get_async::<SlotByNumber>(&slot_num)
                        .await
                        .with_context(|| format!("Failed to query slot with number: {slot_num}"))
                        .and_then(|slot_opt| {
                            slot_opt.with_context(|| {
                                format!("Slot with number: {slot_num} does not exist in storage")
                            })
                        })
                };
                let (slots_result_start, slots_result_end) =
                    tokio::try_join!(read_slot(slot_height_start), read_slot(slot_height_end))?;

                let batch_start_num = slots_result_start.batches.start;
                let batch_end_num = slots_result_end.batches.end;

                ensure!(batch_end_num.0 - batch_start_num.0 < MAX_BATCHES_PER_REQUEST);

                let read_batch = |batch_num: BatchNumber| async move {
                    self.db
                        .get_async::<BatchByNumber>(&batch_num)
                        .await
                        .with_context(|| {
                            format!("Failed to query batch with number: {}", batch_num.0)
                        })
                        .and_then(|slot_opt| {
                            slot_opt.with_context(|| {
                                format!(
                                    "Batch with number: {} does not exist in storage",
                                    batch_num.0
                                )
                            })
                        })
                };
                let (batch_result_start, batch_result_end) =
                    tokio::try_join!(read_batch(batch_start_num), read_batch(batch_end_num))?;

                let txn_start_num = batch_result_start.txs.start;
                let txn_end_num = batch_result_end.txs.end;
                ((txn_start_num.0, txn_end_num.0), None)
            }
            Some(wrapped_next) => {
                let key_bytes = hex::decode(wrapped_next)?;
                let composite_key: ((u64, u64), String) =
                    BorshDeserialize::try_from_slice(&key_bytes)?;
                (composite_key.0, Some(composite_key.1))
            }
        };

        let paginated_query_response = self
            .get_events_by_key_helper::<E>(
                event_key,
                Some((txn_range.0, txn_range.1)),
                num_events,
                next_key.as_deref(),
            )
            .await?;
        let (event_response, next_key) = (
            paginated_query_response.events_response,
            paginated_query_response.next,
        );

        let re_encoded_next = next_key.and_then(|inner_next| {
            borsh::to_vec(&(txn_range, inner_next))
                .ok()
                .map(hex::encode)
        });

        Ok(PaginatedEventResponse {
            events_response: event_response,
            next: re_encoded_next,
        })
    }
}

fn event_match_helper(
    scanned_key: &EventKey,
    provided_key: &str,
    scanned_txn_num: u64,
    provided_txn_range: Option<(u64, u64)>,
) -> bool {
    let event_key_match = scanned_key.inner().as_slice() == provided_key.as_bytes();
    let txn_num_match = match provided_txn_range {
        Some(txn_range) => scanned_txn_num >= txn_range.0 && scanned_txn_num <= txn_range.1,
        None => true, // If transaction_num is not provided, always true
    };
    event_key_match && txn_num_match
}

/// Fetches a list of events by their key, with support for optional filtering based on a transaction range.
///
/// This function provides a way to query events stored in a ledger database, allowing for precise data retrieval through optional transaction ranges. Pagination is supported via a cursor passed as the `next` parameter, allowing for efficient data fetching in large datasets.
///
/// # Parameters
/// - `ledger_db`: Reference to the `LedgerDB` storage.
/// - `event_key`: The key associated with the desired events.
/// - `txn_range`: An optional range of transactions `(start, end)` to filter the events by. If `None`, events are not filtered by transaction range.
/// - `num_events`: The maximum number of events to return. This acts as a limit for the query, useful for pagination.
/// - `next`: An optional pagination cursor indicating where to continue fetching events. If `None`, fetching starts from the beginning of the dataset.
///
/// # Returns
/// A collection of events that match the given criteria, limited by `num_events`. The exact return type depends on the `E` type parameter, which is determined by the `StoredEventToResponseConverter` trait implementation.
pub async fn get_events_by_key_helper<E>(
    ledger_db: &LedgerDb,
    event_key: &str,
    txn_range: Option<(u64, u64)>,
    num_events: usize,
    next: Option<&str>,
) -> anyhow::Result<PaginatedEventResponse<E>>
where
    E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
{
    ledger_db
        .get_rpc_reader()
        .get_events_by_key_helper(event_key, txn_range, num_events, next)
        .await
}

/// Fetches a list of events by their key within a specified slot height range
///
/// This function enables precise event retrieval from the ledger database by combining primary key matching with slot height range filtering.
/// Pagination is facilitated through the `next` parameter.
///
/// # Parameters
/// - `ledger_db`: Reference to the `LedgerDB` instance where events are stored.
/// - `event_key`: The primary key associated with the events of interest.
/// - `slot_height_start`: The starting slot height for the range filter.
/// - `slot_height_end`: The ending slot height for the range filter.
/// - `num_events`: The maximum number of events to retrieve, useful for controlling query load and for pagination.
/// - `next`: An optional cursor for pagination, specifying where to continue fetching events. If `None`, starts from the beginning of the matching dataset.
///
/// # Returns
/// A collection of events that fit the specified criteria, constrained by `num_events`. The exact return type is determined by the `E` type parameter based on the `StoredEventToResponseConverter` trait.
pub async fn get_events_by_key_slot_range_helper<E>(
    ledger_db: &LedgerDb,
    event_key: &str,
    slot_height_start: SlotNumber,
    slot_height_end: SlotNumber,
    num_events: usize,
    next: Option<&str>,
) -> anyhow::Result<PaginatedEventResponse<E>>
where
    E: for<'a> TryFrom<(u64, &'a StoredEvent), Error = anyhow::Error> + Send + Sync,
{
    ledger_db
        .get_rpc_reader()
        .get_events_by_key_slot_range_helper(
            event_key,
            slot_height_start,
            slot_height_end,
            num_events,
            next,
        )
        .await
}
