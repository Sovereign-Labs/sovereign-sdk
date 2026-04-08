use std::collections::{BTreeMap, HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;

use sov_modules_api::capabilities::TransactionAuthenticator;

use futures::task::Poll;
use futures::{Future, FutureExt, Stream, StreamExt};
use sov_db::ledger_db::LedgerDb;
use sov_modules_api::capabilities::get_maybe_timestamp_from_sequencing_data;
use sov_modules_api::{HexString, Runtime, RuntimeEventResponse, Spec, TxHash};
use sov_rollup_interface::node::ledger_api::{EventIdentifier, LedgerStateProvider, QueryMode};
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use crate::common::{SequencerTxStream, SubscriptionStreamError};
use crate::preferred::{AcceptedTx, Confirmation};
use crate::rest_api::ApiAcceptedTx;

type TxStreamItem<S, Rt> = Result<ApiAcceptedTx<Confirmation<S, Rt>>, SubscriptionStreamError>;
type GetNextChunkFuture<S, Rt> = Pin<
    Box<
        dyn Future<
                Output = Result<
                    (
                        Vec<ApiAcceptedTx<Confirmation<S, Rt>>>,
                        Option<BroadcastStream<AcceptedTx<Confirmation<S, Rt>>>>,
                    ),
                    SubscriptionStreamError,
                >,
            > + Send,
    >,
>;

/// The number of txs to fetch from the cache and/or the DB at a time. The larger the chunk size, the more
/// memory we will consume.
///
/// Note that we subscribe to the broadcast channel as soon as we've gotten within 1 chunk of the current tx number,
/// so if the chunk size is very large we run a risk that we'll lag in receiving from the broadcast while trying to clear
/// out the chunk of txs - which will cause the websocket to disconnect.
const CHUNK_SIZE: u64 = 100;

pub struct TxResultWriter<S: Spec, Rt: Runtime<S>> {
    inner: ArcInner<S, Rt>,
}

impl<S: Spec, Rt: Runtime<S>> Clone for TxResultWriter<S, Rt> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

type ArcInner<S, Rt> = Arc<tokio::sync::RwLock<TransactionCacheInner<S, Rt>>>;

impl<S: Spec, Rt: Runtime<S>> TxResultWriter<S, Rt> {
    pub async fn insert(&self, tx: AcceptedTx<Confirmation<S, Rt>>) {
        let mut transaction_cache = self.inner.write().await;
        assert_eq!(
            tx.confirmation.tx_number, transaction_cache.next_tx_number,
            "Transactions must be inserted in order"
        );
        transaction_cache
            .cache
            .insert(tx.confirmation.tx_number, tx.clone());
        transaction_cache
            .tx_hash_index
            .insert(tx.tx_hash, tx.confirmation.tx_number);
        transaction_cache.next_tx_number = tx.confirmation.tx_number + 1;
        if let Some((latest_event_number, _)) =
            transaction_cache.event_numbers_index.last_key_value()
        {
            if let Some(newest_event_number) = tx.confirmation.events.first().map(|e| e.number) {
                assert_eq!(
                    newest_event_number,
                    latest_event_number + 1,
                    "Events must be inserted in order"
                );
            }
        }
        for event in tx.confirmation.events.iter() {
            transaction_cache
                .event_numbers_index
                .insert(event.number, tx.confirmation.tx_number);
        }
        let _ = transaction_cache.tx_response_sender.send(tx); // We don't care if there are no listeners
    }

    pub async fn clean_and_overwrite_next_tx_number(&self, tx_number: u64) {
        tracing::debug!(
            "Cleaning and overwriting transaction cache up to {}",
            tx_number
        );
        let mut transaction_cache = self.inner.write().await;
        transaction_cache.next_tx_number = tx_number;
        transaction_cache.cache.clear();
        transaction_cache.tx_hash_index.clear();
        transaction_cache.event_numbers_index.clear();
    }

    pub async fn prune(&self, next_tx_number: u64) {
        tracing::trace!(pruned_up_to = %next_tx_number, "Pruning transaction cache");
        let mut transaction_cache = self.inner.write().await;
        let TransactionCacheInner {
            cache,
            tx_hash_index,
            event_numbers_index,
            ..
        } = &mut *transaction_cache;
        let mut event_number_to_prune = None;
        // Split off returns everything greater than or equal to the key, so call it and then do a swap so that the items we *don't* want to prune are in the new cache
        // and the items we do want to prune are left behind in the struct.
        let mut to_retain = cache.split_off(&next_tx_number);
        std::mem::swap(&mut to_retain, cache);
        let to_drop = to_retain; // Rename the variables to reflect the swap

        // Iterate over the items we want to drop and remove them from the tx_hash_index and event_numbers_index
        for tx in to_drop.values() {
            tx_hash_index.remove(&tx.tx_hash);
            if let Some(event_number) = tx.confirmation.events.last().map(|e| e.number) {
                event_number_to_prune = Some(event_number);
            }
        }
        if let Some(event_number) = event_number_to_prune {
            let mut to_retain = event_numbers_index.split_off(&(event_number + 1)); // Retain everything after the last event number to prune
            std::mem::swap(&mut to_retain, event_numbers_index);
        }
    }
}

#[derive(Debug)]
pub(crate) struct TransactionCache<S: Spec, Rt: Runtime<S>> {
    inner: ArcInner<S, Rt>,
    ledger_db: LedgerDb,
    // A receiver we can clone so that we don't have to acquire the lock to subscribe
    tx_response_receiver: broadcast::Receiver<AcceptedTx<Confirmation<S, Rt>>>,
}

impl<S: Spec, Rt: Runtime<S>> TransactionCache<S, Rt> {
    pub fn write_handle(&self) -> TxResultWriter<S, Rt> {
        TxResultWriter {
            inner: self.inner.clone(),
        }
    }
}

impl<S: Spec, Rt: Runtime<S>> TransactionCache<S, Rt> {
    pub fn new(ledger_db: LedgerDb, next_tx_number: u64, broadcast_channel_size: usize) -> Self {
        let (tx_response_sender, tx_response_receiver) = broadcast::channel(broadcast_channel_size);
        Self {
            inner: Arc::new(RwLock::new(TransactionCacheInner {
                cache: BTreeMap::new(),
                tx_response_sender,
                next_tx_number,
                tx_hash_index: HashMap::new(),
                event_numbers_index: BTreeMap::new(),
            })),
            ledger_db,
            tx_response_receiver,
        }
    }

    pub async fn list_events(
        &self,
        event_numbers: std::ops::Range<u64>,
    ) -> anyhow::Result<Vec<RuntimeEventResponse<Rt::RuntimeEvent>>> {
        let transaction_cache = self.inner.read().await;
        let Some(last_needed_event) = event_numbers.end.checked_sub(1) else {
            return Ok(vec![]);
        };
        let first_needed_tx = transaction_cache
            .event_numbers_index
            .get_key_value(&event_numbers.start)
            .map(|(_, tx_number)| *tx_number)
            .unwrap_or(0);
        let last_needed_tx = transaction_cache
            .event_numbers_index
            .get_key_value(&last_needed_event)
            .map(|(_, tx_number)| *tx_number)
            .unwrap_or(u64::MAX);
        let cached_events = transaction_cache
            .cache
            .range(first_needed_tx..=last_needed_tx)
            .flat_map(|(_, tx)| tx.confirmation.events.iter().cloned())
            .collect::<Vec<_>>();

        // If we found any events in cache, we don't need to fetch those ones from the DB - adjust the range accordingly
        let db_range_end = if let Some(first_event_from_cache) = cached_events.first() {
            // Edge case: If we found the first event in cache, then all the relevant events must be in cache (excluding the ones that don't exist yet).
            // Return what we have.
            if first_event_from_cache.number == event_numbers.start {
                return Ok(cached_events);
            }
            first_event_from_cache.number
        } else {
            event_numbers.end
        };

        // Otherwise, we need to fall back to the DB.
        let needed_event_numbers = (event_numbers.start..db_range_end)
            .map(EventIdentifier::Number)
            .collect::<Vec<_>>();
        let db_event_opts = self
            .ledger_db
            .get_events::<RuntimeEventResponse<Rt::RuntimeEvent>>(&needed_event_numbers)
            .await?;

        if let Some(first_cached_event) = cached_events.first() {
            // If we had some any of these events in cache, then all of the preceeding events must have been present in the DB.
            // Assert that this is the case. Note that this only holds if we aren't too aggressive about pruning the ledger DB.
            // If we add more aggressive pruning, we can safely remove this assertion.
            assert!(
                db_event_opts.first().is_some_and(|first_db_event_opt| {
                    first_db_event_opt.as_ref().is_some_and(|first_db_event| {
                        first_db_event.number == event_numbers.start
                    })
                }),
                "Some events were cached, but earlier events were not present in either cache or the DB. This is a bug, please report it."
            );
            assert!(
                db_event_opts.last().is_some_and(|last_db_event_opt| {
                    last_db_event_opt.as_ref().is_some_and(|last_db_event| {
                        last_db_event.number == first_cached_event.number - 1
                    })
                }),
                "Some events were cached, but earlier events were not present in either cache or the DB. This is a bug, please report it."
            );
        }

        Ok(db_event_opts
            .into_iter()
            .flatten()
            .chain(cached_events.into_iter())
            .collect())
    }

    pub async fn get_tx_by_hash(
        &self,
        tx_hash: TxHash,
    ) -> anyhow::Result<Option<AcceptedTx<Confirmation<S, Rt>>>> {
        let transaction_cache = self.inner.read().await;
        if let Some(tx_number) = transaction_cache.tx_hash_index.get(&tx_hash) {
            let tx = transaction_cache.cache.get(tx_number).expect(
                "Tx hash was in cache, but contents are missing. This is a bug, please report it.",
            );
            return Ok(Some(tx.clone()));
        }
        let Some((tx_number, tx)) = self
            .ledger_db
            .get_tx_by_hash(&tx_hash.0, QueryMode::Full)
            .await?
        else {
            return Ok(None);
        };
        let maybe_timestamp = tx
            .body
            .as_ref()
            .and_then(|body| get_maybe_timestamp_from_sequencing_data::<S, Rt>(body, false));

        Ok(Some(AcceptedTx {
            tx: tx.body.unwrap_or_default(),
            tx_hash,
            confirmation: Confirmation {
                events: tx
                    .events
                    .expect("TxResponse::events cannot be None when query mode is Full"),
                receipt: tx.receipt.into(),
                tx_number,
                timestamp_nanos: maybe_timestamp,
            },
        }))
    }

    /// Subscribe to transactions with tx number tracking for lag notifications.  
    pub fn subscribe_txs(&self) -> SequencerTxStream<Confirmation<S, Rt>> {
        let broadcast_stream = BroadcastStream::new(self.tx_response_receiver.resubscribe());

        let mut last_tx_number: Option<u64> = None;
        broadcast_stream
            .map(move |result| match result {
                Ok(tx) => {
                    last_tx_number = Some(tx.confirmation.tx_number);
                    Ok(ApiAcceptedTx::from_accepted_tx::<Rt, S>(tx))
                }
                Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                    Err(SubscriptionStreamError::Lagged {
                        skipped,
                        disconnected_at: last_tx_number,
                        resumed_at: last_tx_number.map(|id| {
                            id.checked_add(skipped)
                                .and_then(|id| id.checked_add(1))
                                .expect("Overflow when adding tx number and skipped count")
                        }),
                    })
                }
            })
            .boxed()
    }

    /// Subscribe to events with event number tracking for lag notifications.
    pub fn subscribe_events(&self) -> crate::common::SequencerEventStream<Rt> {
        let broadcast_stream = BroadcastStream::new(self.tx_response_receiver.resubscribe());

        // Track the last event number seen. Option<u64> is Copy, so it can be
        // captured by the inner `async move` block without moving out of the closure.
        let mut last_event_number: Option<u64> = None;

        broadcast_stream
            .flat_map(move |result| {
                match result {
                    Ok(tx) => {
                        // Update last_event_number for each event as we collect them
                        last_event_number = tx.confirmation.events.last().map(|e| e.number);
                        let events = tx.confirmation.events.into_iter().map(Ok);
                        futures::stream::iter(events).left_stream()
                    }
                    Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                        // last_event_number is Copy, so this captures a copy
                        futures::stream::once(async move {
                            Err(SubscriptionStreamError::Lagged {
                                skipped,
                                disconnected_at: last_event_number,
                                resumed_at: None, // Unknown until next event arrives
                            })
                        })
                        .right_stream()
                    }
                }
            })
            .boxed()
    }

    pub async fn subscribe_starting_from_tx_number(
        &self,
        starting_from: Option<u64>,
    ) -> Result<SequencerTxStream<Confirmation<S, Rt>>, SubscriptionStreamError> {
        let Some(starting_from) = starting_from else {
            return Ok(self.subscribe_txs());
        };
        let transaction_cache = self.inner.read().await;
        let next_tx_number = transaction_cache.next_tx_number;
        if starting_from > next_tx_number {
            return Err(SubscriptionStreamError::RequestedFutureData {
                next_available: next_tx_number,
            });
        }

        // If the caller is starting from the next tx number, we can just return the broadcast stream
        if starting_from == transaction_cache.next_tx_number {
            return Ok(self.subscribe_txs());
        }

        let stream = AcceptedTxStream {
            ledger_db: self.ledger_db.clone(),
            inner: self.inner.clone(),
            starting_from,
            next_chunk: VecDeque::new(),
            maybe_subscription: None,
            pending_get_next_chunk: None,
            last_sent_id: None,
        };
        Ok(Box::pin(stream))
    }
}

struct AcceptedTxStream<S: Spec, Rt: Runtime<S>> {
    ledger_db: LedgerDb,
    inner: ArcInner<S, Rt>,
    starting_from: u64,
    next_chunk: VecDeque<ApiAcceptedTx<Confirmation<S, Rt>>>,
    maybe_subscription: Option<BroadcastStream<AcceptedTx<Confirmation<S, Rt>>>>,
    pending_get_next_chunk: Option<GetNextChunkFuture<S, Rt>>,
    /// Tracks the last tx_number that was successfully yielded, for lag error reporting.
    last_sent_id: Option<u64>,
}

impl<S: Spec, Rt: Runtime<S>> AcceptedTxStream<S, Rt> {
    /// Gets the next chunk of transactions from the cache and/or the DB. If this chunk of transactions
    /// is sufficient to catch us up to the next tx number, then we'll also atomically subscribe to the broadcast channel while
    /// reading from the cache.
    ///
    /// Returns a tuple of the next chunk of transactions and an optional subscription to the broadcast channel. The subscription is `None` unless
    /// we'll be caught up after this chunk.
    async fn get_next_chunk(
        starting_from: u64,
        tx_cache: ArcInner<S, Rt>,
        ledger_db: LedgerDb,
    ) -> Result<
        (
            Vec<ApiAcceptedTx<Confirmation<S, Rt>>>,
            Option<BroadcastStream<AcceptedTx<Confirmation<S, Rt>>>>,
        ),
        SubscriptionStreamError,
    > {
        let tx_cache = tx_cache.read().await;
        let next_tx_number = tx_cache.next_tx_number;
        let first_tx_not_needed = std::cmp::min(starting_from + CHUNK_SIZE, next_tx_number);
        let will_be_caught_up_after_this_chunk = first_tx_not_needed == next_tx_number;
        // If we're going to be caught up after this chunk, subscribe now while we're holding the lock so that the cache and the broadcast stream are in sync.
        let maybe_subscription = if will_be_caught_up_after_this_chunk {
            Some(BroadcastStream::new(
                tx_cache.tx_response_sender.subscribe(),
            ))
        } else {
            None
        };
        // If there are no historical txs to fetch, we're done.
        if next_tx_number == 0 || starting_from == next_tx_number {
            return Ok((vec![], maybe_subscription));
        }
        // We can't subscribe starting from the future.
        if starting_from > next_tx_number {
            return Err(SubscriptionStreamError::RequestedFutureData {
                next_available: next_tx_number,
            });
        }
        // Get the next chunk of txs from the cache right away so that we can drop the lock.
        let txs_from_cache = tx_cache
            .cache
            .range(starting_from..starting_from + CHUNK_SIZE)
            .map(|(_, tx)| ApiAcceptedTx::from_accepted_tx::<Rt, S>(tx.clone()))
            .collect::<Vec<_>>();

        // If the start of the chunk was in cache we're done - every later tx that exists will also be in cache, and
        // there's no point looking in the DB for txs that haven't happened yet.
        if txs_from_cache
            .first()
            .map(|tx| tx.confirmation.tx_number == starting_from)
            .unwrap_or(false)
        {
            return Ok((txs_from_cache, maybe_subscription));
        }
        // Unlock while we backfill txs from the DB.
        drop(tx_cache);
        // Otherwise, we need to get some txs from the db to complete the chunk. There are some edge cases when the cache is empty...
        // - If the DB is also empty, we're done. In that case, next_tx_number will be 0;
        // - If the DB is *not* empty, then our last_needed_tx_number will be next_tx_number - 1;
        // - Otherwise (if the cache is not empty), our next needed tx number is the min of...
        // - ... the first tx number that we got from the cache and "starting_from + CHUNK_SIZE"
        let last_needed_tx_number = std::cmp::min(
            // The first entry of txs_from_cache must have a non-zero number (otherwise we would have returned early above)
            // and we've just checked that next_tx_number is non-zero. Therefore, this number is non-zero.
            txs_from_cache
                .first()
                .map(|tx| tx.confirmation.tx_number)
                .unwrap_or(next_tx_number),
            starting_from + CHUNK_SIZE,
        )
        .checked_sub(1) // Subtract 1 because `get_transactions_range` is inclusive
        .expect("The min of two non-zero numbers cannot be zero, but it was! This is a bug, please report it.");

        let maybe_txs = ledger_db
            .get_transactions_range(starting_from, last_needed_tx_number, QueryMode::Full)
            .await
            .map_err(|error| {
                tracing::error!(%error, "Internal server error while serving tx stream");
                SubscriptionStreamError::Internal
            })?;
        let num_txs_requested_from_db = maybe_txs.len();
        let txs = maybe_txs
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(idx, tx)| {
                let timestamp_nanos = tx.body.as_ref().and_then(|body| {
                    get_maybe_timestamp_from_sequencing_data::<S, Rt>(body, false)
                });
                let tx_body = tx
                    .body
                    .and_then(|body| Rt::Auth::decode_serialized_tx(&body).ok())
                    .map(|tx| {
                        serde_json::to_value(Rt::wrap_call(tx))
                            .expect("Txs must be json serializable")
                    });
                ApiAcceptedTx {
                    tx: tx_body,
                    id: HexString(tx.hash),
                    confirmation: Confirmation {
                        events: tx
                            .events
                            .expect("TxResponse::events cannot be None when query mode is Full"),
                        receipt: tx.receipt.into(),
                        tx_number: starting_from + idx as u64,
                        timestamp_nanos,
                    },
                }
            });
        let num_txs_from_cache = txs_from_cache.len();
        let output = txs.chain(txs_from_cache).collect::<Vec<_>>();
        assert_eq!(output.len() - num_txs_from_cache, num_txs_requested_from_db, "get_transactions_range returned `None` for some transactions that should have been present in the DB! This is a bug, please report it.");

        Ok((output, maybe_subscription))
    }

    fn poll_subscription(
        &mut self,
        cx: &mut futures::task::Context<'_>,
    ) -> Poll<Option<TxStreamItem<S, Rt>>> {
        let Some(subscription) = self.maybe_subscription.as_mut() else {
            return Poll::Pending;
        };
        let last_sent_id = self.last_sent_id;
        let result: Poll<Option<TxStreamItem<S, Rt>>> =
            Pin::new(subscription).poll_next(cx).map(|opt| {
                opt.map(|result| {
                    result
                        .map(|tx| ApiAcceptedTx::from_accepted_tx::<Rt, S>(tx))
                        .map_err(|BroadcastStreamRecvError::Lagged(n)| {
                            SubscriptionStreamError::Lagged {
                                skipped: n,
                                disconnected_at: last_sent_id,
                                resumed_at: last_sent_id.map(|id| {
                                    id.checked_add(n)
                                        .and_then(|id| id.checked_add(1))
                                        .expect("Overflow when adding tx number and skipped count")
                                }),
                            }
                        })
                })
            });
        if let Poll::Ready(Some(Ok(ref tx))) = result {
            self.last_sent_id = Some(tx.confirmation.tx_number);
        }
        result
    }
}

impl<S: Spec, Rt: Runtime<S>> Stream for AcceptedTxStream<S, Rt> {
    type Item = Result<ApiAcceptedTx<Confirmation<S, Rt>>, SubscriptionStreamError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut futures::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        // Step 1: Drain any buffered historical transactions from the last fetch.
        // These come from cache/DB and must be served before switching to live data.
        if let Some(tx) = self.next_chunk.pop_front() {
            self.last_sent_id = Some(tx.confirmation.tx_number);
            return Poll::Ready(Some(Ok(tx)));
        }

        // Step 2: Check if we have alreaady have a live subscription from the previous iteration of this function.
        // If so, that means we're done with backfill. Simply poll the subscription for the next transaction.
        if self.maybe_subscription.is_some() {
            return self.poll_subscription(cx);
        }

        // Step 3: If we don't have a live subscription (checked above) we need to fetch the next chunk of historical
        // transactions from cache/DB. This call will return a subscription if this chunk brings us up to date.
        let mut pending = self.pending_get_next_chunk.take().unwrap_or_else(|| {
            Box::pin(Self::get_next_chunk(
                self.starting_from,
                self.inner.clone(),
                self.ledger_db.clone(),
            ))
        });

        match pending.poll_unpin(cx) {
            Poll::Ready(Ok((txs, maybe_subscription))) => {
                // Store results. If maybe_subscription is Some, the next poll will send these transaction first
                // (Step 1) before serving data from the subscription (Step 2).
                self.starting_from += txs.len() as u64;
                self.next_chunk = txs.into();
                self.maybe_subscription = maybe_subscription;

                if let Some(tx) = self.next_chunk.pop_front() {
                    self.last_sent_id = Some(tx.confirmation.tx_number);
                    return Poll::Ready(Some(Ok(tx)));
                }
                if self.maybe_subscription.is_some() {
                    return self.poll_subscription(cx);
                }
                unreachable!(
                    "get_next_chunk must return a subscription, transactions, or an error"
                );
            }
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => {
                self.pending_get_next_chunk = Some(pending);
                Poll::Pending
            }
        }
    }
}

type TxNumber = u64;
type EventNumber = u64;
#[derive(Debug)]
pub(crate) struct TransactionCacheInner<S: Spec, Rt: Runtime<S>> {
    // TODO: Arc the acceptedTxs
    cache: BTreeMap<u64, AcceptedTx<Confirmation<S, Rt>>>,
    tx_response_sender: broadcast::Sender<AcceptedTx<Confirmation<S, Rt>>>,
    // The next tx number, needed in case the cache is empty
    next_tx_number: u64,
    // A map of tx hashes to tx numbers
    tx_hash_index: HashMap<TxHash, TxNumber>,
    event_numbers_index: BTreeMap<EventNumber, TxNumber>,
}

#[cfg(test)]
mod tests {
    use sov_db::ledger_db::SlotCommit;
    use sov_mock_da::{MockAddress, MockBlob, MockBlock};
    use sov_modules_api::{
        ApiTxEffect, BatchReceipt, FullyBakedTx, Gas, SuccessfulTxContents, TransactionReceipt,
        TxEffect, TxReceiptContents,
    };
    use sov_test_utils::storage::SimpleLedgerStorageManager;
    use sov_test_utils::{generate_optimistic_runtime, TestSpec as S};

    use super::*;

    generate_optimistic_runtime!(TestRuntime <=);

    fn build_mock_confirmation(tx_number: u64) -> AcceptedTx<Confirmation<S, TestRuntime<S>>> {
        AcceptedTx {
            tx: FullyBakedTx::new(vec![]),
            tx_hash: HexString([tx_number as u8; 32]),
            confirmation: Confirmation {
                events: vec![],
                receipt: ApiTxEffect::Successful {
                    data: SuccessfulTxContents {
                        gas_used: <<S as Spec>::Gas as Gas>::zero(),
                    },
                },
                tx_number,
                timestamp_nanos: None,
            },
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_catchup_to_stream() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
        let ledger_db = LedgerDb::with_reader(storage_manager.create_ledger_storage()).unwrap();
        let cache = TransactionCache::new(ledger_db, 0, 100);
        let writer = cache.write_handle();

        let initial_num_txs = 201;
        let mut num_txs = initial_num_txs;
        let txs = (0..num_txs).map(build_mock_confirmation);
        for tx in txs {
            writer.insert(tx).await;
        }

        // Check that we don't have issues no matter where we start the stream from
        for i in 0..initial_num_txs {
            let mut stream = cache
                .subscribe_starting_from_tx_number(Some(i))
                .await
                .unwrap();

            for j in i..num_txs {
                let next_tx =
                    tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap();
                assert_eq!(next_tx.confirmation.tx_number, j);
            }
            // Occasionally, insert a new tx to test that the stream continues working after catchup
            if i % 13 == 0 {
                let new_tx = build_mock_confirmation(num_txs);
                writer.insert(new_tx.clone()).await;
                let next_tx =
                    tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap();
                assert_eq!(next_tx.confirmation.tx_number, num_txs);
                num_txs += 1;
            } else {
                // If we're not inserting a new tx, then the stream should be empty. Check that it is.
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(50), stream.next())
                        .await
                        .is_err()
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_subscribe_from_head() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
        let ledger_db = LedgerDb::with_reader(storage_manager.create_ledger_storage()).unwrap();
        let cache = TransactionCache::new(ledger_db, 0, 100);
        let writer = cache.write_handle();

        let num_txs = 5;
        let txs = (0..num_txs).map(build_mock_confirmation);
        for tx in txs {
            writer.insert(tx).await;
        }

        let mut stream = cache.subscribe_starting_from_tx_number(None).await.unwrap();
        // If we're not inserting a new tx, then the stream should be empty. Check that it is.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), stream.next())
                .await
                .is_err()
        );

        // Push a new tx to the stream and check that it comes through
        writer.insert(build_mock_confirmation(num_txs)).await;
        let next_tx = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(next_tx.confirmation.tx_number, num_txs);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_subscribe_reports_correct_identifier_on_lag() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
        let ledger_db = LedgerDb::with_reader(storage_manager.create_ledger_storage()).unwrap();
        // Use a small channel size to easily trigger lag
        let cache = TransactionCache::<S, TestRuntime<S>>::new(ledger_db, 0, 4);
        let writer = cache.write_handle();

        // Subscribe first (broadcast channel only sends new messages to subscribers)
        let mut stream = cache.subscribe_txs();

        // Insert transactions and receive them to update the tracked identifier
        for i in 0..3 {
            writer.insert(build_mock_confirmation(i)).await;
            let item = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(item.confirmation.tx_number, i);
        }

        // Now insert more transactions than the channel can hold WITHOUT reading them.
        // This will cause the subscriber to lag.
        // Channel size is 4, so inserting 6 more should cause lag.
        for i in 3..9 {
            writer.insert(build_mock_confirmation(i)).await;
        }

        // The next item should be a lag error with disconnected_at = 2 (last received tx)
        let item = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .unwrap()
            .unwrap();

        match item {
            Err(SubscriptionStreamError::Lagged {
                disconnected_at,
                resumed_at,
                skipped,
            }) => {
                assert_eq!(
                    disconnected_at,
                    Some(2),
                    "disconnected_at should be the last successfully received tx_number"
                );
                // resumed_at should be disconnected_at + skipped + 1
                assert_eq!(resumed_at, Some(2 + skipped + 1));
            }
            Ok(_) => panic!("Expected Lagged error, got Ok"),
            Err(other) => panic!("Expected Lagged error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_tx_stream_falls_back_to_db_for_uncached_txs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut storage_manager = SimpleLedgerStorageManager::new(temp_dir.path());
        let ledger_db = LedgerDb::with_reader(storage_manager.create_ledger_storage()).unwrap();
        let cache = TransactionCache::new(ledger_db.clone(), 0, 100);
        let writer = cache.write_handle();
        let num_txs = 215;
        // Populate the Ledger DB and tx cache
        {
            let mut slot = SlotCommit::<_, MockBlob, TxReceiptContents<S>>::new(
                MockBlock::default(),
                Default::default(),
            );
            let mut batch = BatchReceipt::<MockBlob, TxReceiptContents<S>> {
                batch_hash: [0; 32],
                tx_receipts: vec![],
                ignored_tx_receipts: vec![],
                inner: MockBlob::new(vec![], MockAddress::new([0; 32]), [0; 32]),
            };

            let txs = (0..num_txs).map(build_mock_confirmation);
            for (i, tx) in txs.enumerate() {
                // Push the first 110 txs to both the ledger db and the tx cache
                if i < 110 {
                    batch.tx_receipts.push(TransactionReceipt {
                        tx_hash: tx.tx_hash,
                        body_to_save: None,
                        events: vec![],
                        receipt: TxEffect::Successful(SuccessfulTxContents {
                            gas_used: <<S as Spec>::Gas as Gas>::zero(),
                        }),
                    });
                }
                writer.insert(tx).await;
            }
            slot.add_batch(batch);
            let commit_data = ledger_db.materialize_slot(slot, b"state-root").unwrap();
            storage_manager.commit(&commit_data);
            ledger_db.replace_reader(storage_manager.create_ledger_storage());
        }
        // Prune the cache to remove the first 105 txs. This forces the stream to fall back to the DB for those txs
        // Note that the range of txs in the cache will still overlap with the DB after pruning. That's intentional to test the
        // handling of that edge case.
        cache.write_handle().prune(105).await;

        let mut stream = cache
            .subscribe_starting_from_tx_number(Some(0))
            .await
            .unwrap();

        // Check that we can get all the txs from the stream in the expected order
        for i in 0..num_txs {
            let next_tx = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(next_tx.confirmation.tx_number, i);
            assert_eq!(next_tx.id, HexString([i as u8; 32]));
        }

        // Push a new tx to the stream and check that it comes through
        writer.insert(build_mock_confirmation(num_txs)).await;
        let next_tx = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(next_tx.confirmation.tx_number, num_txs);
    }
}
