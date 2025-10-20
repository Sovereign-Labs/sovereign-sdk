use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::event_receiver::{EventReceiver, PAGE_SIZE};
use async_trait::async_trait;
use tokio::sync::watch;
use tokio::task::JoinHandle;

#[async_trait]
pub(crate) trait ReplicaEventHandler: Send + Sync + 'static {
    async fn on_da_event(&self, batch: DbData);
}

pub(crate) struct ReplicaTaskHandles {
    pub(crate) data_fetcher_handle: JoinHandle<()>,
    pub(crate) sync_task_handle: JoinHandle<()>,
}

pub(crate) struct ReplicaSyncTask {
    shutdown_sender: watch::Sender<()>,
    postgres_connection_string: String,
    page_size: usize,
}

impl ReplicaSyncTask {
    pub(crate) async fn new(
        postgres_connection_string: String,
        shutdown_sender: watch::Sender<()>,
    ) -> anyhow::Result<Self> {
        Self::new_with_page_size(postgres_connection_string, shutdown_sender, PAGE_SIZE).await
    }

    pub(crate) async fn new_with_page_size(
        postgres_connection_string: String,
        shutdown_sender: watch::Sender<()>,
        page_size: usize,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            postgres_connection_string,
            shutdown_sender,
            page_size,
        })
    }

    pub(crate) async fn start<R: ReplicaEventHandler>(&mut self, handler: R) -> ReplicaTaskHandles {
        let (event_receiver, mut db_data_receiver) = EventReceiver::new(
            self.postgres_connection_string.clone(),
            self.shutdown_sender.clone(),
            self.page_size,
        )
        .await;

        let data_fetcher_handle = event_receiver.spawn_db_data_fetcher().await;
        let shutdown_receiver = self.shutdown_sender.subscribe();

        let sync_task_handle = tokio::spawn(async move {
            loop {
                if shutdown_receiver.has_changed().unwrap_or(true) {
                    break;
                }

                if let Some(data) = db_data_receiver.recv().await {
                    handler.on_da_event(data).await;
                }
            }
        });

        ReplicaTaskHandles {
            data_fetcher_handle,
            sync_task_handle,
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
    use sov_test_utils::postgres::{
        connection_string_from_postgres_container, create_postgres_container, CreatePostgresError,
    };

    use std::num::NonZero;
    use std::vec;
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
            let _ = self.send.send(data).await;
        }
    }

    fn new_batch_to_store(sequence_number: u64) -> BatchToStore {
        BatchToStore {
            sequence_number,
            blob_id: (sequence_number + 99) as u128,
            visible_slot_number_after_increase: VisibleSlotNumber::new_dangerous(1),
            visible_slots_to_advance: NonZero::new(1).unwrap(),
        }
    }

    #[derive(Debug, Clone)]
    enum TestCase {
        CompleteBatch(u64, usize),
        BatchStart(u64),
        Transaction,
        BatchEnd(u64),
    }

    async fn execute(mut db: PostgresBackend, data: Vec<TestCase>) {
        let mut index = 0;
        let mut sequence_nr = 0;
        for db_data in data {
            match db_data {
                TestCase::CompleteBatch(seq_nr, nb_of_txs) => {
                    index = 0;
                    let stored_batch = new_batch_to_store(seq_nr);
                    db.begin_rollup_block(stored_batch.clone()).await.unwrap();

                    for i in 0..nb_of_txs {
                        let tx = FullyBakedTx::new(vec![i as u8]);
                        db.add_tx(seq_nr, i as u64, tx, TxHash::new([1; 32]))
                            .await
                            .unwrap();
                    }

                    db.end_rollup_block(stored_batch).await.unwrap();
                }
                TestCase::BatchStart(seq_nr) => {
                    sequence_nr = seq_nr;
                    index = 0;
                    let stored_batch = new_batch_to_store(seq_nr);
                    db.begin_rollup_block(stored_batch).await.unwrap();
                }
                TestCase::Transaction => {
                    let tx = FullyBakedTx::new(vec![index as u8]);
                    db.add_tx(sequence_nr, index, tx, TxHash::new([1; 32]))
                        .await
                        .unwrap();
                    index += 1;
                }
                TestCase::BatchEnd(seq_nr) => {
                    let stored_batch = new_batch_to_store(seq_nr);
                    db.end_rollup_block(stored_batch).await.unwrap();
                }
            };
        }
    }

    fn to_db_data(test_cases: &Vec<TestCase>) -> Vec<DbData> {
        let mut data = Vec::new();
        let mut index = 0;
        for test_case in test_cases {
            match test_case {
                TestCase::CompleteBatch(seq_nr, nb_of_txs) => {
                    data.push(DbData::BatchStart(new_batch_to_store(*seq_nr)));
                    for i in 0..*nb_of_txs {
                        data.push(DbData::Transaction(FullyBakedTx::new(vec![i as u8])));
                    }
                    data.push(DbData::BatchEnd(new_batch_to_store(*seq_nr)));
                }
                TestCase::BatchStart(seq_nr) => {
                    index = 0;
                    data.push(DbData::BatchStart(new_batch_to_store(*seq_nr)));
                }
                TestCase::Transaction => {
                    data.push(DbData::Transaction(FullyBakedTx::new(vec![index as u8])));
                    index += 1;
                }
                TestCase::BatchEnd(seq_nr) => {
                    data.push(DbData::BatchEnd(new_batch_to_store(*seq_nr)));
                }
            };
        }
        data
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_notifications() {
        let test_data = vec![
            TestCase::BatchStart(1),
            TestCase::Transaction,
            TestCase::Transaction,
            TestCase::Transaction,
            TestCase::BatchEnd(1),
            TestCase::CompleteBatch(2, 0),
            TestCase::CompleteBatch(3, 1000),
            TestCase::CompleteBatch(4, 2),
        ];

        let expected = to_db_data(&test_data);
        run(test_data, expected).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_notifications_start_event_id() {
        let test_data = vec![
            TestCase::Transaction,
            TestCase::Transaction,
            TestCase::Transaction,
            TestCase::CompleteBatch(7, 3),
            TestCase::BatchStart(8),
            TestCase::Transaction,
            TestCase::Transaction,
        ];

        let expected = to_db_data(&test_data).into_iter().skip(3).collect();
        run(test_data, expected).await;
    }

    async fn run(test_cases: Vec<TestCase>, expected: Vec<DbData>) {
        let dir = tempfile::tempdir().unwrap();

        let postgres = create_postgres_container(&dir.path().join("postgres_data")).await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let postgres_connection_string = connection_string_from_postgres_container(&postgres)
            .await
            .unwrap();

        let db = PostgresBackend::connect(&postgres_connection_string)
            .await
            .unwrap();

        let (shutdown_snd, _shutdown_rcv) = watch::channel(());
        let mut sync_task =
            ReplicaSyncTask::new_with_page_size(postgres_connection_string, shutdown_snd, 8)
                .await
                .unwrap();

        let (test_handler, mut recv) = TestHandler::new();
        sync_task.start(test_handler).await;

        execute(db, test_cases).await;

        for data in expected {
            let recv_data = recv.recv().await.unwrap();
            assert_eq!(recv_data, data, "Expected: {data:?}, got: {recv_data:?}");
        }
    }
}
