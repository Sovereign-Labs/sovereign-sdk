use std::{collections::VecDeque, num::NonZero, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::{FullyBakedTx, TxHash, VisibleSlotNumber};
use tokio::sync::watch;

use crate::preferred::db::{BatchToStore, PreferredSequencerCache, PreferredSequencerDb};

pub struct ReplicaSequencerDb {
    shutdown_sender: watch::Sender<()>,
}

impl ReplicaSequencerDb {
    pub(crate) fn new(shutdown_sender: watch::Sender<()>) -> Self {
        Self {
            shutdown_sender: shutdown_sender.clone(),
        }
    }
}

#[async_trait]
impl PreferredSequencerDb for ReplicaSequencerDb {
    async fn initial_data(&self) -> Result<(SequenceNumber, PreferredSequencerCache)> {
        Ok((
            0, // TODO this will be revisited when we enable the replica sync task.
            PreferredSequencerCache::new(
                VecDeque::default(),
                Option::None,
                self.shutdown_sender.clone(),
            ),
        ))
    }

    async fn bulk_insert_txs(
        &mut self,
        _txs: Vec<(FullyBakedTx, TxHash)>,
        _sequence_number: SequenceNumber,
        _tx_idx_within_batch: u64,
    ) -> Result<()> {
        Ok(())
    }

    async fn start_batch(
        &mut self,
        _visible_slot_number_after_increase: VisibleSlotNumber,
        _visible_slots_to_advance: NonZero<u8>,
        sequence_number: SequenceNumber,
        _blob_id: BlobInternalId,
    ) -> Result<SequenceNumber> {
        Ok(sequence_number)
    }

    async fn insert_proof_blob(
        &mut self,
        _blob_id: BlobInternalId,
        _data: Arc<[u8]>,
        sequence_number: SequenceNumber,
    ) -> Result<SequenceNumber> {
        Ok(sequence_number)
    }

    async fn terminate_batch(&mut self, _batch: BatchToStore) -> Result<()> {
        Ok(())
    }

    async fn prune_db(&mut self, _prune_up_to_including: SequenceNumber) -> Result<()> {
        Ok(())
    }
}
