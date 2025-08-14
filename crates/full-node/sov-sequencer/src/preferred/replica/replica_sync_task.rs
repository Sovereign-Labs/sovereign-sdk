use crate::preferred::exit_rollup;
use crate::preferred::replica::event_receiver::EventReceiverError;
use crate::preferred::replica::event_receiver::{EventReceiver, EventsNotificationPayload};
use tokio::sync::watch;
use tracing::error;

const MAX_DB_ERRORS_ALLOWED: u32 = 10;

pub(crate) trait ReplicaEventHandler {
    async fn on_notification(&self, notification: EventsNotificationPayload);
}

pub(crate) struct ReplicaSyncTask<R: ReplicaEventHandler> {
    event_receiver: EventReceiver,
    handler: R,
    shutdown_sender: watch::Sender<()>,
}

impl<R: ReplicaEventHandler> ReplicaSyncTask<R> {
    pub(crate) async fn new(
        postgres_connection_string: &str,
        handler: R,
        shutdown_sender: watch::Sender<()>,
    ) -> anyhow::Result<Self> {
        let event_receiver =
            EventReceiver::new(postgres_connection_string, &shutdown_sender).await?;
        Ok(Self {
            event_receiver,
            handler,
            shutdown_sender,
        })
    }

    pub(crate) async fn start(&mut self) {
        let shutdown_receiver = self.shutdown_sender.subscribe();
        let mut nb_of_consecutive_db_errors = 0;

        loop {
            tokio::select! {
                notification_result = self.event_receiver.recv() => {
                    match notification_result {
                        Ok(notification) => {
                            nb_of_consecutive_db_errors = 0;
                            self.handler.on_notification(notification).await;
                        }

                        Err(EventReceiverError::ParsingError(e)) => {
                            // This should never happen, so we shut down the replica immediately
                            error!("Failed to parse notification: {e:?}. Shutting down replica.");
                            exit_rollup(&self.shutdown_sender).await;
                        }
                        Err(EventReceiverError::ListenerError(e)) => {
                            error!("Failed to receive notifications from database: {e:?}. Shutting down replica.");

                            if shutdown_receiver.has_changed().unwrap_or(true) {
                                break;
                            }

                            // Since network errors can occur, we will retry receiving a few times before initiating replica shutdown.
                            if nb_of_consecutive_db_errors >= MAX_DB_ERRORS_ALLOWED {
                                error!("Failed to connect to the database after {nb_of_consecutive_db_errors} attempts. Shutting down replica.");
                                exit_rollup(&self.shutdown_sender).await;
                            }

                            nb_of_consecutive_db_errors += 1;
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferred::db::postgres::PostgresBackend;
    use crate::preferred::db::PreferredSequencerDbBackend;
    use sov_test_utils::postgres::connection_string_from_postgres_container;
    use sov_test_utils::postgres::create_postgres_container;
    use std::num::NonZero;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct TestHandler {
        send: mpsc::Sender<EventsNotificationPayload>,
    }

    impl TestHandler {
        pub fn new() -> (Self, mpsc::Receiver<EventsNotificationPayload>) {
            let (send, recv) = mpsc::channel(1);
            (Self { send }, recv)
        }
    }

    impl ReplicaEventHandler for TestHandler {
        async fn on_notification(&self, notification: EventsNotificationPayload) {
            self.send.send(notification).await.unwrap();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_notifications() {
        let dir = tempfile::tempdir().unwrap();
        let postgres = create_postgres_container(&dir.path().join("postgres_data"))
            .await
            .unwrap();

        let postgres_connection_string = connection_string_from_postgres_container(&postgres)
            .await
            .unwrap();

        let (sync_task_ready_snd, mut sync_task_ready_rcv) = mpsc::channel(1);

        // Insert `postgres_db_backend_begin_rollup_block` into the db.
        {
            let conn_str = postgres_connection_string.clone();
            tokio::spawn(async move {
                let mut db = PostgresBackend::connect(&conn_str).await.unwrap();
                sync_task_ready_rcv.recv().await.unwrap();
                db.begin_rollup_block(99, 11, Default::default(), NonZero::new(1).unwrap())
                    .await
                    .unwrap();
            });
        }

        // Check if replica sync task received the notification.
        let (shutdown_snd, _shutdown_rcv) = watch::channel(());

        let (test_handler, mut recv) = TestHandler::new();
        {
            let conn_str = postgres_connection_string.clone();
            let test_handler = test_handler.clone();
            let shutdown_snd = shutdown_snd.clone();
            tokio::spawn(async move {
                let mut sync_task = ReplicaSyncTask::new(&conn_str, test_handler, shutdown_snd)
                    .await
                    .unwrap();
                sync_task_ready_snd.send(()).await.unwrap();
                sync_task.start().await;
            });
        }

        let event = recv.recv().await.unwrap();
        assert_eq!(event.sequence_number, 99);

        shutdown_snd.send(()).unwrap();
    }
}
