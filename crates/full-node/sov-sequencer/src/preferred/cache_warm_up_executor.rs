#![allow(dead_code)]
use crate::preferred::block_executor::StartBlockData;
use crate::preferred::PreferredSequencerConfig;
use crate::preferred::RollupBlockExecutor;
use crate::preferred::RollupBlockExecutorConfig;
use crate::RollupHeight;
use crate::SequencerConfig;
use sov_modules_api::Spec;
use sov_modules_api::StateCheckpoint;
use sov_modules_api::StateUpdateInfo;
use sov_modules_api::Storage;
use sov_modules_api::TxChangeSet;
use sov_modules_api::{FullyBakedTx, Runtime};
use std::collections::BTreeMap;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

// We have several work-stealing executor workers for a single main worker, so we don't expect the channel to become full.
// Even if it does, the sender uses a non-blocking method, meaning a few updates may simply be skipped.
const TX_CHANNEL_SIZE: usize = 16;

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
}

impl<S: Spec> Clone for StartBlockNotification<S> {
    fn clone(&self) -> Self {
        Self {
            state_roots: self.state_roots.clone(),
            data: self.data.clone(),
            checkpoint: self
                .checkpoint
                .clone_with_empty_witness_dropping_temp_cache(),
        }
    }
}

struct FullyBakedTxWithTxChangeSetSender {
    tx: FullyBakedTx,
    sender: oneshot::Sender<TxChangeSet>,
}

#[derive(Clone)]
pub(crate) struct CacheWarmUpExecutor<S: Spec> {
    start_block_notification_sender: tokio::sync::watch::Sender<Option<StartBlockNotification<S>>>,
    tx_sender: flume::Sender<FullyBakedTxWithTxChangeSetSender>,
}

impl<S: Spec> CacheWarmUpExecutor<S> {
    pub(crate) fn send_batch_start_notification(&self, data: StartBlockNotification<S>) {
        // This `send` does not block.
        let _ = self.start_block_notification_sender.send(Some(data));
    }

    pub(crate) fn send_tx(&self, tx: FullyBakedTx) -> FullyBakedTxWithMaybeChangeSet {
        let (sender, receiver) = oneshot::channel();

        // Skip update if consumer is too slow.
        let res = self.tx_sender.try_send(FullyBakedTxWithTxChangeSetSender {
            tx: tx.clone(),
            sender,
        });

        if res.is_err() {
            FullyBakedTxWithMaybeChangeSet { tx, receiver: None }
        } else {
            FullyBakedTxWithMaybeChangeSet {
                tx,
                receiver: Some(receiver),
            }
        }
    }

    pub(crate) async fn spawn_execution_task<Rt: Runtime<S>>(
        info: StateUpdateInfo<S::Storage>,
        exec_config: RollupBlockExecutorConfig<S>,
        seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
    ) -> (Self, Vec<JoinHandle<()>>) {
        let (tx_sender, tx_receiver) = flume::bounded(TX_CHANNEL_SIZE);

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
                tx_sender,
                start_block_notification_sender,
            },
            handles,
        )
    }

    fn spawn_worker<Rt: Runtime<S>>(
        info: StateUpdateInfo<S::Storage>,
        exec_config: RollupBlockExecutorConfig<S>,
        seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
        tx_receiver: flume::Receiver<FullyBakedTxWithTxChangeSetSender>,
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
            );

            let mut is_started = false;
            loop {
                tokio::select! {
                    _ = start_block_notification_receiver.changed() => {

                        let notify = start_block_notification_receiver.borrow().clone();
                        if let Some(notify) = notify {
                              let _ = executor.shutdown().await;
                              Self::start_block(notify, &mut executor).await;
                              is_started = true;
                        }
                    }

                    tx = tx_receiver.recv_async() => {
                        let tx_with_sender = match tx {
                             Ok(tx) => tx,
                             Err(flume::RecvError::Disconnected) => {
                                // Quit if channel closed.
                                return
                            },
                        };
                        if is_started {
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
    ) {
        executor
            .start_rollup_block_with_provided_state_roots(
                notify.data,
                notify.checkpoint,
                notify.state_roots,
            )
            .await;
    }
}
