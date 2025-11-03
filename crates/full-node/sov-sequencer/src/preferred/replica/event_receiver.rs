use crate::preferred::replica::db_data::row_to_event;
use crate::preferred::replica::db_data::rows;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::db_data::EventType;
use crate::preferred::replica::db_data::EventsNotificationPayload;
use crate::preferred::replica::db_data::ParsingError;
use sov_rollup_interface::node::future_or_shutdown;
use sov_rollup_interface::node::FutureOrShutdownOutput;
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{error, trace};

const MAX_DB_ERRORS_ALLOWED: u32 = 10;

#[derive(thiserror::Error, Debug)]
pub(crate) enum EventReceiverError {
    #[error("Error while querying for  db data: {0}")]
    DbError(#[from] sqlx::Error),

    #[error("Error while parsing the db notification: {0}")]
    ParsingError(#[from] ParsingError),

    #[error("Invalid event sequence in the db: {0:?} {1:?}. Replica shutting down.")]
    InvalidEventSequence(Option<EventType>, EventType),

    #[error("DB row does not exist: {0}")]
    DbRowDoesNotExist(u64),
}

pub(crate) struct EventReceiverStartNotifier {
    notify: watch::Sender<()>,
}

impl EventReceiverStartNotifier {
    pub(crate) fn new() -> (Self, watch::Receiver<()>) {
        let (notify, mut receiver) = watch::channel(());
        receiver.borrow_and_update();
        (Self { notify }, receiver)
    }

    pub(crate) fn notify(&self) {
        let _ = self.notify.send(());
    }
}

pub(crate) struct EventReceiver {
    connection_string: String,
    db_data_sender: tokio::sync::mpsc::Sender<DbData>,
    shutdown_sender: watch::Sender<()>,
    query_pool: PgPool,
    page_size: usize,
    ready_to_process_db_events_recv: watch::Receiver<()>,
}

impl EventReceiver {
    pub(crate) async fn new(
        connection_string: String,
        shutdown_sender: watch::Sender<()>,
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
                shutdown_sender,
                query_pool,
                page_size,
                ready_to_process_db_events_recv,
            },
            db_data_receiver,
        )
    }

    pub(crate) async fn spawn_db_data_fetcher(mut self) -> JoinHandle<()> {
        let mut nb_of_consecutive_db_errors = 0;
        let shutdown_receiver = self.shutdown_sender.subscribe();
        let mut start_replica_task_receiver = self.ready_to_process_db_events_recv.clone();

        tokio::spawn(async move {
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

            loop {
                let fut = future_or_shutdown(
                    self.fetch_data(start_event_id, prev_event_type, &mut listener),
                    &shutdown_receiver,
                );

                let FutureOrShutdownOutput::Output(res) = fut.await else {
                    break;
                };

                match res {
                    Ok((event_id, event_type)) => {
                        nb_of_consecutive_db_errors = 0;
                        start_event_id = Some(event_id + 1);
                        prev_event_type = event_type;
                    }
                    Err(err) => {
                        match err {
                            EventReceiverError::ParsingError(e) => {
                                // This should never happen, so we shut down the replica immediately
                                panic!(
                                    "Failed to parse notification: {e:?}. Shutting down replica."
                                );
                            }
                            EventReceiverError::DbError(e) => {
                                error!("Failed to receive notifications from database: {e:?}. Shutting down replica.");

                                if shutdown_receiver.has_changed().unwrap_or(true) {
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
                                panic!("Invalid event sequence in the db: {prev_event_type:?} {event_type:?}. Replica shutting down.");
                            }
                            EventReceiverError::DbRowDoesNotExist(event_id) => {
                                panic!(
                                    "Db row does not exist for event id: {event_id:?} {start_event_id:?} {prev_event_type:?}. Replica shutting down."
                                );
                            }
                        }
                    }
                }
            }
        })
    }

    async fn recv_notifications(
        &mut self,
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
        &mut self,
        start_event_id: Option<u64>,
        mut prev_event_type: Option<EventType>,
        listener: &mut PgListener,
    ) -> Result<(u64, Option<EventType>), EventReceiverError> {
        let (start_event_id, target_event_id) = match start_event_id {
            // If we already have a start id, just wait for the next notification
            // and use its event_id as the target.
            Some(start_id) => {
                let notify = self.recv_notifications(listener).await?;
                assert!(notify.event_id >= start_id);
                (start_id, notify.event_id)
            }
            // Otherwise, `None` indicates the replica has just started.
            // Keep listening for events until a `BatchStart` notification is received,
            // then use that event's ID as both the starting and target event ID.
            None => loop {
                let notify = listener.recv().await?;
                let notify = EventsNotificationPayload::parse_csv(notify.payload())?;

                if matches!(notify.event_type, EventType::BatchStart) {
                    break (notify.event_id, notify.event_id);
                }
            },
        };

        prev_event_type = self
            .backfill_to_event_id(start_event_id, target_event_id, prev_event_type)
            .await?;

        Ok((target_event_id, prev_event_type))
    }

    async fn backfill_to_event_id(
        &mut self,
        mut current_event_id: u64,
        target_event_id: u64,
        mut prev_event_type: Option<EventType>,
    ) -> Result<Option<EventType>, EventReceiverError> {
        trace!(
            "Backfilling events from {} to {}",
            current_event_id,
            target_event_id
        );

        // Currently, we fetch data in a loop. One possible (but not yet necessary) optimization would be to issue
        // multiple parallel queries to the DB for different `event_id`` ranges.
        // After analyzing real-world workloads, we can revisit this optimization. Implementing it would only affect
        // the contents of this method and would not require significant refactoring.
        while current_event_id <= target_event_id {
            let page_end = std::cmp::min(current_event_id + self.page_size as u64, target_event_id);

            trace!(
                "Processing backfill page: events {} to {}",
                current_event_id,
                page_end
            );

            // Query and process events for this page
            let db_rows = rows(&self.query_pool, page_end, current_event_id).await?;

            if db_rows.is_empty() {
                return Err(EventReceiverError::DbRowDoesNotExist(current_event_id));
            }

            for row in db_rows {
                let (event, event_type) = row_to_event(row)?;

                if !(EventType::is_event_sequence_valid(prev_event_type, event_type)) {
                    return Err(EventReceiverError::InvalidEventSequence(
                        prev_event_type,
                        event_type,
                    ));
                }

                let _ = self.db_data_sender.send(event).await;
                prev_event_type = Some(event_type);
            }

            current_event_id = page_end + 1;
        }

        Ok(prev_event_type)
    }
}
