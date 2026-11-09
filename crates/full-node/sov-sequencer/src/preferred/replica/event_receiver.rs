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

    #[error("Invalid event sequence in the db: {0:?} {1:?}. Replica shutting down.")]
    InvalidEventSequence(Option<EventType>, EventType),
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
            },
            db_data_receiver,
        )
    }

    pub(crate) async fn spawn_db_data_fetcher(mut self) -> BackgroundHandle<()> {
        let mut nb_of_consecutive_db_errors = 0;
        let shutdown_receiver = self.primary_shutdown.clone();
        let mut start_replica_task_receiver = self.ready_to_process_db_events_recv.clone();

        BackgroundHandle::spawn("replica-db-fetcher", async move {
            let mut start_event_id = None;
            let mut prev_event_type = None;

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
            loop {
                let fut = shutdown_receiver.future_or_shutdown(self.fetch_data(
                    start_event_id,
                    prev_event_type,
                    &mut listener,
                ));

                let FutureOrShutdownOutput::Output(res) = fut.await else {
                    break;
                };

                match res {
                    Ok((next_event_id, event_type)) => {
                        nb_of_consecutive_db_errors = 0;
                        start_event_id = Some(next_event_id);
                        prev_event_type = event_type;
                    }
                    Err(err) => {
                        match err {
                            EventReceiverError::ParsingError(err) => {
                                // This should never happen, so we shut down the replica immediately
                                panic!(
                                    "Failed to parse notification: {err:?}. Shutting down replica."
                                );
                            }
                            EventReceiverError::DbError(err) => {
                                error!(?err, ?start_event_id, "Failed to receive notifications from database. Shutting down replica.");

                                if shutdown_receiver.is_triggered() {
                                    break;
                                }

                                // Since network errors can occur, we will retry receiving a few times before initiating replica shutdown.
                                if nb_of_consecutive_db_errors >= MAX_DB_ERRORS_ALLOWED {
                                    panic!("Failed to connect to the database after {nb_of_consecutive_db_errors} attempts. Shutting down replica.");
                                }

                                nb_of_consecutive_db_errors += 1;
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                continue;
                            }
                            EventReceiverError::InvalidEventSequence(
                                prev_event_type,
                                event_type,
                            ) => {
                                // Apart from the leader writing an inconsistent sequence, this happens
                                // when the rows from `start_event_id` up to the first existing row have
                                // already been pruned by the leader, i.e. the replica fell too far behind.
                                panic!("Invalid event sequence in the db: {prev_event_type:?} {event_type:?} (next expected event id: {start_event_id:?}). Replica shutting down.");
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

    async fn fetch_data(
        &mut self,
        start_event_id: Option<u64>,
        prev_event_type: Option<EventType>,
        listener: &mut PgListener,
    ) -> Result<(u64, Option<EventType>), EventReceiverError> {
        let start_event_id = self.wait_for_new_rows(start_event_id, listener).await?;
        self.fetch_from_event_id(start_event_id, prev_event_type)
            .await
    }

    /// Fetches every existing event with `event_id >= start_event_id`, page by page, and
    /// returns the next event id to fetch together with the type of the last event sent.
    async fn fetch_from_event_id(
        &mut self,
        start_event_id: u64,
        mut prev_event_type: Option<EventType>,
    ) -> Result<(u64, Option<EventType>), EventReceiverError> {
        trace!(start_event_id, "Fetching events");
        let mut next_event_id = start_event_id;

        // Currently, we fetch data in a loop. One possible (but not yet necessary) optimization would be to issue
        // multiple parallel queries to the DB for different `event_id`` ranges.
        // After analyzing real-world workloads, we can revisit this optimization. Implementing it would only affect
        // the contents of this method and would not require significant refactoring.
        loop {
            trace!(next_event_id, "Processing page starting at event");

            let db_rows = rows(&self.query_pool, next_event_id, self.page_size).await?;
            let is_last_page = db_rows.len() < self.page_size;

            for row in db_rows {
                let event_id = row.get::<i64, _>("event_id") as u64;
                let (event, event_type) = row_to_event(row)?;

                if !(EventType::is_event_sequence_valid(prev_event_type, event_type)) {
                    return Err(EventReceiverError::InvalidEventSequence(
                        prev_event_type,
                        event_type,
                    ));
                }

                let _ = self.db_data_sender.send(event).await;
                prev_event_type = Some(event_type);
                next_event_id = event_id + 1;
            }

            // A short page means we have read every row that currently exists.
            if is_last_page {
                return Ok((next_event_id, prev_event_type));
            }
        }
    }
}
