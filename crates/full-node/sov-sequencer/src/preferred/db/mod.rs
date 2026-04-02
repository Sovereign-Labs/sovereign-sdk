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

pub mod heartbeat_task;
pub mod postgres;
pub mod rocksdb;
use crate::preferred::PostgresBackend;
use crate::preferred::PreferredProofToReplay;
use crate::preferred::RocksDbBackend;
use crate::PreferredProofDataBytes;
use anyhow::Result;
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_blob_sender::{new_blob_id, BlobInternalId};
use sov_blob_storage::{PreferredBatchData, SequenceNumber};
use sov_full_node_configs::sequencer::ConfiguredNodeRole;
use sov_full_node_configs::sequencer::PostgresConfig;
use sov_modules_api::capabilities::BlobSelector;
use sov_modules_api::{
    FullyBakedTx, KernelStateAccessor, Runtime, Spec, StateCheckpoint, StateUpdateInfo, TxHash,
    VisibleSlotNumber,
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

use crate::common::WithCachedTxHashes;
use crate::preferred::{exit_rollup, track_in_progress_batch_size};

#[derive(Debug)]
pub(crate) enum DbReadOutcome<T> {
    Success(T),
    AbortedBecauseReplica { db_leader: Option<String> },
}

#[derive(Debug, PartialEq, strum::Display)]
pub(crate) enum FailedOperation {
    BeginBlock,
    BatchAddTxs,
    AddTx,
    EndBlock,
    Prune { db_leader: Option<String> },
    ReadBatch,
    AddProof,
    CurrentData { db_leader: Option<String> },
}

#[derive(Debug)]
pub(crate) enum DbError {
    Database(anyhow::Error),
    ReplicaDisallowed {
        self_node_id: String,
        operation: FailedOperation,
    },
}

impl<T: Into<anyhow::Error>> From<T> for DbError {
    fn from(error: T) -> Self {
        DbError::Database(error.into())
    }
}

// Don’t prune the data from the database immediately — give the replica some time to read it before it is pruned.
const PRUNING_LAG: u64 = 10;

#[async_trait]
pub trait DbBackend: Send + Sync + 'static {
    async fn begin_rollup_block(&mut self, stored_batch: BatchToStore) -> Result<(), DbError>;

    /// Calls to this method MUST be "sandwiched" between
    /// [`DbBackend::begin_rollup_block`] and
    /// [`DbBackend::end_rollup_block`].
    async fn add_tx(
        &mut self,
        sequence_number_of_in_progress_batch: SequenceNumber,
        tx_idx_within_batch: u64,
        tx: FullyBakedTx,
        hash: TxHash,
    ) -> anyhow::Result<(), DbError>;

    async fn batch_add_txs(
        &mut self,
        sequence_number_of_in_progress_batch: SequenceNumber,
        mut tx_idx_within_batch: u64,
        txs: &[(FullyBakedTx, TxHash)],
    ) -> anyhow::Result<(), DbError> {
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

    async fn end_rollup_block(&mut self, stored_batch: BatchToStore) -> Result<(), DbError>;

    async fn read_in_progress_batch(&self) -> Result<Option<InProgressBatch>, DbError>;

    /// Reads completed blobs, in-progress batch, and latest event_id.
    /// Bundling this as a single function allows the Postgres backend to do this atomically, which
    /// is necessary to support replica initialization in the presence of concurrent writes.
    async fn current_data(&self) -> Result<SnapshotData, DbError>;

    async fn add_proof_blob(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        data: PreferredProofDataBytes,
    ) -> Result<(), DbError>;

    /// Instructs the database it MAY delete all data up to the given
    /// [`SequenceNumber`] (included).
    ///
    /// This method exists because the sequencer has no use for data that is
    /// already finalized.
    async fn prune(&mut self, up_to_including: SequenceNumber) -> Result<(), DbError>;
}

/// The return type of `DbBackend::current_data()`.
/// Primarily used to populate in-memory caches on initialization.
#[derive(Debug, Default, Clone)]
pub struct SnapshotData {
    pub completed_blobs: Vec<ReadBlob>,
    pub in_progress_batch: Option<InProgressBatch>,
}

impl SnapshotData {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.completed_blobs.is_empty() && self.in_progress_batch.is_none()
    }
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
        data: PreferredProofDataBytes,
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
pub(crate) enum DbEvent {
    TxAccepted(FullyBakedTx, TxHash),
    BatchStarted {
        sequence_number: SequenceNumber,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
    },
    BatchClosed(SequenceNumber),
    ProofBlobAccepted {
        sequence_number: SequenceNumber,
        proof_bytes: PreferredProofDataBytes,
    },
}

pub struct BlobsCache {
    proofs_and_completed_batches: BTreeMap<SequenceNumber, ReadBlob>,
    in_progress_batch: Option<InProgressBatch>,
    event_stream: Option<mpsc::Sender<DbEvent>>,
    shutdown_sender: watch::Sender<()>,
}

impl BlobsCache {
    pub fn new(
        completed_blobs: BTreeMap<SequenceNumber, ReadBlob>,
        in_progress_batch: Option<InProgressBatch>,
        shutdown_sender: watch::Sender<()>,
    ) -> Self {
        Self {
            proofs_and_completed_batches: completed_blobs,
            in_progress_batch,
            event_stream: None,
            shutdown_sender,
        }
    }

    pub fn in_progress_batch_opt(&self) -> Option<&InProgressBatch> {
        self.in_progress_batch.as_ref()
    }

    pub fn all_proofs_and_completed_blobs(&self) -> Vec<ReadBlob> {
        self.proofs_and_completed_batches
            .values()
            .cloned()
            .collect()
    }

    // Ensure that the provided batch number is wihin the range of sequence numbers that might plausibly be needed for replay.
    fn sanity_check_batch_sequence_number_is_in_range(
        &self,
        batch_sequence_number: SequenceNumber,
    ) {
        // The highest allowed sequence number is either the in-progress batch sequence number (if one exists) or the next sequence number (if no batch is in progress).
        // If no batch is in progress *and* we don't have any completed blobs in cache, we don't know what the next sequence number should be so we allow any value.
        let highest_allowed_sequence_number = self
            .in_progress_batch
            .as_ref()
            .map(|b| b.sequence_number)
            .unwrap_or_else(|| {
                self.proofs_and_completed_batches
                    .keys()
                    .next_back()
                    .map(|k| k.saturating_add(1))
                    .unwrap_or(u64::MAX)
            });

        // The lowest allowed sequence number is either the sequence number of the first completed blob (if one exists) or the sequence number of the in-progress batch (if one exists).
        // If no batch is in progress *and* we don't have any completed blobs in cache, we don't know what the next sequence number should be so we allow any value.
        let lowest_allowed_sequence_number = self
            .proofs_and_completed_batches
            .keys()
            .cloned()
            .next()
            .unwrap_or_else(|| {
                self.in_progress_batch
                    .as_ref()
                    .map(|b| b.sequence_number)
                    .unwrap_or(0)
            });

        assert!(batch_sequence_number <= highest_allowed_sequence_number, "The requested batch sequence number {batch_sequence_number} is greater than the highest allowed sequence number {highest_allowed_sequence_number}. This is a bug, please report it.");
        assert!(batch_sequence_number >= lowest_allowed_sequence_number, "The requested batch sequence number {batch_sequence_number} is less than the lowest allowed sequence number {lowest_allowed_sequence_number}. This is a bug, please report it.");
    }

    /// Fetch all proofs that need to be played at the start of the batch with the given sequence number.
    pub fn proofs_for_replay(
        &self,
        target_batch_sequence_number: SequenceNumber,
    ) -> Vec<PreferredProofToReplay> {
        self.sanity_check_batch_sequence_number_is_in_range(target_batch_sequence_number);

        let mut output = Vec::new();
        // Given the sequence number of a batch, we want to return all proofs with sequence numbers between the previous batch and the requested batch.
        for blob in self.proofs_and_completed_batches.values() {
            match blob {
                ReadBlob::Batch(batch) => {
                    // Since we're looking for all proofs in between two batches, we either need to return (if the batch has the sequence number we're looking for)
                    // or we need to reset our output (since the proofs we've already seen would have been handled during processing of the batch we just hit).
                    if batch.sequence_number == target_batch_sequence_number {
                        break;
                    }
                    output.retain(|p: &PreferredProofToReplay| {
                        p.sequence_number > batch.sequence_number
                    });
                }
                ReadBlob::Proof {
                    sequence_number,
                    data,
                    ..
                } => {
                    // We might have some proofs in cache that come *after* the in-progress batch
                    // If we've passed the requested batch number, we're done.
                    if *sequence_number > target_batch_sequence_number {
                        break;
                    }
                    assert_ne!(*sequence_number, target_batch_sequence_number, "A proof sequence number {sequence_number} was provided to proof_for_replay, which must have a batch sequence number. This is a bug, please report it.");
                    // It it's a proof, just push it to our current output.
                    output.push(PreferredProofToReplay {
                        sequence_number: *sequence_number,
                        data: data.clone(),
                    });
                }
            }
        }
        assert!(
            output
                .last()
                .map(|p| p.sequence_number == target_batch_sequence_number.saturating_sub(1))
                .unwrap_or(true),
            "The last proof in the list should be the one before the sequence number"
        );
        output
    }

    pub fn all_proofs_and_completed_batches_greater_than_or_equal_to(
        &self,
        sequence_number: SequenceNumber,
    ) -> Vec<ReadBlob> {
        self.proofs_and_completed_batches
            .values()
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
        self.in_progress_batch = Some(ReadBatch {
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

    pub fn clean_all_batches(&mut self) {
        self.proofs_and_completed_batches.clear();
        self.in_progress_batch = None;
    }

    pub async fn insert_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: PreferredProofDataBytes,
        sequence_number: SequenceNumber,
    ) {
        self.proofs_and_completed_batches.insert(
            sequence_number,
            ReadBlob::Proof {
                blob_id,
                sequence_number,
                data: data.clone(),
            },
        );

        self.send_event_if_necessary(DbEvent::ProofBlobAccepted {
            sequence_number,
            proof_bytes: data,
        })
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

        self.proofs_and_completed_batches
            .insert(sequence_number, ReadBlob::Batch(batch.clone()));

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
        self.proofs_and_completed_batches
            .retain(|sequence_number, _| *sequence_number > prune_up_to_including);
    }

    pub fn subscribe_to_events(&mut self, sender: mpsc::Sender<DbEvent>) {
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

/// The role of the sequencer in a distributed setup.
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SequencerRole {
    /// Node that does not sync with the `BatchProducer` and relies on DA for updates.
    DaOnlyReplica,
    /// Node that syncs with the `BatchProducer` via PostgreSQL.
    PgSyncReplica,
    /// Node that accepts transactions and produces batches.
    BatchProducer,
}

pub struct PreferredSequencerDb {
    backend: Option<Box<dyn DbBackend>>,
    shutdown_sender: watch::Sender<()>,
}

impl PreferredSequencerDb {
    pub(crate) async fn new(
        shutdown_sender: watch::Sender<()>,
        storage_path: &Path,
        postgres_config: &Option<PostgresConfig>,
        bind_addr: SocketAddr,
    ) -> anyhow::Result<(Self, SequencerRole)> {
        let (backend, role): (Option<Box<dyn DbBackend>>, _) = {
            if let Some(postgres_config) = &postgres_config {
                match postgres_config.node_role {
                    ConfiguredNodeRole::ReplicaNoLeaderSync => (None, SequencerRole::DaOnlyReplica),
                    ConfiguredNodeRole::Replica => (None, SequencerRole::PgSyncReplica),
                    ConfiguredNodeRole::Leader => {
                        let backend = PostgresBackend::connect(postgres_config, bind_addr).await?;
                        let _ = backend
                            .heartbeat(Some(postgres_config.leader_election))
                            .await?;

                        (Some(Box::new(backend)), SequencerRole::BatchProducer)
                    }
                    ConfiguredNodeRole::DbElected => {
                        let backend = PostgresBackend::connect(postgres_config, bind_addr).await?;
                        let maybe_leader = backend
                            .heartbeat(Some(postgres_config.leader_election))
                            .await?;

                        let is_leader = maybe_leader
                            .map(|leader| leader.node_id == postgres_config.node_id)
                            .unwrap_or(false);

                        if is_leader {
                            tracing::info!(
                                node_id = %postgres_config.node_id,
                                "DbElected node acquired leadership, running as BatchProducer"
                            );
                            (Some(Box::new(backend)), SequencerRole::BatchProducer)
                        } else {
                            tracing::info!(
                                node_id = %postgres_config.node_id,
                                "DbElected node did not acquire leadership, running as PgSyncReplica"
                            );
                            (None, SequencerRole::PgSyncReplica)
                        }
                    }
                }
            } else {
                (
                    Some(Box::new(RocksDbBackend::new(storage_path).await?)),
                    SequencerRole::BatchProducer,
                )
            }
        };
        Ok((
            Self {
                backend,
                shutdown_sender: shutdown_sender.clone(),
            },
            role,
        ))
    }

    pub(crate) async fn initial_data(&self) -> Result<(SequenceNumber, BlobsCache)> {
        if let Some(backend) = &self.backend {
            match backend.current_data().await {
                Ok(SnapshotData {
                    completed_blobs,
                    in_progress_batch,
                }) => {
                    let completed_blobs: BTreeMap<u64, ReadBlob> = completed_blobs
                        .into_iter()
                        .map(|blob| (blob.sequence_number(), blob))
                        .collect();

                    let sequence_number_of_next_blob =
                        match (completed_blobs.keys().next_back(), &in_progress_batch) {
                            (Some(sequence_number), None) => sequence_number + 1,
                            (None, Some(batch)) => batch.sequence_number + 1,
                            (Some(sequence_number), Some(batch)) => {
                                std::cmp::max(*sequence_number, batch.sequence_number) + 1
                            }
                            (None, None) => 0,
                        };

                    Ok((
                        sequence_number_of_next_blob,
                        BlobsCache::new(
                            completed_blobs,
                            in_progress_batch,
                            self.shutdown_sender.clone(),
                        ),
                    ))
                }
                Err(DbError::Database(err)) => Err(err),
                Err(DbError::ReplicaDisallowed {
                    self_node_id,
                    operation,
                }) => {
                    tracing::error!(
                        %self_node_id,
                        %operation,
                        "The primary has become a replica. Shutting down.",
                    );
                    exit_rollup(&self.shutdown_sender).await;
                    unreachable!()
                }
            }
        } else {
            Ok((
                0, // TODO this will be revisited when we enable the replica sync task.
                BlobsCache::new(
                    Default::default(),
                    Option::None,
                    self.shutdown_sender.clone(),
                ),
            ))
        }
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub(crate) async fn bulk_insert_txs(
        &mut self,
        txs: Vec<(FullyBakedTx, TxHash)>,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
    ) -> Result<()> {
        if let Some(backend) = &mut self.backend {
            if let Err(err) = backend
                .batch_add_txs(sequence_number, tx_idx_within_batch, &txs)
                .await
            {
                return Err(self.check_replica_err(err).await);
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub(crate) async fn start_batch(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
    ) -> Result<()> {
        if let Some(backend) = &mut self.backend {
            Self::debug_assert_in_progress_batch_is_none(
                "Cached in-progress batch state (None) didn't match backend db state",
                backend,
                &self.shutdown_sender,
            )
            .await;

            tracing::debug!(
                sequence_number,
                blob_id,
                %visible_slot_number_after_increase,
                visible_slots_to_advance,
                "Storing new rollup block"
            );

            let batch_to_store = BatchToStore {
                sequence_number,
                blob_id,
                visible_slot_number_after_increase,
                visible_slots_to_advance,
            };

            if let Err(err) = backend.begin_rollup_block(batch_to_store).await {
                return Err(self.check_replica_err(err).await);
            }
        }

        Ok(())
    }

    async fn check_replica_err(&self, err: DbError) -> anyhow::Error {
        match err {
            DbError::Database(err) => err,
            DbError::ReplicaDisallowed {
                self_node_id,
                operation,
            } => {
                tracing::error!(
                    %self_node_id,
                    %operation,
                    "The primary has become a replica. Shutting down.",
                );
                exit_rollup(&self.shutdown_sender).await;
                unreachable!()
            }
        }
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub(crate) async fn insert_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: PreferredProofDataBytes,
        sequence_number: SequenceNumber,
    ) -> Result<()> {
        if let Some(backend) = &mut self.backend {
            if let Err(err) = backend.add_proof_blob(sequence_number, blob_id, data).await {
                return Err(self.check_replica_err(err).await);
            };
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub(crate) async fn terminate_batch(&mut self, batch: BatchToStore) -> Result<()> {
        if let Some(backend) = &mut self.backend {
            if let Err(err) = backend.end_rollup_block(batch).await {
                return Err(self.check_replica_err(err).await);
            };
            Self::debug_assert_in_progress_batch_is_none(
                "Backend didn't remove in-progress batch from database when ending rollup block",
                backend,
                &self.shutdown_sender,
            )
            .await;
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    pub(super) async fn prune_db(&mut self, prune_up_to_including: SequenceNumber) -> Result<()> {
        if let Some(backend) = &mut self.backend {
            if let Some(prune_up_to_including) = prune_up_to_including.checked_sub(PRUNING_LAG) {
                if let Err(err) = backend.prune(prune_up_to_including).await {
                    return Err(self.check_replica_err(err).await);
                }
            }
        }
        Ok(())
    }

    async fn debug_assert_in_progress_batch_is_none(
        msg: &str,
        backend: &mut Box<dyn DbBackend>,
        shutdown_sender: &watch::Sender<()>,
    ) {
        if cfg!(debug_assertions) {
            match backend.read_in_progress_batch().await {
                Ok(_) => {}
                other => {
                    tracing::error!("{msg}: {other:?}");
                    exit_rollup(shutdown_sender).await;
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
    let mut checkpoint =
        StateCheckpoint::new(latest_state_info.storage.clone(), &runtime.kernel(), None);
    let mut state = KernelStateAccessor::from_checkpoint(&runtime.kernel(), &mut checkpoint);
    state.read_from_storage_at_slot_number(latest_state_info.latest_finalized_slot_number);

    runtime
        .kernel()
        .next_sequence_number(&mut state)
        .checked_sub(1)
}
