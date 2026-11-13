use crate::preferred::db::SequencerRole;
use crate::preferred::replica::db_data::row_to_event;
use crate::preferred::replica::db_data::rows;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::db_data::EventType;
use crate::preferred::replica::db_data::EventsNotificationPayload;
use crate::preferred::replica::db_data::ParsingError;
use crate::SequencerNotReadyDetails;
use sov_shutdown::BackgroundHandle;
use sov_shutdown::FutureOrShutdownOutput;
use sov_shutdown::PrimaryShutdownController;
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::PgPool;
use sqlx::Row;
use tokio::sync::watch;
use tracing::{debug, error, trace};

const MAX_DB_ERRORS_ALLOWED: u32 = 20;

#[derive(thiserror::Error, Debug)]
pub(crate) enum EventReceiverError {
    #[error("Error while querying for  db data: {0}")]
    DbError(#[from] sqlx::Error),

    #[error("Error while parsing the db notification: {0}")]
    ParsingError(#[from] ParsingError),

    #[error(
        "Invalid event sequence in the db: {0:?} {1:?} at event id {2}. Replica shutting down."
    )]
    InvalidEventSequence(Option<EventType>, EventType, u64),
}

pub(crate) struct EventReceiverStartNotifier {
    notify: watch::Sender<()>,
    replica_processed_first_batch: bool,
    seq_role: SequencerRole,
}

impl EventReceiverStartNotifier {
    pub(crate) fn new(seq_role: SequencerRole) -> (Self, watch::Receiver<()>) {
        let (notify, mut receiver) = watch::channel(());
        receiver.borrow_and_update();
        (
            Self {
                notify,
                replica_processed_first_batch: false,
                seq_role,
            },
            receiver,
        )
    }

    /// Notifies the replica sync task that it’s time to start processing leader sequencer DB events.
    /// This method only has an effect when called on a replica sequencer, since the leader produces
    /// the DB events rather than consuming them.
    pub(crate) fn notify(&self) {
        if self.seq_role == SequencerRole::BatchProducer {
            return;
        }

        let _ = self.notify.send(());
    }

    /// If the sequencer is in replica mode, we check whether the replica sync task has processed
    /// its first batch, which indicates that all services are up and running.
    /// For the leader, this method always returns Ok(()).
    pub(crate) fn check_replica_status_or_ok_for_leader(
        &self,
    ) -> Result<(), SequencerNotReadyDetails> {
        if self.seq_role == SequencerRole::BatchProducer {
            return Ok(());
        }

        if self.replica_processed_first_batch {
            return Ok(());
        }

        Err(SequencerNotReadyDetails::ReplicaNotReady)
    }

    /// Marks that the replica has successfully processed its first batch of events.
    ///
    /// # Panics
    /// Panics if this method is called on a batch-producing sequencer.
    pub(crate) fn set_replica_processed_first_batch(&mut self) {
        assert_eq!(
            self.seq_role,
            SequencerRole::PgSyncReplica,
            "set_replica_processed_first_batch can only be called on a PgSyncReplica sequencer"
        );
        self.replica_processed_first_batch = true;
    }
}

pub(crate) struct EventReceiver {
    connection_string: String,
    db_data_sender: tokio::sync::mpsc::Sender<DbData>,
    primary_shutdown: PrimaryShutdownController,
    query_pool: PgPool,
    page_size: usize,
    ready_to_process_db_events_recv: watch::Receiver<()>,
    /// Id of the next event to fetch. `None` until the first event has been sent to the handler.
    next_event_id: Option<u64>,
    /// Type of the last event sent to the handler.
    prev_event_type: Option<EventType>,
    /// Number of database errors since the last successful events query.
    nb_of_consecutive_db_errors: u32,
}

impl EventReceiver {
    pub(crate) async fn new(
        connection_string: String,
        primary_shutdown: PrimaryShutdownController,
        page_size: usize,
        ready_to_process_db_events_recv: watch::Receiver<()>,
    ) -> (Self, tokio::sync::mpsc::Receiver<DbData>) {
        let (db_data_sender, db_data_receiver) = tokio::sync::mpsc::channel(page_size);

        // Create a separate persistent connection pool for querying transaction data
        let query_pool = match PgPoolOptions::default()
            .max_connections(5) // Small pool since we're just doing simple queries
            .connect(&connection_string)
            .await
        {
            Ok(pool) => pool,
            Err(e) => {
                panic!("Failed to connect to PostgreSQL: {e:?}. Replica shutting down.");
            }
        };

        (
            Self {
                connection_string,
                db_data_sender,
                primary_shutdown,
                query_pool,
                page_size,
                ready_to_process_db_events_recv,
                next_event_id: None,
                prev_event_type: None,
                nb_of_consecutive_db_errors: 0,
            },
            db_data_receiver,
        )
    }

    pub(crate) async fn spawn_db_data_fetcher(mut self) -> BackgroundHandle<()> {
        let shutdown_receiver = self.primary_shutdown.clone();
        let mut start_replica_task_receiver = self.ready_to_process_db_events_recv.clone();

        BackgroundHandle::spawn("replica-db-fetcher", async move {
            if start_replica_task_receiver.changed().await.is_err() {
                return;
            }

            // Create a dedicated listener for PostgreSQL LISTEN/NOTIFY
            let mut listener = match PgListener::connect(&self.connection_string).await {
                Ok(listener) => listener,
                Err(e) => {
                    panic!("Failed to create PostgreSQL listener: {e:?}. Replica shutting down.");
                }
            };

            if let Err(e) = listener.listen("events_changes").await {
                panic!("Failed to listen on events_changes channel: {e:?}. Replica shutting down.");
            }

            debug!("Replica event receiver started.");
            let mut fetch_immediately = false;
            loop {
                let fut = shutdown_receiver
                    .future_or_shutdown(self.fetch_data(&mut listener, fetch_immediately));

                let FutureOrShutdownOutput::Output(res) = fut.await else {
                    break;
                };

                match res {
                    Ok(()) => fetch_immediately = false,
                    Err(err) => {
                        match err {
                            EventReceiverError::ParsingError(err) => {
                                // This should never happen, so we shut down the replica immediately
                                panic!(
                                    "Failed to parse notification: {err:?}. Shutting down replica."
                                );
                            }
                            EventReceiverError::DbError(err) => {
                                error!(?err, next_event_id = ?self.next_event_id, "Database error in the replica event receiver. Retrying; the replica shuts down after {MAX_DB_ERRORS_ALLOWED} consecutive errors.");

                                if shutdown_receiver.is_triggered() {
                                    break;
                                }

                                // Since network errors can occur, we will retry receiving a few times before initiating replica shutdown.
                                if self.nb_of_consecutive_db_errors >= MAX_DB_ERRORS_ALLOWED {
                                    panic!("Failed to connect to the database after {} attempts. Shutting down replica.", self.nb_of_consecutive_db_errors);
                                }

                                self.nb_of_consecutive_db_errors += 1;
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                // The failed fetch may have left committed rows unread, and the
                                // notifications for them may already be consumed (or lost with the
                                // connection), so don't wait for a new one before fetching again.
                                fetch_immediately = true;
                                continue;
                            }
                            EventReceiverError::InvalidEventSequence(
                                prev_event_type,
                                event_type,
                                event_id,
                            ) => {
                                // Apart from the leader writing an inconsistent sequence, this happens
                                // when the leader pruned the rest of the batch the replica was in the
                                // middle of. Pruning of whole batches leaves a valid sequence and is
                                // not detected here: the handler deals with gaps in sequence numbers.
                                panic!("Invalid event sequence in the db: {prev_event_type:?} {event_type:?} at event id {event_id}. Replica shutting down.");
                            }
                        }
                    }
                }
            }
        })
    }

    /// Returns the event id to fetch from once new rows may be available.
    ///
    /// Notifications are used purely as wake-up signals: `sqlx` hands them over one at a time
    /// in arrival order, so the notification we wake up on is often not the most recent one.
    /// The actual fetch therefore never targets a notification's event id; it reads everything
    /// from `start_event_id` onwards.
    async fn wait_for_new_rows(
        &mut self,
        start_event_id: Option<u64>,
        listener: &mut PgListener,
    ) -> Result<u64, EventReceiverError> {
        loop {
            let notify = listener.recv().await?;
            let notify = EventsNotificationPayload::parse_csv(notify.payload())?;

            match start_event_id {
                // Notifications for rows we have already fetched carry no new information,
                // so skip them without querying the database.
                Some(start_id) if notify.event_id < start_id => {
                    trace!(?notify, start_id, "Skipping stale notification.");
                }
                Some(start_id) => return Ok(start_id),
                // `None` indicates the replica has just started. Keep listening until a
                // `BatchStart` notification is received, so that the replica never starts
                // in the middle of a batch, and fetch from that event onwards.
                None if matches!(notify.event_type, EventType::BatchStart) => {
                    debug!(?notify, "Replica received first BatchStart notification.");
                    return Ok(notify.event_id);
                }
                None => {}
            }
        }
    }

    /// Waits for a notification and fetches all new events. With `fetch_immediately` set, the
    /// wait is skipped if the position to fetch from is already known.
    async fn fetch_data(
        &mut self,
        listener: &mut PgListener,
        fetch_immediately: bool,
    ) -> Result<(), EventReceiverError> {
        let start_event_id = match self.next_event_id {
            Some(next_event_id) if fetch_immediately => next_event_id,
            next_event_id => self.wait_for_new_rows(next_event_id, listener).await?,
        };
        self.fetch_from_event_id(start_event_id).await
    }

    /// Fetches every existing event with `event_id >= start_event_id`, page by page.
    ///
    /// Progress is recorded in `self` as each event is sent, so that after an error on a later
    /// page the next fetch resumes right after the last event sent.
    async fn fetch_from_event_id(&mut self, start_event_id: u64) -> Result<(), EventReceiverError> {
        trace!(start_event_id, "Fetching events");
        let mut next_event_id = start_event_id;

        // Currently, we fetch data in a loop. One possible (but not yet necessary) optimization would be to issue
        // multiple parallel queries to the DB for different `event_id`` ranges.
        // After analyzing real-world workloads, we can revisit this optimization. Implementing it would only affect
        // the contents of this method and would not require significant refactoring.
        loop {
            trace!(next_event_id, "Processing page starting at event");

            let db_rows = rows(&self.query_pool, next_event_id, self.page_size).await?;
            self.nb_of_consecutive_db_errors = 0;
            let is_last_page = db_rows.len() < self.page_size;

            for row in db_rows {
                let event_id = row.get::<i64, _>("event_id") as u64;
                let (event, event_type) = row_to_event(row)?;

                if !(EventType::is_event_sequence_valid(self.prev_event_type, event_type)) {
                    return Err(EventReceiverError::InvalidEventSequence(
                        self.prev_event_type,
                        event_type,
                        event_id,
                    ));
                }

                let _ = self.db_data_sender.send(event).await;
                self.prev_event_type = Some(event_type);
                next_event_id = event_id + 1;
                self.next_event_id = Some(next_event_id);
            }

            // A short page means we have read every row that currently exists.
            if is_last_page {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferred::db::postgres::PostgresBackend;
    use crate::preferred::db::BatchToStore;
    use crate::preferred::db::DbBackend;
    use sov_full_node_configs::sequencer::ConfiguredNodeRole;
    use sov_modules_api::FullyBakedTx;
    use sov_modules_api::TxHash;
    use sov_modules_api::VisibleSlotNumber;
    use sov_test_utils::postgres::config_from_postgres_container;
    use sov_test_utils::postgres::{create_postgres_container, CreatePostgresError};
    use sov_test_utils::postgres::{ContainerAsync, Postgres};
    use std::future::Future;
    use std::net::Ipv4Addr;
    use std::net::SocketAddr;
    use std::num::NonZero;
    use std::time::Duration;
    use tokio::sync::mpsc;

    const PAGE_SIZE: usize = 2;

    struct TestSetup {
        // Stops the container when dropped.
        _postgres: ContainerAsync<Postgres>,
        connection_string: String,
        leader_db: PostgresBackend,
        receiver: EventReceiver,
        /// Subscribed to the events notifications before any event is written.
        listener: PgListener,
        db_data_receiver: mpsc::Receiver<DbData>,
    }

    /// Returns `None` if docker is not available.
    async fn setup() -> Option<TestSetup> {
        let postgres = match create_postgres_container().await {
            Ok(postgres) => postgres,
            Err(CreatePostgresError::DockerNotSupported) => return None,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let postgres_config = config_from_postgres_container(
            &postgres,
            "Replica".into(),
            ConfiguredNodeRole::Replica,
        )
        .await
        .unwrap();
        let connection_string = postgres_config.postgres_connection_string.clone();

        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let leader_db = PostgresBackend::connect(&postgres_config, addr)
            .await
            .unwrap();
        let _ = leader_db
            .heartbeat(Some(postgres_config.leader_election))
            .await
            .unwrap();

        let (receiver, db_data_receiver) = EventReceiver::new(
            connection_string.clone(),
            PrimaryShutdownController::new(),
            PAGE_SIZE,
            watch::channel(()).1,
        )
        .await;

        let mut listener = PgListener::connect(&connection_string).await.unwrap();
        listener.listen("events_changes").await.unwrap();

        Some(TestSetup {
            _postgres: postgres,
            connection_string,
            leader_db,
            receiver,
            listener,
            db_data_receiver,
        })
    }

    /// Writes a complete batch as the leader and returns the events a replica should receive.
    async fn write_batch(
        db: &mut PostgresBackend,
        sequence_number: u64,
        nb_of_txs: u64,
    ) -> Vec<DbData> {
        let batch = BatchToStore {
            sequence_number,
            blob_id: 0x1234,
            visible_slot_number_after_increase: VisibleSlotNumber::new_dangerous(1),
            visible_slots_to_advance: NonZero::new(1).unwrap(),
        };

        db.begin_rollup_block(batch).await.unwrap();
        let mut events = vec![DbData::BatchStart(batch)];

        for i in 0..nb_of_txs {
            let tx = FullyBakedTx::new(vec![i as u8]);
            db.add_tx(sequence_number, i, tx.clone(), TxHash::new([1; 32]))
                .await
                .unwrap();
            events.push(DbData::Transaction(
                sequence_number,
                tx,
                TxHash::new([1; 32]),
            ));
        }

        db.end_rollup_block(batch).await.unwrap();
        events.push(DbData::BatchEnd(batch));
        events
    }

    /// Drives `fut` to completion while collecting the events sent in the meantime. A fetch
    /// cannot simply be awaited: the events channel only holds one page, so it blocks until
    /// the events are received.
    async fn drain_until<T>(
        fut: impl Future<Output = T>,
        db_data_receiver: &mut mpsc::Receiver<DbData>,
        received: &mut Vec<DbData>,
    ) -> T {
        tokio::pin!(fut);
        loop {
            tokio::select! {
                Some(event) = db_data_receiver.recv() => received.push(event),
                output = &mut fut => {
                    // The sender is still alive, so the only possible error is an empty
                    // channel, which means we have collected everything.
                    while let Ok(event) = db_data_receiver.try_recv() {
                        received.push(event);
                    }
                    return output;
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_single_notification_fetches_all_committed_events() {
        let Some(TestSetup {
            _postgres,
            mut leader_db,
            mut receiver,
            mut listener,
            mut db_data_receiver,
            ..
        }) = setup().await
        else {
            return;
        };

        // All the events are committed before the replica receives its first notification.
        let expected = write_batch(&mut leader_db, 1, 6).await;

        let mut received = Vec::new();
        drain_until(
            receiver.fetch_data(&mut listener, false),
            &mut db_data_receiver,
            &mut received,
        )
        .await
        .unwrap();

        assert_eq!(
            received, expected,
            "A single wake-up should fetch every committed event, not just the one it was notified about"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_fetch_resumes_after_db_error_without_resending_events() {
        let Some(TestSetup {
            _postgres,
            connection_string,
            mut leader_db,
            mut receiver,
            mut listener,
            mut db_data_receiver,
        }) = setup().await
        else {
            return;
        };

        // 8 events: 4 full pages.
        let expected = write_batch(&mut leader_db, 1, 6).await;

        let query_pool = receiver.query_pool.clone();
        let fetch = tokio::spawn(async move {
            let res = receiver.fetch_data(&mut listener, false).await;
            (receiver, listener, res)
        });

        // Once the first event arrives, the fetch has sent the first page. It cannot have queried
        // the third page yet: the channel only holds one page, so sending the second page blocks
        // until more events are received.
        let mut received = vec![db_data_receiver.recv().await.unwrap()];

        // Make all further queries fail, as if the database became unreachable mid-fetch.
        query_pool.close().await;
        let (mut receiver, mut listener, res) =
            drain_until(fetch, &mut db_data_receiver, &mut received)
                .await
                .unwrap();
        assert!(
            matches!(res, Err(EventReceiverError::DbError(_))),
            "The fetch should fail once the pool is closed, got: {res:?}"
        );

        // The database is reachable again. All notifications for the batch were sent before
        // the failure, so a retry that waited for a new one would never complete.
        receiver.query_pool = PgPoolOptions::default()
            .connect(&connection_string)
            .await
            .unwrap();
        let retry = drain_until(
            receiver.fetch_data(&mut listener, true),
            &mut db_data_receiver,
            &mut received,
        );
        tokio::time::timeout(Duration::from_secs(60), retry)
            .await
            .expect("The retry should not wait for a new notification")
            .unwrap();

        assert_eq!(
            received, expected,
            "Every event should be delivered exactly once and in order"
        );
    }
}
