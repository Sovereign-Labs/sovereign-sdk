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
mod primary;
mod replica;
pub mod rocksdb;
use anyhow::Result;
use axum::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_blob_sender::{new_blob_id, BlobInternalId};
use sov_blob_storage::{PreferredBatchData, SequenceNumber};
use sov_full_node_configs::sequencer::PostgresConfig;
use sov_modules_api::capabilities::BlobSelector;
use sov_modules_api::{
    FullyBakedTx, KernelStateAccessor, Runtime, Spec, StateCheckpoint, StateUpdateInfo, TxHash,
    VisibleSlotNumber,
};
use std::collections::VecDeque;
use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

use crate::common::WithCachedTxHashes;
use crate::preferred::db::primary::PrimarySequencerDb;
use crate::preferred::db::replica::ReplicaSequencerDb;
use crate::preferred::{exit_rollup, track_in_progress_batch_size};

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn begin_rollup_block(&mut self, stored_batch: BatchToStore) -> Result<()>;

    /// Calls to this method MUST be "sandwiched" between
    /// [`DbBackend::begin_rollup_block`] and
    /// [`DbBackend::end_rollup_block`].
    async fn add_tx(
        &mut self,
        sequence_number_of_in_progress_batch: SequenceNumber,
        tx_idx_within_batch: u64,
        tx: FullyBakedTx,
        hash: TxHash,
    ) -> Result<()>;

    async fn batch_add_txs(
        &mut self,
        sequence_number_of_in_progress_batch: SequenceNumber,
        mut tx_idx_within_batch: u64,
        txs: &[(FullyBakedTx, TxHash)],
    ) -> Result<()> {
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

    async fn end_rollup_block(&mut self, stored_batch: BatchToStore) -> Result<()>;

    async fn read_in_progress_batch(&self) -> Result<Option<InProgressBatch>>;

    /// Reads completed blobs, in-progress batch, and latest event_id.
    /// Bundling this as a single function allows the Postgres backend to do this atomically, which
    /// is necessary to support replica initialization in the presence of concurrent writes.
    async fn current_data(&self) -> Result<SnapshotData>;

    async fn add_proof_blob(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
    ) -> Result<()>;

    /// Instructs the database it MAY delete all data up to the given
    /// [`SequenceNumber`] (included).
    ///
    /// This method exists because the sequencer has no use for data that is
    /// already finalized.
    async fn prune(&mut self, up_to_including: SequenceNumber) -> Result<()>;
}

/// The return type of `DbBackend::current_data()`.
/// Primarily used to populate in-memory caches on initialization.
#[derive(Debug, Default, Clone)]
pub struct SnapshotData {
    pub completed_blobs: Vec<ReadBlob>,
    pub in_progress_batch: Option<InProgressBatch>,
}

/// See [`PreferredSequencerReadBlob::Batch`].
#[derive(Debug, Clone)]
pub struct ReadBatch<Txs = Arc<Vec<FullyBakedTx>>, TxHashes = Arc<Vec<TxHash>>> {
    pub sequence_number: SequenceNumber,
    pub visible_slot_number_after_increase: VisibleSlotNumber,
    pub visible_slots_to_advance: NonZero<u8>,
    pub blob_id: BlobInternalId,
    pub txs: Txs,
    pub tx_hashes: TxHashes,
}

pub type InProgressBatch = ReadBatch<Vec<FullyBakedTx>, Vec<TxHash>>;

impl From<InProgressBatch> for ReadBatch {
    fn from(batch: InProgressBatch) -> Self {
        ReadBatch {
            sequence_number: batch.sequence_number,
            visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
            visible_slots_to_advance: batch.visible_slots_to_advance,
            blob_id: batch.blob_id,
            txs: Arc::new(batch.txs),
            tx_hashes: batch.tx_hashes.into(),
        }
    }
}

impl ReadBatch {
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

/// See [`DbBackend::read_completed_blobs`].
#[derive(Debug, Clone)]
pub enum ReadBlob<Inner = ReadBatch> {
    Batch(Inner),
    Proof {
        blob_id: BlobInternalId,
        sequence_number: SequenceNumber,
        data: Arc<[u8]>,
    },
}

impl ReadBlob {
    pub fn sequence_number(&self) -> SequenceNumber {
        match &self {
            ReadBlob::Batch(batch) => batch.sequence_number,
            ReadBlob::Proof {
                sequence_number, ..
            } => *sequence_number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    TxAccepted(FullyBakedTx, TxHash),
    BatchStarted {
        sequence_number: SequenceNumber,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
    },
    BatchClosed(SequenceNumber),
    ProofBlobAccepted(SequenceNumber),
}

pub struct Cache {
    completed_blobs: VecDeque<ReadBlob>,
    in_progress_batch: Option<InProgressBatch>,
    event_stream: Option<mpsc::Sender<Event>>,
    shutdown_sender: watch::Sender<()>,
}

impl Cache {
    pub fn new(
        completed_blobs: VecDeque<ReadBlob>,
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

    pub fn all_completed_blobs(&self) -> Vec<ReadBlob> {
        self.completed_blobs.clone().into()
    }

    pub fn all_completed_blobs_greater_than_or_equal_to(
        &self,
        sequence_number: SequenceNumber,
    ) -> Vec<ReadBlob> {
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
        self.send_event_if_necessary(Event::TxAccepted(tx, hash))
            .await;
    }

    async fn send_event_if_necessary(&mut self, event: Event) {
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
        self.in_progress_batch = Some(ReadBatch {
            sequence_number,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            blob_id,
            txs: vec![],
            tx_hashes: vec![],
        });

        self.send_event_if_necessary(Event::BatchStarted {
            sequence_number,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        })
        .await;
        blob_id
    }

    pub fn clean_all_batches(&mut self) {
        self.completed_blobs.clear();
        self.in_progress_batch = None;
    }

    pub async fn insert_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        sequence_number: SequenceNumber,
    ) {
        self.completed_blobs.push_back(ReadBlob::Proof {
            blob_id,
            sequence_number,
            data,
        });
        self.send_event_if_necessary(Event::ProofBlobAccepted(sequence_number))
            .await;
    }

    pub async fn terminate_batch(&mut self) -> ReadBatch {
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

        let batch: ReadBatch = batch.into();

        self.completed_blobs
            .push_back(ReadBlob::Batch(batch.clone()));

        self.send_event_if_necessary(Event::BatchClosed(sequence_number))
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

    pub fn subscribe_to_events(&mut self, sender: mpsc::Sender<Event>) {
        self.event_stream = Some(sender);
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
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

/// High-level trait for preferred sequencer database operations.
/// Implemented differently for primary sequencers and replicas.
#[async_trait]
pub trait Db: Send + Sync {
    async fn initial_data(&self) -> Result<(SequenceNumber, Cache)>;

    async fn bulk_insert_txs(
        &mut self,
        txs: Vec<(FullyBakedTx, TxHash)>,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
    ) -> Result<()>;

    async fn start_batch(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
    ) -> Result<SequenceNumber>;

    async fn insert_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        sequence_number: SequenceNumber,
    ) -> Result<SequenceNumber>;

    async fn terminate_batch(&mut self, batch: BatchToStore) -> Result<()>;

    async fn prune_db(&mut self, prune_up_to_including: SequenceNumber) -> Result<()>;
}

/// Factory function to create the appropriate database implementation.
pub(crate) async fn create_preferred_sequencer_db(
    shutdown_sender: watch::Sender<()>,
    is_replica: bool,
    storage_path: &Path,
    postgres_config: &Option<PostgresConfig>,
) -> Result<Box<dyn Db>> {
    if is_replica {
        Ok(Box::new(ReplicaSequencerDb::new(shutdown_sender)))
    } else {
        Ok(Box::new(
            PrimarySequencerDb::new(shutdown_sender, storage_path, postgres_config).await?,
        ))
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
    let mut checkpoint =
        StateCheckpoint::new(latest_state_info.storage.clone(), &runtime.kernel(), None);
    let mut state = KernelStateAccessor::from_checkpoint(&runtime.kernel(), &mut checkpoint);
    state.read_from_storage_at_slot_number(latest_state_info.latest_finalized_slot_number);

    runtime
        .kernel()
        .next_sequence_number(&mut state)
        .checked_sub(1)
}
