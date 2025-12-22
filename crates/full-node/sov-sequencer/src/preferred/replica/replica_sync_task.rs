use crate::preferred::db::SequencerRole;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::event_receiver::EventReceiver;
use crate::preferred::replica::event_receiver::EventReceiverStartNotifier;
use async_trait::async_trait;
use sov_full_node_configs::sequencer::PostgresConfig;
use sov_rollup_interface::node::future_or_shutdown;
use sov_rollup_interface::node::FutureOrShutdownOutput;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Duration;

// Process events in pages to avoid excessive memory consumption
const PAGE_SIZE: usize = 2000;

#[derive(Debug)]
pub(crate) enum DBDataRejected {
    ExecutorBehind(DbData),
    ExecutorAhead(u64),
}

#[async_trait]
pub(crate) trait ReplicaEventHandler: Send + Sync + 'static {
    async fn on_db_event(&self, batch: DbData) -> Result<(), DBDataRejected>;
}

pub(crate) struct ReplicaTaskHandles {
    pub(crate) data_fetcher_handle: JoinHandle<()>,
    pub(crate) sync_task_handle: JoinHandle<()>,
}

pub(crate) struct ReplicaSyncTask {
    shutdown_sender: watch::Sender<()>,
    page_size: usize,
    start_replica_task_receiver: watch::Receiver<()>,
}

impl ReplicaSyncTask {
    pub(crate) async fn new(
        shutdown_sender: watch::Sender<()>,
        seq_role: SequencerRole,
    ) -> anyhow::Result<(Self, EventReceiverStartNotifier)> {
        Self::new_with_page_size(shutdown_sender, PAGE_SIZE, seq_role).await
    }

    pub(crate) async fn new_with_page_size(
        shutdown_sender: watch::Sender<()>,
        page_size: usize,
        seq_role: SequencerRole,
    ) -> anyhow::Result<(Self, EventReceiverStartNotifier)> {
        let (start_replica_task_notifier, start_replica_task_receiver) =
            EventReceiverStartNotifier::new(seq_role);
        Ok((
            Self {
                shutdown_sender,
                page_size,
                start_replica_task_receiver,
            },
            start_replica_task_notifier,
        ))
    }

    pub(crate) async fn start<R: ReplicaEventHandler>(
        &mut self,
        handler: R,
        postgres_config: &PostgresConfig,
    ) -> ReplicaTaskHandles {
        let (event_receiver, db_data_receiver) = EventReceiver::new(
            postgres_config.postgres_connection_string.clone(),
            self.shutdown_sender.clone(),
            self.page_size,
            self.start_replica_task_receiver.clone(),
        )
        .await;

        let data_fetcher_handle = event_receiver.spawn_db_data_fetcher().await;
        let shutdown_receiver = self.shutdown_sender.subscribe();

        let sync_task_handle = tokio::spawn(async move {
            Self::run_handler(handler, db_data_receiver, shutdown_receiver).await;
        });

        ReplicaTaskHandles {
            data_fetcher_handle,
            sync_task_handle,
        }
    }

    async fn run_handler<R: ReplicaEventHandler>(
        handler: R,
        mut db_data_receiver: tokio::sync::mpsc::Receiver<DbData>,
        shutdown_receiver: watch::Receiver<()>,
    ) {
        'outer: loop {
            let fut = future_or_shutdown(db_data_receiver.recv(), &shutdown_receiver);
            let FutureOrShutdownOutput::Output(Some(mut data)) = fut.await else {
                break 'outer;
            };

            'inner: loop {
                if shutdown_receiver.has_changed().unwrap_or(true) {
                    break 'outer;
                }

                match handler.on_db_event(data).await {
                    Ok(_) => {
                        // The data was applied on the executor.
                        break 'inner;
                    }

                    Err(DBDataRejected::ExecutorAhead(executor_seq_nr)) => {
                        // The executor is ahead of the db drain the queue and wait until we catch up.
                        loop {
                            let fut =
                                future_or_shutdown(db_data_receiver.recv(), &shutdown_receiver);

                            let FutureOrShutdownOutput::Output(Some(new_data)) = fut.await else {
                                break 'outer;
                            };

                            assert!(
                                new_data.sequence_number() <= executor_seq_nr,
                                "The sequence number must be consecutive"
                            );

                            if new_data.sequence_number() == executor_seq_nr {
                                assert!(matches!(new_data, DbData::BatchStart(_)));
                                data = new_data;
                                continue 'inner;
                            }
                        }
                    }

                    Err(DBDataRejected::ExecutorBehind(db_data)) => {
                        // The executor is behind the db: wait briefly and retry.
                        data = db_data;
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue 'inner;
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
    use crate::preferred::db::BatchToStore;
    use crate::preferred::db::DbBackend;
    use sov_modules_api::FullyBakedTx;
    use sov_modules_api::TxHash;
    use sov_modules_api::VisibleSlotNumber;
    use sov_test_utils::postgres::config_from_postgres_container;
    use sov_test_utils::postgres::{create_postgres_container, CreatePostgresError};
    use std::sync::atomic::Ordering;
    use tokio::sync::mpsc::error::TryRecvError;

    use std::num::NonZero;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use std::vec;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct TestHandler {
        send: mpsc::Sender<DbData>,
        exec_seq_nr: Arc<AtomicU64>,
    }

    impl TestHandler {
        pub fn new(seq_nr: u64) -> (Self, mpsc::Receiver<DbData>) {
            let (send, recv) = mpsc::channel(100);
            (
                Self {
                    send,
                    exec_seq_nr: Arc::new(AtomicU64::new(seq_nr)),
                },
                recv,
            )
        }

        fn inc_seq_nr(&self) {
            self.exec_seq_nr.fetch_add(1, Ordering::SeqCst);
        }

        fn seq_nr(&self) -> u64 {
            self.exec_seq_nr.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ReplicaEventHandler for TestHandler {
        async fn on_db_event(&self, data: DbData) -> Result<(), DBDataRejected> {
            if data.sequence_number() < self.seq_nr() {
                return Err(DBDataRejected::ExecutorAhead(self.seq_nr()));
            }

            if data.sequence_number() > self.seq_nr() + 1 {
                return Err(DBDataRejected::ExecutorBehind(data));
            }

            if matches!(data, DbData::BatchEnd { .. }) {
                self.inc_seq_nr();
            }

            let _ = self.send.send(data).await;
            Ok(())
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

    #[derive(Debug, Clone, Copy)]
    enum TestCase {
        CompleteBatch(u64, usize),
        BatchStart(u64),
        Transaction(u64),
        BatchEnd(u64),
    }

    async fn execute(mut db: PostgresBackend, data: Vec<TestCase>) {
        let mut index = 0;
        for db_data in data {
            match db_data {
                TestCase::CompleteBatch(seq_nr, nb_of_txs) => {
                    index = 0;
                    let stored_batch = new_batch_to_store(seq_nr);
                    db.begin_rollup_block(stored_batch).await.unwrap();

                    for i in 0..nb_of_txs {
                        let tx = FullyBakedTx::new(vec![i as u8]);
                        db.add_tx(seq_nr, i as u64, tx, TxHash::new([1; 32]))
                            .await
                            .unwrap();
                    }

                    db.end_rollup_block(stored_batch).await.unwrap();
                }
                TestCase::BatchStart(seq_nr) => {
                    index = 0;
                    let stored_batch = new_batch_to_store(seq_nr);
                    db.begin_rollup_block(stored_batch).await.unwrap();
                }
                TestCase::Transaction(seq_nr) => {
                    let tx = FullyBakedTx::new(vec![index as u8]);
                    db.add_tx(seq_nr, index, tx, TxHash::new([1; 32]))
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
            match *test_case {
                TestCase::CompleteBatch(seq_nr, nb_of_txs) => {
                    data.push(DbData::BatchStart(new_batch_to_store(seq_nr)));
                    for i in 0..nb_of_txs {
                        data.push(DbData::Transaction(
                            seq_nr,
                            FullyBakedTx::new(vec![i as u8]),
                            TxHash::new([1; 32]),
                        ));
                    }
                    data.push(DbData::BatchEnd(new_batch_to_store(seq_nr)));
                }
                TestCase::BatchStart(seq_nr) => {
                    index = 0;
                    data.push(DbData::BatchStart(new_batch_to_store(seq_nr)));
                }
                TestCase::Transaction(seq_nr) => {
                    data.push(DbData::Transaction(
                        seq_nr,
                        FullyBakedTx::new(vec![index as u8]),
                        TxHash::new([1; 32]),
                    ));
                    index += 1;
                }
                TestCase::BatchEnd(seq_nr) => {
                    data.push(DbData::BatchEnd(new_batch_to_store(seq_nr)));
                }
            };
        }
        data
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_notifications() {
        let test_data = vec![
            TestCase::BatchStart(1),
            TestCase::Transaction(1),
            TestCase::Transaction(1),
            TestCase::Transaction(1),
            TestCase::BatchEnd(1),
            TestCase::CompleteBatch(2, 0),
            TestCase::CompleteBatch(3, 1000),
            TestCase::CompleteBatch(4, 2),
        ];

        let expected = to_db_data(&test_data);
        run(test_data, expected, 0).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_notifications_start_event_id() {
        //sov_test_utils::initialize_logging();
        let test_data = vec![
            TestCase::Transaction(6),
            TestCase::Transaction(6),
            TestCase::Transaction(6),
            TestCase::CompleteBatch(7, 3),
            TestCase::BatchStart(8),
            TestCase::Transaction(8),
            TestCase::Transaction(8),
        ];

        let expected = to_db_data(&test_data).into_iter().skip(3).collect();
        run(test_data, expected, 6).await;
    }

    async fn run(test_cases: Vec<TestCase>, expected: Vec<DbData>, exec_seq_nr: u64) {
        let postgres = create_postgres_container().await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let postgres_config = config_from_postgres_container(&postgres, "Replica".into())
            .await
            .unwrap();

        let db = PostgresBackend::connect(&postgres_config).await.unwrap();

        let (shutdown_snd, _shutdown_rcv) = watch::channel(());
        let (mut sync_task, start_replica_task_notifier) =
            ReplicaSyncTask::new_with_page_size(shutdown_snd, 8)
                .await
                .unwrap();

        start_replica_task_notifier.notify();

        let (test_handler, mut recv) = TestHandler::new(exec_seq_nr);
        sync_task.start(test_handler, &postgres_config).await;

        // Wait for sync_task.start to spawn the sync task
        tokio::time::sleep(Duration::from_millis(100)).await;

        execute(db, test_cases).await;

        for data in expected {
            let recv_data = recv.recv().await.unwrap();
            assert_eq!(recv_data, data, "Expected: {data:?}, got: {recv_data:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sync_task_on_time() {
        // The first batch has a seq_nr = 1 and an executor has seq_nr = 0.
        let test_cases = vec![
            TestCase::CompleteBatch(1, 3),
            TestCase::CompleteBatch(2, 3),
            TestCase::CompleteBatch(3, 3),
        ];
        let seq_nr = 0;

        let test_cases = to_db_data(&test_cases);
        check_sync_task(test_cases.clone(), test_cases, seq_nr).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sync_task_ahead() {
        // The first batch has a seq_nr = 1 and an executor has seq_nr = 3.
        let test_cases = vec![
            TestCase::CompleteBatch(1, 3),
            TestCase::CompleteBatch(2, 3),
            TestCase::CompleteBatch(3, 3),
            TestCase::CompleteBatch(4, 3),
            TestCase::CompleteBatch(5, 3),
        ];

        let seq_nr = 3;
        let test_cases = to_db_data(&test_cases);
        // We skip batch 1, 2 so 10 db messages in total.
        let expected = test_cases.iter().skip(10).cloned().collect();
        check_sync_task(test_cases.clone(), expected, seq_nr).await;
    }

    async fn check_sync_task(test_cases: Vec<DbData>, expected: Vec<DbData>, exec_seq_nr: u64) {
        let (_shutdown_snd, shutdown_rcv) = watch::channel(());
        let (db_data_sender, db_data_receiver) = tokio::sync::mpsc::channel(100);

        for db_data in test_cases.clone() {
            db_data_sender.send(db_data).await.unwrap();
        }

        let (test_handler, mut recv) = TestHandler::new(exec_seq_nr);
        tokio::task::spawn(async move {
            ReplicaSyncTask::run_handler(test_handler, db_data_receiver, shutdown_rcv).await;
        });

        for exp in expected.into_iter() {
            let data = recv.recv().await.unwrap();
            assert_eq!(data, exp);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sync_task_behind() {
        // The first batch has a seq_nr = 7 and an executor has seq_nr = 3.
        let test_case = vec![TestCase::CompleteBatch(7, 3), TestCase::CompleteBatch(8, 3)];
        let seq_nr = 3;
        let test_case = to_db_data(&test_case);
        let expected = test_case.clone();

        let (_shutdown_snd, shutdown_rcv) = watch::channel(());
        let (db_data_sender, db_data_receiver) = tokio::sync::mpsc::channel(100);

        for db_data in test_case.clone() {
            db_data_sender.send(db_data).await.unwrap();
        }

        let (test_handler, mut recv) = TestHandler::new(seq_nr);
        let test_handler_clone = test_handler.clone();

        tokio::task::spawn(async move {
            ReplicaSyncTask::run_handler(test_handler, db_data_receiver, shutdown_rcv).await;
        });

        for exp in expected {
            let data = loop {
                match recv.try_recv() {
                    Ok(ok) => break ok,
                    Err(TryRecvError::Empty) => {
                        // Simulate executor catching up.
                        test_handler_clone.inc_seq_nr();
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    Err(_) => unreachable!(),
                };
            };
            assert_eq!(data, exp);
        }
    }
}
