use crate::preferred::replica::event_receiver::DbData;
use crate::preferred::replica::event_receiver::EventReceiver;
use tokio::sync::watch;

pub(crate) trait ReplicaEventHandler {
    async fn on_da_event(&self, batch: DbData);
}

pub(crate) struct ReplicaSyncTask<R: ReplicaEventHandler> {
    event_receiver: EventReceiver,
    handler: R,
    shutdown_sender: watch::Sender<()>,
}

impl<R: ReplicaEventHandler> ReplicaSyncTask<R> {
    pub(crate) async fn new(
        postgres_connection_string: String,
        handler: R,
        shutdown_sender: watch::Sender<()>,
        start_event_id: u64,
    ) -> anyhow::Result<Self> {
        let event_receiver = EventReceiver::new(
            postgres_connection_string,
            shutdown_sender.clone(),
            start_event_id,
        )
        .await;
        Ok(Self {
            event_receiver,
            handler,
            shutdown_sender,
        })
    }

    pub(crate) async fn start(&mut self) {
        self.event_receiver.spawn_db_data_fetcher().await;
        let shutdown_receiver = self.shutdown_sender.subscribe();

        loop {
            if shutdown_receiver.has_changed().unwrap_or(true) {
                break;
            }

            if let Some(data) = self.event_receiver.recv().await {
                self.handler.on_da_event(data).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferred::db::postgres::PostgresBackend;
    use crate::preferred::db::BatchToStore;
    use crate::preferred::db::PreferredSequencerDbBackend;
    use sov_modules_api::FullyBakedTx;
    use sov_modules_api::TxHash;
    use sov_modules_api::VisibleSlotNumber;
    use sov_test_utils::postgres::connection_string_from_postgres_container;
    use sov_test_utils::postgres::create_postgres_container;
    use std::num::NonZero;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct TestHandler {
        send: mpsc::Sender<DbData>,
    }

    impl TestHandler {
        pub fn new() -> (Self, mpsc::Receiver<DbData>) {
            let (send, recv) = mpsc::channel(1);
            (Self { send }, recv)
        }
    }

    impl ReplicaEventHandler for TestHandler {
        async fn on_da_event(&self, data: DbData) {
            self.send.send(data).await.unwrap();
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

        let sequence_number = 10;
        let blob_id = 99;
        let visible_slot_number_after_increase = VisibleSlotNumber::new_dangerous(11);
        let visible_slots_to_advance = NonZero::new(1).unwrap();
        // Insert `postgres_db_backend_begin_rollup_block` into the db.
        {
            let conn_str = postgres_connection_string.clone();
            tokio::spawn(async move {
                let mut db = PostgresBackend::connect(&conn_str).await.unwrap();
                sync_task_ready_rcv.recv().await.unwrap();

                db.begin_rollup_block(
                    sequence_number,
                    blob_id,
                    visible_slot_number_after_increase,
                    visible_slots_to_advance,
                )
                .await
                .unwrap();

                db.add_tx(
                    sequence_number,
                    0,
                    FullyBakedTx::new(vec![1, 2, 3]),
                    TxHash::new([11; 32]),
                )
                .await
                .unwrap();

                db.end_rollup_block(BatchToStore {
                    blob_id,
                    sequence_number,
                    visible_slot_number_after_increase,
                    visible_slots_to_advance,
                })
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
                let mut sync_task = ReplicaSyncTask::new(conn_str, test_handler, shutdown_snd, 0)
                    .await
                    .unwrap();
                sync_task_ready_snd.send(()).await.unwrap();
                sync_task.start().await;
            });
        }

        let _event = recv.recv().await.unwrap();
        let _event = recv.recv().await.unwrap();
        let _event = recv.recv().await.unwrap();

        shutdown_snd.send(()).unwrap();
    }
}
