use crate::preferred::block_executor::RollupBlockExecutor;
use crate::preferred::db::BatchToStore;
use crate::preferred::rate_limiter::IpAndCredentialId;
use crate::preferred::replica::event_handler::ReplicaError;
use crate::preferred::sync_sequencer_state::Message;
use crate::preferred::AcceptTxError;
use crate::preferred::AcceptedTx;
use crate::preferred::Confirmation;
use crate::preferred::DbEvent;
use crate::preferred::FetchBatches;
use crate::preferred::PreferredSeqOperation;
use crate::preferred::ProcessFinalCatchupData;
use crate::{SequencerNotReadyDetails, TxHash};
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{FullyBakedTx, Runtime, Spec, StateUpdateInfo};
use sov_state::Storage;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{error, info};

pub(crate) struct SequencerStateUpdator<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub(crate) channel_size: Arc<AtomicU32>,
    pub(crate) message_sender: mpsc::Sender<Message<S, Rt>>,
    pub(crate) shutdown_receiver: watch::Receiver<()>,
}

#[derive(Debug)]
/// Describes errors that can occur when updating the sequencer state.
/// This type intentionally does *not* implement `std::error::Error` or `std::fmt::Debug` so that it cannot be directly converted to an `anyhow::Error`.
/// To convert to anyhow, first convert to a `StateUpdateError` or similar. This is done to ensure backward compatibility with existing code that
/// creates a local error type to represent shutdown and then attempts anyhow errors into that type to distinguish between shutdown and unexpected errors.
pub(crate) enum SequencerStateUpdatorError {
    Shutdown,
    Unexpected,
}

impl SequencerStateUpdatorError {
    /// Converts a [`SequencerStateUpdatorError`] into an [`anyhow::Error`] that can downcast into the correct "StateUpdateError::Shutdown" error type.
    /// This allows `UpdateState` to distinguish between graceful shutdowns and unexpected errors.
    pub fn into_state_update_error(self) -> anyhow::Error {
        match self {
            SequencerStateUpdatorError::Shutdown => crate::preferred::StateUpdateError::Shutdown.into(),
            SequencerStateUpdatorError::Unexpected => anyhow::anyhow!("The sequencer experienced an unexpected error and cannot accept transactions! See logs for more details."),
        }
    }
}

impl<S, Rt> SequencerStateUpdator<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub(crate) async fn next_sequence_number_msg(
        &self,
        reason: &'static str,
    ) -> Result<SequenceNumber, SequencerStateUpdatorError> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::NextSequenceNumber { resp, reason })
            .await?;

        self.recv(recv).await
    }

    pub(crate) async fn fetch_completed_batches_msg(
        &self,
        next_sequence_number: u64,
        reason: &'static str,
    ) -> Result<(FetchBatches, Duration), SequencerStateUpdatorError> {
        let start_time = std::time::Instant::now();
        let (resp, recv) = oneshot::channel();
        self.send(Message::FetchCompletedBatches {
            resp,
            next_sequence_number,
            reason,
        })
        .await?;

        Ok((self.recv(recv).await?, start_time.elapsed()))
    }

    pub(crate) async fn sequencer_conditions_msg(
        &self,
        info: &StateUpdateInfo<S::Storage>,
        next_sequence_number_according_to_node: u64,
        reason: &'static str,
    ) -> Result<PreferredSeqOperation<S, Rt>, SequencerStateUpdatorError> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::SequencerConditions {
            resp,
            info: info.clone(),
            next_sequence_number_according_to_node,
            reason,
        })
        .await?;

        self.recv(recv).await
    }

    pub(crate) async fn check_readiness_msg(
        &self,
        max_concurrent_blobs: usize,
        height_to_stop_at: Option<RollupHeight>,
        reason: &'static str,
    ) -> Result<Result<(), SequencerNotReadyDetails>, SequencerStateUpdatorError> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::CheckReadiness {
            resp,
            max_concurrent_blobs,
            height_to_stop_at,
            reason,
        })
        .await?;

        self.recv(recv).await
    }

    pub(crate) async fn accept_tx_msg(
        &self,
        baked_tx: &FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        ip_and_credential: IpAndCredentialId<S::Address>,
        reason: &'static str,
    ) -> Result<
        Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>,
        SequencerStateUpdatorError,
    > {
        let (resp, recv) = oneshot::channel();
        self.send(Message::AcceptTx {
            resp,
            baked_tx: baked_tx.clone(),
            tx_hash,
            original_tx_queue_id,
            ip_and_credential,
            reason,
        })
        .await?;

        self.recv(recv).await
    }

    pub(crate) async fn final_catchup_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
        db_event_subscription: mpsc::Receiver<DbEvent>,
        executor: Box<RollupBlockExecutor<S, Rt>>,
        node_state_root: <S::Storage as Storage>::Root,
        data: ProcessFinalCatchupData,
        reason: &'static str,
    ) -> Result<(anyhow::Result<ProcessFinalCatchupData>, Duration), SequencerStateUpdatorError>
    {
        let start_time = std::time::Instant::now();
        let (resp, recv) = oneshot::channel();
        self.send(Message::FinalCatchup {
            resp,
            info,
            db_event_subscription,
            executor,
            node_state_root,
            data,
            reason,
        })
        .await?;

        Ok((self.recv(recv).await?, start_time.elapsed()))
    }

    pub(crate) async fn prune_sequencer_db_msg(
        &self,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::PruneSequencerDb { reason }).await
    }

    pub(crate) async fn force_overite_state_for_recovery_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::ForceOverwriteStateForRecovery { info, reason })
            .await
    }

    pub(crate) async fn wait_for_node_resync_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
        distance: u64,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::WaitNodeResync {
            info,
            distance,
            reason,
        })
        .await
    }

    /// Closes the current batch
    #[cfg(feature = "test-utils")]
    pub(crate) async fn force_close_current_batch_msg(
        &self,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::ForceCloseCurrentBatch { reason }).await
    }

    pub(crate) async fn proof_blob_msg(
        &self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::ProofBlob {
            blob_id,
            data,
            reason,
        })
        .await
    }

    pub(crate) async fn trigger_batch_production_if_convenient_msg(
        &self,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::TriggerBatchProductionIfConvenient { reason })
            .await
    }

    pub(crate) async fn send_simple_state_update_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::SimpleStateUpdate { info }).await
    }

    async fn send(&self, message: Message<S, Rt>) -> Result<(), SequencerStateUpdatorError> {
        self.channel_size.fetch_add(1, Ordering::Relaxed);
        if self.message_sender.send(message).await.is_err() {
            if self.shutdown_receiver.has_changed().unwrap_or(true) {
                info!("SynchronizedSequencerState(send) task exited, this is ok since the sequencer is shutting down.");
                return Err(SequencerStateUpdatorError::Shutdown);
            }
            return Err(SequencerStateUpdatorError::Unexpected);
        }
        Ok(())
    }

    async fn recv<T>(&self, recv: oneshot::Receiver<T>) -> Result<T, SequencerStateUpdatorError> {
        if let Ok(ret) = recv.await {
            Ok(ret)
        } else {
            if self.shutdown_receiver.has_changed().unwrap_or(true) {
                info!("SynchronizedSequencerState(recv) task exited, this is ok since the sequencer is shutting down.");
                return Err(SequencerStateUpdatorError::Shutdown);
            }
            error!("SynchronizedSequencerState(recv) task has shut down unexpectedly.");
            Err(SequencerStateUpdatorError::Unexpected)
        }
    }

    pub(crate) async fn do_batch_start_msg_replica(
        &self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::ReplicaBatchStartMsg {
            resp,
            batch_from_master,
            reason,
        })
        .await?;

        self.recv(recv).await??;
        Ok(())
    }

    pub(crate) async fn do_new_tx_msg_replica(
        &self,
        seq_nr_from_master: u64,
        tx_hash: TxHash,
        baked_tx: FullyBakedTx,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::ReplicaNewTx {
            seq_nr_from_master,
            resp,
            tx_hash,
            baked_tx,
            reason,
        })
        .await?;

        self.recv(recv).await??;
        Ok(())
    }

    pub(crate) async fn close_current_batch_msg_replica(
        &self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::ReplicaCloseCurrentBatch {
            resp,
            batch_from_master,
            reason,
        })
        .await?;
        self.recv(recv).await??;
        Ok(())
    }
}
