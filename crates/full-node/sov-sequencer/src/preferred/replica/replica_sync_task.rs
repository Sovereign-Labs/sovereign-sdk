use crate::preferred::replica::event_receiver::{EventReceiver, EventsNotificationPayload};
use tracing::error;

pub(crate) trait ReplicaEventHandler {
    async fn on_notification(&self, notification: EventsNotificationPayload);
}

pub(crate) struct ReplicaSyncTask<R: ReplicaEventHandler> {
    event_receiver: EventReceiver,
    handler: R,
}

impl<R: ReplicaEventHandler> ReplicaSyncTask<R> {
    pub(crate) async fn new(postgres_connection_string: &str, handler: R) -> anyhow::Result<Self> {
        let event_receiver = EventReceiver::new(postgres_connection_string).await?;
        Ok(Self {
            event_receiver,
            handler,
        })
    }

    pub(crate) async fn start(&mut self) {
        loop {
            tokio::select! {
                notification_result = self.event_receiver.recv() => {
                    match notification_result {
                        Ok(notification) => {
                            self.handler.on_notification(notification.unwrap()).await;
                        }
                        Err(e) => {
                            error!("Error receiving notifications: {e:?}");
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
        let (test_handler, mut recv) = TestHandler::new();
        {
            let conn_str = postgres_connection_string.clone();
            let test_handler = test_handler.clone();
            tokio::spawn(async move {
                let mut sync_task = ReplicaSyncTask::new(&conn_str, test_handler).await.unwrap();
                sync_task_ready_snd.send(()).await.unwrap();
                sync_task.start().await;
            });
        }

        let event = recv.recv().await.unwrap();
        assert_eq!(event.sequence_number, 99);
    }
}
