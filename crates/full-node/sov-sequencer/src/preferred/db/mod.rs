//! Database for sequencer-related data.
//!
//! TODO(@neysofu): Remove *all* blocking code inside async functions.
//!
//! # About [`assert!`]
//!
//! Preferred sequencer logic is hard to reason about, hard to get right, and
//! most importantly business-critical. We strive to be intentional about
//! invariants and we'd rather have an application crash due to broken
//! invariants than to have bugs that result in subtle state inconsistencies.

pub mod postgres;
pub mod rocksdb;

use std::collections::VecDeque;
use std::marker::PhantomData;
use std::num::NonZero;
use std::sync::Arc;

use axum::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_blob_sender::{new_blob_id, BlobInternalId};
use sov_blob_storage::{PreferredBatchData, SequenceNumber};
use sov_modules_api::capabilities::BlobSelector;
use sov_modules_api::{
    FullyBakedTx, KernelStateAccessor, Runtime, Spec, StateCheckpoint, StateUpdateInfo, TxHash,
    VisibleSlotNumber,
};
use tokio::sync::{mpsc, watch};

use crate::common::WithCachedTxHashes;
use crate::preferred::{exit_rollup, track_in_progress_batch_size};

#[async_trait]
pub trait PreferredSequencerDbBackend: Send + Sync + 'static {
    async fn begin_rollup_block(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visibile_slots_to_advance: NonZero<u8>,
    ) -> anyhow::Result<()>;

    /// Calls to this method MUST be "sandwiched" between
    /// [`PreferredSequencerDbBackend::begin_rollup_block`] and
    /// [`PreferredSequencerDbBackend::end_rollup_block`].
    async fn add_tx(
        &mut self,
        sequence_number_of_in_progress_batch: SequenceNumber,
        tx_idx_within_batch: u64,
        tx: FullyBakedTx,
        hash: TxHash,
    ) -> anyhow::Result<()>;

    async fn batch_add_txs(
        &mut self,
        sequence_number_of_in_progress_batch: SequenceNumber,
        mut tx_idx_within_batch: u64,
        txs: &[(FullyBakedTx, TxHash)],
    ) -> anyhow::Result<()> {
        for (tx, hash) in txs {
            self.add_tx(
                sequence_number_of_in_progress_batch,
                tx_idx_within_batch,
                tx.clone(),
                *hash,
            )
            .await?;
            tx_idx_within_batch += 1;
        }
        Ok(())
    }

    async fn end_rollup_block(&mut self, stored_batch: BatchToStore) -> anyhow::Result<()>;

    async fn read_in_progress_batch(&self) -> anyhow::Result<Option<InProgressBatch>>;

    /// Reads completed blobs, in-progress batch, and latest event_id.
    /// Bundling this as a single function allows the Postgres backend to do this atomically, which
    /// is necessary to support replica initialization in the presence of concurrent writes.
    async fn current_data(&self) -> anyhow::Result<DbSnapshotData>;

    async fn add_proof_blob(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
    ) -> anyhow::Result<()>;

    /// Instructs the database it MAY delete all data up to the given
    /// [`SequenceNumber`] (included).
    ///
    /// This method exists because the sequencer has no use for data that is
    /// already finalized.
    async fn prune(&mut self, up_to_including: SequenceNumber) -> anyhow::Result<()>;
}

/// The return type of `PreferredSequencerDbBackend::current_data()`.
/// Primarily used to populate in-memory caches on initialization.
#[derive(Debug, Default, Clone)]
pub struct DbSnapshotData {
    pub completed_blobs: Vec<PreferredSequencerReadBlob>,
    pub in_progress_batch: Option<InProgressBatch>,
    pub latest_event_id: Option<u64>,
}

/// See [`PreferredSequencerReadBlob::Batch`].
#[derive(Debug, Clone)]
pub struct PreferredSequencerReadBatch<Txs = Arc<Vec<FullyBakedTx>>, TxHashes = Arc<Vec<TxHash>>> {
    pub sequence_number: SequenceNumber,
    pub visible_slot_number_after_increase: VisibleSlotNumber,
    pub visible_slots_to_advance: NonZero<u8>,
    pub blob_id: BlobInternalId,
    pub txs: Txs,
    pub tx_hashes: TxHashes,
}

pub type InProgressBatch = PreferredSequencerReadBatch<Vec<FullyBakedTx>, Vec<TxHash>>;

impl From<InProgressBatch> for PreferredSequencerReadBatch {
    fn from(batch: InProgressBatch) -> Self {
        PreferredSequencerReadBatch {
            sequence_number: batch.sequence_number,
            visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
            visible_slots_to_advance: batch.visible_slots_to_advance,
            blob_id: batch.blob_id,
            txs: Arc::new(batch.txs),
            tx_hashes: batch.tx_hashes.into(),
        }
    }
}

impl PreferredSequencerReadBatch {
    pub(crate) fn into_with_cached_tx_hashes(self) -> WithCachedTxHashes<PreferredBatchData> {
        WithCachedTxHashes {
            tx_hashes: self.tx_hashes.clone(),
            inner: PreferredBatchData {
                sequence_number: self.sequence_number,
                visible_slots_to_advance: self.visible_slots_to_advance,
                data: self.txs,
            },
        }
    }
}

/// See [`PreferredSequencerDbBackend::read_completed_blobs`].
#[derive(Debug, Clone)]
pub enum PreferredSequencerReadBlob<Inner = PreferredSequencerReadBatch> {
    Batch(Inner),
    Proof {
        blob_id: BlobInternalId,
        sequence_number: SequenceNumber,
        data: Arc<[u8]>,
    },
}

impl PreferredSequencerReadBlob {
    pub fn sequence_number(&self) -> SequenceNumber {
        match &self {
            PreferredSequencerReadBlob::Batch(batch) => batch.sequence_number,
            PreferredSequencerReadBlob::Proof {
                sequence_number, ..
            } => *sequence_number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DbEvent {
    TxAccepted(FullyBakedTx, TxHash),
    BatchStarted {
        sequence_number: SequenceNumber,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
    },
    BatchClosed(SequenceNumber),
    ProofBlobAccepted(SequenceNumber),
}

pub struct PreferredSequencerCache {
    completed_blobs: VecDeque<PreferredSequencerReadBlob>,
    in_progress_batch: Option<InProgressBatch>,
    event_stream: Option<mpsc::Sender<DbEvent>>,
    shutdown_sender: watch::Sender<()>,
}

impl PreferredSequencerCache {
    pub fn new(
        completed_blobs: VecDeque<PreferredSequencerReadBlob>,
        in_progress_batch: Option<InProgressBatch>,
        shutdown_sender: watch::Sender<()>,
    ) -> Self {
        Self {
            completed_blobs,
            in_progress_batch,
            event_stream: None,
            shutdown_sender,
        }
    }

    pub fn in_progress_batch_opt(&self) -> Option<&InProgressBatch> {
        self.in_progress_batch.as_ref()
    }

    pub fn all_completed_blobs(&self) -> Vec<PreferredSequencerReadBlob> {
        self.completed_blobs.clone().into()
    }

    pub fn all_completed_blobs_greater_than_or_equal_to(
        &self,
        sequence_number: SequenceNumber,
    ) -> Vec<PreferredSequencerReadBlob> {
        self.completed_blobs
            .iter()
            .filter(|b| {
                // Pruning invariants say it MAY remove older blobs, but we don't know for sure.
                b.sequence_number() >= sequence_number
            })
            .cloned()
            .collect()
    }

    pub async fn insert_tx(&mut self, tx: FullyBakedTx, hash: TxHash) {
        let Some(batch) = self.in_progress_batch.as_mut() else {
            tracing::error!("No in-progress batch; this is a bug, please report it");
            exit_rollup(&self.shutdown_sender).await;
            unreachable!();
        };
        batch.txs.push(tx.clone());
        batch.tx_hashes.push(hash);
        // If there are no receivers, we don't send the tx. This is as it should be.
        self.send_event_if_necessary(DbEvent::TxAccepted(tx, hash))
            .await;
    }

    async fn send_event_if_necessary(&mut self, event: DbEvent) {
        let Some(open_stream) = &self.event_stream else {
            return;
        };

        match open_stream.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(event)) => {
                // If the operation would block, print a warning before blocking.
                tracing::warn!("DbEvent stream is full, accepting txs is temporarily blocked; this means that `update_state` is taking too long to catch up causing the channel to become full. Consider bumping the db event channel size.");
                let res = open_stream.send(event).await;
                // If the receiver was dropped, we don't need to send events anymore.
                tracing::info!(
                    max_capacity = open_stream.max_capacity(),
                    remaining_capacity = open_stream.capacity(),
                    "The event stream is no longer full. accepting txs is unblocked"
                );
                if res.is_err() {
                    self.event_stream = None;
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // If the receiver was dropped, we don't need to send events anymore.
                self.event_stream = None;
            }
        }
    }

    #[must_use]
    pub async fn start_batch(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
        sequence_number: SequenceNumber,
    ) -> BlobInternalId {
        if self.in_progress_batch.is_some() {
            tracing::error!(
                "There's already an in-progress batch; this is a bug, please report it"
            );
            exit_rollup(&self.shutdown_sender).await;
        };
        let blob_id = new_blob_id();
        self.in_progress_batch = Some(PreferredSequencerReadBatch {
            sequence_number,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            blob_id,
            txs: vec![],
            tx_hashes: vec![],
        });

        self.send_event_if_necessary(DbEvent::BatchStarted {
            sequence_number,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        })
        .await;
        blob_id
    }

    pub async fn insert_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        sequence_number: SequenceNumber,
    ) {
        self.completed_blobs
            .push_back(PreferredSequencerReadBlob::Proof {
                blob_id,
                sequence_number,
                data,
            });
        self.send_event_if_necessary(DbEvent::ProofBlobAccepted(sequence_number))
            .await;
    }

    pub async fn terminate_batch(&mut self) -> PreferredSequencerReadBatch {
        let Some(in_progress_batch) = self.in_progress_batch.as_ref() else {
            tracing::error!("No in-progress batch; this is a bug, please report it");
            exit_rollup(&self.shutdown_sender).await;
            unreachable!();
        };

        let sequence_number = in_progress_batch.sequence_number;
        let Some(batch) = self.in_progress_batch.take() else {
            tracing::error!("No in-progress batch; this is a bug, please report it");
            exit_rollup(&self.shutdown_sender).await;
            unreachable!();
        };

        let batch: PreferredSequencerReadBatch = batch.into();

        self.completed_blobs
            .push_back(PreferredSequencerReadBlob::Batch(batch.clone()));

        self.send_event_if_necessary(DbEvent::BatchClosed(sequence_number))
            .await;

        // Update the metrics.
        track_in_progress_batch_size(
            self.in_progress_batch_opt()
                .map(|b| b.txs.len() as u64)
                .unwrap_or(0),
        );

        batch
    }

    pub async fn prune(&mut self, prune_up_to_including: SequenceNumber) {
        // We could also do binary search, but this seems fast enough.
        while let Some(blob) = self.completed_blobs.front() {
            if blob.sequence_number() > prune_up_to_including {
                break;
            }

            self.completed_blobs.pop_front();
        }
    }

    pub fn subscribe_to_events(&mut self, sender: mpsc::Sender<DbEvent>) {
        self.event_stream = Some(sender);
    }
}
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub(crate) struct BatchToStore {
    pub blob_id: BlobInternalId,
    pub sequence_number: SequenceNumber,
    pub visible_slot_number_after_increase: VisibleSlotNumber,
    pub visible_slots_to_advance: NonZero<u8>,
}

impl From<BatchToStore> for StoredBlob {
    fn from(batch: BatchToStore) -> Self {
        StoredBlob::Batch {
            blob_id: batch.blob_id,
            visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
            visible_slots_to_advance: batch.visible_slots_to_advance,
        }
    }
}

pub struct PreferredSequencerDb<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    backend: Box<dyn PreferredSequencerDbBackend>,
    phantom: PhantomData<S>,
    is_replica: bool,
    shutdown_sender: watch::Sender<()>,
    phantom_runtime: PhantomData<Rt>,
}

impl<S, Rt> PreferredSequencerDb<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    /// Returns the constructed PreferredSequencerDb, and the latest EventID observed during
    /// construction, for backends that allow atomic initialization (i.e. postgres).
    pub async fn new(
        backend: Box<dyn PreferredSequencerDbBackend>,
        shutdown_sender: watch::Sender<()>,
        is_replica: bool,
    ) -> anyhow::Result<(Self, Option<u64>, SequenceNumber, PreferredSequencerCache)> {
        let DbSnapshotData {
            completed_blobs,
            in_progress_batch,
            latest_event_id,
        } = backend.current_data().await?;
        let completed_blobs = VecDeque::from(completed_blobs);

        let sequence_number_of_next_blob = match (completed_blobs.back(), &in_progress_batch) {
            (Some(blob), None) => blob.sequence_number() + 1,
            (None, Some(batch)) => batch.sequence_number + 1,
            (Some(blob), Some(batch)) => {
                std::cmp::max(blob.sequence_number(), batch.sequence_number) + 1
            }
            (None, None) => 0,
        };

        Ok((
            Self {
                backend,
                phantom: PhantomData,
                is_replica,
                shutdown_sender: shutdown_sender.clone(),
                phantom_runtime: PhantomData,
            },
            latest_event_id,
            sequence_number_of_next_blob,
            PreferredSequencerCache::new(completed_blobs, in_progress_batch, shutdown_sender),
        ))
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub async fn bulk_insert_txs(
        &mut self,
        txs: Vec<(FullyBakedTx, TxHash)>,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
    ) -> anyhow::Result<()> {
        if !self.is_replica {
            self.backend
                .batch_add_txs(sequence_number, tx_idx_within_batch, &txs)
                .await?;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub async fn insert_tx(
        &mut self,
        tx: FullyBakedTx,
        hash: TxHash,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
    ) -> anyhow::Result<()> {
        if !self.is_replica {
            self.backend
                .add_tx(sequence_number, tx_idx_within_batch, tx.clone(), hash)
                .await?;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub async fn start_batch(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
    ) -> anyhow::Result<SequenceNumber> {
        if !self.is_replica {
            self.debug_assert_in_progress_batch_is_none(
                "Cached in-progress batch state (None) didn't match backend db state",
            )
            .await;
        }

        tracing::debug!(
            sequence_number,
            blob_id,
            %visible_slot_number_after_increase,
            visible_slots_to_advance,
            "Storing new rollup block"
        );

        if !self.is_replica {
            self.backend
                .begin_rollup_block(
                    sequence_number,
                    blob_id,
                    visible_slot_number_after_increase,
                    visible_slots_to_advance,
                )
                .await?;
        }

        Ok(sequence_number)
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub async fn insert_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        sequence_number: SequenceNumber,
    ) -> anyhow::Result<SequenceNumber> {
        if !self.is_replica {
            self.backend
                .add_proof_blob(sequence_number, blob_id, data.clone())
                .await?;
        }

        Ok(sequence_number)
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub async fn terminate_batch(&mut self, batch: BatchToStore) -> anyhow::Result<()> {
        if !self.is_replica {
            self.backend.end_rollup_block(batch).await?;
            self.debug_assert_in_progress_batch_is_none(
                "Backend didn't remove in-progress batch from database when ending rollup block",
            )
            .await;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub(super) async fn prune(
        &mut self,
        prune_up_to_including: SequenceNumber,
    ) -> anyhow::Result<()> {
        if !self.is_replica {
            self.backend.prune(prune_up_to_including).await?;
        }
        Ok(())
    }

    async fn debug_assert_in_progress_batch_is_none(&self, msg: &str) {
        if cfg!(debug_assertions) {
            match self.backend.read_in_progress_batch().await {
                Ok(None) => {}
                other => {
                    tracing::error!("{msg}: {other:?}");
                    exit_rollup(&self.shutdown_sender).await;
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum StoredBlob {
    Batch {
        blob_id: BlobInternalId,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
    },
    Proof {
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
    },
}

pub(crate) fn latest_finalized_sequence_number<S, Rt>(
    latest_state_info: &StateUpdateInfo<S::Storage>,
    runtime: &mut Rt,
) -> Option<SequenceNumber>
where
    S: Spec,
    Rt: Runtime<S>,
{
    let mut checkpoint = StateCheckpoint::new(latest_state_info.storage.clone(), &runtime.kernel());
    let mut state = KernelStateAccessor::from_checkpoint(&runtime.kernel(), &mut checkpoint);

    state.update_true_slot_number(latest_state_info.latest_finalized_slot_number);
    runtime
        .kernel()
        .next_sequence_number(&mut state)
        .checked_sub(1)
}
