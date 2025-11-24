#![allow(dead_code)]
use crate::preferred::block_executor::StartBlockData;
use crate::preferred::PreferredSequencerConfig;
use crate::preferred::RollupBlockExecutor;
use crate::preferred::RollupBlockExecutorConfig;
use crate::preferred::SequenceNumber;
use crate::RollupHeight;
use crate::SequencerConfig;
use sov_metrics::Metric;
use sov_modules_api::Spec;
use sov_modules_api::StateCheckpoint;
use sov_modules_api::StateUpdateInfo;
use sov_modules_api::Storage;
use sov_modules_api::TxChangeSet;
use sov_modules_api::{FullyBakedTx, Runtime};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

// We have several work-stealing executor workers for a single main worker, so we don't expect the channel to become full.
// Even if it does, the sender uses a non-blocking method, meaning a few updates may simply be skipped.
const TX_CHANNEL_SIZE: usize = 16;

// Maximum number of transactions ignored by an executor because they are too new or too old.
const MAX_NB_OF_IGNORED_TXS: u64 = 16;

/// A transaction with a way to retrieve the corresponding state changes it may produce.
pub struct FullyBakedTxWithMaybeChangeSet {
    /// The original transaction.
    pub tx: FullyBakedTx,
    /// The worker executor will race against the main executor, attempting to compute  
    /// the changeset before the main executor processes the transaction.  
    /// Then, the worker will send the computed changeset through this channel,  
    /// allowing the main sequencer executor to reuse these values before executing the transaction.
    pub receiver: Option<oneshot::Receiver<TxChangeSet>>,
}

impl FullyBakedTxWithMaybeChangeSet {
    /// Creates new `FullyBakedTxWithMaybeChangeSet`
    pub fn new(tx: FullyBakedTx) -> Self {
        Self { tx, receiver: None }
    }
}

pub(crate) struct StartBlockNotification<S: Spec> {
    pub(crate) data: StartBlockData<S>,
    pub(crate) checkpoint: StateCheckpoint<S>,
    pub(crate) state_roots: BTreeMap<RollupHeight, <S::Storage as Storage>::Root>,
    pub(crate) sequence_number: SequenceNumber,
}

impl<S: Spec> Clone for StartBlockNotification<S> {
    fn clone(&self) -> Self {
        Self {
            state_roots: self.state_roots.clone(),
            data: self.data.clone(),
            checkpoint: self
                .checkpoint
                .clone_with_empty_witness_dropping_temp_cache_and_ignoring_pinned_cache(),
            sequence_number: self.sequence_number,
        }
    }
}

struct FullyBakedTxWithTxChangeSetSender {
    tx: FullyBakedTx,
    sender: oneshot::Sender<TxChangeSet>,
    sequence_number: SequenceNumber,
}

#[derive(Clone)]
struct TxReceiver {
    size: Arc<AtomicU64>,
    receiver: flume::Receiver<FullyBakedTxWithTxChangeSetSender>,
}

#[derive(Clone)]
pub(crate) struct CacheWarmUpExecutorInner<S: Spec> {
    start_block_notification_sender: tokio::sync::watch::Sender<Option<StartBlockNotification<S>>>,
    tx_sender: flume::Sender<FullyBakedTxWithTxChangeSetSender>,
    size: Arc<AtomicU64>,
}

#[derive(Clone)]
pub(crate) struct CacheWarmUpExecutor<S: Spec> {
    inner: Option<CacheWarmUpExecutorInner<S>>,
}

impl<S: Spec> CacheWarmUpExecutor<S> {
    pub(crate) fn send_batch_start_notification(&self, data: StartBlockNotification<S>) {
        let Some(inner) = &self.inner else {
            return;
        };
        // This `send` does not block.
        let _ = inner.start_block_notification_sender.send(Some(data));
    }

    pub(crate) fn send_tx(
        &self,
        tx: FullyBakedTx,
        sequence_number: u64,
    ) -> FullyBakedTxWithMaybeChangeSet {
        let Some(inner) = &self.inner else {
            return FullyBakedTxWithMaybeChangeSet { tx, receiver: None };
        };

        // We need to update the `size` field before inserting the thx into tx_sender, otherwise the workers may see an outdated channel size.
        let size = inner.size.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();

        // Skip update if consumer is too slow.
        let res = inner.tx_sender.try_send(FullyBakedTxWithTxChangeSetSender {
            tx: tx.clone(),
            sender,
            sequence_number,
        });

        let maybe_receiver = match res {
            Ok(_) => {
                sov_metrics::track_metrics(|t| {
                    t.submit(CacheWarmUpMetrics {
                        tx_channel_size: size + 1,
                    });
                });

                Some(receiver)
            }
            Err(flume::TrySendError::Full(_)) => {
                let size = inner.size.fetch_sub(1, Ordering::Relaxed);
                tracing::warn!(size, "The tx queue is full. You may want to increase the number of workers for cache warmup.");
                None
            }
            Err(flume::TrySendError::Disconnected(_)) => {
                inner.size.fetch_sub(1, Ordering::Relaxed);
                None
            }
        };

        FullyBakedTxWithMaybeChangeSet {
            tx,
            receiver: maybe_receiver,
        }
    }

    pub(crate) async fn spawn_execution_task<Rt: Runtime<S>>(
        info: StateUpdateInfo<S::Storage>,
        exec_config: RollupBlockExecutorConfig<S>,
        seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
    ) -> (Self, Vec<JoinHandle<()>>) {
        if seq_config.sequencer_kind_config.is_replica {
            return (Self { inner: None }, vec![]);
        }

        let (tx_sender, tx_receiver) = flume::bounded(TX_CHANNEL_SIZE);
        let size = Arc::new(AtomicU64::new(0));
        let tx_receiver = TxReceiver {
            size: size.clone(),
            receiver: tx_receiver,
        };

        // Option<StartBlockNotification<S>> is niche-optimized, so keeping it instead of using
        // StartBlockNotification directly in the channel does not introduce any overhead.
        // Moreover, this is only used for the watch channel.
        let (start_block_notification_sender, start_block_notification_receiver) =
            tokio::sync::watch::channel(None);

        let mut handles = Vec::new();
        for _ in 0..seq_config.sequencer_kind_config.num_cache_warmup_workers {
            let worker = Self::spawn_worker::<Rt>(
                info.clone(),
                exec_config.clone(),
                seq_config.clone(),
                tx_receiver.clone(),
                start_block_notification_receiver.clone(),
            );

            handles.push(worker);
        }

        (
            Self {
                inner: Some(CacheWarmUpExecutorInner {
                    tx_sender,
                    start_block_notification_sender,
                    size,
                }),
            },
            handles,
        )
    }

    fn spawn_worker<Rt: Runtime<S>>(
        info: StateUpdateInfo<S::Storage>,
        exec_config: RollupBlockExecutorConfig<S>,
        seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
        tx_receiver: TxReceiver,
        mut start_block_notification_receiver: tokio::sync::watch::Receiver<
            Option<StartBlockNotification<S>>,
        >,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut shutdown_receiver = exec_config.shutdown_receiver.clone();
            let mut executor = RollupBlockExecutor::<_, Rt>::new(
                &info,
                exec_config,
                seq_config.clone(),
                Default::default(),
                None, // TODO: Consider adding a pinned cache to the warmup executors
            );

            let mut maybe_executor_sequence_number = None;
            let mut nb_of_ignored_txs = 0;
            loop {
                tokio::select! {
                    _ = start_block_notification_receiver.changed() => {
                        let notify = start_block_notification_receiver.borrow().clone();
                        if let Some(notify) = notify {
                              maybe_executor_sequence_number = Some(Self::start_block(notify, &mut executor).await);
                        }
                    }

                    tx = tx_receiver.receiver.recv_async() => {
                        let tx_with_sender = match tx {
                             Ok(tx) =>
                             {
                                tx_receiver.size.fetch_sub(1, Ordering::Relaxed);
                                tx
                             },
                             Err(flume::RecvError::Disconnected) => {
                                // Quit if channel closed.
                                return
                            },
                        };

                        if let Some(executor_sequence_number) = maybe_executor_sequence_number {
                            if tx_with_sender.sequence_number != executor_sequence_number {
                                // Due to timing issues, the received transaction may have either older or newer sequence number than the executor.
                                // In this case, we simply continue the loop until the executor and the transactions received from the `accept_tx` task are synchronized.
                                // If this occurs too frequently, we escalate the log level from warning to error.
                                if nb_of_ignored_txs > MAX_NB_OF_IGNORED_TXS {
                                    tracing::error!(%tx_with_sender.sequence_number, %executor_sequence_number, %nb_of_ignored_txs, "Cache warm up task: Transaction could not be applied on the executor.");
                                }else{
                                    tracing::warn!(%tx_with_sender.sequence_number, %executor_sequence_number, %nb_of_ignored_txs, "Cache warm up task: Transaction could not be applied on the executor.");
                                }

                                nb_of_ignored_txs += 1;
                                continue;
                            }

                            nb_of_ignored_txs = 0;

                            let baked_tx = FullyBakedTxWithMaybeChangeSet::new(tx_with_sender.tx);
                            let res = executor.apply_tx_to_in_progress_batch(baked_tx).await;

                            match res{
                                Ok((_, tx_change_set)) => {
                                    // It ok safe to ignore the error if the receiver was dropped.
                                    // This can happen if the transaction on the main executor has already finished.
                                    let _ = tx_with_sender.sender.send(tx_change_set);
                                },
                                Err(err) => {
                                    tracing::trace!(%err, "WarmUp worker task failed to execute transaction.");
                                    continue;
                                }
                            }
                        }

                    }
                   _ = shutdown_receiver.changed() => {
                        // Quit on shutdown.
                        return;
                   }
                }
            }
        })
    }

    async fn start_block<Rt: Runtime<S>>(
        notify: StartBlockNotification<S>,
        executor: &mut RollupBlockExecutor<S, Rt>,
    ) -> SequenceNumber {
        let _ = executor.shutdown().await;
        let seq_nr_from_start_block = notify.sequence_number;

        executor
            .start_rollup_block_with_provided_state_roots(
                notify.data,
                notify.checkpoint,
                notify.state_roots,
            )
            .await;

        seq_nr_from_start_block
    }
}

#[derive(Debug)]
pub(crate) struct CacheWarmUpMetrics {
    tx_channel_size: u64,
}

impl Metric for CacheWarmUpMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_sequencer_cache_warmup_metrics"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} tx_channel_size={}",
            self.measurement_name(),
            self.tx_channel_size,
        )
    }
}
