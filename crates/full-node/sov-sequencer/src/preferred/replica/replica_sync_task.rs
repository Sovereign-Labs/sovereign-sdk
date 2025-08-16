use crate::preferred::replica::event_receiver::DbData;
use crate::preferred::replica::event_receiver::EventReceiver;
use async_trait::async_trait;
use tokio::sync::watch;

#[async_trait]
pub(crate) trait ReplicaEventHandler: Send + Sync + 'static {
    async fn on_da_event(&self, batch: DbData);
}

pub(crate) struct ReplicaSyncTask {
    shutdown_sender: watch::Sender<()>,
    postgres_connection_string: String,
}

impl ReplicaSyncTask {
    pub(crate) async fn new(
        postgres_connection_string: String,

        shutdown_sender: watch::Sender<()>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            postgres_connection_string,
            shutdown_sender,
        })
    }

    pub(crate) async fn start<R: ReplicaEventHandler>(&mut self, handler: R) {
        let mut event_receiver = EventReceiver::new(
            self.postgres_connection_string.clone(),
            self.shutdown_sender.clone(),
        )
        .await;

        event_receiver.spawn_db_data_fetcher().await;
        let shutdown_receiver = self.shutdown_sender.subscribe();

        tokio::spawn(async move {
            loop {
                if shutdown_receiver.has_changed().unwrap_or(true) {
                    break;
                }

                if let Some(data) = event_receiver.recv().await {
                    handler.on_da_event(data).await;
                }
            }
        });
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

    #[async_trait]
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

        let mut db = PostgresBackend::connect(&postgres_connection_string)
            .await
            .unwrap();

        let (sync_task_ready_snd, mut sync_task_ready_rcv) = mpsc::channel(1);

        let sequence_number = 10;
        let blob_id = 99;
        let visible_slot_number_after_increase = VisibleSlotNumber::new_dangerous(11);
        let visible_slots_to_advance = NonZero::new(1).unwrap();

        // Insert `postgres_db_backend_begin_rollup_block` into the db.
        {
            tokio::spawn(async move {
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
                let mut sync_task = ReplicaSyncTask::new(conn_str, shutdown_snd).await.unwrap();

                sync_task.start(test_handler).await;
                sync_task_ready_snd.send(()).await.unwrap();
            });
        }

        let _event = recv.recv().await.unwrap();
        let _event = recv.recv().await.unwrap();
        let _event = recv.recv().await.unwrap();

        shutdown_snd.send(()).unwrap();
    }
}
